//! Idempotency-Store — Semantik wie master_broker.py:
//! - Erfolgreiche (ok && 2xx) Antworten werden pro `action:key` gecacht (TTL).
//! - Wiederholung mit gleichem Payload-Hash → gecachte Antwort, `cached: true`.
//! - Wiederholung mit anderem Payload → 409 idempotency_conflict.
//! - Läuft dieselbe Aktion noch → Warten auf das Ergebnis (Timeout → 504).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct IdempotencyConfig {
    pub ttl: Duration,
    pub max_entries: usize,
    pub inflight_ttl: Duration,
    pub waiter_timeout: Duration,
}

impl IdempotencyConfig {
    /// ENV-Namen wie das Original (MASTER_BROKER_IDEMPOTENCY_*).
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let pos_f64 = |key: &str, default: f64| -> f64 {
            lookup(key)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| *v > 0.0)
                .unwrap_or(default)
        };
        let ttl = pos_f64("MASTER_BROKER_IDEMPOTENCY_TTL_SECONDS", 600.0);
        Self {
            ttl: Duration::from_secs_f64(ttl),
            max_entries: lookup("MASTER_BROKER_IDEMPOTENCY_MAX_ENTRIES")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(1024),
            inflight_ttl: Duration::from_secs_f64(pos_f64(
                "MASTER_BROKER_IDEMPOTENCY_INFLIGHT_TTL_SECONDS",
                ttl.max(300.0),
            )),
            waiter_timeout: Duration::from_secs_f64(pos_f64(
                "MASTER_BROKER_IDEMPOTENCY_WAITER_TIMEOUT_SECONDS",
                15.0,
            )),
        }
    }
}

struct Record {
    payload_hash: String,
    status: u16,
    body: Value,
    created: Instant,
}

struct Inflight {
    payload_hash: String,
    tx: watch::Sender<Option<(u16, Value)>>,
    rx: watch::Receiver<Option<(u16, Value)>>,
    created: Instant,
}

#[derive(Default)]
struct Inner {
    records: HashMap<String, Record>,
    inflight: HashMap<String, Inflight>,
}

pub enum Decision {
    /// Aktion ausführen — Aufrufer MUSS danach `settle` rufen.
    Execute,
    /// Gecachte Antwort (Status, Body) — Body noch ohne `cached: true`.
    Cached(u16, Value),
    /// Eine andere Anfrage läuft — auf deren Ergebnis warten.
    Pending(watch::Receiver<Option<(u16, Value)>>),
}

pub struct IdempotencyStore {
    config: IdempotencyConfig,
    inner: tokio::sync::Mutex<Inner>,
}

impl IdempotencyStore {
    pub fn new(config: IdempotencyConfig) -> Self {
        Self {
            config,
            inner: tokio::sync::Mutex::new(Inner::default()),
        }
    }

    pub fn waiter_timeout(&self) -> Duration {
        self.config.waiter_timeout
    }

    fn cache_key(action: &str, key: &str) -> String {
        format!("{action}:{key}")
    }

    /// Fehler-String = Vertrag ("idempotency key already used with different payload").
    pub async fn begin(
        &self,
        action: &str,
        idempotency_key: &str,
        payload_hash: &str,
    ) -> Result<Decision, String> {
        let cache_key = Self::cache_key(action, idempotency_key);
        let now = Instant::now();
        let mut inner = self.inner.lock().await;
        Self::prune(&mut inner, &self.config, now);

        if let Some(record) = inner.records.get(&cache_key) {
            if record.payload_hash != payload_hash {
                return Err("idempotency key already used with different payload".to_string());
            }
            return Ok(Decision::Cached(record.status, record.body.clone()));
        }
        if let Some(inflight) = inner.inflight.get(&cache_key) {
            if inflight.payload_hash != payload_hash {
                return Err("idempotency key already used with different payload".to_string());
            }
            return Ok(Decision::Pending(inflight.rx.clone()));
        }

        let (tx, rx) = watch::channel(None);
        inner.inflight.insert(
            cache_key,
            Inflight {
                payload_hash: payload_hash.to_string(),
                tx,
                rx,
                created: now,
            },
        );
        Ok(Decision::Execute)
    }

