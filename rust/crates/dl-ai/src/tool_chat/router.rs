use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::config::Target;
use super::types::{ToolChatRequest, ToolChatResponse, Usage};

pub struct LedgerOutcome<'a> {
    pub usage: Option<Usage>,
    pub request_id: Option<&'a str>,
    pub error_code: Option<&'static str>,
}

#[async_trait]
pub trait LedgerEntry: Send {
    async fn finish(self: Box<Self>, outcome: LedgerOutcome<'_>);
}

#[async_trait]
pub trait UsageLedger: Send + Sync {
    async fn begin(
        &self,
        slot: &str,
        provider: &str,
        model: &str,
    ) -> Result<Box<dyn LedgerEntry>, BackendError>;
}

pub struct ProviderHandle {
    pub backend: Arc<dyn ToolBackend>,
    pub permits: Arc<Semaphore>,
    pub queue_timeout: Duration,
}

impl ProviderHandle {
    pub fn new(backend: Arc<dyn ToolBackend>, concurrency: usize, queue_timeout: Duration) -> Self {
        Self {
            backend,
            permits: Arc::new(Semaphore::new(concurrency)),
            queue_timeout,
        }
    }
}

pub struct Routed {
    pub response: ToolChatResponse,
    pub provider: String,
    pub attempts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    UnknownSlot,
    Failed { last: BackendError, attempts: usize },
}

pub struct SlotRouter {
    slots: BTreeMap<String, Vec<Target>>,
    providers: BTreeMap<String, ProviderHandle>,
    ledger: Arc<dyn UsageLedger>,
}

impl SlotRouter {
    pub fn new(
        slots: BTreeMap<String, Vec<Target>>,
        providers: BTreeMap<String, ProviderHandle>,
        ledger: Arc<dyn UsageLedger>,
    ) -> Self {
        Self {
            slots,
            providers,
            ledger,
        }
    }

    pub fn slot_names(&self) -> Vec<&str> {
        self.slots.keys().map(String::as_str).collect()
    }

    pub async fn complete(
        &self,
        slot: &str,
        request: &ToolChatRequest,
    ) -> Result<Routed, RouteError> {
        let targets = self.slots.get(slot).ok_or(RouteError::UnknownSlot)?;
        let mut last = BackendError::new(FailureKind::Unavailable, "no_target");
        for (index, target) in targets.iter().enumerate() {
            match self.attempt(slot, target, request).await {
                Ok(response) => {
                    if index > 0 {
                        tracing::warn!(
                            slot,
                            provider = %target.provider,
                            attempt = index + 1,
                            "AI_GATEWAY_FALLBACK_SERVED"
                        );
                    }
                    return Ok(Routed {
                        response,
                        provider: target.provider.clone(),
                        attempts: index + 1,
                    });
                }
                Err(error) => {
                    let next = targets
                        .get(index + 1)
                        .filter(|_| error.kind.allows_fallback());
                    tracing::warn!(
                        slot,
                        provider = %target.provider,
                        kind = error.kind.as_str(),
                        reason = error.reason,
                        next_provider = next.map(|next| next.provider.as_str()).unwrap_or(""),
                        "AI_GATEWAY_ATTEMPT_FAILED"
                    );
                    last = error;
                    if next.is_none() {
                        return Err(RouteError::Failed {
                            last,
                            attempts: index + 1,
                        });
                    }
                }
            }
        }
        Err(RouteError::Failed {
            last,
            attempts: targets.len(),
        })
    }

