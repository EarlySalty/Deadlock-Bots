//! dl-brain-feeder — deterministischer Wochen-Digest-Automat des Second-Brain-Wikis.
//!
//! Rendert zuerst den deterministischen Wochen-Digest und startet danach die
//! davon isolierte LLM-Plan-Phase. Beide Läufe werden getrennt protokolliert.

use std::collections::BTreeSet;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use clap::Parser;
use dl_ai::{FireworksClient, GenerateRequest, TextGenerator};
use dl_brain_feeder::plan::{
    evaluate_response, render_operating_health, render_plan_markdown, EvaluatedPlan,
    OperatingHealth, PlanSources, ServiceErrors, DEFAULT_PLAN_MODEL, PLAN_SYSTEM_PROMPT,
};
use dl_brain_feeder::{
    log_entry, pick_digest_relpath, render_digest, AuditAgg, BrainReportExcerpt, DigestData,
    LedgerAgg, PatchSummary, PulseRow, ReasonAgg, SteamAgg, SurveyRow, TwitchSection,
    TwitchVerdict,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

const DEFAULT_WIKI_ROOT: &str = "/home/naniadm/Documents/Deadlock-2nd-Brain";
const TWITCH_DSN_ENV: &str = "TWITCH_ANALYTICS_DSN";
const TURNIER_DSN_ENV: &str = "DEADLOCK_CENTRAL_DSN";
const PLAN_MODEL_ENV: &str = "DL_BRAIN_PLAN_MODEL";

const PLAN_BASE_PG_TABLES: [&str; 7] = [
    "activity.weekly_pulse",
    "bot.ai_decision_ledger",
    "bot.brain_reports",
    "bot.survey_wave_summary",
    "brain.patch_events",
    "core.discord_audit_log",
    "steam.bot_event_log",
];
const PLAN_TWITCH_PG_TABLES: [&str; 3] = [
    "twitch_crew_radar_log",
    "twitch_scam_guard_verdicts",
    "twitch_stats_tracked",
];
const PLAN_TURNIER_PG_TABLES: [&str; 7] = [
    "turnier.bracket_matches",
    "turnier.draft_sessions",
    "turnier.group_matches",
    "turnier.groups",
    "turnier.tournament_checkins",
    "turnier.tournament_signups",
    "turnier.tournaments",
];
const TOURNAMENT_AGG_QUERY: &str = "WITH scoped_tournaments AS MATERIALIZED (
         SELECT id FROM turnier.tournaments
          WHERE COALESCE(checkin_start, group_phase_start, bracket_start,
                         registration_start, created_at) >= $1
            AND COALESCE(checkin_start, group_phase_start, bracket_start,
                         registration_start, created_at) < $2
     )
     SELECT
         (SELECT COUNT(*) FROM scoped_tournaments),
         (SELECT COUNT(*) FROM turnier.tournament_signups s
           JOIN scoped_tournaments t ON t.id = s.tournament_id),
         (SELECT COUNT(*) FROM turnier.tournament_checkins c
           JOIN scoped_tournaments t ON t.id = c.tournament_id),
         ((SELECT COUNT(*) FROM turnier.bracket_matches m
             JOIN scoped_tournaments t ON t.id = m.tournament_id
            WHERE m.played_at >= $1 AND m.played_at < $2)
          +
          (SELECT COUNT(*) FROM turnier.group_matches m
             JOIN turnier.groups g ON g.id = m.group_id
             JOIN scoped_tournaments t ON t.id = g.tournament_id
            WHERE m.played_at >= $1 AND m.played_at < $2)),
         ((SELECT COUNT(*) FROM turnier.bracket_matches m
             JOIN scoped_tournaments t ON t.id = m.tournament_id
            WHERE m.status NOT IN ('completed', 'forfeit', 'cancelled'))
          +
          (SELECT COUNT(*) FROM turnier.group_matches m
             JOIN turnier.groups g ON g.id = m.group_id
             JOIN scoped_tournaments t ON t.id = g.tournament_id
            WHERE m.status NOT IN ('completed', 'forfeit', 'cancelled'))),
         (SELECT COUNT(*) FROM turnier.draft_sessions d
           JOIN turnier.bracket_matches m ON m.id = d.bracket_match_id
           JOIN scoped_tournaments t ON t.id = m.tournament_id
          WHERE d.status = 'cancelled' AND d.created_at >= $1 AND d.created_at < $2)";

#[derive(Debug, Parser)]
#[command(
    name = "dl-brain-feeder",
    about = "Rendert den deterministischen Wochen-Digest ins Second-Brain-Wiki"
)]
struct Args {}

/// Alles, was ein Lauf in brain.feeder_runs hinterlässt.
struct RunOutcome {
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    digest_path: Option<String>,
    gesehen: i32,
    relevant: i32,
    kategorien: Vec<String>,
    status: &'static str,
    error: Option<String>,
    committed: bool,
    pushed: bool,
}

#[derive(Clone, Debug)]
struct WikiDocument {
    path: String,
    content: String,
}

#[derive(Clone, Debug)]
enum TournamentSection {
    Available(TournamentAgg),
    Unavailable(String),
}

#[derive(Clone, Debug)]
struct TournamentAgg {
    tournaments: i64,
    signups: i64,
    checkins: i64,
    checkin_rate: f64,
    played_matches: i64,
    open_matches: i64,
    cancelled_drafts: i64,
}

#[derive(Clone, Debug)]
struct FeedbackItem {
    titel: String,
    bereich: String,
    kommentar: Option<String>,
}

