//! dl-brain-feeder — deterministischer Wochen-Digest-Automat des Second-Brain-Wikis.
//!
//! Kein LLM: liest read-only Aggregate aus der zentralen PG (und optional der
//! Twitch-Analytics-PG), rendert einen Markdown-Digest (reine Funktion in lib.rs),
//! schreibt ihn immutable ins Wiki-Repo, pflegt log.md und protokolliert jeden
//! Lauf in brain.feeder_runs — auch im Fehlerfall, sofern die zentrale DB
//! erreichbar ist; frühe Verbindungsfehler stehen nur im Journal.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use clap::Parser;
use dl_brain_feeder::{
    log_entry, pick_digest_relpath, render_digest, AuditAgg, BrainReportExcerpt, DigestData,
    LedgerAgg, PatchSummary, PulseRow, ReasonAgg, SteamAgg, SurveyRow, TwitchSection,
    TwitchVerdict,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

const DEFAULT_WIKI_ROOT: &str = "/home/naniadm/Documents/Deadlock-2nd-Brain";
const TWITCH_DSN_ENV: &str = "TWITCH_ANALYTICS_DSN";

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

    let (committed, pushed, status, error) =
        publish(&wiki, &relpath, &commit_message);

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
    let pulse = load_pulse(pool).await.context("activity.weekly_pulse lesen")?;
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
    Ok(row.map(|(period_start, period_end, report_text)| BrainReportExcerpt {
        period_start,
        period_end,
        report_text,
    }))
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
