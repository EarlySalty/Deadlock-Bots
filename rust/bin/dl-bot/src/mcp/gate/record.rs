use dl_access_gate::{
    AuthorizeRequest, OutcomeRequest, MAX_OUTCOME_DETAIL_CHARS, MAX_REDACTED_ARGS_BYTES,
};
use serde_json::{json, Map, Value};
use std::{
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

const QUEUE: usize = 1024;
const PENDING: u8 = 0;
const DELIVERED: u8 = 1;
const ABANDONED: u8 = 2;
const REVOKE_ATTEMPTS: u32 = 20;
const SECRET_MARKERS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "key",
    "authorization",
    "cookie",
    "credential",
];

pub struct Decision {
    pub decision_id: String,
    pub request: AuthorizeRequest,
    pub user_id: Option<i64>,
    pub channel_id: Option<i64>,
    pub target_user_id: Option<i64>,
    pub target_role_id: Option<i64>,
    pub allowed: bool,
    pub reason: &'static str,
    pub shadow: bool,
    pub confirm_required: bool,
    pub place_ok: bool,
}

enum Entry {
    Refusal(Box<Decision>),
    Outcome(OutcomeRequest),
}

#[derive(Clone, Copy)]
pub struct Retention {
    pub purge_every: Duration,
    pub days: i32,
}

#[derive(Clone)]
pub struct Recorder {
    pool: sqlx::PgPool,
    queue: mpsc::Sender<Entry>,
    revoke_every: Duration,
}

impl Recorder {
    pub fn start(pool: sqlx::PgPool, revoke_every: Duration) -> Self {
        let (queue, receiver) = mpsc::channel(QUEUE);
        tokio::spawn(write_loop(pool.clone(), receiver));
        Self {
            pool,
            queue,
            revoke_every,
        }
    }

    pub async fn allow(&self, decision: Decision, deadline: Instant) -> bool {
        let claim = Arc::new(AtomicU8::new(PENDING));
        let (done, finished) = oneshot::channel();
        tokio::spawn(store_allow(
            self.pool.clone(),
            decision,
            claim.clone(),
            self.revoke_every,
            done,
        ));
        if tokio::time::timeout_at(deadline, finished).await.is_err() {
            tracing::warn!("access gate allow not recorded before the decision timeout");
        }
        match claim.compare_exchange(PENDING, ABANDONED, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => false,
            Err(state) => state == DELIVERED,
        }
    }

    pub fn refusal(&self, decision: Decision) {
        if self
            .queue
            .try_send(Entry::Refusal(Box::new(decision)))
            .is_err()
        {
            tracing::warn!("access gate refusal dropped, record queue full");
        }
    }

    pub fn outcome(&self, outcome: OutcomeRequest) -> bool {
        self.queue.try_send(Entry::Outcome(outcome)).is_ok()
    }
}

fn without_nul(text: &str) -> String {
    text.replace('\0', "")
}

fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(redact_map(map)),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(text) => Value::String(without_nul(text)),
        other => other.clone(),
    }
}

fn redact_map(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .map(|(key, value)| {
            let key = without_nul(key);
            let lowered = key.to_ascii_lowercase();
            if SECRET_MARKERS.iter().any(|marker| lowered.contains(marker)) {
                (key, json!("[redacted]"))
            } else {
                (key, redact(value))
            }
        })
        .collect()
}

pub fn bounded(value: Option<&Map<String, Value>>) -> Option<Value> {
    let redacted = Value::Object(redact_map(value?));
    let bytes = redacted.to_string().len();
    if bytes > MAX_REDACTED_ARGS_BYTES {
        return Some(json!({"truncated": true, "bytes": bytes}));
    }
    Some(redacted)
}

pub fn bounded_detail(detail: Option<String>) -> Option<String> {
    detail
        .map(|text| {
            without_nul(&text)
                .chars()
                .take(MAX_OUTCOME_DETAIL_CHARS)
                .collect::<String>()
        })
        .filter(|text| !text.is_empty())
}

