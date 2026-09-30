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
//!    invites` und spiegelt sie über die Twitch-User-ID; der Login bleibt
//!    veränderliches Anzeige-Metadatum.
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

use std::collections::HashMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use dl_activity::join_source::classify;
use serde::Deserialize;
use serde_json::{Value, json};
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
    #[serde(default)]
    twitch_user_id: Option<String>,
    #[serde(default)]
    channel_id: Option<i64>,
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

async fn lock_invite_sync_guilds(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entries: &[InviteEntry],
) -> anyhow::Result<()> {
    // Lock existing and incoming Guilds in one order, then reject stale discovery.
    let logins: Vec<String> = entries
        .iter()
        .map(|entry| entry.streamer_login.trim().to_lowercase())
        .filter(|login| !login.is_empty())
        .collect();
    let twitch_user_ids: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.twitch_user_id.clone())
        .collect();
    let new_guilds: Vec<i64> = entries
        .iter()
        .filter(|entry| !entry.streamer_login.trim().is_empty())
        .map(|entry| entry.guild_id)
        .collect();

    let mut guilds: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT guild_id FROM bot.twitch_streamer_invites
         WHERE guild_id IS NOT NULL
           AND (streamer_login = ANY($1::text[]) OR twitch_user_id = ANY($2::text[]))",
    )
    .bind(&logins)
    .bind(&twitch_user_ids)
    .fetch_all(&mut **tx)
    .await?;
    guilds.extend(new_guilds);
    guilds.sort_unstable();
    guilds.dedup();

    for guild_id in &guilds {
        dl_activity::qualified_invites::lock_changes(&mut **tx, *guild_id).await?;
    }

    let current_guilds: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT guild_id FROM bot.twitch_streamer_invites
         WHERE guild_id IS NOT NULL
           AND (streamer_login = ANY($1::text[]) OR twitch_user_id = ANY($2::text[]))",
    )
    .bind(&logins)
    .bind(&twitch_user_ids)
    .fetch_all(&mut **tx)
    .await?;
    if current_guilds
        .iter()
        .any(|guild_id| guilds.binary_search(guild_id).is_err())
    {
        anyhow::bail!("Twitch-Invite-Zuordnung änderte sich während Guild-Sperren erworben wurden");
    }
    Ok(())
}

