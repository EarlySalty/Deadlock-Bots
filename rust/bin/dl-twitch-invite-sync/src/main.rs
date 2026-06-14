//! Sync der Streamer→Discord-Invite-Zuordnung aus dem Twitch-Bot (interne API)
//! in die geteilte Deadlock-sqlite — plus rückwirkende Twitch-Neu-Klassifikation
//! der Beitritts-Events.
//!
//! ## Warum
//! Die Streamer→Invite-Zuordnung lebt im Twitch-Bot (Postgres). In der
//! Deadlock-sqlite war `twitch_streamer_invites` ein Phantom (nie geschrieben),
//! deshalb wurden Joins über vom Bot erstellte Streamer-Invites als
//! `bot_invite` verbucht. Die Dashboard-Auswertung (Python wie Rust) löst zwar
//! `twitch_streamer_login` aus der Tabelle auf, kippt einen bereits gültigen
//! Bucket aber **nie** auf `twitch` — der Twitch-Override fehlte. Folge: solche
//! Joins zählten dauerhaft nicht als Twitch.
//!
//! ## Was dieses Bin tut
//! 1. **Populate:** Holt alle Zuordnungen über `/internal/twitch/v1/streamer-
//!    invites` und spiegelt sie (UPSERT auf `streamer_login`).
//! 2. **Reclassify (chirurgisch):** Geht alle Join-Events durch und korrigiert
//!    *ausschließlich* die, die laut [`dl_activity::join_source::classify`] auf
//!    `twitch` auflösen, aber einen anderen Bucket gespeichert haben. Nur
//!    `→twitch`, nie weg davon → idempotent, oszilliert nicht. Schreibt einen
//!    konsistenten Twitch-Feldsatz, damit das Live-Python-Dashboard (liest den
//!    gespeicherten Bucket) es sofort korrekt zählt.
//!
//! Per systemd-Timer periodisch lauffähig: neue Streamer landen in der Tabelle,
//! ihre Alt-Joins werden beim nächsten Lauf nachgezogen.
//!
//! Env:
//! - `TWITCH_INTERNAL_API_TOKEN` (Pflicht) — Auth gegen die interne Twitch-API.
//! - `TWITCH_BOT_API_BASE` (Default `http://127.0.0.1:8776`).
//! - `DEADLOCK_DB_PATH` (Default `data/deadlock.sqlite3`).
//! - `DRY_RUN=1` — Tabelle wird befüllt (additiv), aber der Reclassify schreibt
//!   nicht, sondern meldet nur, was er ändern würde.

use std::collections::HashMap;

use dl_activity::join_source::classify;
use dl_db::Db;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

/// Website-Unterseiten-Slugs (wie `dl-dashboard::server_stats`). Ein Invite, der
/// einer Website-Quelle zuordenbar ist, darf nie als Twitch geflippt werden
/// (Website-Override gewinnt in `classify`).
const WEBSITE_SLUGS: [&str; 6] = [
    "landing",
    "streamer",
    "mitspieler",
    "coaching",
    "helden",
    "guides",
];

#[derive(Debug, Deserialize)]
struct InviteEntry {
    streamer_login: String,
    guild_id: i64,
    invite_code: String,
    invite_url: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    last_sent_at: Option<String>,
}

struct SyncSummary {
    table_rows: i64,
    flipped: usize,
    from_buckets: Vec<(String, i64)>,
    dry_run: bool,
}

