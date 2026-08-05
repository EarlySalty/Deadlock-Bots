//! dl-verbinder — Shadow-Agent R1 (Verbinder) des Discord-Agenten-Teams.
//!
//! Läuft per systemd-Timer. Wirkung: ausschließlich bot.ai_decision_ledger
//! plus Staff-Posts. Es wird nie ein Community-Mitglied kontaktiert.
//! Prompts und Nonce kommen zur Laufzeit aus der Personalakte (akte.md).

use std::collections::{BTreeMap, HashSet};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use dl_ai::{ChatMessage, ChatParams, ChatProvider, ChatProviderError, LlmProviderConfig, LlmUseCase};
use dl_brain_community::{anonymize_for_privacy, Decision, LedgerEntry};
use dl_verbinder::{
    agreement_je_kategorie, chunk_message, ledger_entry_fuer_kritik, ledger_entry_fuer_match_fehler,
    ledger_entry_fuer_match_urteil, parse_kritik_antwort, parse_match_antwort, render_prompt,
    render_staff_post, render_tages_summary, render_wochenbericht, resolve_outcomes,
    text_verstoesse, waechter_filter, Akte, Caps, Kandidat, Kategorie, WochenberichtInput,
    SOURCE_KRITIK, SOURCE_MATCH,
};
use serde_json::json;
use sqlx::{PgPool, Postgres, Row, Transaction};

const DEFAULT_AKTE_PFAD: &str =
    "/home/naniadm/Documents/Deadlock-Bots/rust/bin/dl-verbinder/akte.md";
const DEFAULT_GUILD_ID: i64 = 1289721245281292288;
const DEFAULT_ROUTER_VC_ID: i64 = 1513468587195633674;
const LLM_TIMEOUT: Duration = Duration::from_secs(45);
/// Discord nimmt hart 2000 Zeichen pro Nachricht; 100 Zeichen Puffer, damit
/// ein angehängter Hinweis den Bericht nicht doch noch über die Kante kippt.
const DISCORD_CONTENT_LIMIT: usize = 1900;

#[derive(Debug, Parser)]
#[command(name = "dl-verbinder", about = "Verbinder-Agent R1, fester Shadow-Modus")]
struct Args {
    #[command(subcommand)]
    befehl: Option<Befehl>,
}

