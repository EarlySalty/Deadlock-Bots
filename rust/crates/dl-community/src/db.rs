use std::num::TryFromIntError;

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

#[derive(Debug, thiserror::Error)]
pub enum CommunityDbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Central(#[from] dl_central_db::CentralDbError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
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
    #[error("PostgreSQL BIGINT {field}={value} passt nicht in u64")]
    BigIntOutOfRange {
        field: &'static str,
        value: i64,
        source: TryFromIntError,
    },
    #[error("Unix-Zeitstempel ist ausserhalb des chrono-Bereichs: {0}")]
    TimestampOutOfRange(i64),
}

pub type CommunityDbResult<T> = Result<T, CommunityDbError>;

pub fn u64_to_i64(value: u64, field: &'static str) -> CommunityDbResult<i64> {
    i64::try_from(value).map_err(|source| CommunityDbError::DiscordIdOutOfRange {
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

pub fn pg_i64_to_u64(value: i64, field: &'static str) -> CommunityDbResult<u64> {
    u64::try_from(value).map_err(|source| CommunityDbError::BigIntOutOfRange {
        field,
        value,
        source,
    })
}

pub fn i64_to_i32(value: i64, field: &'static str) -> CommunityDbResult<i32> {
    i32::try_from(value).map_err(|source| CommunityDbError::IntOutOfRange {
        field,
        value,
        source,
    })
}

pub fn utc_from_unix(value: i64) -> CommunityDbResult<DateTime<Utc>> {
    DateTime::from_timestamp(value, 0).ok_or(CommunityDbError::TimestampOutOfRange(value))
}

pub fn unix_from_utc(value: DateTime<Utc>) -> i64 {
    value.timestamp()
}

pub async fn advisory_lock(
    tx: &mut Transaction<'_, Postgres>,
    key: i64,
) -> Result<(), sqlx::Error> {
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