    async fn attempt(
        &self,
        slot: &str,
        target: &Target,
        request: &ToolChatRequest,
    ) -> Result<ToolChatResponse, BackendError> {
        let handle = self
            .providers
            .get(&target.provider)
            .ok_or(BackendError::new(
                FailureKind::Unavailable,
                "unknown_provider",
            ))?;
        let _permit = tokio::time::timeout(
            handle.queue_timeout.min(target.timeout),
            Arc::clone(&handle.permits).acquire_owned(),
        )
        .await
        .map_err(|_| BackendError::new(FailureKind::Busy, "queue_timeout"))?
        .map_err(|_| BackendError::new(FailureKind::Unavailable, "pool_closed"))?;
        let call = CallTarget {
            model: target.model.clone(),
            max_tokens: request
                .max_tokens
                .map_or(target.max_tokens, |asked| asked.min(target.max_tokens)),
        };
        let entry = if handle.backend.records_own_usage() {
            None
        } else {
            Some(
                self.ledger
                    .begin(slot, &target.provider, &target.model)
                    .await?,
            )
        };
        let result = crate::usage::with_purpose(
            slot,
            tokio::time::timeout(target.timeout, handle.backend.complete(request, &call)),
        )
        .await
        .unwrap_or(Err(BackendError::new(FailureKind::Timeout, "slot_timeout")));
        if let Some(entry) = entry {
            let outcome = match &result {
                Ok(response) => LedgerOutcome {
                    usage: response.usage,
                    request_id: response.request_id.as_deref(),
                    error_code: None,
                },
                Err(error) => LedgerOutcome {
                    usage: None,
                    request_id: None,
                    error_code: Some(error.kind.ledger_code()),
                },
            };
            entry.finish(outcome).await;
        }
        result
    }
}

pub struct PostgresLedger;

struct PostgresEntry(crate::usage::Attempt);

#[async_trait]
impl LedgerEntry for PostgresEntry {
    async fn finish(self: Box<Self>, outcome: LedgerOutcome<'_>) {
        self.0
            .finish_call(
                outcome
                    .usage
                    .map(|usage| (usage.total_input(), usage.output_tokens)),
                outcome.request_id,
                outcome.error_code,
            )
            .await;
    }
}

#[async_trait]
impl UsageLedger for PostgresLedger {
    async fn begin(
        &self,
        slot: &str,
        provider: &str,
        model: &str,
    ) -> Result<Box<dyn LedgerEntry>, BackendError> {
        crate::usage::Attempt::begin_call(provider, slot, model)
            .await
            .map(|attempt| Box::new(PostgresEntry(attempt)) as Box<dyn LedgerEntry>)
            .map_err(|_| BackendError::new(FailureKind::Unavailable, "ledger_unavailable"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_chat::types::{ContentBlock, Message, Role, StopReason};
    use std::sync::Mutex;

    struct FakeBackend {
        outcomes: Mutex<Vec<Result<ToolChatResponse, BackendError>>>,
        calls: Mutex<Vec<CallTarget>>,
    }

    impl FakeBackend {
        fn new(outcomes: Vec<Result<ToolChatResponse, BackendError>>) -> Arc<Self> {
            Arc::new(Self {
                outcomes: Mutex::new(outcomes),
                calls: Mutex::new(Vec::new()),
            })
        }
        fn calls(&self) -> Vec<CallTarget> {
            self.calls.lock().map(|c| c.clone()).unwrap_or_default()
        }
    }

    #[async_trait]
    impl ToolBackend for FakeBackend {
        async fn complete(
            &self,
            _request: &ToolChatRequest,
            target: &CallTarget,
        ) -> Result<ToolChatResponse, BackendError> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(target.clone());
            }
            let mut outcomes = self
                .outcomes
                .lock()
                .map_err(|_| BackendError::new(FailureKind::Unavailable, "poisoned"))?;
            if outcomes.is_empty() {
                return Err(BackendError::new(FailureKind::Unavailable, "exhausted"));
            }
            outcomes.remove(0)
        }
    }

    type LedgerRows = Arc<Mutex<Vec<(String, String, String, Option<&'static str>, Option<u64>)>>>;

    #[derive(Default)]
    struct FakeLedger {
        rows: LedgerRows,
    }

    struct FakeEntry {
        rows: LedgerRows,
        key: (String, String, String),
    }

    #[async_trait]
    impl LedgerEntry for FakeEntry {
        async fn finish(self: Box<Self>, outcome: LedgerOutcome<'_>) {
            if let Ok(mut rows) = self.rows.lock() {
                rows.push((
                    self.key.0.clone(),
                    self.key.1.clone(),
                    self.key.2.clone(),
                    outcome.error_code,
                    outcome.usage.map(|usage| usage.output_tokens),
                ));
            }
        }
    }

    #[async_trait]
    impl UsageLedger for FakeLedger {
        async fn begin(
            &self,
            slot: &str,
            provider: &str,
            model: &str,
        ) -> Result<Box<dyn LedgerEntry>, BackendError> {
            Ok(Box::new(FakeEntry {
                rows: Arc::clone(&self.rows),
                key: (slot.into(), provider.into(), model.into()),
            }))
        }
    }

    fn answer(text: &str) -> ToolChatResponse {
        ToolChatResponse {
            model: "served".into(),
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Some(Usage {
                input_tokens: 1,
                output_tokens: 2,
                ..Usage::default()
            }),
            request_id: None,
        }
    }

    fn request(max_tokens: Option<u32>) -> ToolChatRequest {
        ToolChatRequest {
            system: String::new(),
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: "q".into() }],
            }],
            tools: Vec::new(),
            max_tokens,
        }
    }

    fn target(provider: &str, model: &str) -> Target {
        Target {
            provider: provider.into(),
            model: model.into(),
            timeout: Duration::from_secs(5),
            max_tokens: 500,
        }
    }

    fn router(
        chain: Vec<Target>,
        backends: Vec<(&str, Arc<FakeBackend>)>,
        ledger: Arc<FakeLedger>,
    ) -> SlotRouter {
        let providers = backends
            .into_iter()
            .map(|(name, backend)| {
                (
                    name.to_owned(),
                    ProviderHandle::new(backend, 1, Duration::from_millis(50)),
                )
            })
            .collect();
        SlotRouter::new(
            BTreeMap::from([("slot_a".to_owned(), chain)]),
            providers,
            ledger,
        )
    }

    #[tokio::test]
    async fn limit_on_the_primary_falls_back_in_order_and_both_attempts_are_ledgered() {
        let primary = FakeBackend::new(vec![Err(BackendError::new(
            FailureKind::Limit,
            "usage_limit",
        ))]);
        let second = FakeBackend::new(vec![Ok(answer("from second"))]);
        let ledger = Arc::new(FakeLedger::default());
        let router = router(
            vec![target("plan", "m1"), target("fake", "m2")],
            vec![("plan", primary.clone()), ("fake", second.clone())],
            ledger.clone(),
        );
        let routed = router
            .complete("slot_a", &request(Some(9000)))
            .await
            .expect("fallback");
        assert_eq!(routed.provider, "fake");
        assert_eq!(routed.attempts, 2);
        assert_eq!(second.calls()[0].max_tokens, 500);
        let rows = ledger.rows.lock().map(|r| r.clone()).unwrap_or_default();
        assert_eq!(
            rows,
            vec![
                (
                    "slot_a".into(),
                    "plan".into(),
                    "m1".into(),
                    Some("rate_limited"),
                    None
                ),
                ("slot_a".into(), "fake".into(), "m2".into(), None, Some(2)),
            ]
        );
    }

    #[tokio::test]
    async fn empty_chain_and_invalid_requests_do_not_fall_back() {
        let primary = FakeBackend::new(vec![Err(BackendError::new(
            FailureKind::Limit,
            "usage_limit",
        ))]);
        let ledger = Arc::new(FakeLedger::default());
        let alone = router(
            vec![target("plan", "m1")],
            vec![("plan", primary)],
            ledger.clone(),
        );
        assert_eq!(
            alone.complete("slot_a", &request(None)).await.err(),
            Some(RouteError::Failed {
                last: BackendError::new(FailureKind::Limit, "usage_limit"),
                attempts: 1
            })
        );
        let invalid = FakeBackend::new(vec![Err(BackendError::new(
            FailureKind::InvalidRequest,
            "x",
        ))]);
        let spare = FakeBackend::new(vec![Ok(answer("never"))]);
        let chained = router(
            vec![target("plan", "m1"), target("fake", "m2")],
            vec![("plan", invalid), ("fake", spare.clone())],
            ledger,
        );
        assert!(chained.complete("slot_a", &request(None)).await.is_err());
        assert!(spare.calls().is_empty());
        assert_eq!(
            chained.complete("missing", &request(None)).await.err(),
            Some(RouteError::UnknownSlot)
        );
    }

    #[tokio::test]
    async fn a_saturated_provider_reports_busy_and_hands_over() {
        let held = FakeBackend::new(Vec::new());
        let second = FakeBackend::new(vec![Ok(answer("second"))]);
        let ledger = Arc::new(FakeLedger::default());
        let router = router(
            vec![target("plan", "m1"), target("fake", "m2")],
            vec![("plan", held), ("fake", second)],
            ledger,
        );
        let permit = router
            .providers
            .get("plan")
            .map(|handle| Arc::clone(&handle.permits))
            .expect("plan handle")
            .acquire_owned()
            .await
            .expect("permit");
        let routed = router
            .complete("slot_a", &request(None))
            .await
            .expect("handover");
        assert_eq!(routed.provider, "fake");
        drop(permit);
    }
}