async fn sync_invites_and_reclassify(
    pool: &PgPool,
    entries: Vec<InviteEntry>,
    dry_run: bool,
) -> anyhow::Result<SyncSummary> {
    {
        let mut populate_tx = pool.begin().await?;
        lock_invite_sync_guilds(&mut populate_tx, &entries).await?;

        for entry in &entries {
            let login = entry.streamer_login.trim().to_lowercase();
            if login.is_empty() {
                continue;
            }
            let invite_code = entry.invite_code.trim().to_string();
            let invite_url = entry.invite_url.trim().to_string();
            let created_at = parse_optional_timestamp(entry.created_at.as_deref())?;
            let last_sent_at = parse_optional_timestamp(entry.last_sent_at.as_deref())?;
            if let Some(twitch_user_id) = entry.twitch_user_id.as_deref() {
                sqlx::query(
                    "UPDATE bot.twitch_streamer_invites
                     SET streamer_login = '__legacy_unresolved__:' || streamer_login || ':'
                         || md5(ctid::text || clock_timestamp()::text)
                     WHERE streamer_login = $1 AND twitch_user_id IS NULL",
                )
                .bind(&login)
                .execute(&mut *populate_tx)
                .await?;

                sqlx::query(
                    "UPDATE bot.twitch_streamer_invites
                     SET streamer_login = '__legacy_recycled__:' || twitch_user_id || ':'
                         || md5(ctid::text || clock_timestamp()::text),
                         channel_id = NULL
                     WHERE streamer_login = $1 AND twitch_user_id IS NOT NULL
                       AND twitch_user_id <> $2",
                )
                .bind(&login)
                .bind(twitch_user_id)
                .execute(&mut *populate_tx)
                .await?;

                sqlx::query(
                    "UPDATE bot.twitch_streamer_invites
                     SET streamer_login = $1, guild_id = $2, invite_code = $3, invite_url = $4,
                         created_at = $5, last_sent_at = $6, channel_id = $8
                     WHERE twitch_user_id = $7",
                )
                .bind(&login)
                .bind(entry.guild_id)
                .bind(&invite_code)
                .bind(&invite_url)
                .bind(created_at)
                .bind(last_sent_at)
                .bind(twitch_user_id)
                .bind(entry.channel_id)
                .execute(&mut *populate_tx)
                .await?;
            }

            sqlx::query(
                "INSERT INTO bot.twitch_streamer_invites
                     (streamer_login, guild_id, invite_code, invite_url, created_at, last_sent_at, twitch_user_id, channel_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (streamer_login) DO UPDATE SET
                     guild_id = EXCLUDED.guild_id, invite_code = EXCLUDED.invite_code,
                     invite_url = EXCLUDED.invite_url, created_at = EXCLUDED.created_at,
                     last_sent_at = EXCLUDED.last_sent_at,
                     twitch_user_id = EXCLUDED.twitch_user_id,
                     channel_id = EXCLUDED.channel_id",
            )
            .bind(&login).bind(entry.guild_id).bind(invite_code).bind(invite_url)
            .bind(created_at).bind(last_sent_at).bind(&entry.twitch_user_id).bind(entry.channel_id)
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
         WHERE twitch_user_id IS NOT NULL AND channel_id IS NOT NULL
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

    let personal: Vec<(String, String)> = sqlx::query_as(
        "SELECT current.streamer_login, personal.invite_code
         FROM bot.twitch_personal_invites AS personal
         JOIN bot.twitch_streamer_invites AS current
           ON current.twitch_user_id = personal.streamer_twitch_user_id
          AND current.channel_id IS NOT NULL
         WHERE personal.revoked_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for (login, code) in personal {
        twitch_lookup
            .entry(code.to_ascii_lowercase())
            .or_insert(login);
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
    let config = dl_core::config::process_bot_config()?.snapshot();
    let base = config
        .runtime
        .bridges
        .twitch_api_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:8776".to_string());
    let token = dl_core::runtime_config::secret_value("TWITCH_INTERNAL_API_TOKEN")
        .ok_or_else(|| anyhow::anyhow!("TWITCH_INTERNAL_API_TOKEN fehlt im Infisical-Bootstrap"))?;
    let dry_run = config.twitch_invites.sync_dry_run;
    let parsed = reqwest::Url::parse(&base)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        })
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        anyhow::bail!(
            "Twitch-Sync benötigt eine numerische Loopback-Adresse ohne Zugangsdaten oder Pfad"
        );
    }
    let central_dsn = dl_central_db::dsn_from_env()?;
    let pool = dl_central_db::connect_pool(&central_dsn).await?;

    let url = format!(
        "{}/internal/twitch/v1/streamer-invites",
        base.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
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

    mod test_database {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test-support/peer_database.rs"
        ));
    }

    fn invite(login: &str, code: &str) -> InviteEntry {
        InviteEntry {
            streamer_login: login.to_string(),
            twitch_user_id: Some("123".to_string()),
            channel_id: Some(2),
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

    async fn insert_qualified_join(
        pool: &PgPool,
        id: i64,
        user_id: i64,
        invite_code: &str,
        joined_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            "INSERT INTO activity.twitch_invite_members
             (guild_id, user_id, first_join_id, first_joined_at, current_joined_at, prior_member)
             VALUES (1, $1, $2, $3, $3, FALSE)",
        )
        .bind(user_id)
        .bind(id)
        .bind(joined_at)
        .execute(pool)
        .await?;
        let metadata = json!({
            "join_source_bucket": "personal",
            "join_source_kind": "invite_link",
            "invite_code": invite_code,
            "discord_joined_at": joined_at.to_rfc3339(),
        });
        sqlx::query(
            "INSERT INTO activity.member_events(id, user_id, guild_id, event_type, occurred_at, metadata)
             VALUES ($1, $2, 1, 'join', $3, $4::jsonb)",
        )
        .bind(id)
        .bind(user_id)
        .bind(joined_at)
        .bind(metadata.to_string())
        .execute(pool)
        .await?;
        Ok(())
    }

    fn test_error(error: impl std::fmt::Display) -> Box<dyn std::error::Error> {
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            error.to_string(),
        ))
    }

    async fn assert_writer_reconcile_serialization(
        commit_writer: bool,
        join_id: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = test_database::database().await;
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO bot.twitch_streamer_invites
             (streamer_login, guild_id, invite_code, invite_url, created_at, twitch_user_id, channel_id)
             VALUES ('serial-login', 1, 'SERIAL_A', 'https://discord.gg/SERIAL_A', now(), '918273645201', 201)",
        )
        .execute(pool)
        .await?;

        let mut incoming = invite("serial-login", "SERIAL_B");
        incoming.twitch_user_id = Some("918273645201".into());
        incoming.channel_id = Some(202);
        incoming.guild_id = 2;
        let writer_pool = pool.clone();
        let (writer_ready, writer_ready_rx) = tokio::sync::oneshot::channel();
        let (probe_reader, probe_reader_rx) = tokio::sync::oneshot::channel();
        let (reader_waiting, reader_waiting_rx) = tokio::sync::oneshot::channel();
        let (release_writer, release_writer_rx) = tokio::sync::oneshot::channel();
        let writer = tokio::spawn(async move {
            let mut tx = writer_pool.begin().await?;
            lock_invite_sync_guilds(&mut tx, std::slice::from_ref(&incoming)).await?;
            let writer_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE bot.twitch_streamer_invites
                 SET guild_id = 2, invite_code = 'SERIAL_B', channel_id = 202
                 WHERE twitch_user_id = '918273645201'",
            )
            .execute(&mut *tx)
            .await?;
            let interval_end: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
                "SELECT valid_until FROM bot.twitch_streamer_invite_code_history
                 WHERE guild_id = 1 AND invite_code = 'SERIAL_A' AND twitch_user_id = '918273645201'",
            )
            .fetch_one(&mut *tx)
            .await?;
            writer_ready
                .send((writer_pid, interval_end))
                .map_err(|_| anyhow::anyhow!("serialization test receiver dropped"))?;
            probe_reader_rx
                .await
                .map_err(|_| anyhow::anyhow!("reconcile probe receiver dropped"))?;
            let reader_waiting_for_writer =
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let waiting: bool = sqlx::query_scalar(
                            "SELECT EXISTS (
                                 SELECT 1
                                 FROM pg_locks AS waiting
                                 JOIN pg_locks AS holding
                                   ON holding.locktype = waiting.locktype
                                  AND holding.database IS NOT DISTINCT FROM waiting.database
                                  AND holding.classid IS NOT DISTINCT FROM waiting.classid
                                  AND holding.objid IS NOT DISTINCT FROM waiting.objid
                                  AND holding.objsubid IS NOT DISTINCT FROM waiting.objsubid
                                 WHERE waiting.locktype = 'advisory'
                                   AND NOT waiting.granted
                                   AND waiting.pid <> pg_backend_pid()
                                   AND holding.pid = pg_backend_pid()
                                   AND holding.granted
                             )",
                        )
                        .fetch_one(&mut *tx)
                        .await?;
                        if waiting {
                            return Ok::<bool, sqlx::Error>(true);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await??;
            reader_waiting
                .send(reader_waiting_for_writer)
                .map_err(|_| anyhow::anyhow!("reconcile waiting receiver dropped"))?;
            let should_commit = release_writer_rx.await.unwrap_or(false);
            if should_commit {
                tx.commit().await?;
            } else {
                tx.rollback().await?;
            }
            Ok::<(), anyhow::Error>(())
        });
        let (writer_pid, interval_end) = writer_ready_rx.await.map_err(test_error)?;
        let held_advisory_locks: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_locks
             WHERE pid = $1 AND locktype = 'advisory' AND granted",
        )
        .bind(writer_pid)
        .fetch_one(pool)
        .await?;

        insert_qualified_join(
            pool,
            join_id,
            930_000 + join_id,
            "SERIAL_A",
            interval_end + chrono::Duration::milliseconds(1),
        )
        .await?;
        let reader_pool = pool.clone();
        let reconcile = tokio::spawn(async move {
            dl_activity::qualified_invites::reconcile_attribution(&reader_pool, 1).await
        });
        probe_reader
            .send(())
            .map_err(|_| test_error("reconcile probe receiver dropped"))?;
        let reader_is_blocked = reader_waiting_rx.await.map_err(test_error)?;
        release_writer.send(commit_writer).map_err(test_error)?;
        writer.await.map_err(test_error)?.map_err(test_error)?;
        let reconciliation_result = reconcile.await.map_err(test_error)?;
        if held_advisory_locks < 2 {
            return Err(test_error("writer did not lock both old and new Guilds"));
        }
        if !reader_is_blocked {
            return Err(test_error(
                "reconcile did not wait for the mapping writer's Guild locks",
            ));
        }
        reconciliation_result.map_err(test_error)?;

        let owner: Option<String> = sqlx::query_scalar(
            "SELECT streamer_twitch_user_id FROM bot.twitch_invite_joins WHERE join_id = $1",
        )
        .bind(join_id)
        .fetch_optional(pool)
        .await?;
        if commit_writer && owner.is_some() {
            return Err(test_error("committed code change attributed the old owner"));
        }
        if !commit_writer && owner.as_deref() != Some("918273645201") {
            return Err(test_error(
                "rolled-back code change did not retain the old owner",
            ));
        }
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
    async fn sync_uses_twitch_ids_and_replaces_incomplete_authority() -> anyhow::Result<()> {
        let db = test_database::database().await;
        let pool = db.pool();
        let mut legacy = invite("current-login", "LEGACYCODE");
        legacy.twitch_user_id = None;
        legacy.channel_id = None;
        legacy.guild_id = 918274;
        legacy.invite_url = "https://discord.gg/LEGACYCODE".into();
        let mut first = invite("former-login", "FIRSTCODE");
        first.twitch_user_id = Some("918273645001".into());
        first.channel_id = Some(111);
        first.guild_id = 918273;
        let mut renamed = invite("current-login", "RENAMEDCODE");
        renamed.twitch_user_id = Some("918273645001".into());
        renamed.channel_id = Some(222);
        renamed.guild_id = 918274;
        let mut recycled = invite("recycled-login", "OLDRECYCLE");
        recycled.twitch_user_id = Some("918273645002".into());
        recycled.channel_id = Some(333);
        recycled.guild_id = 918273;
        let mut new_owner = invite("recycled-login", "NEWRECYCLE");
        new_owner.twitch_user_id = Some("918273645003".into());
        new_owner.channel_id = None;
        new_owner.guild_id = 918273;
        let mut complete = invite("incomplete-login", "COMPLETECODE");
        complete.twitch_user_id = Some("918273645004".into());
        complete.channel_id = Some(444);
        complete.guild_id = 918273;
        let mut incomplete = invite("incomplete-login", "INCOMPLETECODE");
        incomplete.twitch_user_id = None;
        incomplete.channel_id = None;
        incomplete.guild_id = 918273;

        sqlx::query(
            "INSERT INTO bot.twitch_personal_invites
             (streamer_twitch_user_id, inviter_twitch_user_id, streamer_login, guild_id,
              channel_id, invite_code, invite_url)
             VALUES ('918273645001', '918273645099', 'former-login', 918273, 111,
                     'PERSONALID', 'https://discord.gg/PERSONALID')",
        )
        .execute(pool)
        .await?;

        sync_invites_and_reclassify(
            pool,
            vec![
                legacy, first, renamed, recycled, new_owner, complete, incomplete,
            ],
            false,
        )
        .await?;

        let renamed_row: (String, Option<String>, Option<i64>, Option<String>, i64) =
            sqlx::query_as(
                "SELECT streamer_login, twitch_user_id, channel_id, invite_code, guild_id
             FROM bot.twitch_streamer_invites WHERE twitch_user_id = '918273645001'",
            )
            .fetch_one(pool)
            .await?;
        assert_eq!(
            renamed_row,
            (
                "current-login".into(),
                Some("918273645001".into()),
                Some(222),
                Some("RENAMEDCODE".into()),
                918274,
            )
        );

        let retired_row: (String, Option<String>, Option<String>, Option<i64>) = sqlx::query_as(
            "SELECT streamer_login, invite_code, invite_url, channel_id
                 FROM bot.twitch_streamer_invites
                 WHERE twitch_user_id = '918273645002'",
        )
        .fetch_one(pool)
        .await?;
        assert!(retired_row.0.starts_with("__legacy_recycled__:"));
        assert_eq!(retired_row.1.as_deref(), Some("OLDRECYCLE"));
        assert_eq!(
            retired_row.2.as_deref(),
            Some("https://discord.gg/OLDRECYCLE")
        );
        assert_eq!(retired_row.3, None);

        let legacy_row: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT streamer_login, twitch_user_id, invite_code, invite_url
                 FROM bot.twitch_streamer_invites WHERE invite_code = 'LEGACYCODE'",
        )
        .fetch_one(pool)
        .await?;
        assert!(legacy_row
            .0
            .starts_with("__legacy_unresolved__:current-login:"));
        assert_eq!(legacy_row.1, None);
        assert_eq!(legacy_row.2.as_deref(), Some("LEGACYCODE"));
        assert_eq!(
            legacy_row.3.as_deref(),
            Some("https://discord.gg/LEGACYCODE")
        );

        let missing_channel: (Option<String>, Option<i64>) = sqlx::query_as(
            "SELECT twitch_user_id, channel_id FROM bot.twitch_streamer_invites
             WHERE twitch_user_id = '918273645003'",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(missing_channel, (Some("918273645003".into()), None));

        let incomplete_row: (Option<String>, Option<i64>) = sqlx::query_as(
            "SELECT twitch_user_id, channel_id FROM bot.twitch_streamer_invites
             WHERE streamer_login = 'incomplete-login'",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(incomplete_row, (None, None));

        let personal_owner: (String, String) = sqlx::query_as(
            "SELECT streamer_twitch_user_id, inviter_twitch_user_id
             FROM bot.twitch_personal_invites WHERE invite_code = 'PERSONALID'",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(
            personal_owner,
            ("918273645001".into(), "918273645099".into())
        );
        Ok(())
    }

    #[tokio::test]
    async fn reconcile_keeps_historical_code_owners_across_rename_recycle_and_legacy_collision(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = test_database::database().await;
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO bot.twitch_streamer_invites
             (streamer_login, guild_id, invite_code, invite_url, created_at, twitch_user_id, channel_id)
             VALUES
               ('old-rename', 1, 'CODE_A', 'https://discord.gg/CODE_A', now(), '918273645101', 101),
               ('recycled-login', 1, 'RECYCLE_A', 'https://discord.gg/RECYCLE_A', now(), '918273645102', 102),
               ('legacy-login', 1, 'UNKNOWN_CODE', 'https://discord.gg/UNKNOWN_CODE', now(), NULL, NULL),
               ('legacy-same', 1, 'SAME_CODE', 'https://discord.gg/SAME_CODE', now(), NULL, NULL),
               ('recycled-same', 1, 'SAME_RECYCLE', 'https://discord.gg/SAME_RECYCLE', now(), '918273645105', 105)",
        )
        .execute(pool)
        .await?;

        let code_a_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'CODE_A' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        let recycle_a_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'RECYCLE_A' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        let unknown_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'UNKNOWN_CODE' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        let same_code_legacy_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'SAME_CODE' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        let same_code_recycled_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'SAME_RECYCLE' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        insert_qualified_join(pool, 910_001, 920_001, "CODE_A", code_a_start).await?;
        insert_qualified_join(pool, 910_002, 920_002, "RECYCLE_A", recycle_a_start).await?;
        insert_qualified_join(
            pool,
            910_003,
            920_003,
            "UNKNOWN_CODE",
            unknown_start + chrono::Duration::milliseconds(1),
        )
        .await?;
        insert_qualified_join(
            pool,
            910_009,
            920_009,
            "SAME_CODE",
            same_code_legacy_start + chrono::Duration::milliseconds(1),
        )
        .await?;
        insert_qualified_join(
            pool,
            910_010,
            920_010,
            "SAME_RECYCLE",
            same_code_recycled_start,
        )
        .await?;

        let mut renamed = invite("new-rename", "CODE_B");
        renamed.twitch_user_id = Some("918273645101".into());
        renamed.channel_id = Some(111);
        let mut recycled_owner = invite("recycled-login", "RECYCLE_B");
        recycled_owner.twitch_user_id = Some("918273645103".into());
        recycled_owner.channel_id = Some(103);
        let mut legacy_replacement = invite("legacy-login", "NEW_CODE");
        legacy_replacement.twitch_user_id = Some("918273645104".into());
        legacy_replacement.channel_id = Some(104);
        let mut same_code_legacy = invite("legacy-same", "SAME_CODE");
        same_code_legacy.twitch_user_id = Some("918273645106".into());
        same_code_legacy.channel_id = Some(106);
        let mut same_code_recycled = invite("recycled-same", "SAME_RECYCLE");
        same_code_recycled.twitch_user_id = Some("918273645107".into());
        same_code_recycled.channel_id = Some(107);
        sync_invites_and_reclassify(
            pool,
            vec![
                renamed,
                recycled_owner,
                legacy_replacement,
                same_code_legacy,
                same_code_recycled,
            ],
            false,
        )
        .await?;

        let code_a_end: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_until FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'CODE_A' AND twitch_user_id = '918273645101'",
        )
        .fetch_one(pool)
        .await?;
        let code_a_history: (
            String,
            Option<i64>,
            Option<String>,
            Option<i64>,
            bool,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT source_login_snapshot, guild_id, twitch_user_id, channel_id,
                    attribution_safe, valid_until
             FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'CODE_A'",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(
            code_a_history,
            (
                "old-rename".into(),
                Some(1),
                Some("918273645101".into()),
                Some(101),
                true,
                Some(code_a_end.clone()),
            )
        );
        let recycle_a_end: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_until FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'RECYCLE_A' AND twitch_user_id = '918273645102'",
        )
        .fetch_one(pool)
        .await?;
        let code_b_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'CODE_B' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        let recycle_b_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'RECYCLE_B' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        let unknown_history_end: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_until FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'UNKNOWN_CODE'
               AND source_login_snapshot = 'legacy-login' AND twitch_user_id IS NULL
               AND valid_until IS NOT NULL",
        )
        .fetch_one(pool)
        .await?;
        let same_code_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'SAME_CODE' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        let same_recycle_start: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT valid_from FROM bot.twitch_streamer_invite_code_history
             WHERE guild_id = 1 AND invite_code = 'SAME_RECYCLE' AND valid_until IS NULL",
        )
        .fetch_one(pool)
        .await?;
        insert_qualified_join(pool, 910_004, 920_004, "CODE_A", code_a_end).await?;
        insert_qualified_join(pool, 910_005, 920_005, "RECYCLE_A", recycle_a_end).await?;
        insert_qualified_join(pool, 910_006, 920_006, "CODE_B", code_b_start).await?;
        insert_qualified_join(pool, 910_007, 920_007, "RECYCLE_B", recycle_b_start).await?;
        insert_qualified_join(pool, 910_008, 920_008, "UNKNOWN_CODE", unknown_history_end).await?;
        insert_qualified_join(pool, 910_011, 920_011, "SAME_CODE", same_code_start).await?;
        insert_qualified_join(pool, 910_012, 920_012, "SAME_RECYCLE", same_recycle_start).await?;

        dl_activity::qualified_invites::reconcile_attribution(pool, 1).await?;
        let owners: Vec<(i64, Option<String>, String)> = sqlx::query_as(
            "SELECT join_id, streamer_twitch_user_id, streamer_login
             FROM bot.twitch_invite_joins WHERE join_id BETWEEN 910001 AND 910012
             ORDER BY join_id",
        )
        .fetch_all(pool)
        .await?;
        assert_eq!(
            owners,
            vec![
                (910_001, Some("918273645101".into()), "new-rename".into()),
                (
                    910_002,
                    Some("918273645102".into()),
                    "recycled-login".into()
                ),
                (910_006, Some("918273645101".into()), "new-rename".into()),
                (
                    910_007,
                    Some("918273645103".into()),
                    "recycled-login".into()
                ),
                (910_010, Some("918273645105".into()), "recycled-same".into()),
                (910_011, Some("918273645106".into()), "legacy-same".into()),
                (910_012, Some("918273645107".into()), "recycled-same".into()),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn reconcile_waits_for_guild_history_writer_commit(
    ) -> Result<(), Box<dyn std::error::Error>> {
        assert_writer_reconcile_serialization(true, 930_021).await
    }

    #[tokio::test]
    async fn reconcile_waits_for_guild_history_writer_rollback(
    ) -> Result<(), Box<dyn std::error::Error>> {
        assert_writer_reconcile_serialization(false, 930_022).await
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn reclassify_flippt_twitch_und_respektiert_website_filter()
    -> Result<(), Box<dyn std::error::Error>> {
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
    async fn invite_upsert_bleibt_persistiert_wenn_reclassify_update_scheitert()
    -> Result<(), Box<dyn std::error::Error>> {
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