impl RunOutcome {
    /// `digest_path` ist `Some`, sobald der Digest bereits auf Platte lag, als der
    /// Lauf abbrach — sonst `None` (Abbruch vor dem Schreiben).
    fn aborted(
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        digest_path: Option<String>,
        error: String,
    ) -> Self {
        Self {
            period_start,
            period_end,
            digest_path,
            gesehen: 0,
            relevant: 0,
            kategorien: Vec::new(),
            status: "error",
            error: Some(error),
            committed: false,
            pushed: false,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let _args = Args::parse();

    let dsn = match dl_central_db::dsn_from_env() {
        Ok(dsn) => dsn,
        Err(error) => {
            tracing::error!(%error, "zentrale DB-Konfiguration fehlt");
            return ExitCode::FAILURE;
        }
    };
    let pool = match dl_central_db::connect_pool(&dsn).await {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(%error, "zentrale DB nicht erreichbar");
            return ExitCode::FAILURE;
        }
    };

    let now = Utc::now();
    let period_end = now;
    let period_start = now - TimeDelta::days(7);

    let outcome = match run(&pool, now, period_start, period_end).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!(%error, "Feeder-Lauf abgebrochen");
            // Abbruch vor dem Digest-Schreiben: kein Pfad vorhanden.
            RunOutcome::aborted(period_start, period_end, None, format!("{error:#}"))
        }
    };

    if let Err(error) = record_run(&pool, &outcome).await {
        // Das Lauf-Protokoll ist selbst ausgefallen — laut, nicht still.
        tracing::error!(%error, "brain.feeder_runs konnte nicht geschrieben werden");
    }

    // Zweite, isolierte Phase: erst nach dem Digest-Commit. Ihr Ergebnis darf
    // weder feeder_runs noch den Exit-Status des Digest-Laufs verändern.
    let outcome = if outcome.committed {
        let digest_path = outcome.digest_path.clone();
        isolate_plan_failure(
            outcome,
            run_plan_phase(
                &pool,
                now.date_naive(),
                period_start,
                period_end,
                digest_path.as_deref(),
            ),
        )
        .await
    } else {
        outcome
    };

    match outcome.status {
        "ok" => {
            tracing::info!(
                gesehen = outcome.gesehen,
                relevant = outcome.relevant,
                path = ?outcome.digest_path,
                "Wochen-Digest veröffentlicht"
            );
            ExitCode::SUCCESS
        }
        "partial" => {
            tracing::error!(error = ?outcome.error, "Digest geschrieben, aber nicht vollständig veröffentlicht");
            ExitCode::FAILURE
        }
        _ => ExitCode::FAILURE,
    }
}

async fn run(
    pool: &PgPool,
    now: DateTime<Utc>,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Result<RunOutcome> {
    // ponytail: Lauftag = UTC-Datum. Der Timer feuert 19:00 Europe/Berlin
    // (17:00–18:00 UTC), also nie über Mitternacht — UTC-Datum == Berlin-Datum.
    let run_date = now.date_naive();

    let data = collect_data(pool, run_date, period_start, period_end).await?;
    let rendered = render_digest(&data);

    let wiki = wiki_root();
    git_pull(&wiki).context("wiki git pull --ff-only")?;

    let relpath = pick_digest_relpath(run_date, |rel| wiki.join(rel).exists());
    write_digest(&wiki, &relpath, &rendered.markdown).context("Digest schreiben")?;
    // Ab hier liegt der Digest auf Platte: Folgefehler dürfen den Pfad nicht mehr
    // verlieren, sonst zeigt das Protokoll fälschlich digest_path = NULL.

    let besonderheit = twitch_note(&data.twitch);
    let log_line = log_entry(
        run_date,
        rendered.iso_week,
        rendered.gesehen,
        rendered.relevant,
        &besonderheit,
    );
    if let Err(error) = append_log(&wiki, &log_line).context("log.md ergänzen") {
        return Ok(RunOutcome::aborted(
            period_start,
            period_end,
            Some(relpath),
            format!("{error:#}"),
        ));
    }

    let commit_message = format!(
        "feed: Wochen-Digest KW{} (gesehen {} / relevant {})",
        rendered.iso_week, rendered.gesehen, rendered.relevant
    );

    let (committed, pushed, status, error) = publish(&wiki, &relpath, &commit_message);

    Ok(RunOutcome {
        period_start,
        period_end,
        digest_path: Some(relpath),
        gesehen: rendered.gesehen,
        relevant: rendered.relevant,
        kategorien: rendered.kategorien,
        status,
        error,
        committed,
        pushed,
    })
}

/// Commit + Push der zwei Pfade. Digest liegt bereits auf Platte, daher werden
/// Git-Fehler als 'partial' festgehalten statt den Lauf zu verwerfen.
fn publish(
    wiki: &Path,
    relpath: &str,
    commit_message: &str,
) -> (bool, bool, &'static str, Option<String>) {
    if let Err(error) = git_add_commit(wiki, relpath, commit_message) {
        return (false, false, "partial", Some(format!("{error:#}")));
    }
    match git_push(wiki) {
        Ok(()) => (true, true, "ok", None),
        Err(error) => (true, false, "partial", Some(format!("{error:#}"))),
    }
}

fn twitch_note(twitch: &TwitchSection) -> String {
    match twitch {
        TwitchSection::Unavailable(reason) => {
            format!("Twitch: Quelle nicht verfügbar ({reason}).")
        }
        TwitchSection::Available { crew: Err(_), .. } => {
            "Twitch verfügbar, Crew-Radar-Quelle fehlt.".to_string()
        }
        TwitchSection::Available { .. } => "Twitch verfügbar.".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Datenerhebung
// ---------------------------------------------------------------------------

async fn collect_data(
    pool: &PgPool,
    run_date: NaiveDate,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Result<DigestData> {
    let brain_report = load_brain_report(pool)
        .await
        .context("bot.brain_reports lesen")?;
    let pulse = load_pulse(pool)
        .await
        .context("activity.weekly_pulse lesen")?;
    let ledger = load_ledger(pool, period_start, period_end)
        .await
        .context("bot.ai_decision_ledger aggregieren")?;
    let reasons = load_reasons(pool, period_start, period_end)
        .await
        .context("bot.ai_decision_ledger Gründe lesen")?;
    let audit = load_audit(pool, period_start, period_end)
        .await
        .context("core.discord_audit_log aggregieren")?;
    let steam = load_steam(pool, period_start, period_end)
        .await
        .context("steam.bot_event_log aggregieren")?;
    let patch = load_patch(pool, period_start, period_end)
        .await
        .context("brain.patch_events aggregieren")?;
    let surveys = load_surveys(pool, period_start, period_end)
        .await
        .context("bot.survey_wave_summary lesen")?;
    // Twitch: separate DB, Ausfälle werden im Digest sichtbar gemacht (nie hart).
    let twitch = load_twitch(period_start, period_end).await;

    Ok(DigestData {
        run_date,
        period_start,
        period_end,
        brain_report,
        pulse,
        ledger,
        reasons,
        audit,
        steam,
        patch,
        surveys,
        twitch,
    })
}

async fn load_brain_report(pool: &PgPool) -> Result<Option<BrainReportExcerpt>, sqlx::Error> {
    let row = sqlx::query_as::<_, (DateTime<Utc>, DateTime<Utc>, String)>(
        "SELECT period_start, period_end, report_text
           FROM bot.brain_reports
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(period_start, period_end, report_text)| BrainReportExcerpt {
            period_start,
            period_end,
            report_text,
        },
    ))
}

async fn load_pulse(pool: &PgPool) -> Result<Vec<PulseRow>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64, i64, i64, f64, i64, i64)>(
        "SELECT guild_id, voice_wau, text_wau, new_members,
                voice_minutes::DOUBLE PRECISION AS voice_minutes,
                open_lfg_watches, fired_lfg_watches
           FROM activity.weekly_pulse
          ORDER BY guild_id",
    )
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(
                |(guild_id, voice_wau, text_wau, new_members, voice_minutes, open, fired)| {
                    PulseRow {
                        guild_id,
                        voice_wau,
                        text_wau,
                        new_members,
                        voice_minutes,
                        open_lfg_watches: open,
                        fired_lfg_watches: fired,
                    }
                },
            )
            .collect()
    })
}

