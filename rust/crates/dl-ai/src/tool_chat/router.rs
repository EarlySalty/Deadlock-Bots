use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;
use tokio::time::Instant;

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::config::Target;
use super::types::{ToolChatRequest, ToolChatResponse, Usage};

const LEDGER_FINISH_BUDGET: Duration = Duration::from_secs(2);

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
        let deadline = Instant::now()
            + targets
                .first()
                .map_or(Duration::ZERO, |target| target.deadline);
        let mut last = BackendError::new(FailureKind::Unavailable, "no_target");
        for (index, target) in targets.iter().enumerate() {
            match self
                .attempt_with_retries(slot, target, request, deadline)
                .await
            {
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

    async fn attempt_with_retries(
        &self,
        slot: &str,
        target: &Target,
        request: &ToolChatRequest,
        deadline: Instant,
    ) -> Result<ToolChatResponse, BackendError> {
        let mut tries = 1;
        loop {
            let wait = target
                .retry_backoff
                .saturating_mul(2u32.saturating_pow(tries - 1));
            match self.attempt(slot, target, request, deadline).await {
                Err(error)
                    if error.kind.retryable()
                        && tries < target.attempts
                        && Instant::now() + wait < deadline =>
                {
                    tracing::warn!(
                        slot,
                        provider = %target.provider,
                        attempt = tries,
                        kind = error.kind.as_str(),
                        reason = error.reason,
                        wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                        "AI_GATEWAY_RETRY"
                    );
                    tokio::time::sleep(wait).await;
                    tries += 1;
                }
                outcome => return outcome,
            }
        }
    }

    async fn attempt(
        &self,
        slot: &str,
        target: &Target,
        request: &ToolChatRequest,
        deadline: Instant,
    ) -> Result<ToolChatResponse, BackendError> {
        if Instant::now() >= deadline {
            return Err(BackendError::new(FailureKind::Timeout, "request_deadline"));
        }
        let handle = self
            .providers
            .get(&target.provider)
            .ok_or(BackendError::new(
                FailureKind::Unavailable,
                "unknown_provider",
            ))?;
        let queue_deadline = deadline.min(Instant::now() + handle.queue_timeout);
        let permit =
            tokio::time::timeout_at(queue_deadline, Arc::clone(&handle.permits).acquire_owned())
                .await
                .map_err(|_| BackendError::new(FailureKind::Busy, "queue_timeout"))?
                .map_err(|_| BackendError::new(FailureKind::Unavailable, "pool_closed"))?;
        let deadline = deadline.min(Instant::now() + target.timeout);
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
                tokio::time::timeout_at(
                    deadline,
                    self.ledger.begin(slot, &target.provider, &target.model),
                )
                .await
                .map_err(|_| BackendError::new(FailureKind::Timeout, "ledger_timeout"))??,
            )
        };
        let result = crate::usage::with_purpose(
            slot,
            tokio::time::timeout_at(deadline, handle.backend.complete(request, &call)),
        )
        .await
        .unwrap_or(Err(BackendError::new(FailureKind::Timeout, "slot_timeout")));
        drop(permit);
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
            if tokio::time::timeout(LEDGER_FINISH_BUDGET, entry.finish(outcome))
                .await
                .is_err()
            {
                tracing::warn!(
                    slot,
                    provider = %target.provider,
                    "AI_GATEWAY_LEDGER_FINISH_LATE"
                );
            }
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

    struct StalledLedger {
        stall_begin: bool,
    }

    struct StalledEntry;

    #[async_trait]
    impl LedgerEntry for StalledEntry {
        async fn finish(self: Box<Self>, _outcome: LedgerOutcome<'_>) {
            std::future::pending::<()>().await;
        }
    }

    #[async_trait]
    impl UsageLedger for StalledLedger {
        async fn begin(
            &self,
            _slot: &str,
            _provider: &str,
            _model: &str,
        ) -> Result<Box<dyn LedgerEntry>, BackendError> {
            if self.stall_begin {
                std::future::pending::<()>().await;
            }
            Ok(Box::new(StalledEntry))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_ledger_neither_outlives_the_slot_timeout_nor_holds_the_permit() {
        for stall_begin in [true, false] {
            let backend = FakeBackend::new(vec![Ok(answer("one")), Ok(answer("two"))]);
            let router = SlotRouter::new(
                BTreeMap::from([("slot_a".to_owned(), vec![target("p", "m")])]),
                BTreeMap::from([(
                    "p".to_owned(),
                    ProviderHandle::new(backend, 1, Duration::from_millis(50)),
                )]),
                Arc::new(StalledLedger { stall_begin }),
            );
            let asked = request(None);
            let started = Instant::now();
            let (first, second) = tokio::join!(
                router.complete("slot_a", &asked),
                router.complete("slot_a", &asked)
            );
            let elapsed = started.elapsed();
            if stall_begin {
                assert!(matches!(
                    first,
                    Err(RouteError::Failed { ref last, .. }) if last.kind == FailureKind::Timeout
                ));
                assert!(elapsed <= Duration::from_secs(5));
            } else {
                assert!(first.is_ok());
                assert!(second.is_ok());
                assert!(elapsed <= LEDGER_FINISH_BUDGET);
            }
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
            attempts: 1,
            retry_backoff: Duration::ZERO,
            deadline: Duration::from_secs(60),
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

    #[tokio::test(start_paused = true)]
    async fn upstream_overload_is_retried_within_the_configured_attempts() {
        let overloaded = || {
            Err(BackendError::new(
                FailureKind::Overloaded,
                "upstream_overloaded",
            ))
        };
        let retried = Target {
            attempts: 3,
            retry_backoff: Duration::from_millis(100),
            ..target("plan", "m1")
        };
        let backend = FakeBackend::new(vec![overloaded(), overloaded(), Ok(answer("third"))]);
        let ledger = Arc::new(FakeLedger::default());
        let served = router(
            vec![retried.clone()],
            vec![("plan", backend.clone())],
            ledger.clone(),
        );
        let started = Instant::now();
        let routed = served
            .complete("slot_a", &request(None))
            .await
            .expect("third attempt serves");
        assert_eq!(backend.calls().len(), 3);
        assert_eq!(started.elapsed(), Duration::from_millis(300));
        assert_eq!(routed.provider, "plan");
        let exhausted = FakeBackend::new(vec![overloaded(), overloaded(), overloaded()]);
        let failing = router(
            vec![retried],
            vec![("plan", exhausted.clone())],
            ledger.clone(),
        );
        assert_eq!(
            failing.complete("slot_a", &request(None)).await.err(),
            Some(RouteError::Failed {
                last: BackendError::new(FailureKind::Overloaded, "upstream_overloaded"),
                attempts: 1
            })
        );
        assert_eq!(exhausted.calls().len(), 3);
        let limited = FakeBackend::new(vec![
            Err(BackendError::new(FailureKind::Limit, "usage_limit")),
            Ok(answer("never")),
        ]);
        let once = router(
            vec![Target {
                attempts: 3,
                ..target("plan", "m1")
            }],
            vec![("plan", limited.clone())],
            ledger,
        );
        assert!(once.complete("slot_a", &request(None)).await.is_err());
        assert_eq!(limited.calls().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_stop_at_the_request_deadline() {
        let overloaded = || {
            Err(BackendError::new(
                FailureKind::Overloaded,
                "upstream_overloaded",
            ))
        };
        let bounded = Target {
            attempts: 8,
            retry_backoff: Duration::from_secs(1),
            deadline: Duration::from_secs(3),
            ..target("plan", "m1")
        };
        let backend = FakeBackend::new((0..8).map(|_| overloaded()).collect());
        let router = router(
            vec![bounded],
            vec![("plan", backend.clone())],
            Arc::new(FakeLedger::default()),
        );
        let started = Instant::now();
        assert!(router.complete("slot_a", &request(None)).await.is_err());
        assert_eq!(backend.calls().len(), 2);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn queued_requests_wait_for_a_permit_without_losing_their_call_time() {
        let backend = FakeBackend::new(vec![Ok(answer("one")), Ok(answer("two"))]);
        let short = Target {
            timeout: Duration::from_millis(100),
            ..target("plan", "m1")
        };
        let router = SlotRouter::new(
            BTreeMap::from([("slot_a".to_owned(), vec![short])]),
            BTreeMap::from([(
                "plan".to_owned(),
                ProviderHandle::new(backend.clone(), 1, Duration::from_secs(60)),
            )]),
            Arc::new(FakeLedger::default()),
        );
        let permit = router
            .providers
            .get("plan")
            .map(|handle| Arc::clone(&handle.permits))
            .expect("plan handle")
            .acquire_owned()
            .await
            .expect("permit");
        let release = async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(permit);
        };
        let asked = request(None);
        let (routed, ()) = tokio::join!(router.complete("slot_a", &asked), release);
        assert!(routed.is_ok());
        assert_eq!(backend.calls().len(), 1);
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