#[derive(Debug, Subcommand)]
enum Befehl {
    /// Regellauf: Grundwahrheit auflösen, Kandidaten finden, bewerten, ledgern.
    Lauf,
    /// Tageslauf: wie Lauf, zusätzlich T2-Zwillinge und Tages-Summary-Post.
    Summary,
    /// Wochenauswertung: Agreement je Kategorie plus 5-Fälle-Stichprobe.
    Auswertung,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let ergebnis = match args.befehl.unwrap_or(Befehl::Lauf) {
        Befehl::Lauf => run(false).await,
        Befehl::Summary => run(true).await,
        Befehl::Auswertung => auswertung().await,
    };
    match ergebnis {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "dl-verbinder fehlgeschlagen");
            ExitCode::FAILURE
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_i64(name: &str, default: i64) -> i64 {
    env(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn caps_from_env() -> Caps {
    let default = Caps::default();
    Caps {
        max_llm_per_run: env_i64(
            "DL_VERBINDER_MAX_LLM_CALLS_PER_RUN",
            default.max_llm_per_run as i64,
        )
        .max(0) as usize,
        max_llm_per_day: env_i64("DL_VERBINDER_MAX_LLM_CALLS_PER_DAY", default.max_llm_per_day),
        pair_cooldown_days: env_i64("DL_VERBINDER_PAIR_COOLDOWN_DAYS", default.pair_cooldown_days),
    }
}

fn lade_akte() -> Result<Akte> {
    let pfad = env("DL_VERBINDER_AKTE").unwrap_or_else(|| DEFAULT_AKTE_PFAD.to_string());
    let raw = std::fs::read_to_string(&pfad)
        .with_context(|| format!("Personalakte nicht lesbar: {pfad}"))?;
    Akte::parse(&raw).map_err(|fehler| anyhow::anyhow!("Personalakte ungültig ({pfad}): {fehler}"))
}

struct LlmSeite {
    provider: Arc<dyn ChatProvider>,
    model_override: Option<String>,
}

impl LlmSeite {
    async fn frage(&self, prompt: &str) -> Result<(String, Option<String>), ChatProviderError> {
        let params = ChatParams {
            model: self.model_override.clone(),
            json_mode: true,
            // None = Modell-Maximum: DeepSeek-Reasoning zählt ins Budget,
            // Some(700) schnitt 142 von 149 Antworten ab (truncated).
            max_tokens: None,
            ..ChatParams::default()
        };
        let antwort = tokio::time::timeout(
            LLM_TIMEOUT,
            self.provider.chat(&[ChatMessage::user(prompt)], params),
        )
        .await
        .map_err(|_| ChatProviderError::Timeout)??;
        Ok((antwort.content, antwort.model))
    }
}

async fn run(taeglich: bool) -> Result<()> {
    let akte = lade_akte()?;
    let caps = caps_from_env();
    let guild_id = env_i64("OUR_GUILD_ID", env_i64("MAIN_GUILD_ID", DEFAULT_GUILD_ID));
    let router_vc = env_i64("DL_VERBINDER_ROUTER_VC_ID", DEFAULT_ROUTER_VC_ID);

    let dsn = dl_central_db::dsn_from_env().context("zentrale DB-Konfiguration")?;
    let pool = dl_central_db::connect_pool(&dsn)
        .await
        .context("zentrale DB verbinden")?;

    // Schritt 0: Grundwahrheit nachziehen, bevor neue Urteile fallen.
    let aufgeloest = resolve_outcomes(&pool, guild_id).await?;
    if aufgeloest > 0 {
        tracing::info!(aufgeloest, "Grundwahrheit aufgelöst");
    }

    // Schritt 1: Kandidaten deterministisch sammeln. Ein Lader-Fehler wird
    // zum error-Ledger-Eintrag, nie zum stillen Weiterlaufen ohne Kategorie.
    let mut kandidaten = Vec::new();
    let mut lade_fehler = Vec::new();
    match lade_t1(&pool, guild_id, router_vc).await {
        Ok(mut liste) => kandidaten.append(&mut liste),
        Err(error) => lade_fehler.push(lader_fehler_entry("lade_t1", &error)),
    }
    match lade_t3(&pool, guild_id).await {
        Ok(mut liste) => kandidaten.append(&mut liste),
        Err(error) => lade_fehler.push(lader_fehler_entry("lade_t3", &error)),
    }
    match lade_t4(&pool, guild_id).await {
        Ok(mut liste) => kandidaten.append(&mut liste),
        Err(error) => lade_fehler.push(lader_fehler_entry("lade_t4", &error)),
    }
    if taeglich {
        match lade_t2(&pool).await {
            Ok(mut liste) => kandidaten.append(&mut liste),
            Err(error) => lade_fehler.push(lader_fehler_entry("lade_t2", &error)),
        }
    }
    let kandidaten_gesamt = kandidaten.len();

    // Schritt 2: Wächter (Kosten, Zirkel, Opt-out) vor jedem Modellaufruf.
    let recent_pairs = lade_recent_pairs(&pool, caps.pair_cooldown_days).await?;
    let llm_calls_today = zaehle_llm_calls_heute(&pool).await?;
    let (zu_bewerten, mut entries) = waechter_filter(kandidaten, &recent_pairs, llm_calls_today, &caps);
    entries.extend(lade_fehler);

    // Schritt 3: LLM-Bewertung (Ersteller) plus Kritiker auf jedes yes.
    let mut kritiken: Vec<(usize, LedgerEntry, bool, String)> = Vec::new();
    if !zu_bewerten.is_empty() {
        let cfg = LlmProviderConfig::from_env(|k| std::env::var(k).ok())
            .context("LLM-Provider-Konfiguration")?;
        let ersteller = LlmSeite {
            provider: cfg
                .build_provider_for_env(LlmUseCase::VerbinderMatch, |k| std::env::var(k).ok())
                .context("Ersteller-Provider")?,
            model_override: env("DL_VERBINDER_MODEL"),
        };
        let kritiker = LlmSeite {
            provider: cfg
                .build_provider_for_env(LlmUseCase::VerbinderKritik, |k| std::env::var(k).ok())
                .context("Kritiker-Provider")?,
            model_override: env("DL_VERBINDER_KRITIK_MODEL"),
        };

        for kandidat in &zu_bewerten {
            let prompt = render_prompt(&akte.prompt_match, "kandidat", &kandidat.daten.to_string());
            let (entry, benutztes_modell, urteil) = match ersteller.frage(&prompt).await {
                Ok((antwort, modell)) => match parse_match_antwort(&antwort) {
                    Ok(urteil) => {
                        let verstoesse = text_verstoesse(&urteil.vorschlagstext);
                        let entry = ledger_entry_fuer_match_urteil(kandidat, &urteil, &verstoesse);
                        (entry, modell, Some(urteil))
                    }
                    Err(parse_fehler) => (
                        ledger_entry_fuer_match_fehler(
                            kandidat,
                            Decision::Error,
                            format!("antwort_unlesbar:{parse_fehler}"),
                        ),
                        modell,
                        None,
                    ),
                },
                Err(ChatProviderError::Timeout) => (
                    ledger_entry_fuer_match_fehler(
                        kandidat,
                        Decision::Timeout,
                        "llm_timeout".into(),
                    ),
                    None,
                    None,
                ),
                Err(fehler) => (
                    ledger_entry_fuer_match_fehler(
                        kandidat,
                        Decision::Error,
                        format!("llm_fehler:{fehler}"),
                    ),
                    None,
                    None,
                ),
            };
            let index = entries.len();
            entries.push(entry);

            // Kritiker: nur auf yes, Ersteller ungleich Prüfer.
            if let Some(urteil) = urteil {
                if urteil.decision == Decision::Yes {
                    let vorschlag = json!({
                        "trigger": kandidat.kategorie.as_str(),
                        "daten": kandidat.daten,
                        "vorschlagstext": urteil.vorschlagstext,
                        "kanal": urteil.kanal,
                        "begruendung": urteil.begruendung,
                    });
                    let prompt =
                        render_prompt(&akte.prompt_kritik, "vorschlag", &vorschlag.to_string());
                    let (kritik_entry, kritik_ok, kritik_begruendung) =
                        match kritiker.frage(&prompt).await {
                            Ok((antwort, kritik_modell)) => match parse_kritik_antwort(&antwort) {
                                Ok(kritik) => {
                                    let same_model = match (&benutztes_modell, &kritik_modell) {
                                        (Some(a), Some(b)) => a == b,
                                        _ => {
                                            ersteller.model_override == kritiker.model_override
                                        }
                                    };
                                    let decision = if kritik.ok {
                                        Decision::Yes
                                    } else {
                                        Decision::No
                                    };
                                    let begruendung = kritik.begruendung.clone();
                                    (
                                        ledger_entry_fuer_kritik(
                                            kandidat,
                                            0,
                                            decision,
                                            if kritik.ok { "ok" } else { "beanstandet" }.into(),
                                            kritik.achsen,
                                            &kritik.begruendung,
                                            same_model,
                                        ),
                                        kritik.ok,
                                        begruendung,
                                    )
                                }
                                Err(parse_fehler) => (
                                    ledger_entry_fuer_kritik(
                                        kandidat,
                                        0,
                                        Decision::Error,
                                        format!("antwort_unlesbar:{parse_fehler}"),
                                        json!({}),
                                        "",
                                        false,
                                    ),
                                    false,
                                    "Kritik unlesbar".into(),
                                ),
                            },
                            Err(ChatProviderError::Timeout) => (
                                ledger_entry_fuer_kritik(
                                    kandidat,
                                    0,
                                    Decision::Timeout,
                                    "llm_timeout".into(),
                                    json!({}),
                                    "",
                                    false,
                                ),
                                false,
                                "Kritik-Timeout".into(),
                            ),
                            Err(fehler) => (
                                ledger_entry_fuer_kritik(
                                    kandidat,
                                    0,
                                    Decision::Error,
                                    format!("llm_fehler:{fehler}"),
                                    json!({}),
                                    "",
                                    false,
                                ),
                                false,
                                "Kritik-Fehler".into(),
                            ),
                        };
                    kritiken.push((index, kritik_entry, kritik_ok, kritik_begruendung));
                }
            }
        }
    }

    // Schritt 4: alles in einer Transaktion ledgern; Kritik referenziert die
    // vergebene Ledger-ID ihres Match-Eintrags.
    let ledger_ids = persist_entries(&pool, &entries, &kritiken).await?;

    // Schritt 5: Staff-Post (nur bei yes oder Fehlern), Pflichtzeile immer dabei.
    let kritik_refs: Vec<(i64, bool, String)> = kritiken
        .iter()
        .map(|(index, _, ok, begruendung)| (ledger_ids[*index], *ok, begruendung.clone()))
        .collect();
    let mut entries_mit_id = entries.clone();
    for (index, id) in ledger_ids.iter().enumerate() {
        if let Some(objekt) = entries_mit_id[index].payload.as_object_mut() {
            objekt.insert("ledger_id".into(), json!(id));
        }
    }
    if let Some(post) = render_staff_post(&akte.nonce, kandidaten_gesamt, &entries_mit_id, &kritik_refs)
    {
        poste_staff(&post).await;
    }

    if taeglich {
        let zaehler = zaehle_klassen_heute(&pool).await?;
        let llm_heute = zaehle_llm_calls_heute(&pool).await?;
        poste_staff(&render_tages_summary(&akte.nonce, &zaehler, llm_heute, &caps)).await;
    }

    tracing::info!(
        kandidaten = kandidaten_gesamt,
        bewertet = zu_bewerten.len(),
        eintraege = entries.len(),
        "Verbinder-Lauf abgeschlossen"
    );
    Ok(())
}

fn lader_fehler_entry(schritt: &str, error: &sqlx::Error) -> LedgerEntry {
    LedgerEntry {
        source: SOURCE_MATCH,
        subject_user_id: None,
        guild_id: None,
        input_summary: format!("load={schritt}"),
        decision: Decision::Error,
        confidence: None,
        reason: format!("{schritt}:{error}"),
        action_taken: "shadow",
        payload: json!({}),
    }
}

const OPT_OUT_A_UND_B: &str = "(
       EXISTS (SELECT 1 FROM core.user_privacy up
                WHERE up.user_id = a.user_id
                  AND (COALESCE(up.opted_out, FALSE) OR up.deleted_at IS NOT NULL))
    OR EXISTS (SELECT 1 FROM activity.user_retention_tracking rt
                WHERE rt.user_id = a.user_id AND COALESCE(rt.opted_out, FALSE))
    OR EXISTS (SELECT 1 FROM core.user_privacy up
                WHERE up.user_id = b.user_id
                  AND (COALESCE(up.opted_out, FALSE) OR up.deleted_at IS NOT NULL))
    OR EXISTS (SELECT 1 FROM activity.user_retention_tracking rt
                WHERE rt.user_id = b.user_id AND COALESCE(rt.opted_out, FALSE))
)";

/// T1: zwei Menschen gleichzeitig allein in verschiedenen Kanälen.
async fn lade_t1(pool: &PgPool, guild_id: i64, router_vc: i64) -> Result<Vec<Kandidat>, sqlx::Error> {
    let sql = format!(
        "WITH counts AS (
            SELECT channel_id, count(*) AS n
              FROM activity.voice_open_sessions
             WHERE guild_id = $1
             GROUP BY channel_id
        ), solo AS (
            SELECT s.user_id, s.channel_id, s.joined_at
              FROM activity.voice_open_sessions s
              JOIN counts c ON c.channel_id = s.channel_id AND c.n = 1
             WHERE s.guild_id = $1
               AND s.joined_at <= now() - INTERVAL '120 seconds'
               AND s.channel_id <> $2
        )
        SELECT a.user_id AS user_a, b.user_id AS user_b,
               EXTRACT(EPOCH FROM (now() - a.joined_at))::BIGINT AS allein_s_a,
               EXTRACT(EPOCH FROM (now() - b.joined_at))::BIGINT AS allein_s_b,
               COALESCE(cp.sessions_together, 0)::BIGINT AS sessions_together,
               sa.deadlock_rank AS rang_a, sb.deadlock_rank AS rang_b,
               {OPT_OUT_A_UND_B} AS opted_out
          FROM solo a
          JOIN solo b ON a.user_id < b.user_id AND a.channel_id <> b.channel_id
          LEFT JOIN activity.user_co_players cp
                 ON cp.user_id = a.user_id AND cp.co_player_id = b.user_id
          LEFT JOIN core.steam_links sa ON sa.discord_id = a.user_id AND sa.primary_account
          LEFT JOIN core.steam_links sb ON sb.discord_id = b.user_id AND sb.primary_account
         ORDER BY a.joined_at
         LIMIT 30"
    );
    let rows = sqlx::query(&sql).bind(guild_id).bind(router_vc).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| Kandidat {
            kategorie: Kategorie::T1SoloDoppel,
            guild_id,
            user_a: row.get("user_a"),
            user_b: row.get("user_b"),
            opted_out: row.get("opted_out"),
            daten: json!({
                "trigger": "t1_solo_doppel",
                "beide_allein_seit_s": [row.get::<i64, _>("allein_s_a"), row.get::<i64, _>("allein_s_b")],
                "sessions_together": row.get::<i64, _>("sessions_together"),
                "rang": [row.get::<Option<i32>, _>("rang_a"), row.get::<Option<i32>, _>("rang_b")],
            }),
        })
        .collect())
}