async fn load_ledger(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<LedgerAgg>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT source, decision, COUNT(*) AS cnt
           FROM bot.ai_decision_ledger
          WHERE decided_at >= $1 AND decided_at < $2
          GROUP BY source, decision
          ORDER BY source, decision",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(source, decision, count)| LedgerAgg {
                source,
                decision,
                count,
            })
            .collect()
    })
}

async fn load_reasons(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<ReasonAgg>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT source, reason, cnt
           FROM (
             SELECT source, reason, COUNT(*) AS cnt,
                    ROW_NUMBER() OVER (PARTITION BY source ORDER BY COUNT(*) DESC, reason) AS rn
               FROM bot.ai_decision_ledger
              WHERE decided_at >= $1 AND decided_at < $2
              GROUP BY source, reason
           ) ranked
          WHERE rn <= 5
          ORDER BY source, cnt DESC, reason",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(source, reason, count)| ReasonAgg {
                source,
                reason,
                count,
            })
            .collect()
    })
}

async fn load_audit(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<AuditAgg>, sqlx::Error> {
    sqlx::query_as::<_, (i32, i64)>(
        "SELECT action_type, COUNT(*) AS cnt
           FROM core.discord_audit_log
          WHERE occurred_at >= $1 AND occurred_at < $2
          GROUP BY action_type
          ORDER BY action_type",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(action_type, count)| AuditAgg { action_type, count })
            .collect()
    })
}

async fn load_steam(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<SteamAgg>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT event_type, decision, COUNT(*) AS cnt
           FROM steam.bot_event_log
          WHERE occurred_at >= $1 AND occurred_at < $2
          GROUP BY event_type, decision
          ORDER BY event_type, decision",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(event_type, decision, count)| SteamAgg {
                event_type,
                decision,
                count,
            })
            .collect()
    })
}

async fn load_patch(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<PatchSummary, sqlx::Error> {
    let total_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM brain.patch_events
          WHERE posted_at >= $1 AND posted_at < $2",
    )
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await?;

    let patch_titles: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT patch_title FROM brain.patch_events
          WHERE posted_at >= $1 AND posted_at < $2 AND patch_title IS NOT NULL
          ORDER BY patch_title",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;

    let top_entities: Vec<(String, i64)> = sqlx::query_as(
        "SELECT entity_name, COUNT(*) AS cnt FROM brain.patch_events
          WHERE posted_at >= $1 AND posted_at < $2 AND entity_name IS NOT NULL
          GROUP BY entity_name
          ORDER BY cnt DESC, entity_name
          LIMIT 10",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;

    Ok(PatchSummary {
        total_events,
        patch_titles,
        top_entities,
    })
}

async fn load_surveys(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<SurveyRow>, sqlx::Error> {
    sqlx::query_as::<_, (i64, DateTime<Utc>, i64, i64, f64, Option<f64>)>(
        "SELECT wave_id, started_at, invited_count, response_count,
                response_rate::DOUBLE PRECISION AS response_rate,
                average_satisfaction::DOUBLE PRECISION AS average_satisfaction
           FROM bot.survey_wave_summary
          WHERE started_at >= $1 AND started_at < $2
          ORDER BY started_at",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(
                |(wave_id, started_at, invited_count, response_count, response_rate, avg)| {
                    SurveyRow {
                        wave_id,
                        started_at,
                        invited_count,
                        response_count,
                        response_rate,
                        average_satisfaction: avg,
                    }
                },
            )
            .collect()
    })
}

