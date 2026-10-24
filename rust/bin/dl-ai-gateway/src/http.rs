use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use dl_ai::tool_chat::{
    FailureKind, Message, RouteError, Routed, SlotRouter, StopReason, ToolChatRequest, ToolSpec,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub const SLOT_HEADER: &str = "x-dl-ai-slot";

static MESSAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
struct WireRequest {
    max_tokens: Option<u32>,
    #[serde(default)]
    system: Option<SystemPrompt>,
    messages: Vec<Message>,
    #[serde(default)]
    tools: Vec<ToolSpec>,
    #[serde(default)]
    tool_choice: Option<ToolChoice>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    slot: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SystemPrompt {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SystemBlock {
    Text { text: String },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToolChoice {
    Auto,
    None,
}

impl SystemPrompt {
    fn into_text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Blocks(blocks) => blocks
                .into_iter()
                .map(|SystemBlock::Text { text }| text)
                .collect::<Vec<_>>()
                .join("\n\n"),
        }
    }
}

pub fn app(router: Arc<SlotRouter>, max_request_bytes: usize) -> Router {
    Router::new()
        .route("/v1/messages", post(messages))
        .route("/healthz", get(health))
        .layer(DefaultBodyLimit::max(max_request_bytes))
        .with_state(router)
}

async fn health(State(router): State<Arc<SlotRouter>>) -> Json<Value> {
    Json(json!({"ok": true, "slots": router.slot_names()}))
}

fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"type": "error", "error": {"type": kind, "message": message}})),
    )
        .into_response()
}

fn invalid(message: &str) -> Response {
    error(StatusCode::BAD_REQUEST, "invalid_request_error", message)
}

fn slot_of(headers: &HeaderMap, body: Option<&str>) -> Result<String, &'static str> {
    let header = match headers.get(SLOT_HEADER) {
        Some(value) => Some(value.to_str().map_err(|_| "slot header is not text")?),
        None => None,
    };
    match (header, body) {
        (Some(header), Some(body)) if header != body => Err("slot header and slot field disagree"),
        (Some(slot), _) | (None, Some(slot)) => Ok(slot.to_owned()),
        (None, None) => Err("slot is required (x-dl-ai-slot header or slot field)"),
    }
}

fn into_request(wire: WireRequest) -> Result<ToolChatRequest, &'static str> {
    if wire.stream {
        return Err("streaming is not supported");
    }
    if wire.messages.is_empty() {
        return Err("messages must not be empty");
    }
    let tools = match wire.tool_choice {
        Some(ToolChoice::None) => Vec::new(),
        Some(ToolChoice::Auto) | None => wire.tools,
    };
    Ok(ToolChatRequest {
        system: wire.system.map(SystemPrompt::into_text).unwrap_or_default(),
        messages: wire.messages,
        tools,
        max_tokens: wire.max_tokens,
    })
}

fn failure_response(failure: RouteError) -> Response {
    match failure {
        RouteError::UnknownSlot => invalid("unknown slot"),
        RouteError::Failed { last, .. } => {
            let (status, kind) = match last.kind {
                FailureKind::Limit => (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error"),
                FailureKind::Busy | FailureKind::Overloaded => (
                    StatusCode::from_u16(529).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
                    "overloaded_error",
                ),
                FailureKind::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout_error"),
                FailureKind::InvalidRequest => (StatusCode::BAD_REQUEST, "invalid_request_error"),
                FailureKind::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "api_error"),
                FailureKind::InvalidOutput => (StatusCode::BAD_GATEWAY, "api_error"),
            };
            error(status, kind, &last.to_string())
        }
    }
}

fn message_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let counter = MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("msg_gw{nanos:x}{counter:x}")
}

fn success_response(slot: &str, routed: Routed) -> Response {
    let response = routed.response;
    let usage = response.usage.unwrap_or_default();
    let stop_reason = match response.stop_reason {
        StopReason::EndTurn => "end_turn",
        StopReason::ToolUse => "tool_use",
        StopReason::MaxTokens => "max_tokens",
    };
    let mut reply = Json(json!({
        "id": message_id(),
        "type": "message",
        "role": "assistant",
        "model": response.model,
        "content": response.content,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": usage,
    }))
    .into_response();
    let headers = reply.headers_mut();
    for (name, value) in [
        (SLOT_HEADER, slot.to_owned()),
        ("x-dl-ai-provider", routed.provider),
        ("x-dl-ai-attempts", routed.attempts.to_string()),
    ] {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }
    reply
}

