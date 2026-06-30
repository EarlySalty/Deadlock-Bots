use std::time::Duration;

use sqlx::postgres::PgPoolOptions;

use crate::CentralDbError;

const CENTRAL_DSN_ENV: &str = "DEADLOCK_CENTRAL_DSN";
const MAX_CONNECTIONS: u32 = 8;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn connect_pool(dsn: &str) -> Result<sqlx::PgPool, CentralDbError> {
    let pool = PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect(dsn)
        .await?;

    Ok(pool)
}

pub fn dsn_from_env() -> Result<String, CentralDbError> {
    std::env::var(CENTRAL_DSN_ENV).map_err(|source| CentralDbError::EnvVar {
        name: CENTRAL_DSN_ENV,
        source,
    })
}
