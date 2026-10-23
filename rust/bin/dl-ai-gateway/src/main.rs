mod http;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use dl_ai::tool_chat::{build_router, GatewayConfig, PostgresLedger};

#[derive(Parser)]
#[command(name = "dl-ai-gateway")]
struct Cli {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    check_config: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    let config = GatewayConfig::load(&cli.config)
        .with_context(|| format!("loading {}", cli.config.display()))?;
    if cli.check_config {
        println!("config ok: {} slots", config.slots.len());
        return Ok(());
    }
    dl_core::token_snapshot::load(&cli.config).map_err(anyhow::Error::msg)?;
    let dsn = dl_core::token_snapshot::value("DEADLOCK_CENTRAL_DSN")
        .context("central database access missing from the private snapshot")?;
    let pool = dl_central_db::connect_pool(dsn)
        .await
        .context("connecting central database")?;
    dl_ai::usage::initialize(pool, "dl-ai-gateway").map_err(anyhow::Error::msg)?;
    let router = build_router(
        &config,
        |key| std::env::var(key).ok(),
        Arc::new(PostgresLedger),
    )?;
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("binding {}", config.listen))?;
    tracing::info!(listen = %config.listen, slots = ?router.slot_names(), "AI_GATEWAY_LISTENING");
    axum::serve(
        listener,
        http::app(Arc::new(router), config.max_request_bytes),
    )
    .with_graceful_shutdown(shutdown())
    .await
    .context("serving")?;
    Ok(())
}

async fn shutdown() {
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate => {}
    }
}
