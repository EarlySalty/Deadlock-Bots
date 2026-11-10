use dl_access_gate::{
    AuthorizeRequest, OutcomeRequest, MAX_OUTCOME_DETAIL_CHARS, MAX_REDACTED_ARGS_BYTES,
};
use serde_json::{json, Map, Value};
use std::time::Duration;
use tokio::{sync::mpsc, time::Instant};

const QUEUE: usize = 1024;
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
}

impl Recorder {
    pub fn start(pool: sqlx::PgPool) -> Self {
        let (queue, receiver) = mpsc::channel(QUEUE);
        tokio::spawn(write_loop(pool.clone(), receiver));
        Self { pool, queue }
    }

    pub async fn allow(&self, decision: &Decision, deadline: Instant) -> bool {
        match tokio::time::timeout_at(deadline, insert(&self.pool, decision)).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(%error, "access gate allow not recorded");
                false
            }
            Err(_) => {
                tracing::warn!("access gate allow not recorded before the decision timeout");
                false
            }
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
