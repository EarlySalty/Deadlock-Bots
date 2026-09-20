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
