use dl_access_gate::{
    AuthorizeRequest, OutcomeRequest, MAX_OUTCOME_DETAIL_CHARS, MAX_REDACTED_ARGS_BYTES,
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;

const QUEUE: usize = 1024;
const PURGE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
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
    Decision(Box<Decision>),
    Outcome(OutcomeRequest),
}

#[derive(Clone)]
pub struct Recorder {
    queue: mpsc::Sender<Entry>,
}

impl Recorder {
    pub fn start(pool: sqlx::PgPool) -> Self {
        let (queue, receiver) = mpsc::channel(QUEUE);
        tokio::spawn(write_loop(pool.clone(), receiver));
        tokio::spawn(purge_loop(pool));
        Self { queue }
    }

    pub fn decision(&self, decision: Decision) -> bool {
        self.queue
            .try_send(Entry::Decision(Box::new(decision)))
            .is_ok()
    }

    pub fn outcome(&self, outcome: OutcomeRequest) -> bool {
        self.queue.try_send(Entry::Outcome(outcome)).is_ok()
    }
}

pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let lowered = key.to_ascii_lowercase();
                    if SECRET_MARKERS.iter().any(|marker| lowered.contains(marker)) {
                        (key.clone(), json!("[redacted]"))
                    } else {
                        (key.clone(), redact(value))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        other => other.clone(),
    }
}

pub fn bounded(value: Option<&Value>) -> Option<Value> {
    let redacted = redact(value?);
    let bytes = redacted.to_string().len();
    if bytes > MAX_REDACTED_ARGS_BYTES {
        return Some(json!({"truncated": true, "bytes": bytes}));
    }
    Some(redacted)
}

pub fn bounded_detail(detail: Option<String>) -> Option<String> {
    detail
        .map(|text| {
            text.chars()
                .take(MAX_OUTCOME_DETAIL_CHARS)
                .collect::<String>()
        })
        .filter(|text| !text.is_empty())
}

async fn write_loop(pool: sqlx::PgPool, mut receiver: mpsc::Receiver<Entry>) {
    while let Some(entry) = receiver.recv().await {
        let result = match entry {
            Entry::Decision(decision) => insert(&pool, &decision).await,
            Entry::Outcome(outcome) => attach(&pool, outcome).await,
        };
        if let Err(error) = result {
            tracing::warn!(%error, "access gate record write failed");
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

async fn attach(pool: &sqlx::PgPool, outcome: OutcomeRequest) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE bot.access_gate_decisions SET outcome = $3, outcome_detail = $4, outcome_at = now() WHERE decision_id = $1 AND caller = $2 AND outcome IS NULL",
    )
    .bind(&outcome.decision_id)
    .bind(&outcome.caller)
    .bind(outcome.status.as_str())
    .bind(bounded_detail(outcome.detail))
    .execute(pool)
    .await
    .map(|_| ())
}

async fn purge_loop(pool: sqlx::PgPool) {
    let mut ticker = tokio::time::interval(PURGE_EVERY);
    loop {
        ticker.tick().await;
        let result = sqlx::query(
            "DELETE FROM bot.access_gate_decisions WHERE decided_at < now() - interval '180 days'",
        )
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