/// T3: Abwanderungs-Kandidat mit gerade aktivem Co-Player (Anker).
/// user_a ist der Anker (aktive Seite), user_b der Abwanderer.
async fn lade_t3(pool: &PgPool, guild_id: i64) -> Result<Vec<Kandidat>, sqlx::Error> {
    let sql = "WITH beste AS (
            SELECT DISTINCT ON (r.user_id)
                   cp.co_player_id AS anker,
                   r.user_id AS abwanderer,
                   r.inactive_days,
                   cp.sessions_together,
                   cp.last_played_together,
                   EXISTS (SELECT 1 FROM activity.voice_open_sessions os
                            WHERE os.user_id = cp.co_player_id AND os.guild_id = r.guild_id)
                       AS anker_im_voice
              FROM activity.at_risk_members r
              JOIN activity.user_co_players cp
                    ON cp.user_id = r.user_id AND cp.sessions_together >= 2
              JOIN activity.user_activity_patterns p
                    ON p.user_id = cp.co_player_id
                   AND p.last_active_at >= now() - INTERVAL '3 days'
             WHERE r.guild_id = $1
             ORDER BY r.user_id, cp.sessions_together DESC
        )
        SELECT a.anker AS anker, a.abwanderer, a.inactive_days, a.sessions_together,
               a.last_played_together, a.anker_im_voice,
               (
                   EXISTS (SELECT 1 FROM core.user_privacy up
                            WHERE up.user_id = a.anker
                              AND (COALESCE(up.opted_out, FALSE) OR up.deleted_at IS NOT NULL))
                OR EXISTS (SELECT 1 FROM activity.user_retention_tracking rt
                            WHERE rt.user_id = a.anker AND COALESCE(rt.opted_out, FALSE))
               ) AS opted_out
          FROM beste a
         ORDER BY a.sessions_together DESC
         LIMIT 15";
    let rows = sqlx::query(sql).bind(guild_id).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| Kandidat {
            kategorie: Kategorie::T3RueckkehrerAnker,
            guild_id,
            user_a: row.get("anker"),
            user_b: row.get("abwanderer"),
            opted_out: row.get("opted_out"),
            daten: json!({
                "trigger": "t3_rueckkehrer_anker",
                "inactive_days": row.get::<i32, _>("inactive_days"),
                "sessions_together": row.get::<i32, _>("sessions_together"),
                "tage_seit_letztem_spiel": row
                    .get::<Option<chrono::DateTime<Utc>>, _>("last_played_together")
                    .map(|t| (Utc::now() - t).num_days()),
                "anker_im_voice": row.get::<bool, _>("anker_im_voice"),
            }),
        })
        .collect())
}