/// Twitch aus der separaten Analytics-PG. Jeder Ausfall (kein DSN, keine
/// Verbindung, kaputte Query) wird zu einem sichtbaren Digest-Abschnitt.
async fn load_twitch(start: DateTime<Utc>, end: DateTime<Utc>) -> TwitchSection {
    let dsn = match std::env::var(TWITCH_DSN_ENV) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return TwitchSection::Unavailable(format!("{TWITCH_DSN_ENV} nicht gesetzt")),
    };

    let pool = match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&dsn)
        .await
    {
        Ok(pool) => pool,
        Err(error) => return TwitchSection::Unavailable(format!("Verbindung: {error}")),
    };

    let stats = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(DISTINCT streamer),
                COALESCE(MAX(viewer_count), 0)::BIGINT
           FROM twitch_stats_tracked
          WHERE ts_utc >= $1 AND ts_utc < $2",
    )
    .bind(start)
    .bind(end)
    .fetch_one(&pool)
    .await;
    let (active_streamers, peak_viewers) = match stats {
        Ok(row) => row,
        Err(error) => return TwitchSection::Unavailable(format!("twitch_stats_tracked: {error}")),
    };

    let scam = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT verdict, COALESCE(category, '-'), COUNT(*) AS cnt
           FROM twitch_scam_guard_verdicts
          WHERE created_at >= $1 AND created_at < $2
          GROUP BY verdict, category
          ORDER BY verdict, category",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&pool)
    .await;
    let scam = match scam {
        Ok(rows) => rows
            .into_iter()
            .map(|(verdict, category, count)| TwitchVerdict {
                verdict,
                category,
                count,
            })
            .collect(),
        Err(error) => {
            return TwitchSection::Unavailable(format!("twitch_scam_guard_verdicts: {error}"))
        }
    };

    // Crew-Radar darf fehlen (Tabelle evtl. nicht in Prod): separat tolerieren.
    let crew = sqlx::query_as::<_, (String, i64)>(
        "SELECT llm_verdict, COUNT(*) AS cnt
           FROM twitch_crew_radar_log
          WHERE created_at >= $1 AND created_at < $2
          GROUP BY llm_verdict
          ORDER BY llm_verdict",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&pool)
    .await
    .map_err(|error| format!("{error}"));

    TwitchSection::Available {
        active_streamers,
        peak_viewers,
        scam,
        crew,
    }
}

// ---------------------------------------------------------------------------
// Plan-Phase
// ---------------------------------------------------------------------------

async fn run_plan_phase(
    pool: &PgPool,
    run_date: NaiveDate,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    digest_path: Option<&str>,
) -> Result<()> {
    let model = std::env::var(PLAN_MODEL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PLAN_MODEL.to_string());
    let wiki = wiki_root();
    let Some(digest_path) = digest_path else {
        record_plan_error(pool, period_start, period_end, &model, "Digest-Pfad fehlt").await?;
        return Ok(());
    };
    let digest = match fs::read_to_string(wiki.join(digest_path)) {
        Ok(digest) => digest,
        Err(error) => {
            record_plan_error(
                pool,
                period_start,
                period_end,
                &model,
                &format!("Digest lesen: {error}"),
            )
            .await?;
            return Ok(());
        }
    };

    let (wiki_documents, mut quellen_fehlend) = collect_wiki_documents(&wiki);
    let health = collect_operating_health(period_start);
    quellen_fehlend.extend(health.quellen_fehlend());
    let tournaments = load_tournaments(period_start, period_end).await;
    if matches!(tournaments, TournamentSection::Unavailable(_)) {
        quellen_fehlend.push("turniere".to_string());
    }
    let feedback = match load_feedback(pool).await {
        Ok(feedback) => feedback,
        Err(error) => {
            tracing::warn!(%error, "Plan-Rückkanal nicht verfügbar");
            quellen_fehlend.push("rueckkanal".to_string());
            Vec::new()
        }
    };
    quellen_fehlend.sort();
    quellen_fehlend.dedup();

    let operating_text = render_operating_health(&health);
    let tournament_text = render_tournaments(&tournaments);
    let prompt = build_plan_prompt(
        period_start,
        period_end,
        &digest,
        &operating_text,
        &tournament_text,
        &wiki_documents,
        &feedback,
        &quellen_fehlend,
    );
    let sources = PlanSources {
        digest_sections: digest
            .lines()
            .filter_map(|line| line.strip_prefix("## ").map(str::to_string))
            .collect(),
        wiki_paths: wiki_documents
            .iter()
            .map(|document| document.path.clone())
            .collect(),
        pg_tables: plan_pg_tables(&digest, &tournaments),
        betrieb_units: health.units(),
    };

    let Some(client) = FireworksClient::from_env(|key| {
        if matches!(key, "FIREWORK_MODEL" | "FIREWORKS_MODEL") {
            Some(model.clone())
        } else {
            std::env::var(key).ok()
        }
    }) else {
        record_plan_failure(
            pool,
            period_start,
            period_end,
            &model,
            &quellen_fehlend,
            "Fireworks-Konfiguration fehlt",
        )
        .await?;
        return Ok(());
    };
    let Some(response) = call_plan_llm(client.as_ref(), &model, prompt).await else {
        record_plan_failure(
            pool,
            period_start,
            period_end,
            &model,
            &quellen_fehlend,
            "Fireworks-Aufruf nach Retry fehlgeschlagen",
        )
        .await?;
        return Ok(());
    };
    let Some(plan) = evaluate_or_record_failure(
        pool,
        period_start,
        period_end,
        &model,
        &quellen_fehlend,
        &response,
        &sources,
    )
    .await?
    else {
        return Ok(());
    };

    persist_and_publish_plan(
        pool,
        &wiki,
        run_date,
        period_start,
        period_end,
        &model,
        &quellen_fehlend,
        &plan,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn evaluate_or_record_failure(
    pool: &PgPool,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    model: &str,
    quellen_fehlend: &[String],
    response: &str,
    sources: &PlanSources,
) -> Result<Option<EvaluatedPlan>> {
    match evaluate_response(response, sources) {
        Ok(plan) => Ok(Some(plan)),
        Err(error) => {
            record_plan_failure(
                pool,
                period_start,
                period_end,
                model,
                quellen_fehlend,
                &format!("LLM-JSON ungültig: {error}"),
            )
            .await?;
            Ok(None)
        }
    }
}

async fn call_plan_llm(
    generator: &dyn TextGenerator,
    model: &str,
    prompt: String,
) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(120), async {
        for attempt in 1..=2 {
            let response = generator
                .generate_text(GenerateRequest {
                    prompt: prompt.clone(),
                    system_prompt: Some(PLAN_SYSTEM_PROMPT.to_string()),
                    model: Some(model.to_string()),
                    max_output_tokens: Some(8_000),
                    reasoning_effort: None,
                    temperature: 0.2,
                })
                .await;
            if response.is_some() {
                return response;
            }
            tracing::warn!(attempt, "Fireworks-Plan-Aufruf fehlgeschlagen");
        }
        None
    })
    .await
    .ok()
    .flatten()
}

