mod alarm;
mod http;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use dl_ai::tool_chat::{build_router, GatewayConfig, PostgresLedger, RefusalAlarm, SlotRouter};
use dl_ai::upstream::GATEWAY_BINARY;

#[derive(Parser)]
#[command(name = GATEWAY_BINARY)]
struct Cli {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    check_config: bool,
}

struct Started {
    router: SlotRouter,
    refusals: Arc<RefusalAlarm>,
}

async fn alarm_ready(config: &GatewayConfig, helper: &Path) -> Result<()> {
    let missing = alarm::readiness(&config.alarm, helper).await;
    alarm::unready(&missing).map_or(Ok(()), Err)
}

async fn prepare(config: &GatewayConfig, helper: PathBuf) -> Result<Started> {
    alarm_ready(config, &helper).await?;
    let dsn = dl_core::token_snapshot::value("DEADLOCK_CENTRAL_DSN")
        .context("central database access missing from the private snapshot")?;
    let pool = dl_central_db::connect_pool(dsn)
        .await
        .context("connecting central database")?;
    dl_ai::usage::initialize(pool, GATEWAY_BINARY).map_err(anyhow::Error::msg)?;
    let router = build_router(
        config,
        |key| {
            dl_core::token_snapshot::value(key)
                .map(str::to_owned)
                .or_else(|| std::env::var(key).ok())
        },
        Arc::new(PostgresLedger),
    )?;
    let refusals = Arc::new(RefusalAlarm::new(
        Arc::new(alarm::ProcessNotifier::new(config.alarm.clone(), helper)),
        config.alarm.standing_after_secs,
    ));
    alarm::scan(&refusals, config.alarm.scan_interval())
        .await
        .map_err(anyhow::Error::msg)
        .context("first provider refusal scan")?;
    Ok(Started { router, refusals })
}

async fn within<T>(budget: Duration, start: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(budget, start).await.map_err(|_| {
        anyhow::anyhow!(
            "gateway start exceeded its budget of {} s for alarm readiness, database pool and first provider refusal scan",
            budget.as_secs()
        )
    })?
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
    let helper = alarm::sibling_helper().context("locating the notify helper of this release")?;
    if cli.check_config {
        let ready = alarm_ready(&config, &helper).await;
        println!("config ok: {} slots", config.slots.len());
        ready?;
        println!("alarm ready");
        return Ok(());
    }
    dl_core::token_snapshot::load(&cli.config).map_err(anyhow::Error::msg)?;
    let Started { router, refusals } =
        within(config.alarm.start_budget(), prepare(&config, helper)).await?;
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("binding {}", config.listen))?;
    tracing::info!(listen = %config.listen, slots = ?router.slot_names(), "AI_GATEWAY_LISTENING");
    let serve = async {
        axum::serve(
            listener,
            http::app(Arc::new(router), config.max_request_bytes),
        )
        .with_graceful_shutdown(shutdown())
        .await
    };
    tokio::select! {
        served = serve => served.context("serving"),
        failure = alarm::watch(refusals, config.alarm.clone()) => Err(failure),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binary_carries_the_name_the_unit_ledger_and_installer_use() {
        assert_eq!(env!("CARGO_BIN_NAME"), GATEWAY_BINARY);
    }

    #[tokio::test]
    async fn within_fails_at_the_budget_and_passes_results_and_errors_through() {
        let stuck = within(Duration::ZERO, std::future::pending::<Result<()>>()).await;
        let reported = stuck.expect_err("budget exceeded").to_string();
        assert!(
            reported.contains("exceeded its budget of 0 s"),
            "{reported}"
        );
        assert_eq!(within(Duration::MAX, async { Ok(7) }).await.ok(), Some(7));
        let cause = "fixture cause";
        let failed = within(Duration::MAX, async {
            Err::<(), _>(anyhow::anyhow!(cause))
        })
        .await;
        assert_eq!(failed.expect_err("own error").to_string(), cause);
    }
}