/// T4: offenes Gesuch ohne Resonanz (Lane leer oder ohne Lane).
async fn lade_t4(pool: &PgPool, guild_id: i64) -> Result<Vec<Kandidat>, sqlx::Error> {
    let sql = "SELECT p.owner_id, p.mode, p.play_window, p.rank_min, p.rank_max,
               p.requested_slots,
               EXTRACT(EPOCH FROM (now() - p.created_at))::BIGINT AS offen_seit_s,
               (p.lane_id IS NULL OR NOT EXISTS (
                    SELECT 1 FROM activity.voice_open_sessions os
                     WHERE os.channel_id = p.lane_id)) AS ohne_resonanz,
               (SELECT count(*) FROM activity.lfg_watches w
                 WHERE w.guild_id = p.guild_id AND w.mode = p.mode
                   AND w.fired_at IS NULL AND w.expires_at > now()) AS scharfe_watches,
               (
                   EXISTS (SELECT 1 FROM core.user_privacy up
                            WHERE up.user_id = p.owner_id
                              AND (COALESCE(up.opted_out, FALSE) OR up.deleted_at IS NOT NULL))
                OR EXISTS (SELECT 1 FROM activity.user_retention_tracking rt
                            WHERE rt.user_id = p.owner_id AND COALESCE(rt.opted_out, FALSE))
               ) AS opted_out
          FROM voice.lfg_posts p
         WHERE p.guild_id = $1
           AND p.status = 'open'
           AND p.created_at <= now() - INTERVAL '20 minutes'
           AND p.expires_at > now()
         LIMIT 10";
    let rows = sqlx::query(sql).bind(guild_id).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .filter(|row| row.get::<bool, _>("ohne_resonanz"))
        .map(|row| Kandidat {
            kategorie: Kategorie::T4GesuchOhneResonanz,
            guild_id,
            user_a: row.get("owner_id"),
            user_b: 0,
            opted_out: row.get("opted_out"),
            daten: json!({
                "trigger": "t4_gesuch_ohne_resonanz",
                "mode": row.get::<String, _>("mode"),
                "play_window": row.get::<Option<String>, _>("play_window"),
                "rank_fenster": [row.get::<Option<i32>, _>("rank_min"), row.get::<Option<i32>, _>("rank_max")],
                "requested_slots": row.get::<i32, _>("requested_slots"),
                "offen_seit_s": row.get::<i64, _>("offen_seit_s"),
                "scharfe_watches": row.get::<i64, _>("scharfe_watches"),
            }),
        })
        .collect())
}