fn collect_wiki_documents(wiki: &Path) -> (Vec<WikiDocument>, Vec<String>) {
    let mut documents = Vec::new();
    let mut missing = Vec::new();
    collect_wiki_file(wiki, Path::new("index.md"), &mut documents, &mut missing);
    for directory in ["projekte", "systeme"] {
        let path = wiki.join(directory);
        let mut files = match fs::read_dir(&path) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|entry| entry.extension().is_some_and(|ext| ext == "md"))
                .collect::<Vec<_>>(),
            Err(_) => {
                missing.push(format!("wiki:{directory}"));
                continue;
            }
        };
        files.sort();
        if files.is_empty() {
            missing.push(format!("wiki:{directory}"));
        }
        for file in files {
            let Ok(relative) = file.strip_prefix(wiki) else {
                continue;
            };
            collect_wiki_file(wiki, relative, &mut documents, &mut missing);
        }
    }
    (documents, missing)
}

fn collect_wiki_file(
    wiki: &Path,
    relative: &Path,
    documents: &mut Vec<WikiDocument>,
    missing: &mut Vec<String>,
) {
    let relative_text = relative.to_string_lossy().replace('\\', "/");
    match fs::read_to_string(wiki.join(relative)) {
        Ok(content) => documents.push(WikiDocument {
            path: relative_text,
            content,
        }),
        Err(_) => missing.push(format!("wiki:{relative_text}")),
    }
}

fn collect_operating_health(period_start: DateTime<Utc>) -> OperatingHealth {
    match collect_operating_health_inner(period_start) {
        Ok(health) => health,
        Err(error) => OperatingHealth::Unavailable(format!("{error:#}")),
    }
}

fn collect_operating_health_inner(period_start: DateTime<Utc>) -> Result<OperatingHealth> {
    let failed_output = run_command(
        "systemctl",
        &[
            "--user",
            "list-units",
            "--type=service",
            "--state=failed",
            "--no-legend",
            "--plain",
        ],
    )?;
    let active_output = run_command(
        "systemctl",
        &[
            "--user",
            "list-units",
            "--type=service",
            "--state=active",
            "--no-legend",
            "--plain",
        ],
    )?;
    let failed_units = unit_names(&failed_output);
    let active_units = unit_names(&active_output)
        .into_iter()
        .filter(|unit| {
            unit.starts_with("deadlock") || unit.starts_with("dl-") || unit.starts_with("tb-")
        })
        .collect::<Vec<_>>();
    let mut all_units: BTreeSet<String> = failed_units.iter().cloned().collect();
    all_units.extend(active_units.iter().cloned());
    let mut error_units = Vec::new();
    let mut ok_units = 0;
    let since = period_start.to_rfc3339();
    for unit in &active_units {
        let journal = run_command(
            "journalctl",
            &[
                "--user",
                "-u",
                unit,
                "--since",
                &since,
                "--priority=err",
                "--no-pager",
                "-q",
            ],
        )?;
        let errors = journal
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count() as u64;
        if errors == 0 {
            ok_units += 1;
        } else {
            error_units.push(ServiceErrors {
                unit: unit.clone(),
                errors,
            });
        }
    }
    Ok(OperatingHealth::Available {
        failed_units,
        checked_units: active_units.len(),
        ok_units,
        error_units,
        all_units,
    })
}

fn run_command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("{program} starten"))?;
    if !output.status.success() {
        bail!("{program} endete mit {}", output.status);
    }
    String::from_utf8(output.stdout).with_context(|| format!("{program}-Ausgabe dekodieren"))
}

fn unit_names(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

async fn load_tournaments(start: DateTime<Utc>, end: DateTime<Utc>) -> TournamentSection {
    let dsn = match std::env::var(TURNIER_DSN_ENV) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return TournamentSection::Unavailable(format!("{TURNIER_DSN_ENV} nicht gesetzt")),
    };
    let pool = match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&dsn)
        .await
    {
        Ok(pool) => pool,
        Err(_) => return TournamentSection::Unavailable("Verbindung fehlgeschlagen".to_string()),
    };

    let counts = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(TOURNAMENT_AGG_QUERY)
        .bind(start)
        .bind(end)
        .fetch_one(&pool)
        .await;
    let (tournaments, signups, checkins, played_matches, open_matches, cancelled_drafts) =
        match counts {
            Ok(counts) => counts,
            Err(error) => {
                return TournamentSection::Unavailable(format!("Aggregate fehlgeschlagen: {error}"))
            }
        };
    let checkin_rate = if signups > 0 {
        checkins as f64 / signups as f64
    } else {
        0.0
    };
    TournamentSection::Available(TournamentAgg {
        tournaments,
        signups,
        checkins,
        checkin_rate,
        played_matches,
        open_matches,
        cancelled_drafts,
    })
}

