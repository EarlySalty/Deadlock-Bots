use std::num::TryFromIntError;

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

const MEMBER_EVENTS_ID_LOCK_KEY: i64 = 0x4143_5449_4d45_5645;
const TEXT_CONVERSATION_LOG_ID_LOCK_KEY: i64 = 0x4143_5454_4558_5443;

#[derive(Debug, thiserror::Error)]
pub enum ActivityDbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("Discord-ID {field}={value} passt nicht in PostgreSQL BIGINT")]
    DiscordIdOutOfRange {
        field: &'static str,
        value: u64,
        source: TryFromIntError,
    },
    #[error("Wert {field}={value} passt nicht in PostgreSQL INTEGER")]
    IntOutOfRange {
        field: &'static str,
        value: i64,
        source: TryFromIntError,
    },
    #[error("Unix-Zeitstempel ist ausserhalb des chrono-Bereichs: {0}")]
    TimestampOutOfRange(i64),
    #[error("ungueltiger JSON-Wert fuer {field}: {source}")]
    InvalidJson {
        field: &'static str,
        source: serde_json::Error,
    },
}

pub type ActivityDbResult<T> = Result<T, ActivityDbError>;

pub fn discord_id_to_i64(value: u64, field: &'static str) -> ActivityDbResult<i64> {
    i64::try_from(value).map_err(|source| ActivityDbError::DiscordIdOutOfRange {
        field,
        value,
        source,
    })
}

pub fn i64_to_u64(value: i64, field: &'static str) -> Option<u64> {
    match u64::try_from(value) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::warn!(%err, field, value, "PostgreSQL BIGINT ist keine gueltige Discord-ID");
            None
        }
    }
}

pub fn i64_to_i32(value: i64, field: &'static str) -> ActivityDbResult<i32> {
    i32::try_from(value).map_err(|source| ActivityDbError::IntOutOfRange {
        field,
        value,
        source,
    })
}

pub fn utc_from_unix_seconds(value: i64) -> ActivityDbResult<DateTime<Utc>> {
    DateTime::from_timestamp(value, 0).ok_or(ActivityDbError::TimestampOutOfRange(value))
}

pub fn validate_json_text(
    raw: String,
    field: &'static str,
    fallback: &'static str,
) -> ActivityDbResult<String> {
    let candidate = if raw.trim().is_empty() {
        fallback.to_string()
    } else {
        raw
    };
    serde_json::from_str::<serde_json::Value>(&candidate)
        .map(|_| candidate)
        .map_err(|source| ActivityDbError::InvalidJson { field, source })
}

pub async fn next_text_conversation_id(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<i64, sqlx::Error> {
    lock_key(tx, TEXT_CONVERSATION_LOG_ID_LOCK_KEY).await?;
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int8 AS "next_id!"
        FROM activity.text_conversation_log
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

pub async fn lock_member_events(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    lock_key(tx, MEMBER_EVENTS_ID_LOCK_KEY).await
}

pub async fn next_member_event_id_in_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int8 AS "next_id!"
        FROM activity.member_events
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

async fn lock_key(tx: &mut Transaction<'_, Postgres>, key: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        SELECT 1 AS "locked!"
        FROM pg_advisory_xact_lock($1)
        "#,
        key,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}