/// T2 (nur Tageslauf): Zeit-Zwillinge ohne gemeinsame Historie. Rang ist
/// Bonus-Signal, kein Filter — nur 45 Prozent Rang-Abdeckung (Q5).
async fn lade_t2(pool: &PgPool) -> Result<Vec<Kandidat>, sqlx::Error> {
    let sql = format!(
        "WITH aktiv AS (
            SELECT p.user_id, p.typical_hours, s.deadlock_rank
              FROM activity.user_activity_patterns p
              LEFT JOIN core.steam_links s
                     ON s.discord_id = p.user_id AND s.primary_account
             WHERE p.last_active_at >= now() - INTERVAL '14 days'
               AND p.sessions_count_2w >= 2
               AND jsonb_typeof(p.typical_hours) = 'array'
        )
        SELECT a.user_id AS user_a, b.user_id AS user_b,
               ov.overlap AS overlap_stunden,
               a.deadlock_rank AS rang_a, b.deadlock_rank AS rang_b,
               {OPT_OUT_A_UND_B} AS opted_out
          FROM aktiv a
          JOIN aktiv b ON a.user_id < b.user_id
          JOIN LATERAL (
               SELECT count(*) AS overlap
                 FROM jsonb_array_elements(a.typical_hours) ha
                WHERE EXISTS (SELECT 1 FROM jsonb_array_elements(b.typical_hours) hb
                               WHERE hb.value = ha.value)
          ) ov ON ov.overlap >= 2
         WHERE NOT EXISTS (
               SELECT 1 FROM activity.user_co_players cp
                WHERE (cp.user_id = a.user_id AND cp.co_player_id = b.user_id)
                   OR (cp.user_id = b.user_id AND cp.co_player_id = a.user_id))
         ORDER BY ov.overlap DESC
         LIMIT 5"
    );
    let guild_id = env_i64("OUR_GUILD_ID", env_i64("MAIN_GUILD_ID", DEFAULT_GUILD_ID));
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| Kandidat {
            kategorie: Kategorie::T2ZeitZwilling,
            guild_id,
            user_a: row.get("user_a"),
            user_b: row.get("user_b"),
            opted_out: row.get("opted_out"),
            daten: json!({
                "trigger": "t2_zeit_zwilling",
                "overlap_stunden": row.get::<i64, _>("overlap_stunden"),
                "rang": [row.get::<Option<i32>, _>("rang_a"), row.get::<Option<i32>, _>("rang_b")],
                "noch_nie_zusammen_gespielt": true,
            }),
        })
        .collect())
}

