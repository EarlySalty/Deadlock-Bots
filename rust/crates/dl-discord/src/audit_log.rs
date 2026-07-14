use std::{fmt::Display, future::Future, num::ParseIntError, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use serenity::{
    all::GuildId,
    http::{LightMethod, Request, Route},
};
use sqlx::PgPool;

use crate::DiscordAdapter;

const DISCORD_EPOCH_MILLIS: i64 = 1_420_070_400_000;
const PAGE_SIZE: u8 = 100;
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, thiserror::Error)]
enum AuditLogError {
    #[error("ungueltige Discord-ID in {field}")]
    InvalidId {
        field: &'static str,
        #[source]
        source: ParseIntError,
    },
    #[error("Discord-ID {field} passt nicht in PostgreSQL bigint")]
    IdOutOfRange { field: &'static str },
    #[error("ungueltiger Discord-Snowflake-Zeitstempel")]
    InvalidTimestamp,
    #[error(transparent)]
    Discord(#[from] Box<serenity::Error>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    CentralDatabase(#[from] dl_central_db::CentralDbError),
}

#[derive(Debug, Deserialize)]
struct RawAuditLogEntry {
    id: String,
    action_type: u8,
    user_id: Option<String>,
    target_id: Option<String>,
    changes: Option<Value>,
    options: Option<Value>,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAuditUser {
    id: String,
    username: Option<String>,
    global_name: Option<String>,
    avatar: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAuditLogPage {
    #[serde(rename = "audit_log_entries")]
    entries: Vec<RawAuditLogEntry>,
    #[serde(default)]
    users: Vec<RawAuditUser>,
}

#[derive(Debug)]
struct AuditLogRow {
    entry_id: i64,
    guild_id: i64,
    action_type: i32,
    user_id: Option<i64>,
    target_id: Option<i64>,
    changes: Option<Value>,
    options: Option<Value>,
    reason: Option<String>,
    occurred_at: DateTime<Utc>,
}

async fn upsert_users_best_effort<E, F, Fut>(users: Vec<RawAuditUser>, mut upsert: F)
where
    E: Display,
    F: FnMut(RawAuditUser) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    for user in users {
        let user_id = user.id.clone();
        if let Err(err) = upsert(user).await {
            tracing::warn!(%err, user_id, "core.users-Upsert aus Audit-Log fehlgeschlagen");
        }
    }
}

async fn upsert_audit_user(pool: &PgPool, user: &RawAuditUser) -> Result<(), AuditLogError> {
    let discord_id = parse_id(&user.id, "audit_user_id")?;
    let username = user
        .username
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let global_name = user
        .global_name
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let avatar = user
        .avatar
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    dl_central_db::upsert_user(pool, discord_id, username, global_name, avatar).await?;
    Ok(())
}

fn parse_id(value: &str, field: &'static str) -> Result<i64, AuditLogError> {
    let value = value
        .parse::<u64>()
        .map_err(|source| AuditLogError::InvalidId { field, source })?;
    i64::try_from(value).map_err(|_| AuditLogError::IdOutOfRange { field })
}

fn row_from_entry(guild_id: u64, entry: RawAuditLogEntry) -> Result<AuditLogRow, AuditLogError> {
    let entry_id = parse_id(&entry.id, "entry_id")?;
    let guild_id =
        i64::try_from(guild_id).map_err(|_| AuditLogError::IdOutOfRange { field: "guild_id" })?;
    let occurred_at = DateTime::from_timestamp_millis(
        DISCORD_EPOCH_MILLIS
            .checked_add(entry_id >> 22)
            .ok_or(AuditLogError::InvalidTimestamp)?,
    )
    .ok_or(AuditLogError::InvalidTimestamp)?;

    Ok(AuditLogRow {
        entry_id,
        guild_id,
        action_type: i32::from(entry.action_type),
        user_id: entry
            .user_id
            .as_deref()
            .map(|id| parse_id(id, "user_id"))
            .transpose()?,
        target_id: entry
            .target_id
            .as_deref()
            .map(|id| parse_id(id, "target_id"))
            .transpose()?,
        changes: entry.changes,
        options: entry.options,
        reason: entry.reason,
        occurred_at,
    })
}

async fn insert_entry(pool: &PgPool, row: &AuditLogRow) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO core.discord_audit_log(
            entry_id, guild_id, action_type, user_id, target_id,
            changes, options, reason, occurred_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (entry_id) DO NOTHING
        "#,
    )
    .bind(row.entry_id)
    .bind(row.guild_id)
    .bind(row.action_type)
    .bind(row.user_id)
    .bind(row.target_id)
    .bind(row.changes.as_ref())
    .bind(row.options.as_ref())
    .bind(row.reason.as_deref())
    .bind(row.occurred_at)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

fn order_oldest_first(rows: &mut [AuditLogRow]) {
    rows.reverse();
}

fn sync_cursor(backfill_pending: bool, highest_entry_id: Option<i64>) -> Option<i64> {
    if backfill_pending {
        None
    } else {
        highest_entry_id
    }
}

async fn highest_entry_id(pool: &PgPool, guild_id: u64) -> Result<Option<i64>, AuditLogError> {
    let guild_id =
        i64::try_from(guild_id).map_err(|_| AuditLogError::IdOutOfRange { field: "guild_id" })?;
    Ok(
        sqlx::query_scalar("SELECT MAX(entry_id) FROM core.discord_audit_log WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn sync_once(
    pool: &PgPool,
    adapter: &DiscordAdapter,
    guild_id: u64,
    highest_entry_id: Option<i64>,
) -> Result<u64, AuditLogError> {
    let mut before: Option<String> = None;
    let mut pending = Vec::new();

    loop {
        let mut params = vec![("limit", PAGE_SIZE.to_string())];
        if let Some(before) = &before {
            params.push(("before", before.clone()));
        }
        let logs: RawAuditLogPage = adapter
            .http
            .fire(
                Request::new(
                    Route::GuildAuditLogs {
                        guild_id: GuildId::new(guild_id),
                    },
                    LightMethod::Get,
                )
                .params(Some(params)),
            )
            .await
            .map_err(Box::new)?;
        upsert_users_best_effort(logs.users, |user| async move {
            upsert_audit_user(pool, &user).await
        })
        .await;
        let page_len = logs.entries.len();
        let Some(oldest_id) = logs.entries.last().map(|entry| entry.id.clone()) else {
            break;
        };
        let mut reached_known_entry = false;

        for entry in logs.entries {
            let row = row_from_entry(guild_id, entry)?;
            if highest_entry_id.is_some_and(|known| row.entry_id <= known) {
                reached_known_entry = true;
                continue;
            }
            pending.push(row);
        }

        if reached_known_entry || page_len < usize::from(PAGE_SIZE) {
            break;
        }
        before = Some(oldest_id);
    }

    order_oldest_first(&mut pending);
    let mut inserted = 0;
    for row in pending {
        inserted += insert_entry(pool, &row).await?;
    }
    Ok(inserted)
}

pub fn spawn(
    pool: PgPool,
    adapter: Arc<DiscordAdapter>,
    guild_id: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        let mut backfill_pending = true;
        loop {
            interval.tick().await;
            let result = match highest_entry_id(&pool, guild_id).await {
                Ok(highest) => {
                    sync_once(
                        &pool,
                        &adapter,
                        guild_id,
                        sync_cursor(backfill_pending, highest),
                    )
                    .await
                }
                Err(err) => Err(err),
            };
            match result {
                Ok(inserted) => {
                    tracing::info!(
                        inserted,
                        guild_id,
                        backfill = backfill_pending,
                        "Discord-Audit-Log synchronisiert"
                    );
                    backfill_pending = false;
                }
                Err(err) => tracing::warn!(%err, guild_id, "Discord-Audit-Log-Sync fehlgeschlagen"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use serde_json::json;

    use super::*;

    fn entry_json() -> serde_json::Value {
        json!({
            "id": "1751000500000000000",
            "action_type": 22,
            "user_id": "123456789",
            "target_id": "987654321",
            "changes": [{"key": "nick", "old_value": "Alt", "new_value": "Neu"}],
            "options": {"channel_id": "456789123", "count": "1"},
            "reason": "Vertragstest"
        })
    }

    fn parse_test_entry(guild_id: u64, value: Value) -> Result<AuditLogRow, AuditLogError> {
        row_from_entry(guild_id, serde_json::from_value(value)?)
    }

    #[test]
    fn audit_entry_json_becomes_row_with_snowflake_timestamp() {
        let row =
            parse_test_entry(1_289_721_245_281_292_288, entry_json()).expect("valid audit entry");

        assert_eq!(row.entry_id, 1_751_000_500_000_000_000);
        assert_eq!(row.guild_id, 1_289_721_245_281_292_288);
        assert_eq!(row.action_type, 22);
        assert_eq!(row.user_id, Some(123_456_789));
        assert_eq!(row.target_id, Some(987_654_321));
        assert_eq!(
            row.changes.as_ref().and_then(|value| value.get(0)),
            Some(&json!({"key": "nick", "old_value": "Alt", "new_value": "Neu"}))
        );
        assert_eq!(
            row.options.as_ref().and_then(|value| value.get("count")),
            Some(&json!("1"))
        );
        assert_eq!(row.reason.as_deref(), Some("Vertragstest"));
        assert_eq!(
            row.occurred_at.timestamp_millis(),
            1_420_070_400_000 + (1_751_000_500_000_000_000_i64 >> 22)
        );
    }

    #[test]
    fn audit_log_page_accepts_null_user_id() {
        let page: RawAuditLogPage = serde_json::from_value(json!({
            "audit_log_entries": [{
                "id": "1751000500000000000",
                "action_type": 1,
                "user_id": null,
                "target_id": null
            }],
            "users": [{
                "id": "123456789",
                "username": "audit-user",
                "global_name": "Audit User",
                "avatar": "avatar-hash"
            }]
        }))
        .expect("audit page with system entry");

        assert_eq!(page.users.len(), 1);
        assert_eq!(page.users[0].id, "123456789");
        assert_eq!(page.users[0].username.as_deref(), Some("audit-user"));
        assert_eq!(page.users[0].global_name.as_deref(), Some("Audit User"));
        assert_eq!(page.users[0].avatar.as_deref(), Some("avatar-hash"));

        let row = row_from_entry(
            1_289_721_245_281_292_288,
            page.entries.into_iter().next().expect("one entry"),
        )
        .expect("system audit entry");
        assert_eq!(row.user_id, None);
    }

    #[test]
    fn pending_backfill_is_written_oldest_first() {
        let mut rows = [
            parse_test_entry(1, json!({"id": "300", "action_type": 1})).expect("newest"),
            parse_test_entry(1, json!({"id": "200", "action_type": 1})).expect("middle"),
            parse_test_entry(1, json!({"id": "100", "action_type": 1})).expect("oldest"),
        ];

        order_oldest_first(&mut rows);

        assert_eq!(
            rows.map(|row| row.entry_id),
            [100, 200, 300],
            "a partial DB failure must leave MAX(entry_id) at a resumable cursor"
        );
    }

    #[test]
    fn startup_backfill_ignores_existing_max_until_complete() {
        assert_eq!(sync_cursor(true, Some(300)), None);
        assert_eq!(sync_cursor(false, Some(300)), Some(300));
    }

    #[tokio::test]
    async fn failed_user_upsert_does_not_stop_following_users() {
        let page: RawAuditLogPage = serde_json::from_value(json!({
            "audit_log_entries": [],
            "users": [
                {"id": "1", "username": "fails"},
                {"id": "2", "username": "continues"}
            ]
        }))
        .expect("audit users");
        let calls = Arc::new(AtomicUsize::new(0));

        upsert_users_best_effort(page.users, {
            let calls = Arc::clone(&calls);
            move |user| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    if user.id == "1" {
                        Err("simulierter Upsert-Fehler")
                    } else {
                        Ok(())
                    }
                }
            }
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn audit_user_upsert_preserves_names_and_raw_when_new_values_are_empty() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("central test db");
        let discord_id = 9_101_000_020_i64;
        let original_raw = json!({"source": "existing"});
        sqlx::query(
            "INSERT INTO core.users (discord_id, username, global_name, raw) VALUES ($1, $2, $3, $4)",
        )
        .bind(discord_id)
        .bind("existing-user")
        .bind("Existing User")
        .bind(&original_raw)
        .execute(db.pool())
        .await
        .expect("seed core user");
        let page: RawAuditLogPage = serde_json::from_value(json!({
            "audit_log_entries": [],
            "users": [{
                "id": discord_id.to_string(),
                "username": null,
                "global_name": "",
                "avatar": null
            }]
        }))
        .expect("audit user");

        upsert_audit_user(db.pool(), &page.users[0])
            .await
            .expect("upsert audit user");

        let stored: (Option<String>, Option<String>, Value) = sqlx::query_as(
            "SELECT username, global_name, raw FROM core.users WHERE discord_id = $1",
        )
        .bind(discord_id)
        .fetch_one(db.pool())
        .await
        .expect("read core user");
        assert_eq!(stored.0.as_deref(), Some("existing-user"));
        assert_eq!(stored.1.as_deref(), Some("Existing User"));
        assert_eq!(stored.2, original_raw);
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn duplicate_entry_is_inserted_once() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("central test db");
        let row =
            parse_test_entry(1_289_721_245_281_292_288, entry_json()).expect("valid audit entry");

        assert_eq!(
            insert_entry(db.pool(), &row).await.expect("first insert"),
            1
        );
        assert_eq!(
            insert_entry(db.pool(), &row).await.expect("second insert"),
            0
        );

        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core.discord_audit_log WHERE entry_id = $1")
                .bind(row.entry_id)
                .fetch_one(db.pool())
                .await
                .expect("count audit entries");
        assert_eq!(count, 1);
    }
}
