#[path = "../src/http.rs"]
mod http;

use async_trait::async_trait;
use dl_ai::tool_chat::{
    BackendError, LedgerEntry, LedgerOutcome, ProviderHandle, SlotRouter, UsageLedger,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

struct NoLedger;
struct NoEntry;

#[async_trait]
impl LedgerEntry for NoEntry {
    async fn finish(self: Box<Self>, _outcome: LedgerOutcome<'_>) {}
}

#[async_trait]
impl UsageLedger for NoLedger {
    async fn begin(
        &self,
        _slot: &str,
        _provider: &str,
        _model: &str,
    ) -> Result<Box<dyn LedgerEntry>, BackendError> {
        Ok(Box::new(NoEntry))
    }
}

#[tokio::main]
async fn main() {
    use dl_ai::tool_chat::{ClaudePlanBackend, GatewayConfig, ProviderKind};
    let config_path = std::env::var_os("M6_GATEWAY_CONFIG").expect("isolated measurement config");
    let config = GatewayConfig::load(std::path::Path::new(&config_path)).expect("config");
    let slot = &config.slots["brain_answer"];
    let targets: Vec<_> = config
        .targets(slot)
        .into_iter()
        .filter(|target| config.providers[&target.provider].kind == ProviderKind::ClaudePlan)
        .collect();
    assert_eq!(targets.len(), 1, "one CLI target required");
    let provider_name = targets[0].provider.clone();
    let provider = &config.providers[&provider_name];
    let work_dir = std::path::PathBuf::from(
        std::env::var_os("M6_GATEWAY_WORK_DIR").expect("private work directory"),
    );
    assert!(work_dir.is_dir());
    assert!(std::fs::read_dir(&work_dir)
        .expect("work dir")
        .next()
        .is_none());
    let backend = Arc::new(ClaudePlanBackend::new(
        provider.claude_bin.clone().expect("CLI binary"),
        work_dir,
        provider.max_output_bytes.expect("output limit"),
    ));
    let router = SlotRouter::new(
        BTreeMap::from([("brain_answer".to_owned(), targets)]),
        BTreeMap::from([(
            provider_name,
            ProviderHandle::new(
                backend,
                provider.concurrency,
                Duration::from_millis(provider.queue_timeout_ms),
            ),
        )]),
        Arc::new(NoLedger),
    );
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    println!(
        "M6_ISOLATED_GATEWAY {}",
        listener.local_addr().expect("address")
    );
    let lifetime_ms: u64 = std::env::var("M6_GATEWAY_LIFETIME_MS")
        .expect("measurement lifetime")
        .parse()
        .expect("milliseconds");
    assert!((1..=3_600_000).contains(&lifetime_ms));
    tokio::select! {
        result = axum::serve(listener, http::app(Arc::new(router), config.max_request_bytes)) => result.expect("serve"),
        _ = tokio::time::sleep(Duration::from_millis(lifetime_ms)) => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