async fn write_loop(pool: sqlx::PgPool, mut receiver: mpsc::Receiver<Entry>) {
    while let Some(entry) = receiver.recv().await {
        match entry {
            Entry::Refusal(decision) => {
                if let Err(error) = insert(&pool, &decision).await {
                    tracing::warn!(%error, "access gate refusal not recorded");
                }
            }
            Entry::Outcome(outcome) => match attach(&pool, outcome).await {
                Ok(0) => tracing::warn!("access gate outcome matched no open decision"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "access gate outcome not recorded"),
            },
        }
    }
}

async fn store_allow(
    pool: sqlx::PgPool,
    decision: Decision,
    claim: Arc<AtomicU8>,
    revoke_every: Duration,
    done: oneshot::Sender<()>,
) {
    let inserted = insert(&pool, &decision).await;
    if let Err(error) = &inserted {
        tracing::warn!(%error, "access gate allow not recorded");
    }
    let delivered = inserted.is_ok()
        && claim
            .compare_exchange(PENDING, DELIVERED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
    let _ = done.send(());
    if !delivered {
        revoke(&pool, &decision.decision_id, revoke_every).await;
    }
}

async fn revoke(pool: &sqlx::PgPool, decision_id: &str, every: Duration) {
    for attempt in 1..=REVOKE_ATTEMPTS {
        match sqlx::query(
            "UPDATE bot.access_gate_decisions SET allowed = false, reason = 'record_unavailable', place_ok = false WHERE decision_id = $1 AND allowed",
        )
        .bind(decision_id)
        .execute(pool)
        .await
        {
            Ok(done) => {
                if done.rows_affected() > 0 {
                    tracing::warn!(decision_id, "access gate allow stored after its refusal, row revoked");
                }
                return;
            }
            Err(error) if attempt < REVOKE_ATTEMPTS => {
                tracing::warn!(%error, decision_id, attempt, "access gate allow revocation failed, retrying");
                tokio::time::sleep(every).await;
            }
            Err(error) => {
                tracing::error!(%error, decision_id, "access gate allow revocation gave up, row may show an allow that was refused");
            }
        }
    }
}

async fn insert(pool: &sqlx::PgPool, decision: &Decision) -> sqlx::Result<()> {
    let request = &decision.request;
    sqlx::query(
        "INSERT INTO bot.access_gate_decisions (decision_id, caller, origin, user_id, channel_id, dm, capability, class, internal, target_user_id, target_role_id, args_hash, args_redacted, allowed, reason, shadow, confirm_required, place_ok) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",
    )
    .bind(&decision.decision_id)
    .bind(&request.caller)
    .bind(request.origin.as_str())
    .bind(decision.user_id)
    .bind(decision.channel_id)
    .bind(request.dm)
    .bind(&request.capability)
    .bind(request.class.as_str())
    .bind(request.internal)
    .bind(decision.target_user_id)
    .bind(decision.target_role_id)
    .bind(&request.args_hash)
    .bind(bounded(request.args_redacted.as_ref()))
    .bind(decision.allowed)
    .bind(decision.reason)
    .bind(decision.shadow)
    .bind(decision.confirm_required)
    .bind(decision.place_ok)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn attach(pool: &sqlx::PgPool, outcome: OutcomeRequest) -> sqlx::Result<u64> {
    sqlx::query(
        "UPDATE bot.access_gate_decisions SET outcome = $3, outcome_detail = $4, outcome_at = now() WHERE decision_id = $1 AND caller = $2 AND outcome IS NULL",
    )
    .bind(&outcome.decision_id)
    .bind(&outcome.caller)
    .bind(outcome.status.as_str())
    .bind(bounded_detail(outcome.detail))
    .execute(pool)
    .await
    .map(|done| done.rows_affected())
}

pub async fn purge_loop(pool: sqlx::PgPool, retention: Retention) {
    let mut ticker = tokio::time::interval(retention.purge_every);
    loop {
        ticker.tick().await;
        let result = sqlx::query(
            "DELETE FROM bot.access_gate_decisions WHERE decided_at < now() - make_interval(days => $1)",
        )
        .bind(retention.days)
        .execute(&pool)
        .await;
        match result {
            Ok(done) if done.rows_affected() > 0 => {
                tracing::info!(rows = done.rows_affected(), "access gate decisions purged")
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "access gate purge failed"),
        }
    }
}
