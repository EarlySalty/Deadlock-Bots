use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use reqwest::header::HeaderValue;

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::config::ConfigError;
use super::types::{ContentBlock, StopReason, ToolChatRequest, ToolChatResponse, Usage};

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub struct AnthropicHttpBackend {
    http: reqwest::Client,
    messages_url: String,
    api_key: Option<HeaderValue>,
    workspace_id: Option<HeaderValue>,
}

impl AnthropicHttpBackend {
    pub fn new(base_url: &str, timeout: Duration) -> Self {
        Self::build(base_url, timeout, None, None)
    }

    pub fn with_api_key(
        base_url: &str,
        timeout: Duration,
        key: &str,
        workspace_id: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let api_key = sensitive_header(key, "API key")?;
        let workspace_id = workspace_id
            .map(|id| sensitive_header(id, "workspace id"))
            .transpose()?;
        Ok(Self::build(base_url, timeout, Some(api_key), workspace_id))
    }

    fn build(
        base_url: &str,
        timeout: Duration,
        api_key: Option<HeaderValue>,
        workspace_id: Option<HeaderValue>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            http,
            messages_url: format!("{}/messages", base_url.trim_end_matches('/')),
            api_key,
            workspace_id,
        }
    }

    fn request(&self, request: &ToolChatRequest, target: &CallTarget) -> reqwest::RequestBuilder {
        let builder = self
            .http
            .post(&self.messages_url)
            .header("anthropic-version", "2023-06-01")
            .json(&wire_request(request, target));
        let builder = match &self.api_key {
            Some(key) => builder.header("x-api-key", key.clone()),
            None => builder,
        };
        match &self.workspace_id {
            Some(id) => builder.header("anthropic-workspace-id", id.clone()),
            None => builder,
        }
    }
}

fn sensitive_header(value: &str, label: &str) -> Result<HeaderValue, ConfigError> {
    let mut header = HeaderValue::from_str(value.trim())
        .map_err(|_| ConfigError::Invalid(format!("{label} is not a valid header value")))?;
    header.set_sensitive(true);
    Ok(header)
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
    if let Some(schema) = &request.output_schema {
        body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
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

fn status_error(status: reqwest::StatusCode, body: &[u8]) -> BackendError {
    match status.as_u16() {
        429 => BackendError::new(FailureKind::Limit, "http_429"),
        529 => BackendError::new(FailureKind::Unavailable, "http_529"),
        400 | 402 | 403 if is_billing_error(body) => {
            BackendError::new(FailureKind::Limit, "billing")
        }
        400 | 404 | 413 | 422 => BackendError::new(FailureKind::InvalidRequest, "http_4xx"),
        _ => BackendError::new(FailureKind::Unavailable, "http_status"),
    }
}

fn is_billing_error(body: &[u8]) -> bool {
    let Ok(body) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let error = &body["error"];
    if error["type"].as_str() == Some("billing_error") {
        return true;
    }
    let message = error["message"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        "credit balance",
        "billing",
        "spend limit",
        "usage limit",
        "budget",
    ]
    .iter()
    .any(|marker| message.contains(marker))
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
            .request(request, target)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    BackendError::new(FailureKind::Timeout, "http_timeout")
                } else {
                    BackendError::new(FailureKind::Unavailable, "transport")
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = bounded_body(response).await.unwrap_or_default();
            return Err(status_error(status, &body));
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
    }

    #[test]
    fn api_failures_that_another_provider_can_serve_allow_fallback() {
        let billing = br#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#;
        for (status, body) in [
            (429, &b""[..]),
            (529, b""),
            (500, b""),
            (503, b""),
            (401, b""),
            (400, billing),
            (400, br#"{"error":{"type":"billing_error","message":"x"}}"#),
        ] {
            let status = reqwest::StatusCode::from_u16(status).expect("status");
            assert!(
                status_error(status, body).kind.allows_fallback(),
                "{status}"
            );
        }
        let invalid =
            br#"{"error":{"type":"invalid_request_error","message":"max_tokens too large"}}"#;
        assert_eq!(
            status_error(reqwest::StatusCode::BAD_REQUEST, invalid).kind,
            FailureKind::InvalidRequest
        );
    }

    fn plain_request(output_schema: Option<Value>) -> ToolChatRequest {
        ToolChatRequest {
            system: "sys".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: None,
            output_schema,
        }
    }

    #[test]
    fn wire_request_adds_output_config_only_when_schema_is_set() {
        let target = CallTarget {
            model: "m".into(),
            max_tokens: 7,
        };
        let schema = json!({"type": "object", "properties": {"answer": {"type": "string"}}, "required": ["answer"], "additionalProperties": false});
        let body = wire_request(&plain_request(Some(schema.clone())), &target);
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["output_config"]["format"]["schema"], schema);
        assert_eq!(body["system"], "sys");
        let plain = wire_request(&plain_request(None), &target);
        assert!(plain.get("output_config").is_none());
    }

    #[test]
    fn api_key_backend_sends_key_and_version_headers() {
        let backend = AnthropicHttpBackend::with_api_key(
            "https://api.example/v1/",
            Duration::from_secs(5),
            "sk-test",
            None,
        )
        .expect("backend");
        let request = ToolChatRequest {
            system: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: None,
            output_schema: None,
        };
        let target = CallTarget {
            model: "m".into(),
            max_tokens: 1,
        };
        let built = backend.request(&request, &target).build().expect("request");
        assert_eq!(built.url().as_str(), "https://api.example/v1/messages");
        assert_eq!(built.headers()["anthropic-version"], "2023-06-01");
        let key = &built.headers()["x-api-key"];
        assert_eq!(key, "sk-test");
        assert!(key.is_sensitive());
        assert!(!format!("{:?}", built.headers()).contains("sk-test"));
        assert!(built.headers().get("anthropic-workspace-id").is_none());
    }

    #[test]
    fn api_key_backend_sends_workspace_header_when_configured() {
        let backend = AnthropicHttpBackend::with_api_key(
            "https://api.example/v1",
            Duration::from_secs(5),
            "sk-test",
            Some("wrkspc_test"),
        )
        .expect("backend");
        let request = ToolChatRequest {
            system: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: None,
            output_schema: None,
        };
        let target = CallTarget {
            model: "m".into(),
            max_tokens: 1,
        };
        let built = backend.request(&request, &target).build().expect("request");
        let workspace = &built.headers()["anthropic-workspace-id"];
        assert_eq!(workspace, "wrkspc_test");
        assert!(workspace.is_sensitive());
    }
}