    /// Ergebnis eintragen: Inflight auflösen, bei `cache_response` Record anlegen.
    pub async fn settle(
        &self,
        action: &str,
        idempotency_key: &str,
        payload_hash: &str,
        status: u16,
        body: &Value,
        cache_response: bool,
    ) {
        let cache_key = Self::cache_key(action, idempotency_key);
        let now = Instant::now();
        let mut inner = self.inner.lock().await;
        Self::prune(&mut inner, &self.config, now);

        if cache_response {
            inner.records.insert(
                cache_key.clone(),
                Record {
                    payload_hash: payload_hash.to_string(),
                    status,
                    body: body.clone(),
                    created: now,
                },
            );
            if inner.records.len() > self.config.max_entries {
                if let Some(oldest) = inner
                    .records
                    .iter()
                    .min_by_key(|(_, r)| r.created)
                    .map(|(k, _)| k.clone())
                {
                    inner.records.remove(&oldest);
                }
            }
        }
        if let Some(inflight) = inner.inflight.remove(&cache_key) {
            let _ = inflight.tx.send(Some((status, body.clone())));
        }
    }

    fn prune(inner: &mut Inner, config: &IdempotencyConfig, now: Instant) {
        inner
            .records
            .retain(|_, record| now.duration_since(record.created) <= config.ttl);

        let expired: Vec<String> = inner
            .inflight
            .iter()
            .filter(|(_, state)| now.duration_since(state.created) > config.inflight_ttl)
            .map(|(k, _)| k.clone())
            .collect();
        for key in expired {
            if let Some(state) = inner.inflight.remove(&key) {
                let idem = key.split_once(':').map(|(_, k)| k).unwrap_or("");
                let _ = state.tx.send(Some((
                    500,
                    json!({
                        "ok": false,
                        "request_id": crate::random_request_id(),
                        "idempotency_key": idem,
                        "cached": false,
                        "result": null,
                        "error": {
                            "code": "idempotency_expired",
                            "message": "idempotent operation expired before completion",
                        },
                    }),
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> IdempotencyStore {
        IdempotencyStore::new(IdempotencyConfig {
            ttl: Duration::from_secs(600),
            max_entries: 2,
            inflight_ttl: Duration::from_secs(600),
            waiter_timeout: Duration::from_millis(200),
        })
    }

    #[tokio::test]
    async fn cache_und_replay() {
        let store = store();
        assert!(matches!(
            store.begin("a", "k1", "h1").await.expect("begin"),
            Decision::Execute
        ));
        let body = json!({"ok": true, "result": {"x": 1}});
        store.settle("a", "k1", "h1", 200, &body, true).await;

        match store.begin("a", "k1", "h1").await.expect("replay") {
            Decision::Cached(status, cached) => {
                assert_eq!(status, 200);
                assert_eq!(cached, body);
            }
            _ => panic!("erwartet Cached"),
        }
    }

    #[tokio::test]
    async fn anderer_payload_ist_konflikt() {
        let store = store();
        let _ = store.begin("a", "k1", "h1").await.expect("begin");
        store
            .settle("a", "k1", "h1", 200, &json!({"ok": true}), true)
            .await;
        let err = match store.begin("a", "k1", "ANDERS").await {
            Err(e) => e,
            Ok(_) => panic!("Konflikt erwartet"),
        };
        assert!(err.contains("different payload"));
    }

    #[tokio::test]
    async fn pending_liefert_ergebnis_an_wartende() {
        let store = store();
        let _ = store.begin("a", "k1", "h1").await.expect("begin");
        let Decision::Pending(mut rx) = store.begin("a", "k1", "h1").await.expect("zweiter") else {
            panic!("erwartet Pending");
        };
        store
            .settle("a", "k1", "h1", 200, &json!({"ok": true}), true)
            .await;
        rx.changed().await.expect("changed");
        let value = rx.borrow().clone().expect("ergebnis");
        assert_eq!(value.0, 200);
    }

    #[tokio::test]
    async fn fehlschlaege_werden_nicht_gecacht() {
        let store = store();
        let _ = store.begin("a", "k1", "h1").await.expect("begin");
        store
            .settle("a", "k1", "h1", 502, &json!({"ok": false}), false)
            .await;
        // Nächster Versuch mit gleichem Key darf erneut ausführen
        assert!(matches!(
            store.begin("a", "k1", "h1").await.expect("retry"),
            Decision::Execute
        ));
    }

    #[tokio::test]
    async fn max_entries_verdraengt_aeltesten() {
        let store = store();
        for (key, hash) in [("k1", "h1"), ("k2", "h2"), ("k3", "h3")] {
            let _ = store.begin("a", key, hash).await.expect("begin");
            store
                .settle("a", key, hash, 200, &json!({"ok": true}), true)
                .await;
        }
        // max_entries = 2 → k1 (ältester) ist verdrängt → Execute statt Cached
        assert!(matches!(
            store.begin("a", "k1", "h1").await.expect("again"),
            Decision::Execute
        ));
    }
}
