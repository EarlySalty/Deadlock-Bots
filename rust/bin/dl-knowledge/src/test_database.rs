//! Optional normal JSON configuration for the existing isolated database harness.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{path::Path, str::FromStr};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    database_url: String,
}

pub async fn pool() -> Result<dl_central_db::TestDb> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/database.local.json");
    pool_with_config_path(&path).await
}

async fn pool_with_config_path(path: &Path) -> Result<dl_central_db::TestDb> {
    // An explicit wrapper DSN must win even over an existing local peer configuration.
    // Empty or invalid values must fail in the central harness, not fall back locally.
    if std::env::var_os("CENTRAL_TEST_DSN").is_some() {
        return Ok(dl_central_db::test_pool().await?);
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(dl_central_db::test_pool().await?);
        }
        Err(error) => return Err(error).context("Lokale Testdatenbank-Konfiguration lesen"),
    };
    let config: Config = serde_json::from_slice(&bytes)?;
    let url = url::Url::parse(&config.database_url)?;
    ensure!(matches!(url.scheme(), "postgres" | "postgresql") && url.password().is_none()
        && url.host_str().is_none() && url.path().starts_with("/dl_knowledge_eval_")
        && url.query_pairs().all(|(key,value)| key == "host" && value.starts_with('/')),
        "Lokale Tests benötigen explizite Peer-Verbindung zur eigenen dl_knowledge_eval_* Testinstanz");
    let options = sqlx::postgres::PgConnectOptions::from_str(&config.database_url)?;
    Ok(dl_central_db::testing::test_pool_with_options(options).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn explicit_wrapper_dsn_wins_over_local_database_configuration() {
        let expected = std::env::var("CENTRAL_TEST_DSN")
            .expect("run knowledge DB tests through central_test_db.sh")
            .parse::<sqlx::postgres::PgConnectOptions>()
            .expect("valid disposable PostgreSQL DSN");
        let config = tempfile::NamedTempFile::new().expect("temporary non-secret test file");
        std::fs::write(config.path(), "not a valid local database configuration")
            .expect("write intentionally invalid local test configuration");
        let db = pool_with_config_path(config.path())
            .await
            .expect("explicit wrapper DSN bypasses local configuration");
        let actual = db.pool().connect_options();
        assert_eq!(actual.get_host(), "127.0.0.1");
        assert_eq!(actual.get_host(), expected.get_host());
        assert_eq!(actual.get_port(), expected.get_port());
        assert!(actual
            .get_database()
            .unwrap_or_default()
            .starts_with("dlcentral_test_"));
    }
}