fn render_tournaments(section: &TournamentSection) -> String {
    match section {
        TournamentSection::Unavailable(reason) => {
            format!("Quelle nicht verfügbar ({reason}).")
        }
        TournamentSection::Available(data) => format!(
            "Turniere im Zeitraum: {}\nAnmeldungen: {}\nCheck-ins: {} (Quote {:.1} %)\nGespielte Matches: {}\nOffene Matches: {}\nAbgebrochene Drafts: {}\nQuellen: turnier.tournaments, turnier.tournament_signups, turnier.tournament_checkins, turnier.bracket_matches, turnier.groups, turnier.group_matches, turnier.draft_sessions.",
            data.tournaments,
            data.signups,
            data.checkins,
            data.checkin_rate * 100.0,
            data.played_matches,
            data.open_matches,
            data.cancelled_drafts,
        ),
    }
}

fn plan_pg_tables(digest: &str, tournaments: &TournamentSection) -> BTreeSet<String> {
    let mut tables = PLAN_BASE_PG_TABLES
        .iter()
        .map(|table| (*table).to_string())
        .collect::<BTreeSet<_>>();

    let twitch = digest
        .split("## Twitch\n")
        .nth(1)
        .and_then(|remainder| remainder.split("\n## ").next());
    if twitch.is_some_and(|section| !section.contains("- Quelle nicht verfügbar")) {
        tables.extend(
            PLAN_TWITCH_PG_TABLES[1..]
                .iter()
                .map(|table| (*table).to_string()),
        );
        if twitch.is_some_and(|section| !section.contains("Crew-Radar: Quelle nicht verfügbar")) {
            tables.insert(PLAN_TWITCH_PG_TABLES[0].to_string());
        }
    }

    if matches!(tournaments, TournamentSection::Available(_)) {
        tables.extend(
            PLAN_TURNIER_PG_TABLES
                .iter()
                .map(|table| (*table).to_string()),
        );
    }
    tables
}

async fn load_feedback(pool: &PgPool) -> Result<Vec<FeedbackItem>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT titel, bereich, kommentar
           FROM brain.plan_items
          WHERE status IN ('abgelehnt', 'erledigt')
          ORDER BY entschieden_am DESC NULLS LAST, created_at DESC
          LIMIT 20",
    )
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(titel, bereich, kommentar)| FeedbackItem {
                titel,
                bereich,
                kommentar,
            })
            .collect()
    })
}