/// Paare mit LLM-Urteil (yes, no, unsure) innerhalb des Cooldowns. Suppressed
/// zählt bewusst nicht — sonst verlängert der Cooldown sich selbst endlos.
async fn lade_recent_pairs(pool: &PgPool, cooldown_days: i64) -> Result<HashSet<(i64, i64)>> {
    let rows = sqlx::query(
        "SELECT payload->'kandidaten' AS paar
           FROM bot.ai_decision_ledger
          WHERE source = $1
            AND decided_at >= now() - make_interval(days => $2::INT)
            AND decision IN ('yes', 'no', 'unsure')",
    )
    .bind(SOURCE_MATCH)
    .bind(cooldown_days)
    .fetch_all(pool)
    .await
    .context("recent_pairs laden")?;
    let mut pairs = HashSet::new();
    for row in rows {
        let paar: Option<serde_json::Value> = row.get("paar");
        if let Some(liste) = paar.as_ref().and_then(|v| v.as_array()) {
            if let (Some(a), Some(b)) = (
                liste.first().and_then(|v| v.as_i64()),
                liste.get(1).and_then(|v| v.as_i64()),
            ) {
                pairs.insert(dl_verbinder::pair_key(a, b));
            }
        }
    }
    Ok(pairs)
}

async fn zaehle_llm_calls_heute(pool: &PgPool) -> Result<i64> {
    sqlx::query_scalar(
        "SELECT count(*) FROM bot.ai_decision_ledger
          WHERE source IN ($1, $2)
            AND decided_at >= date_trunc('day', now())
            AND payload->>'llm' = 'true'",
    )
    .bind(SOURCE_MATCH)
    .bind(SOURCE_KRITIK)
    .fetch_one(pool)
    .await
    .context("Tagesdeckel zählen")
}

async fn zaehle_klassen_heute(pool: &PgPool) -> Result<BTreeMap<String, i64>> {
    let rows = sqlx::query(
        "SELECT decision, count(*) AS n
           FROM bot.ai_decision_ledger
          WHERE source = $1 AND decided_at >= date_trunc('day', now())
          GROUP BY decision",
    )
    .bind(SOURCE_MATCH)
    .fetch_all(pool)
    .await
    .context("Tagesklassen zählen")?;
    Ok(rows
        .into_iter()
        .map(|row| (row.get::<String, _>("decision"), row.get::<i64, _>("n")))
        .collect())
}

