//! Master-Broker — localhost-IPC für Discord-Aktionen.
//!
//! API-kompatibel zu `service/master_broker.py` (`/internal/master/v1/*`):
//! gleiche Routen, gleicher Antwort-Envelope
//! `{ok, request_id, idempotency_key, cached, result, error}`, gleiche
//! Fehlertexte, gleiche Idempotenz-Semantik. Konsumenten (Twitch-Bot u. a.)
//! merken beim Cutover nichts.

// Handler geben frühe HTTP-Fehlerantworten als Err(Response) zurück — axum-idiomatisch.
#![allow(clippy::result_large_err)]

mod handlers;
mod idempotency;
pub mod payload;
pub mod port;

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};

pub use idempotency::{IdempotencyConfig, IdempotencyStore};
pub use port::{DiscordPort, PortError};

pub const TOKEN_HEADER: &str = "X-Internal-Token";
pub const IDEMPOTENCY_HEADER: &str = "X-Idempotency-Key";
pub const REQUEST_ID_HEADER: &str = "X-Request-Id";

/// Allowlist (Channel/Guild/Role) — ENV-Namen wie das Original.
#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    pub enabled: bool,
    pub ids: HashSet<u64>,
}

impl Allowlist {
    fn from_env(lookup: &impl Fn(&str) -> Option<String>, names: &[&str]) -> Self {
        for name in names {
            if let Some(raw) = lookup(name) {
                let ids: HashSet<u64> = raw
                    .split([',', ';', ' '])
                    .filter_map(|t| t.trim().parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .collect();
                // Gesetzt (auch leer) → Allowlist aktiv, wie im Original.
                return Self { enabled: true, ids };
            }
        }
        Self::default()
    }

    pub fn permits(&self, value: u64) -> bool {
        !self.enabled || self.ids.contains(&value)
    }
}

pub struct BrokerState {
    pub port: Arc<dyn DiscordPort>,
    pub token: String,
    pub store: IdempotencyStore,
    pub channel_allowlist: Allowlist,
    pub guild_allowlist: Allowlist,
    pub role_allowlist: Allowlist,
}

pub type SharedBroker = Arc<BrokerState>;

impl BrokerState {
    /// `token` ist Pflicht (wie im Original: leerer Token = Konstruktionsfehler).
    pub fn new(
        port: Arc<dyn DiscordPort>,
        token: String,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<SharedBroker, String> {
        let token = token.trim().to_string();
        if token.is_empty() {
            return Err("master broker token must not be empty".to_string());
        }
        Ok(Arc::new(Self {
            port,
            token,
            store: IdempotencyStore::new(IdempotencyConfig::from_env(&lookup)),
            channel_allowlist: Allowlist::from_env(
                &lookup,
                &[
                    "MASTER_BROKER_ALLOWED_CHANNEL_IDS",
                    "MASTER_BROKER_ALLOW_CHANNEL_IDS",
                    "MASTER_BROKER_CHANNEL_ALLOWLIST_IDS",
                ],
            ),
            guild_allowlist: Allowlist::from_env(
                &lookup,
                &[
                    "MASTER_BROKER_ALLOWED_GUILD_IDS",
                    "MASTER_BROKER_ALLOW_GUILD_IDS",
                    "MASTER_BROKER_GUILD_ALLOWLIST_IDS",
                ],
            ),
            role_allowlist: Allowlist::from_env(
                &lookup,
                &[
                    "MASTER_BROKER_ALLOWED_ROLE_IDS",
                    "MASTER_BROKER_ALLOW_ROLE_IDS",
                    "MASTER_BROKER_ROLE_ALLOWLIST_IDS",
                ],
            ),
        }))
    }
}

pub fn router(state: SharedBroker) -> Router {
    Router::new()
        .route("/internal/master/v1/health", get(handlers::health))
        .route(
            "/internal/master/v1/discord/roles",
            get(handlers::list_roles),
        )
        .route(
            "/internal/master/v1/discord/role-members",
            get(handlers::role_members),
        )
        .route(
            "/internal/master/v1/discord/member-access",
            get(handlers::member_access),
        )
        .route(
            "/internal/master/v1/discord/resolve-names",
            get(handlers::resolve_names),
        )
        .route(
            "/internal/master/v1/discord/guild-stats",
            get(handlers::guild_stats),
        )
        .route(
            "/internal/master/v1/discord/send-message",
            post(handlers::send_message),
        )
        .route(
            "/internal/master/v1/discord/create-channel",
            post(handlers::create_channel),
        )
        .route(
            "/internal/master/v1/discord/delete-channel",
            post(handlers::delete_channel),
        )
        .route(
            "/internal/master/v1/discord/send-rich-message",
            post(handlers::send_rich_message),
        )
        .route(
            "/internal/master/v1/discord/edit-rich-message",
            post(handlers::edit_rich_message),
        )
        .route(
            "/internal/master/v1/discord/member/add-role",
            post(handlers::add_role),
        )
        .route(
            "/internal/master/v1/discord/member/remove-role",
            post(handlers::remove_role),
        )
        .route(
            "/internal/master/v1/discord/member/move-voice",
            post(handlers::move_voice),
        )
        .route(
            "/internal/master/v1/discord/voice-channel/members",
            post(handlers::voice_members),
        )
        .route(
            "/internal/master/v1/discord/create-invite",
            post(handlers::create_invite),
        )
        .route(
            "/internal/master/v1/discord/send-dm",
            post(handlers::send_dm),
        )
        .with_state(state)
}

// ── Envelope + Auth ────────────────────────────────────────────────────────

pub(crate) fn random_request_id() -> String {
    hex::encode(rand::random::<[u8; 8]>())
}

pub(crate) fn request_id(headers: &HeaderMap) -> String {
    let candidate = headers
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim();
    if !candidate.is_empty() && !candidate.contains(['\r', '\n']) {
        candidate.chars().take(128).collect()
    } else {
        random_request_id()
    }
}

pub(crate) fn error_body(
    request_id: &str,
    idempotency_key: Option<&str>,
    code: &str,
    message: &str,
) -> Value {
    json!({
        "ok": false,
        "request_id": request_id,
        "idempotency_key": idempotency_key,
        "cached": false,
        "result": null,
        "error": {"code": code, "message": message},
    })
}

pub(crate) fn success_body(
    request_id: &str,
    idempotency_key: Option<&str>,
    result: Value,
) -> Value {
    json!({
        "ok": true,
        "request_id": request_id,
        "idempotency_key": idempotency_key,
        "cached": false,
        "result": result,
        "error": null,
    })
}

pub(crate) fn respond(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(body),
    )
        .into_response()
}

/// Konstante-Zeit-Vergleich (wie secrets.compare_digest; Längen-Leak ok).
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Loopback + Token — exakt wie `_authorize` (403 forbidden / 401 unauthorized).
pub(crate) fn authorize(
    state: &BrokerState,
    peer: &ConnectInfo<std::net::SocketAddr>,
    headers: &HeaderMap,
    rid: &str,
) -> Result<(), Response> {
    if !peer.0.ip().is_loopback() {
        tracing::warn!(peer = %peer.0, "Broker: Nicht-Loopback-Anfrage abgelehnt");
        return Err(respond(
            403,
            error_body(rid, None, "forbidden", "loopback requests only"),
        ));
    }
    let presented = headers
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim();
    if presented.is_empty() || !constant_time_eq(presented, &state.token) {
        tracing::warn!(peer = %peer.0, "Broker: ungültiger Token abgelehnt");
        return Err(respond(
            401,
            error_body(
                rid,
                None,
                "unauthorized",
                &format!("missing or invalid {TOKEN_HEADER}"),
            ),
        ));
    }
    Ok(())
}

/// Nur-Loopback-Check für die Diagnose-Routen (kein Token).
pub(crate) fn require_loopback(
    peer: &ConnectInfo<std::net::SocketAddr>,
    rid: &str,
) -> Result<(), Response> {
    if peer.0.ip().is_loopback() {
        Ok(())
    } else {
        Err(respond(
            403,
            error_body(rid, None, "forbidden", "loopback requests only"),
        ))
    }
}

/// Kanonischer Payload-Hash. Nur prozess-intern relevant (Konflikt-Erkennung
/// bei Key-Wiederverwendung) — muss NICHT byte-gleich zu Pythons
/// `json.dumps(ensure_ascii=True, …)` sein.
pub(crate) fn payload_hash(payload: &Map<String, Value>) -> String {
    use sha2::Digest;
    let sorted: std::collections::BTreeMap<&String, &Value> = payload.iter().collect();
    let encoded = serde_json::to_string(&sorted).unwrap_or_default();
    hex::encode(sha2::Sha256::digest(encoded.as_bytes()))
}

/// Idempotenz-Rahmen wie `_run_idempotent_action`.
pub(crate) async fn run_idempotent<F, Fut>(
    state: &BrokerState,
    rid: &str,
    action: &str,
    idem_key: &str,
    hash: &str,
    operation: F,
) -> Response
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = (u16, Value)>,
{
    use idempotency::Decision;

    let decision = match state.store.begin(action, idem_key, hash).await {
        Ok(d) => d,
        Err(message) => {
            return respond(
                409,
                error_body(rid, Some(idem_key), "idempotency_conflict", &message),
            )
        }
    };

    match decision {
        Decision::Cached(status, mut body) => {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("cached".to_string(), json!(true));
            }
            respond(status, body)
        }
        Decision::Pending(mut rx) => {
            let waited = tokio::time::timeout(state.store.waiter_timeout(), async {
                loop {
                    if let Some(result) = rx.borrow().clone() {
                        return result;
                    }
                    if rx.changed().await.is_err() {
                        return (
                            500,
                            error_body(
                                rid,
                                Some(idem_key),
                                "internal_error",
                                "failed to await idempotent result",
                            ),
                        );
                    }
                }
            })
            .await;
            match waited {
                Ok((status, mut body)) => {
                    if let Some(obj) = body.as_object_mut() {
                        obj.insert("cached".to_string(), json!(true));
                    }
                    respond(status, body)
                }
                Err(_) => respond(
                    504,
                    error_body(
                        rid,
                        Some(idem_key),
                        "idempotency_wait_timeout",
                        "idempotent operation is still pending; retry later",
                    ),
                ),
            }
        }
        Decision::Execute => {
            let (status, body) = operation().await;
            let cache = body.get("ok").and_then(Value::as_bool).unwrap_or(false)
                && (200..300).contains(&status);
            state
                .store
                .settle(action, idem_key, hash, status, &body, cache)
                .await;
            respond(status, body)
        }
    }
}
