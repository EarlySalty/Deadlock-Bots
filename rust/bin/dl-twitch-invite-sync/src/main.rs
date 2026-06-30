//! Sync der Streamer→Discord-Invite-Zuordnung aus dem Twitch-Bot (interne API)
//! in die zentrale Deadlock-DB — plus rückwirkende Twitch-Neu-Klassifikation
//! der Beitritts-Events.
//!
//! ## Warum
//! Die Streamer→Invite-Zuordnung lebt im Twitch-Bot (Postgres). In der
//! Deadlock-DB war `twitch_streamer_invites` ein Phantom (nie geschrieben),
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
//! - `DEADLOCK_CENTRAL_DSN` (Pflicht, via `dl_central_db::dsn_from_env`).
//! - `DRY_RUN=1` — Tabelle wird befüllt (additiv), aber der Reclassify schreibt
//!   nicht, sondern meldet nur, was er ändern würde.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use dl_activity::join_source::classify;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

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

fn parse_optional_timestamp(raw: Option<&str>) -> anyhow::Result<Option<DateTime<Utc>>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(parsed.with_timezone(&Utc)));
    }
    let parsed = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|err| anyhow::anyhow!("ungueltiger Twitch-Invite-Timestamp `{raw}`: {err}"))?;
    Ok(Some(parsed.and_utc()))
}

