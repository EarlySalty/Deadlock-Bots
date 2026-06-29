//! dl-etage - Coaching-Etage API neben dem Python-Backend.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use anyhow::Context;
use axum::{routing::get, Router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dl_core::observability::init_tracing("info");

    let cfg = dl_core::Config::from_env().context("load config")?;
    let host = std::env::var("DL_ETAGE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let db = dl_db::Db::open(&cfg.db_path)
        .with_context(|| format!("open shared DB: {}", cfg.db_path.display()))?;

    log_db_identity(&db, cfg.db_path.clone()).await?;

    let addr = format!("{host}:{}", cfg.ports.etage);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind dl-etage port: {addr}"))?;
    tracing::info!(addr = %addr, "dl-etage bound");

    tracing::info!("dl-etage running; stop with Ctrl+C");
    axum::serve(listener, health_router())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("dl-etage server")?;

    Ok(())
}

fn health_router() -> Router {
    Router::new().route("/api/health", get(health))
}

async fn health() -> &'static str {
    "OK"
}

async fn shutdown_signal() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!("dl-etage stopped"),
        Err(error) => tracing::error!(%error, "failed to install Ctrl+C handler"),
    }
}

async fn log_db_identity(db: &dl_db::Db, configured_path: PathBuf) -> anyhow::Result<()> {
    let resolved_path = std::fs::canonicalize(&configured_path)
        .with_context(|| format!("resolve DB path: {}", configured_path.display()))?;
    let metadata = std::fs::metadata(&resolved_path)
        .with_context(|| format!("read DB metadata: {}", resolved_path.display()))?;

    let database_list = db
        .write(|conn| {
            let mut stmt = conn.prepare("PRAGMA database_list")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .await
        .context("read PRAGMA database_list")?;

    tracing::info!(
        configured_path = %configured_path.display(),
        db_handle_path = %db.path().display(),
        resolved_path = %resolved_path.display(),
        device = metadata.dev(),
        inode = metadata.ino(),
        database_list = ?database_list,
        "dl-etage DB identity checked"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_returns_200() {
        let response = health_router()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(axum::body::Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("health request");

        assert_eq!(response.status(), StatusCode::OK);
    }
}