async fn messages(
    State(router): State<Arc<SlotRouter>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let wire: WireRequest = match serde_json::from_slice(&body) {
        Ok(wire) => wire,
        Err(_) => return invalid("body is not a supported Messages request"),
    };
    let slot = match slot_of(&headers, wire.slot.as_deref()) {
        Ok(slot) => slot,
        Err(reason) => return invalid(reason),
    };
    let request = match into_request(wire) {
        Ok(request) => request,
        Err(reason) => return invalid(reason),
    };
    let started = Instant::now();
    let outcome = router.complete(&slot, &request).await;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match outcome {
        Ok(routed) => {
            tracing::info!(
                slot = %slot,
                provider = %routed.provider,
                attempts = routed.attempts,
                tools = request.tools.len(),
                latency_ms,
                "AI_GATEWAY_SERVED"
            );
            success_response(&slot, routed)
        }
        Err(failure) => {
            tracing::warn!(slot = %slot, latency_ms, failure = ?failure, "AI_GATEWAY_FAILED");
            failure_response(failure)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use dl_ai::tool_chat::{
        BackendError, CallTarget, ContentBlock, LedgerEntry, LedgerOutcome, ProviderHandle, Target,
        ToolBackend, ToolChatResponse, Usage, UsageLedger,
    };
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::time::Duration;
    use tower::ServiceExt;

    struct ToolThenAnswer {
        seen: Mutex<Vec<ToolChatRequest>>,
    }

    #[async_trait]
    impl ToolBackend for ToolThenAnswer {
        async fn complete(
            &self,
            request: &ToolChatRequest,
            target: &CallTarget,
        ) -> Result<ToolChatResponse, BackendError> {
            if let Ok(mut seen) = self.seen.lock() {
                seen.push(request.clone());
            }
            let has_result = request.messages.iter().any(|message| {
                message
                    .content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
            });
            let (content, stop_reason) = if has_result {
                (
                    vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    StopReason::EndTurn,
                )
            } else {
                (
                    vec![ContentBlock::ToolUse {
                        id: "toolu_1".into(),
                        name: "lookup".into(),
                        input: json!({"q": "x"}),
                    }],
                    StopReason::ToolUse,
                )
            };
            Ok(ToolChatResponse {
                model: target.model.clone(),
                content,
                stop_reason,
                usage: Some(Usage {
                    input_tokens: 4,
                    output_tokens: 1,
                    ..Usage::default()
                }),
                request_id: None,
            })
        }
    }

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

    fn test_app() -> (Router, Arc<ToolThenAnswer>) {
        let backend = Arc::new(ToolThenAnswer {
            seen: Mutex::new(Vec::new()),
        });
        let router = SlotRouter::new(
            BTreeMap::from([(
                "brain_answer".to_owned(),
                vec![Target {
                    provider: "fake".into(),
                    model: "fake-model".into(),
                    timeout: Duration::from_secs(5),
                    max_tokens: 100,
                    attempts: 1,
                    retry_backoff: Duration::ZERO,
                    deadline: Duration::from_secs(5),
                }],
            )]),
            BTreeMap::from([(
                "fake".to_owned(),
                ProviderHandle::new(backend.clone(), 1, Duration::from_secs(1)),
            )]),
            Arc::new(NoLedger),
        );
        (app(Arc::new(router), 1 << 20), backend)
    }

    async fn post(app: &Router, slot: Option<&str>, body: Value) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::post("/v1/messages").header("content-type", "application/json");
        if let Some(slot) = slot {
            request = request.header(SLOT_HEADER, slot);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body");
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn messages_api_runs_a_tool_round_on_the_requested_slot() {
        let (app, backend) = test_app();
        let tools = json!([{"name": "lookup", "description": "d", "input_schema": {"type": "object"}, "cache_control": {"type": "ephemeral"}}]);
        let (status, headers, first) = post(
            &app,
            Some("brain_answer"),
            json!({"model": "ignored", "max_tokens": 50, "system": [{"type": "text", "text": "sys"}], "tools": tools, "messages": [{"role": "user", "content": "q"}], "temperature": 0.1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers
                .get("x-dl-ai-provider")
                .and_then(|v| v.to_str().ok()),
            Some("fake")
        );
        assert_eq!(first["stop_reason"], "tool_use");
        assert_eq!(first["model"], "fake-model");
        assert_eq!(first["usage"]["input_tokens"], 4);
        let tool_use = first["content"][0].clone();
        assert_eq!(tool_use["type"], "tool_use");
        let (status, _, second) = post(
            &app,
            None,
            json!({"slot": "brain_answer", "max_tokens": 50, "tools": tools, "messages": [
                {"role": "user", "content": "q"},
                {"role": "assistant", "content": [tool_use]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": [{"type": "text", "text": "r"}]}]}
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second["stop_reason"], "end_turn");
        assert_eq!(second["content"], json!([{"type": "text", "text": "done"}]));
        let seen = backend.seen.lock().map(|s| s.clone()).unwrap_or_default();
        assert_eq!(seen[0].system, "sys");
        assert_eq!(seen[0].max_tokens, Some(50));
        assert_eq!(seen[0].tools.len(), 1);
    }

    #[tokio::test]
    async fn missing_conflicting_or_unknown_slots_and_streams_are_rejected() {
        let (app, _) = test_app();
        let base = json!({"max_tokens": 5, "messages": [{"role": "user", "content": "q"}]});
        assert_eq!(
            post(&app, None, base.clone()).await.0,
            StatusCode::BAD_REQUEST
        );
        let mut conflicting = base.clone();
        conflicting["slot"] = json!("other");
        assert_eq!(
            post(&app, Some("brain_answer"), conflicting).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            post(&app, Some("nope"), base.clone()).await.0,
            StatusCode::BAD_REQUEST
        );
        let mut stream = base;
        stream["stream"] = json!(true);
        assert_eq!(
            post(&app, Some("brain_answer"), stream).await.0,
            StatusCode::BAD_REQUEST
        );
    }
}