#[allow(clippy::too_many_arguments)]
fn build_plan_prompt(
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    digest: &str,
    operating: &str,
    tournaments: &str,
    wiki_documents: &[WikiDocument],
    feedback: &[FeedbackItem],
    missing: &[String],
) -> String {
    let wiki = if wiki_documents.is_empty() {
        "(keine Wiki-Dateien verfügbar)".to_string()
    } else {
        wiki_documents
            .iter()
            .map(|document| format!("### {}\n\n{}", document.path, document.content))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let feedback = if feedback.is_empty() {
        "(noch nichts)".to_string()
    } else {
        feedback
            .iter()
            .map(|item| {
                format!(
                    "- [{}] {} — Kommentar: {}",
                    item.bereich,
                    item.titel,
                    item.kommentar.as_deref().unwrap_or("(kein Kommentar)")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let missing = if missing.is_empty() {
        String::new()
    } else {
        format!("\n\nFehlende Quellen: {}", missing.join(", "))
    };
    format!(
        "## Zeitraum\n\n{} bis {}\n\n## Gemessene Lage (Digest)\n\n{}\n\n## Betriebszustand\n\n{}\n\n## Turniere\n\n{}\n\n## Interne Lage (Wiki — behauptet, evtl. veraltet)\n\n{}{}\n\n## Schon abgeräumt (nicht wiederholen)\n\n{}",
        period_start.to_rfc3339(),
        period_end.to_rfc3339(),
        digest,
        operating,
        tournaments,
        wiki,
        missing,
        feedback,
    )
}

#[allow(clippy::too_many_arguments)]
async fn persist_and_publish_plan(
    pool: &PgPool,
    wiki: &Path,
    run_date: NaiveDate,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    model: &str,
    quellen_fehlend: &[String],
    plan: &EvaluatedPlan,
) -> Result<()> {
    let relpath = format!("plaene/{}-vorschlaege.md", run_date.format("%Y-%m-%d"));
    let run_id = insert_plan_run_and_items(
        pool,
        period_start,
        period_end,
        &relpath,
        model,
        quellen_fehlend,
        plan,
    )
    .await?;
    let markdown = render_plan_markdown(run_date, model, plan);
    if let Err(error) = write_plan(wiki, &relpath, &markdown) {
        update_plan_run(
            pool,
            run_id,
            "error",
            false,
            false,
            Some(&format!("{error:#}")),
        )
        .await?;
        return Ok(());
    }
    let log = format!(
        "## [{}] plan — PLATZHALTER: Handlungsvorschläge\n\nvorgeschlagen {} / übernommen {} / verworfen {}.\n",
        run_date.format("%Y-%m-%d"),
        plan.gate.vorgeschlagen,
        plan.gate.uebernommen,
        plan.gate.verworfen,
    );
    if let Err(error) = append_log(wiki, &log) {
        update_plan_run(
            pool,
            run_id,
            "error",
            false,
            false,
            Some(&format!("{error:#}")),
        )
        .await?;
        return Ok(());
    }
    let commit_message = format!(
        "plan: Vorschläge {} (übernommen {} / verworfen {})",
        run_date.format("%Y-%m-%d"),
        plan.gate.uebernommen,
        plan.gate.verworfen,
    );
    let (committed, pushed, status, error) = publish(wiki, &relpath, &commit_message);
    update_plan_run(pool, run_id, status, committed, pushed, error.as_deref()).await?;
    Ok(())
}

async fn insert_plan_run_and_items(
    pool: &PgPool,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    relpath: &str,
    model: &str,
    quellen_fehlend: &[String],
    plan: &EvaluatedPlan,
) -> Result<i64> {
    let reasons = serde_json::to_value(&plan.gate.verworfen_gruende)?;
    let mut transaction = pool.begin().await?;
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO brain.plan_runs(
             period_start, period_end, plan_path, modell, lage,
             vorgeschlagen, uebernommen, verworfen, verworfen_gruende,
             quellen_fehlend, status
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'partial')
         RETURNING id",
    )
    .bind(period_start)
    .bind(period_end)
    .bind(relpath)
    .bind(model)
    .bind(&plan.lage)
    .bind(plan.gate.vorgeschlagen)
    .bind(plan.gate.uebernommen)
    .bind(plan.gate.verworfen)
    .bind(reasons)
    .bind(quellen_fehlend)
    .fetch_one(&mut *transaction)
    .await?;
    for item in &plan.gate.items {
        sqlx::query(
            "INSERT INTO brain.plan_items(
                 run_id, prioritaet, bereich, titel, begruendung, aktion,
                 beleg, beleg_art, belegt_gemessen, fingerprint
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(run_id)
        .bind(item.prioritaet)
        .bind(&item.bereich)
        .bind(&item.titel)
        .bind(&item.begruendung)
        .bind(&item.aktion)
        .bind(&item.beleg)
        .bind(&item.beleg_art)
        .bind(item.belegt_gemessen)
        .bind(&item.fingerprint)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(run_id)
}

fn write_plan(wiki: &Path, relpath: &str, markdown: &str) -> Result<()> {
    let target = wiki.join(relpath);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Verzeichnis anlegen: {}", parent.display()))?;
    }
    fs::write(&target, markdown).with_context(|| format!("schreiben: {}", target.display()))
}

// ---------------------------------------------------------------------------
// Wiki-/Git-Seiteneffekte
// ---------------------------------------------------------------------------

fn wiki_root() -> PathBuf {
    std::env::var("DL_BRAIN_WIKI_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_WIKI_ROOT))
}

fn write_digest(wiki: &Path, relpath: &str, markdown: &str) -> Result<()> {
    let target = wiki.join(relpath);
    if target.exists() {
        // Immutable-Regel: pick_digest_relpath sollte das verhindern; falls doch,
        // hart abbrechen statt eine bestehende Quelle zu überschreiben.
        bail!("Digest-Datei existiert bereits: {}", target.display());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Verzeichnis anlegen: {}", parent.display()))?;
    }
    std::fs::write(&target, markdown)
        .with_context(|| format!("schreiben: {}", target.display()))?;
    Ok(())
}

fn append_log(wiki: &Path, entry: &str) -> Result<()> {
    use std::io::Write;
    let log_path = wiki.join("log.md");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("log.md öffnen: {}", log_path.display()))?;
    // Leerzeile als Separator vor dem neuen Eintrag.
    write!(file, "\n{entry}").context("log.md schreiben")?;
    Ok(())
}

fn git_pull(wiki: &Path) -> Result<()> {
    run_git(wiki, &["pull", "--ff-only"])
}

fn git_add_commit(wiki: &Path, relpath: &str, message: &str) -> Result<()> {
    run_git(wiki, &["add", "--", relpath, "log.md"])?;
    run_git(wiki, &["commit", "-m", message])
}

fn git_push(wiki: &Path) -> Result<()> {
    run_git(wiki, &["push", "origin", "main"])
}

fn run_git(wiki: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(wiki)
        .args(args)
        .output()
        .with_context(|| format!("git {} starten", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "git {} fehlgeschlagen: {}",
            args.join(" "),
            stderr.trim()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Lauf-Protokoll
// ---------------------------------------------------------------------------

async fn record_run(pool: &PgPool, outcome: &RunOutcome) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO brain.feeder_runs(
             period_start, period_end, digest_path, gesehen, relevant,
             kategorien, status, error, committed, pushed
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(outcome.period_start)
    .bind(outcome.period_end)
    .bind(outcome.digest_path.as_deref())
    .bind(outcome.gesehen)
    .bind(outcome.relevant)
    .bind(&outcome.kategorien)
    .bind(outcome.status)
    .bind(outcome.error.as_deref())
    .bind(outcome.committed)
    .bind(outcome.pushed)
    .execute(pool)
    .await?;
    Ok(())
}

/// Die Plan-Phase darf den bereits veröffentlichten Digest-Zustand nicht verändern.
async fn isolate_plan_failure<F>(digest: RunOutcome, plan_phase: F) -> RunOutcome
where
    F: Future<Output = Result<()>>,
{
    if let Err(error) = plan_phase.await {
        tracing::error!(%error, "Plan-Phase fehlgeschlagen; Digest bleibt unverändert");
    }
    digest
}

async fn record_plan_error(
    pool: &PgPool,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    model: &str,
    error: &str,
) -> Result<(), sqlx::Error> {
    record_plan_failure(pool, period_start, period_end, model, &[], error).await
}

async fn record_plan_failure(
    pool: &PgPool,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    model: &str,
    quellen_fehlend: &[String],
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO brain.plan_runs(
             period_start, period_end, modell, quellen_fehlend, status, error
         ) VALUES ($1, $2, $3, $4, 'error', $5)",
    )
    .bind(period_start)
    .bind(period_end)
    .bind(model)
    .bind(quellen_fehlend)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_plan_run(
    pool: &PgPool,
    run_id: i64,
    status: &str,
    committed: bool,
    pushed: bool,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE brain.plan_runs
            SET status = $2, committed = $3, pushed = $4, error = $5
          WHERE id = $1",
    )
    .bind(run_id)
    .bind(status)
    .bind(committed)
    .bind(pushed)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod plan_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn digest_bleibt_gruen_wenn_plan_phase_stirbt() {
        let digest = RunOutcome {
            period_start: Utc::now() - TimeDelta::days(7),
            period_end: Utc::now(),
            digest_path: Some("raw/2026-07/test.md".to_string()),
            gesehen: 5,
            relevant: 2,
            kategorien: vec!["Community-Puls".to_string()],
            status: "ok",
            error: None,
            committed: true,
            pushed: true,
        };

        let digest = isolate_plan_failure(digest, async { bail!("LLM kaputt") }).await;

        assert_eq!(digest.status, "ok");
        assert!(digest.committed);
        assert!(digest.pushed);
    }

    #[test]
    fn turnier_abschnitt_nennt_seine_aufloesbaren_pg_quellen() {
        let rendered = render_tournaments(&TournamentSection::Available(TournamentAgg {
            tournaments: 1,
            signups: 10,
            checkins: 8,
            checkin_rate: 0.8,
            played_matches: 3,
            open_matches: 2,
            cancelled_drafts: 1,
        }));

        for table in [
            "turnier.tournaments",
            "turnier.tournament_signups",
            "turnier.tournament_checkins",
            "turnier.bracket_matches",
            "turnier.groups",
            "turnier.group_matches",
            "turnier.draft_sessions",
        ] {
            assert!(rendered.contains(table), "PG-Quelle fehlt: {table}");
        }
    }

    #[test]
    fn turnier_aggregat_zaehlt_alle_nicht_terminalen_matches_als_offen() {
        assert_eq!(
            TOURNAMENT_AGG_QUERY
                .matches("status NOT IN ('completed', 'forfeit', 'cancelled')")
                .count(),
            2
        );
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN"]
    async fn turnier_aggregat_passt_zur_migrierten_pg_struktur() -> Result<()> {
        let db = dl_central_db::testing::test_pool().await?;
        let start = Utc::now() - TimeDelta::days(7);
        let end = Utc::now();

        let counts = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(TOURNAMENT_AGG_QUERY)
            .bind(start)
            .bind(end)
            .fetch_one(db.pool())
            .await?;

        assert_eq!(counts, (0, 0, 0, 0, 0, 0));
        Ok(())
    }

    #[test]
    fn pg_gate_akzeptiert_nur_im_prompt_genannte_tabellen() {
        let digest_sources = "activity.weekly_pulse bot.ai_decision_ledger bot.brain_reports bot.survey_wave_summary brain.patch_events core.discord_audit_log steam.bot_event_log twitch_crew_radar_log twitch_scam_guard_verdicts twitch_stats_tracked";
        let tournament_sources = render_tournaments(&TournamentSection::Available(TournamentAgg {
            tournaments: 0,
            signups: 0,
            checkins: 0,
            checkin_rate: 0.0,
            played_matches: 0,
            open_matches: 0,
            cancelled_drafts: 0,
        }));
        let collected = format!("{digest_sources} {tournament_sources}");

        for table in plan_pg_tables(
            digest_sources,
            &TournamentSection::Available(TournamentAgg {
                tournaments: 0,
                signups: 0,
                checkins: 0,
                checkin_rate: 0.0,
                played_matches: 0,
                open_matches: 0,
                cancelled_drafts: 0,
            }),
        ) {
            assert!(
                collected.contains(&table),
                "nicht erhobene PG-Quelle: {table}"
            );
        }
    }

    #[test]
    fn pg_gate_entfernt_ausgefallene_quellen() {
        let digest = "## Community-Puls\n\n## Twitch\n\n- Quelle nicht verfügbar (DSN fehlt)";
        let tables = plan_pg_tables(
            digest,
            &TournamentSection::Unavailable("DSN fehlt".to_string()),
        );

        assert!(tables.contains("activity.weekly_pulse"));
        assert!(!tables.iter().any(|table| table.starts_with("twitch_")));
        assert!(!tables.iter().any(|table| table.starts_with("turnier.")));
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN"]
    async fn kaputtes_llm_json_hinterlaesst_error_plan_run() -> Result<()> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let start = Utc::now() - TimeDelta::days(7);
        let end = Utc::now();

        let plan = evaluate_or_record_failure(
            pool,
            start,
            end,
            "deepseek-v4-pro",
            &[],
            "{kaputt",
            &PlanSources::default(),
        )
        .await?;

        assert!(plan.is_none());
        let row: (String, i64) =
            sqlx::query_as("SELECT status, COUNT(*) FROM brain.plan_runs GROUP BY status")
                .fetch_one(pool)
                .await?;
        assert_eq!(row, ("error".to_string(), 1));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN"]
    async fn persistierter_plan_bleibt_bis_publish_sichtbar_partial() -> Result<()> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let start = Utc::now() - TimeDelta::days(7);
        let end = Utc::now();
        let plan = evaluate_response(r#"{"lage":"Ruhig.","items":[]}"#, &PlanSources::default())?;

        let run_id = insert_plan_run_and_items(
            pool,
            start,
            end,
            "plaene/test.md",
            "deepseek-v4-pro",
            &[],
            &plan,
        )
        .await?;
        let status: String = sqlx::query_scalar("SELECT status FROM brain.plan_runs WHERE id=$1")
            .bind(run_id)
            .fetch_one(pool)
            .await?;

        assert_eq!(status, "partial");
        Ok(())
    }
}
