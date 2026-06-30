use std::num::TryFromIntError;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

pub type VoiceDbResult<T> = Result<T, VoiceDbError>;

#[derive(Debug, thiserror::Error)]
pub enum VoiceDbError {
    #[error(transparent)]
    Central(#[from] dl_central_db::CentralDbError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{field}={value} passt nicht in PostgreSQL BIGINT")]
    U64ToI64 {
        field: &'static str,
        value: u64,
        source: TryFromIntError,
    },
    #[error("{field}={value} passt nicht in Discord-u64")]
    I64ToU64 {
        field: &'static str,
        value: i64,
        source: TryFromIntError,
    },
    #[error("{field}={value} passt nicht in PostgreSQL INTEGER")]
    I64ToI32 {
        field: &'static str,
        value: i64,
        source: TryFromIntError,
    },
    #[error("ungueltiger Unix-Zeitstempel fuer {field}: {value}")]
    InvalidTimestamp { field: &'static str, value: i64 },
}

pub fn u64_to_i64(field: &'static str, value: u64) -> VoiceDbResult<i64> {
    i64::try_from(value).map_err(|source| VoiceDbError::U64ToI64 {
        field,
        value,
        source,
    })
}

pub fn opt_u64_to_i64(field: &'static str, value: Option<u64>) -> VoiceDbResult<Option<i64>> {
    value.map(|value| u64_to_i64(field, value)).transpose()
}

pub fn i64_to_u64(field: &'static str, value: i64) -> VoiceDbResult<u64> {
    u64::try_from(value).map_err(|source| VoiceDbError::I64ToU64 {
        field,
        value,
        source,
    })
}

pub fn opt_i64_to_u64(field: &'static str, value: Option<i64>) -> VoiceDbResult<Option<u64>> {
    value.map(|value| i64_to_u64(field, value)).transpose()
}

pub fn i64_to_i32(field: &'static str, value: i64) -> VoiceDbResult<i32> {
    i32::try_from(value).map_err(|source| VoiceDbError::I64ToI32 {
        field,
        value,
        source,
    })
}

pub fn unix_to_utc(field: &'static str, value: i64) -> VoiceDbResult<DateTime<Utc>> {
    Utc.timestamp_opt(value, 0)
        .single()
        .ok_or(VoiceDbError::InvalidTimestamp { field, value })
}

pub fn naive_utc(value: NaiveDateTime) -> DateTime<Utc> {
    DateTime::from_naive_utc_and_offset(value, Utc)
}
