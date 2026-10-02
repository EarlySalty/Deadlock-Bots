//! `POST /internal/master/v1/clips/submit` — Twitch-Clips von Partner-Streamern
//! für den wöchentlichen Clip-Contest (Community-Streamer-Brücke, Paket D).
//!
//! Gleiche Auth wie alle Broker-Routen (Loopback + `X-Internal-Token`),
//! gleicher Envelope. Fachliche Entscheidungen (Partnerkanal, Clip-URL,
//! Duplikat je Woche, Idempotenz über `idempotency_key`) trifft der Port;
//! die Route prüft nur die Form der Anfrage.
//!
//! Antwort im `result`: `{"status":"accepted"|"duplicate"|"rejected",
//! "submission_id":123,"reason":null}` mit HTTP 200. Formfehler → 400.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::Response,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{authorize, error_body, request_id, respond, success_body, SharedBroker};

pub const MAX_BODY_BYTES: usize = 8192;
pub const MAX_IDEMPOTENCY_KEY_LEN: usize = 128;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_CLIP_URL_CHARS: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwitchClipSubmission {
    pub clip_url: String,
    pub streamer_twitch_user_id: String,
    pub streamer_login: String,
    pub submitted_by_twitch_user_id: Option<String>,
    pub title: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipSubmitStatus {
    Accepted,
    Duplicate,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClipSubmitOutcome {
    pub status: ClipSubmitStatus,
    pub submission_id: Option<i64>,
    pub reason: Option<String>,
}

#[async_trait::async_trait]
pub trait ClipSubmitPort: Send + Sync {
    async fn submit_twitch_clip(
        &self,
        submission: TwitchClipSubmission,
    ) -> Result<ClipSubmitOutcome, String>;
}

#[derive(Clone)]
struct ClipState {
    broker: SharedBroker,
    port: Arc<dyn ClipSubmitPort>,
}

pub fn router(broker: SharedBroker, port: Arc<dyn ClipSubmitPort>) -> Router {
    Router::new()
        .route("/internal/master/v1/clips/submit", post(submit))
        .with_state(ClipState { broker, port })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitRequest {
    source: String,
    clip_url: String,
    streamer_twitch_user_id: String,
    streamer_login: String,
    #[serde(default)]
    submitted_by_twitch_user_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    idempotency_key: String,
}

fn valid_twitch_id(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('0')
        && value.len() <= 20
        && value.bytes().all(|b| b.is_ascii_digit())
}

fn valid_login(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 25
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn valid_idempotency_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDEMPOTENCY_KEY_LEN
        && value.bytes().all(|b| b.is_ascii_graphic())
}

/// Form prüfen und normalisieren (Login klein, Titel getrimmt, leer → None).
fn validate(request: SubmitRequest) -> Option<TwitchClipSubmission> {
    if request.source != "twitch" {
        return None;
    }
    let streamer_login = request.streamer_login.trim().to_ascii_lowercase();
    let clip_url = request.clip_url.trim().to_string();
    let title = request
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let submitted_by = request
        .submitted_by_twitch_user_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    let ok = valid_twitch_id(&request.streamer_twitch_user_id)
        && valid_login(&streamer_login)
        && valid_idempotency_key(&request.idempotency_key)
        && !clip_url.is_empty()
        && clip_url.chars().count() <= MAX_CLIP_URL_CHARS
        && submitted_by.as_deref().is_none_or(valid_twitch_id)
        && title.as_deref().is_none_or(|t| {
            t.chars().count() <= MAX_TITLE_CHARS && !t.chars().any(char::is_control)
        });
    ok.then_some(TwitchClipSubmission {
        clip_url,
        streamer_twitch_user_id: request.streamer_twitch_user_id,
        streamer_login,
        submitted_by_twitch_user_id: submitted_by,
        title,
        idempotency_key: request.idempotency_key,
    })
}

fn failure(rid: &str, status: u16, code: &str) -> Response {
    respond(
        status,
        error_body(rid, None, code, "Clip-Einreichung nicht verfügbar"),
    )
}

async fn submit(
    State(state): State<ClipState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = authorize(&state.broker, &peer, &headers, &rid) {
        return response;
    }
    if body.len() > MAX_BODY_BYTES {
        return failure(&rid, 413, "payload_too_large");
    }
    let Ok(request) = serde_json::from_slice::<SubmitRequest>(&body) else {
        return failure(&rid, 400, "bad_request");
    };
    let Some(submission) = validate(request) else {
        return failure(&rid, 400, "bad_request");
    };
    let key = submission.idempotency_key.clone();
    match state.port.submit_twitch_clip(submission).await {
        Ok(outcome) => respond(200, success_body(&rid, Some(&key), json!(outcome))),
        Err(error) => {
            tracing::warn!(%error, "Clip-Einreichung nicht verfügbar");
            failure(&rid, 503, "unavailable")
        }
    }
}

#[cfg(test)]
#[path = "clips_tests.rs"]
mod tests;
