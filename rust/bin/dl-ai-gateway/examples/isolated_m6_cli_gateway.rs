#[path = "../src/http.rs"]
mod http;

use async_trait::async_trait;
use dl_ai::tool_chat::{
    BackendError, FailureKind, LedgerEntry, LedgerOutcome, ProviderHandle, SlotRouter, UsageLedger,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct MeasurementLedger {
    file: Arc<Mutex<File>>,
    next_id: AtomicU64,
}

struct MeasurementEntry {
    file: Arc<Mutex<File>>,
    id: u64,
    finished: bool,
}

fn record(file: &Mutex<File>, event: &Value) -> io::Result<()> {
    let mut file = file.lock().map_err(|_| io::Error::other("ledger lock"))?;
    writeln!(file, "{event}")?;
    file.sync_data()
}

impl Drop for MeasurementEntry {
    fn drop(&mut self) {
        if !self.finished && record(&self.file, &json!({"event":"abandoned","id":self.id})).is_err()
        {
            tracing::error!("M6_LEDGER_WRITE_FAILED");
        }
    }
}

#[async_trait]
impl LedgerEntry for MeasurementEntry {
    async fn finish(mut self: Box<Self>, outcome: LedgerOutcome<'_>) {
        let event =
            json!({"event":"finish","id":self.id,"usage":outcome.usage,"error":outcome.error_code});
        self.finished = record(&self.file, &event).is_ok();
        if !self.finished {
            tracing::error!("M6_LEDGER_WRITE_FAILED");
        }
    }
}

#[async_trait]
impl UsageLedger for MeasurementLedger {
    async fn begin(
        &self,
        slot: &str,
        provider: &str,
        model: &str,
    ) -> Result<Box<dyn LedgerEntry>, BackendError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        record(
            &self.file,
            &json!({"event":"begin","id":id,"slot":slot,"provider":provider,"model":model}),
        )
        .map_err(|_| BackendError::new(FailureKind::Unavailable, "measurement_ledger"))?;
        Ok(Box::new(MeasurementEntry {
            file: Arc::clone(&self.file),
            id,
            finished: false,
        }))
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
        Arc::new(MeasurementLedger {
            file: Arc::new(Mutex::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(std::env::var_os("M6_GATEWAY_LEDGER").expect("measurement ledger path"))
                    .expect("create measurement ledger"),
            )),
            next_id: AtomicU64::new(1),
        }),
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
        _ = shutdown() => {}
    }
}