async fn sync_invites_and_reclassify(
    pool: &PgPool,
    entries: Vec<InviteEntry>,
    dry_run: bool,
) -> anyhow::Result<SyncSummary> {
    {
        let mut populate_tx = pool.begin().await?;

        for entry in &entries {
            let login = entry.streamer_login.trim().to_lowercase();
            if login.is_empty() {
                continue;
            }
            let invite_code = entry.invite_code.trim().to_string();
            let invite_url = entry.invite_url.trim().to_string();
            let created_at = parse_optional_timestamp(entry.created_at.as_deref())?;
            let last_sent_at = parse_optional_timestamp(entry.last_sent_at.as_deref())?;
            sqlx::query!(
                r#"
                INSERT INTO bot.twitch_streamer_invites(
                    streamer_login, guild_id, invite_code, invite_url, created_at, last_sent_at
                )
                VALUES($1, $2, $3, $4, $5, $6)
                ON CONFLICT(streamer_login) DO UPDATE SET
                    guild_id = EXCLUDED.guild_id,
                    invite_code = EXCLUDED.invite_code,
                    invite_url = EXCLUDED.invite_url,
                    created_at = EXCLUDED.created_at,
                    last_sent_at = EXCLUDED.last_sent_at
                "#,
                login,
                entry.guild_id,
                invite_code,
                invite_url,
                created_at,
                last_sent_at,
            )
            .execute(&mut *populate_tx)
            .await?;
        }

        populate_tx.commit().await?;
    }

    let table_rows =
        sqlx::query!(r#"SELECT COUNT(*) AS "count!" FROM bot.twitch_streamer_invites"#)
            .fetch_one(pool)
            .await?
            .count;

    let twitch_rows = sqlx::query!(
        r#"
        SELECT streamer_login, invite_code
          FROM bot.twitch_streamer_invites
        "#
    )
    .fetch_all(pool)
    .await?;
    let mut twitch_lookup: HashMap<String, String> = HashMap::new();
    for row in twitch_rows {
        let login = row.streamer_login.trim().to_lowercase();
        let code = row.invite_code.unwrap_or_default().trim().to_lowercase();
        if !login.is_empty() && !code.is_empty() {
            twitch_lookup.entry(code).or_insert(login);
        }
    }

    let website_rows = sqlx::query!(
        r#"
        SELECT k, v
          FROM bot.kv_store
         WHERE ns = 'website_invites'
        "#
    )
    .fetch_all(pool)
    .await?;
    let mut website_lookup: HashMap<String, String> = HashMap::new();
    for row in website_rows {
        let slug = if row.k == "main" {
            "landing".to_string()
        } else {
            row.k.trim().to_lowercase()
        };
        if !WEBSITE_SLUGS.contains(&slug.as_str()) {
            continue;
        }
        if let Some(code) = serde_json::from_str::<Value>(&row.v)
            .ok()
            .and_then(|payload| {
                payload
                    .get("code")
                    .and_then(Value::as_str)
                    .map(|code| code.trim().to_string())
            })
            .filter(|code| !code.is_empty())
        {
            website_lookup.entry(code.to_lowercase()).or_insert(slug);
        }
    }

    let join_rows = sqlx::query!(
        r#"
        SELECT id, metadata::text AS "metadata?"
          FROM activity.member_events
         WHERE event_type = 'join'
        "#
    )
    .fetch_all(pool)
    .await?;
    let mut candidates: Vec<(i64, String)> = Vec::new();
    let mut from_counts: HashMap<String, i64> = HashMap::new();
    for row in join_rows {
        let mut meta = row
            .metadata
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let classified = classify(&meta, &twitch_lookup, &website_lookup);
        let stored = meta
            .get("join_source_bucket")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if classified.bucket != "twitch" || stored == "twitch" {
            continue;
        }
        let Some(login) = classified.twitch_login.clone() else {
            continue;
        };
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("join_source_bucket".into(), json!("twitch"));
            obj.insert("join_source_kind".into(), json!("twitch_streamer"));
            obj.insert(
                "join_source_label".into(),
                json!(format!("Twitch: {login}")),
            );
            obj.insert("twitch_streamer_login".into(), json!(login));
            if let Some(url) = classified.invite_url.clone() {
                obj.insert("invite_url".into(), json!(url));
            }
            candidates.push((row.id, serde_json::to_string(&meta)?));
            let key = if stored.is_empty() {
                "(leer)".to_string()
            } else {
                stored
            };
            *from_counts.entry(key).or_insert(0) += 1;
        }
    }

    let flipped = candidates.len();
    if !dry_run && !candidates.is_empty() {
        let mut reclassify_tx = pool.begin().await?;
        for (id, metadata) in &candidates {
            sqlx::query!(
                r#"
                UPDATE activity.member_events
                   SET metadata = $1::text::jsonb
                 WHERE id = $2
                "#,
                metadata,
                id,
            )
            .execute(&mut *reclassify_tx)
            .await?;
        }
        reclassify_tx.commit().await?;
    }

    let mut from_buckets: Vec<(String, i64)> = from_counts.into_iter().collect();
    from_buckets.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(SyncSummary {
        table_rows,
        flipped,
        from_buckets,
        dry_run,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = std::env::var("TWITCH_BOT_API_BASE")
        .unwrap_or_else(|_| "http://127.0.0.1:8776".to_string());
    let token = std::env::var("TWITCH_INTERNAL_API_TOKEN")
        .map_err(|_| anyhow::anyhow!("TWITCH_INTERNAL_API_TOKEN fehlt"))?;
    let dry_run = matches!(
        std::env::var("DRY_RUN").ok().as_deref(),
        Some("1") | Some("true")
    );
    let central_dsn = dl_central_db::dsn_from_env()?;
    let pool = dl_central_db::connect_pool(&central_dsn).await?;

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

    let summary = sync_invites_and_reclassify(&pool, entries, dry_run).await?;

    println!(
        "twitch_streamer_invites: {fetched} geholt, {} Zeilen in zentraler DB",
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

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use sqlx::Row as _;

    fn invite(login: &str, code: &str) -> InviteEntry {
        InviteEntry {
            streamer_login: login.to_string(),
            guild_id: 1,
            invite_code: code.to_string(),
            invite_url: format!("https://discord.gg/{code}"),
            created_at: Some("2026-06-01T10:00:00Z".to_string()),
            last_sent_at: None,
        }
    }

    async fn insert_join(
        pool: &PgPool,
        id: i64,
        invite_code: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let metadata = json!({
            "join_source_bucket": "personal",
            "join_source_kind": "invite_link",
            "invite_code": invite_code,
        });
        sqlx::query(
            r#"
            INSERT INTO activity.member_events(id, user_id, guild_id, event_type, metadata)
            VALUES ($1, $2, 1, 'join', $3::jsonb)
            "#,
        )
        .bind(id)
        .bind(10_000 + id)
        .bind(metadata.to_string())
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn metadata_bucket(pool: &PgPool, id: i64) -> Result<String, Box<dyn std::error::Error>> {
        let raw: String = sqlx::query_scalar(
            r#"
            SELECT metadata::text
              FROM activity.member_events
             WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
        let metadata: Value = serde_json::from_str(&raw)?;
        Ok(metadata
            .get("join_source_bucket")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn reclassify_flippt_twitch_und_respektiert_website_filter(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_join(pool, 9_100_001, "ABC123").await?;
        insert_join(pool, 9_100_002, "WEB999").await?;
        sqlx::query(
            r#"
            INSERT INTO bot.kv_store(ns, k, v)
            VALUES ('website_invites', 'main', '{"code":"WEB999"}')
            "#,
        )
        .execute(pool)
        .await?;

        let summary = sync_invites_and_reclassify(
            pool,
            vec![
                invite(" CoolStreamer ", " ABC123 "),
                invite("WebStreamer", "WEB999"),
            ],
            false,
        )
        .await?;

        assert_eq!(summary.flipped, 1);
        assert_eq!(summary.table_rows, 2);
        assert_eq!(metadata_bucket(pool, 9_100_001).await?, "twitch");
        assert_eq!(metadata_bucket(pool, 9_100_002).await?, "personal");
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn invite_upsert_bleibt_persistiert_wenn_reclassify_update_scheitert(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_join(pool, 9_100_101, "ABC123").await?;
        sqlx::query(
            r#"
            CREATE OR REPLACE FUNCTION activity.fail_twitch_invite_sync_update()
            RETURNS trigger
            LANGUAGE plpgsql
            AS $$
            BEGIN
                RAISE EXCEPTION 'forced reclassify failure';
            END;
            $$
            "#,
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            CREATE TRIGGER fail_twitch_invite_sync_update
            BEFORE UPDATE ON activity.member_events
            FOR EACH ROW
            EXECUTE FUNCTION activity.fail_twitch_invite_sync_update()
            "#,
        )
        .execute(pool)
        .await?;

        let result =
            sync_invites_and_reclassify(pool, vec![invite("CoolStreamer", "ABC123")], false).await;
        assert!(result.is_err(), "trigger must fail the reclassify update");

        let row = sqlx::query(
            r#"
            SELECT streamer_login, invite_code, invite_url, guild_id
              FROM bot.twitch_streamer_invites
             WHERE streamer_login = 'coolstreamer'
            "#,
        )
        .fetch_one(pool)
        .await?;
        let streamer_login: String = row.try_get("streamer_login")?;
        let invite_code: Option<String> = row.try_get("invite_code")?;
        let invite_url: Option<String> = row.try_get("invite_url")?;
        let guild_id: Option<i64> = row.try_get("guild_id")?;

        assert_eq!(streamer_login, "coolstreamer");
        assert_eq!(invite_code.as_deref(), Some("ABC123"));
        assert_eq!(invite_url.as_deref(), Some("https://discord.gg/ABC123"));
        assert_eq!(guild_id, Some(1));
        assert_eq!(metadata_bucket(pool, 9_100_101).await?, "personal");
        Ok(())
    }
}