/// Ledgert alle Einträge einer Transaktion; Kritik-Einträge bekommen die
/// echte Ledger-ID ihres Match-Eintrags in den Payload. Vor jedem Insert
/// läuft der Privacy-Recheck unter dem Lock — für beide Beteiligten.
async fn persist_entries(
    pool: &PgPool,
    entries: &[LedgerEntry],
    kritiken: &[(usize, LedgerEntry, bool, String)],
) -> Result<Vec<i64>> {
    let mut transaction = pool.begin().await.context("Transaktion beginnen")?;
    let mut ids = Vec::with_capacity(entries.len());
    for entry in entries {
        let id = insert_mit_privacy_recheck(&mut transaction, entry).await?;
        ids.push(id);
    }
    for (match_index, kritik_entry, _, _) in kritiken {
        let mut kritik = kritik_entry.clone();
        let match_id = ids[*match_index];
        kritik.input_summary = format!("match_ledger_id={match_id}");
        if let Some(objekt) = kritik.payload.as_object_mut() {
            objekt.insert("match_ledger_id".into(), json!(match_id));
        }
        insert_mit_privacy_recheck(&mut transaction, &kritik).await?;
    }
    transaction.commit().await.context("Transaktion committen")?;
    Ok(ids)
}

async fn insert_mit_privacy_recheck(
    transaction: &mut Transaction<'_, Postgres>,
    entry: &LedgerEntry,
) -> Result<i64> {
    // Beide beteiligten IDs prüfen: subject_user_id und den Partner im Payload.
    let mut betroffen: Vec<i64> = Vec::new();
    if let Some(user_id) = entry.subject_user_id {
        betroffen.push(user_id);
    }
    if let Some(partner) = entry.payload["kandidaten"].as_array().and_then(|liste| {
        liste
            .iter()
            .filter_map(|wert| wert.as_i64())
            .find(|id| Some(*id) != entry.subject_user_id && *id != 0)
    }) {
        betroffen.push(partner);
    }
    let anonymized;
    let mut zu_schreiben = entry;
    for user_id in betroffen {
        if dl_central_db::lock_user_privacy_and_is_opted_out(transaction, user_id)
            .await
            .context("Privacy-Recheck")?
        {
            anonymized = anonymize_for_privacy(entry);
            zu_schreiben = &anonymized;
            break;
        }
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO bot.ai_decision_ledger(
             source, subject_user_id, guild_id, input_summary, decision,
             confidence, reason, action_taken, payload
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id",
    )
    .bind(zu_schreiben.source)
    .bind(zu_schreiben.subject_user_id)
    .bind(zu_schreiben.guild_id)
    .bind(&zu_schreiben.input_summary)
    .bind(zu_schreiben.decision.as_str())
    .bind(zu_schreiben.confidence)
    .bind(&zu_schreiben.reason)
    .bind(zu_schreiben.action_taken)
    .bind(&zu_schreiben.payload)
    .fetch_one(&mut **transaction)
    .await
    .context("Ledger-Insert")?;
    Ok(id)
}

async fn auswertung() -> Result<()> {
    let akte = lade_akte()?;
    let dsn = dl_central_db::dsn_from_env().context("zentrale DB-Konfiguration")?;
    let pool = dl_central_db::connect_pool(&dsn)
        .await
        .context("zentrale DB verbinden")?;

    let agreement_rows: Vec<(String, String)> = sqlx::query(
        "SELECT payload->>'trigger' AS kategorie, outcome
           FROM bot.ai_decision_ledger
          WHERE source = $1 AND decision = 'yes' AND outcome IS NOT NULL
            AND decided_at >= now() - INTERVAL '7 days'",
    )
    .bind(SOURCE_MATCH)
    .fetch_all(&pool)
    .await
    .context("Agreement laden")?
    .into_iter()
    .filter_map(|row| {
        let kategorie: Option<String> = row.get("kategorie");
        let outcome: Option<String> = row.get("outcome");
        Some((kategorie?, outcome?))
    })
    .collect();

    let offene_yes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bot.ai_decision_ledger
          WHERE source = $1 AND decision = 'yes' AND outcome IS NULL
            AND decided_at >= now() - INTERVAL '7 days'",
    )
    .bind(SOURCE_MATCH)
    .fetch_one(&pool)
    .await
    .context("offene yes zählen")?;

    let klassen_7d: BTreeMap<String, i64> = sqlx::query(
        "SELECT decision, count(*) AS n
           FROM bot.ai_decision_ledger
          WHERE source = $1 AND decided_at >= now() - INTERVAL '7 days'
          GROUP BY decision",
    )
    .bind(SOURCE_MATCH)
    .fetch_all(&pool)
    .await
    .context("Klassen laden")?
    .into_iter()
    .map(|row| (row.get::<String, _>("decision"), row.get::<i64, _>("n")))
    .collect();

    // 5 zufällige Fälle für den Owner: ohne User-IDs, mit Kritik und Outcome.
    let stichprobe: Vec<String> = sqlx::query(
        "SELECT m.payload->>'trigger' AS kategorie,
                m.payload->>'geplanter_text' AS text,
                m.payload->>'geplanter_kanal' AS kanal,
                m.outcome,
                k.decision AS kritik_decision,
                k.payload->>'begruendung' AS kritik_begruendung
           FROM bot.ai_decision_ledger m
           LEFT JOIN bot.ai_decision_ledger k
                  ON k.source = $2
                 AND (k.payload->>'match_ledger_id')::BIGINT = m.id
          WHERE m.source = $1 AND m.decision = 'yes'
            AND m.decided_at >= now() - INTERVAL '7 days'
          ORDER BY random()
          LIMIT 5",
    )
    .bind(SOURCE_MATCH)
    .bind(SOURCE_KRITIK)
    .fetch_all(&pool)
    .await
    .context("Stichprobe laden")?
    .into_iter()
    .enumerate()
    .map(|(index, row)| {
        let kritik = match row.get::<Option<String>, _>("kritik_decision").as_deref() {
            Some("yes") => "Kritiker ok".to_string(),
            Some("no") => format!(
                "Kritiker beanstandet: {}",
                row.get::<Option<String>, _>("kritik_begruendung").unwrap_or_default()
            ),
            Some(andere) => format!("Kritiker {andere}"),
            None => "ohne Kritik".to_string(),
        };
        let outcome = match row.get::<Option<String>, _>("outcome").as_deref() {
            Some("met") => "trafen sich",
            Some("not_met") => "trafen sich nicht",
            _ => "offen",
        };
        format!(
            "{}. [{}] über {}: „{}“ | {} | Realität: {}",
            index + 1,
            row.get::<Option<String>, _>("kategorie").unwrap_or_default(),
            row.get::<Option<String>, _>("kanal").unwrap_or_default(),
            row.get::<Option<String>, _>("text").unwrap_or_default(),
            kritik,
            outcome
        )
    })
    .collect();

    let bericht = render_wochenbericht(
        &akte.nonce,
        &WochenberichtInput {
            agreement: agreement_je_kategorie(&agreement_rows),
            offene_yes: offene_yes.max(0) as u64,
            stichprobe,
            klassen_7d,
        },
    );
    poste_staff(&bericht).await;
    Ok(())
}

