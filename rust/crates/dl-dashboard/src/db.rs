use std::num::TryFromIntError;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

#[derive(Debug, thiserror::Error)]
pub enum DashboardDbError {
    #[error(transparent)]
    RuntimeGate(#[from] dl_central_db::scrim_runtime::ScrimRuntimeGateError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Central(#[from] dl_central_db::CentralDbError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("Unix-Zeitstempel ist ausserhalb des chrono-Bereichs: {0}")]
    TimestampOutOfRange(i64),
    #[error("Wert {field}={value} passt nicht in PostgreSQL INTEGER")]
    IntOutOfRange {
        field: &'static str,
        value: i64,
        source: TryFromIntError,
    },
    #[error("Eindeutigkeitsverletzung fuer {0}")]
    UniqueViolation(&'static str),
}

pub type DashboardDbResult<T> = Result<T, DashboardDbError>;

/// Lokale Integrationstests nutzen Peer-Auth, keine Secrets oder ENV-Konfiguration.
#[cfg(all(test, feature = "testing"))]
pub async fn test_pool() -> Result<dl_central_db::TestDb, dl_central_db::CentralDbError> {
    #[derive(serde::Deserialize)]
    struct TestConfig {
        socket: String,
        user: String,
        database: String,
    }
    let config: TestConfig = serde_json::from_str(include_str!("../tests/postgres.json"))
        .map_err(|err| dl_central_db::CentralDbError::TestHarness(err.to_string()))?;
    let options = sqlx::postgres::PgConnectOptions::new()
        .host(&config.socket)
        .username(&config.user)
        .database(&config.database);
    dl_central_db::testing::test_pool_with_options(options).await
}

pub fn unix_to_utc(value: i64) -> DashboardDbResult<DateTime<Utc>> {
    DateTime::from_timestamp(value, 0).ok_or(DashboardDbError::TimestampOutOfRange(value))
}

pub fn utc_to_unix(value: Option<DateTime<Utc>>) -> Option<i64> {
    value.map(|dt| dt.timestamp())
}

pub fn utc_to_sqlite_text(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|dt| dt.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn utc_to_json_unix(value: Option<DateTime<Utc>>) -> Value {
    utc_to_unix(value).map(Value::from).unwrap_or(Value::Null)
}

pub fn i64_to_i32(value: i64, field: &'static str) -> DashboardDbResult<i32> {
    i32::try_from(value).map_err(|source| DashboardDbError::IntOutOfRange {
        field,
        value,
        source,
    })
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