fn serde_to_sql(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = std::env::var("TWITCH_BOT_API_BASE")
        .unwrap_or_else(|_| "http://127.0.0.1:8776".to_string());
    let token = std::env::var("TWITCH_INTERNAL_API_TOKEN")
        .map_err(|_| anyhow::anyhow!("TWITCH_INTERNAL_API_TOKEN fehlt"))?;
    let db_path =
        std::env::var("DEADLOCK_DB_PATH").unwrap_or_else(|_| "data/deadlock.sqlite3".to_string());
    let dry_run = matches!(
        std::env::var("DRY_RUN").ok().as_deref(),
        Some("1") | Some("true")
    );

    let url = format!(
        "{}/internal/twitch/v1/streamer-invites",
        base.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let entries: Vec<InviteEntry> = client
        .get(&url)
        .header("X-Internal-Token", &token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let fetched = entries.len();

    let db = Db::open(&db_path)?;
    let summary = db
        .write(move |conn| {
            // ── 1. Populate twitch_streamer_invites ──────────────────────────
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS twitch_streamer_invites (
                    streamer_login TEXT PRIMARY KEY,
                    guild_id       INTEGER,
                    invite_code    TEXT,
                    invite_url     TEXT,
                    created_at     TEXT,
                    last_sent_at   TEXT
                 );",
            )?;
            {
                let tx = conn.transaction()?;
                {
                    let mut stmt = tx.prepare(
                        "INSERT INTO twitch_streamer_invites
                           (streamer_login, guild_id, invite_code, invite_url, created_at, last_sent_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(streamer_login) DO UPDATE SET
                           guild_id     = excluded.guild_id,
                           invite_code  = excluded.invite_code,
                           invite_url   = excluded.invite_url,
                           created_at   = excluded.created_at,
                           last_sent_at = excluded.last_sent_at",
                    )?;
                    for e in &entries {
                        let login = e.streamer_login.trim().to_lowercase();
                        if login.is_empty() {
                            continue;
                        }
                        stmt.execute(params![
                            login,
                            e.guild_id,
                            e.invite_code.trim(),
                            e.invite_url.trim(),
                            e.created_at,
                            e.last_sent_at,
                        ])?;
                    }
                }
                tx.commit()?;
            }
            let table_rows: i64 = conn.query_row(
                "SELECT COUNT(*) FROM twitch_streamer_invites",
                [],
                |r| r.get(0),
            )?;

            // ── 2. Lookups aufbauen (code→login bzw. code→website-slug) ───────
            let mut twitch_lookup: HashMap<String, String> = HashMap::new();
            {
                let mut s =
                    conn.prepare("SELECT streamer_login, invite_code FROM twitch_streamer_invites")?;
                let mut rows = s.query([])?;
                while let Some(r) = rows.next()? {
                    let login = r.get::<_, Option<String>>(0)?.unwrap_or_default().trim().to_lowercase();
                    let code = r.get::<_, Option<String>>(1)?.unwrap_or_default().trim().to_lowercase();
                    if !login.is_empty() && !code.is_empty() {
                        twitch_lookup.entry(code).or_insert(login);
                    }
                }
            }
            let mut website_lookup: HashMap<String, String> = HashMap::new();
            {
                let mut s = conn.prepare("SELECT k, v FROM kv_store WHERE ns = 'website_invites'")?;
                let mut rows = s.query([])?;
                while let Some(r) = rows.next()? {
                    let k: String = r.get(0)?;
                    let v: Option<String> = r.get(1)?;
                    let slug = if k == "main" {
                        "landing".to_string()
                    } else {
                        k.trim().to_lowercase()
                    };
                    if !WEBSITE_SLUGS.contains(&slug.as_str()) {
                        continue;
                    }
                    if let Some(code) = v
                        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                        .and_then(|p| {
                            p.get("code")
                                .and_then(Value::as_str)
                                .map(|c| c.trim().to_string())
                        })
                        .filter(|c| !c.is_empty())
                    {
                        website_lookup.entry(code.to_lowercase()).or_insert(slug);
                    }
                }
            }

            // ── 3. Reclassify-Kandidaten sammeln (nur →twitch) ────────────────
            let mut candidates: Vec<(i64, String)> = Vec::new();
            let mut from_counts: HashMap<String, i64> = HashMap::new();
            {
                let mut stmt =
                    conn.prepare("SELECT id, metadata FROM member_events WHERE event_type = 'join'")?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    let id: i64 = r.get(0)?;
                    let meta_raw: Option<String> = r.get(1)?;
                    let mut meta: Value = meta_raw
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                        .filter(Value::is_object)
                        .unwrap_or_else(|| json!({}));
                    let c = classify(&meta, &twitch_lookup, &website_lookup);
                    let stored = meta
                        .get("join_source_bucket")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_lowercase();
                    if c.bucket != "twitch" || stored == "twitch" {
                        continue;
                    }
                    let Some(login) = c.twitch_login.clone() else {
                        continue;
                    };
                    if let Some(obj) = meta.as_object_mut() {
                        obj.insert("join_source_bucket".into(), json!("twitch"));
                        obj.insert("join_source_kind".into(), json!("twitch_streamer"));
                        obj.insert("join_source_label".into(), json!(format!("Twitch: {login}")));
                        obj.insert("twitch_streamer_login".into(), json!(login));
                        if let Some(url) = c.invite_url.clone() {
                            obj.insert("invite_url".into(), json!(url));
                        }
                        candidates.push((id, serde_json::to_string(&meta).map_err(serde_to_sql)?));
                        let key = if stored.is_empty() {
                            "(leer)".to_string()
                        } else {
                            stored
                        };
                        *from_counts.entry(key).or_insert(0) += 1;
                    }
                }
            }

            // ── 4. Anwenden (sofern kein Dry-Run) ─────────────────────────────
            let flipped = candidates.len();
            if !dry_run && !candidates.is_empty() {
                let tx = conn.transaction()?;
                {
                    let mut up =
                        tx.prepare("UPDATE member_events SET metadata = ?1 WHERE id = ?2")?;
                    for (id, m) in &candidates {
                        up.execute(params![m, id])?;
                    }
                }
                tx.commit()?;
            }

            let mut from_buckets: Vec<(String, i64)> = from_counts.into_iter().collect();
            from_buckets.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            Ok(SyncSummary {
                table_rows,
                flipped,
                from_buckets,
                dry_run,
            })
        })
        .await?;

    println!(
        "twitch_streamer_invites: {fetched} geholt, {} Zeilen in {db_path}",
        summary.table_rows
    );
    let verb = if summary.dry_run {
        "würden geflippt (DRY_RUN)"
    } else {
        "auf twitch korrigiert"
    };
    println!("Reclassify: {} Join-Events {verb}", summary.flipped);
    for (bucket, n) in &summary.from_buckets {
        println!("    aus {bucket:12} {n}");
    }
    Ok(())
}