/// Staff-Post nach Discord; ohne Kanal-Konfiguration auf stdout (dann sieht
/// journalctl den Bericht). Fehler beim Senden fallen auf stdout zurück —
/// ein Bericht darf nie stillschweigend verloren gehen.
///
/// Lange Berichte gehen als mehrere Nachrichten raus: Discord nimmt 2000
/// Zeichen pro Nachricht, ein Lauf mit vollem Deckel oder ein Wochenbericht
/// mit Stichprobe liegt darüber.
async fn poste_staff(text: &str) {
    let kanal = match env("DL_VERBINDER_STAFF_CHANNEL_ID") {
        Some(wert) => wert,
        None => {
            println!("{text}");
            return;
        }
    };
    let token = match env("DISCORD_TOKEN").or_else(|| env("BOT_TOKEN")) {
        Some(token) => token,
        None => {
            tracing::error!("DISCORD_TOKEN oder BOT_TOKEN fehlt, Bericht auf stdout");
            println!("{text}");
            return;
        }
    };
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(30)).build() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "HTTP-Client, Bericht auf stdout");
            println!("{text}");
            return;
        }
    };
    let teile = chunk_message(text, DISCORD_CONTENT_LIMIT);
    if teile.is_empty() {
        tracing::warn!("Staff-Post ohne Inhalt, nichts gesendet");
        return;
    }
    let gesamt = teile.len();
    let mut fehlgeschlagen = 0_usize;
    for (nummer, teil) in teile.iter().enumerate() {
        let antwort = client
            .post(format!("https://discord.com/api/v10/channels/{kanal}/messages"))
            .header("Authorization", format!("Bot {token}"))
            .header("User-Agent", "dl-verbinder/0.1")
            .json(&json!({
                "content": teil,
                "allowed_mentions": {"parse": []},
            }))
            .send()
            .await;
        // Ein gescheiterter Teil stoppt die restlichen nicht: lieber der halbe
        // Bericht im Kanal als gar keiner.
        if let Err(error) = antwort.and_then(|r| r.error_for_status()) {
            fehlgeschlagen += 1;
            tracing::error!(%error, teil = nummer + 1, gesamt, "Staff-Post-Teil fehlgeschlagen");
        }
    }
    if fehlgeschlagen > 0 {
        tracing::error!(
            fehlgeschlagen,
            gesamt,
            "Staff-Post unvollständig, Bericht auf stdout"
        );
        println!("{text}");
    } else {
        tracing::info!(gesamt, "Staff-Post gesendet");
    }
}
