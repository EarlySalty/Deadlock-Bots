use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::types::{ContentBlock, StopReason, ToolChatRequest, ToolChatResponse, Usage};

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub struct AnthropicHttpBackend {
    http: reqwest::Client,
    messages_url: String,
}

impl AnthropicHttpBackend {
    pub fn new(base_url: &str, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            http,
            messages_url: format!("{}/messages", base_url.trim_end_matches('/')),
        }
    }
}

pub fn wire_request(request: &ToolChatRequest, target: &CallTarget) -> Value {
    let mut body = json!({
        "model": target.model,
        "max_tokens": target.max_tokens,
        "messages": request.messages,
    });
    if !request.system.is_empty() {
        body["system"] = json!(request.system);
    }
    if !request.tools.is_empty() {
        body["tools"] = json!(request.tools);
    }
    body
}

pub fn parse_response(body: &Value) -> Result<ToolChatResponse, BackendError> {
    let invalid = |reason| BackendError::new(FailureKind::InvalidOutput, reason);
    let model = body["model"]
        .as_str()
        .ok_or_else(|| invalid("missing_model"))?
        .to_owned();
    let mut content = Vec::new();
    for block in body["content"]
        .as_array()
        .ok_or_else(|| invalid("missing_content"))?
    {
        match block["type"].as_str() {
            Some("thinking" | "redacted_thinking") => {}
            Some("text" | "tool_use") => content.push(
                serde_json::from_value::<ContentBlock>(block.clone())
                    .map_err(|_| invalid("content_block"))?,
            ),
            _ => return Err(invalid("unknown_block")),
        }
    }
    let stop_reason = match body["stop_reason"].as_str() {
        Some("end_turn" | "stop_sequence") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        _ => return Err(invalid("stop_reason")),
    };
    let usage = serde_json::from_value::<Usage>(body["usage"].clone()).ok();
    Ok(ToolChatResponse {
        model,
        content,
        stop_reason,
        usage,
        request_id: body["id"].as_str().map(str::to_owned),
    })
}

fn status_error(status: reqwest::StatusCode) -> BackendError {
    match status.as_u16() {
        429 => BackendError::new(FailureKind::Limit, "http_429"),
        400 | 404 | 413 | 422 => BackendError::new(FailureKind::InvalidRequest, "http_4xx"),
        _ => BackendError::new(FailureKind::Unavailable, "http_status"),
    }
}

async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, BackendError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| BackendError::new(FailureKind::Unavailable, "body_transport"))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(BackendError::new(
                FailureKind::InvalidOutput,
                "body_too_large",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[async_trait]
impl ToolBackend for AnthropicHttpBackend {
    async fn complete(
        &self,
        request: &ToolChatRequest,
        target: &CallTarget,
    ) -> Result<ToolChatResponse, BackendError> {
        let response = self
            .http
            .post(&self.messages_url)
            .header("anthropic-version", "2023-06-01")
            .json(&wire_request(request, target))
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    BackendError::new(FailureKind::Timeout, "http_timeout")
                } else {
                    BackendError::new(FailureKind::Unavailable, "transport")
                }
            })?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let body = bounded_body(response).await?;
        let body: Value = serde_json::from_slice(&body)
            .map_err(|_| BackendError::new(FailureKind::InvalidOutput, "invalid_json"))?;
        parse_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_response_maps_blocks_stop_reason_and_usage() {
        let parsed = parse_response(&json!({
            "id": "msg_1", "model": "m", "stop_reason": "tool_use",
            "content": [{"type": "thinking", "thinking": "x", "signature": "s"}, {"type": "tool_use", "id": "t1", "name": "n", "input": {}}],
            "usage": {"input_tokens": 3, "output_tokens": 4}
        }))
        .expect("parsed");
        assert_eq!(parsed.stop_reason, StopReason::ToolUse);
        assert_eq!(parsed.content.len(), 1);
        assert_eq!(parsed.usage.map(|u| u.output_tokens), Some(4));
        assert_eq!(
            status_error(reqwest::StatusCode::TOO_MANY_REQUESTS).kind,
            FailureKind::Limit
        );
        assert_eq!(
            status_error(reqwest::StatusCode::BAD_GATEWAY).kind,
            FailureKind::Unavailable
        );
    }
}
