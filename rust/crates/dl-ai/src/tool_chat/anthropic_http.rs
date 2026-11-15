use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use reqwest::header::HeaderValue;

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::config::ConfigError;
use super::types::{ContentBlock, StopReason, ToolChatRequest, ToolChatResponse, Usage};
use crate::upstream::{status_reason, BILLING};

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub struct AnthropicHttpBackend {
    http: reqwest::Client,
    messages_url: String,
    api_key: Option<HeaderValue>,
    workspace_id: Option<HeaderValue>,
    prompt_cache: bool,
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
            prompt_cache: api_key.is_some(),
            api_key,
            workspace_id,
        }
    }

    fn request(&self, request: &ToolChatRequest, target: &CallTarget) -> reqwest::RequestBuilder {
        let mut body = wire_request(request, target);
        if self.prompt_cache {
            mark_prompt_cache(&mut body);
        }
        let builder = self
            .http
            .post(&self.messages_url)
            .header("anthropic-version", "2023-06-01")
            .json(&body);
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

fn mark_prompt_cache(body: &mut Value) {
    let marker = json!({"type": "ephemeral"});
    if let Some(last) = body
        .get_mut("tools")
        .and_then(Value::as_array_mut)
        .and_then(|tools| tools.last_mut())
        .and_then(Value::as_object_mut)
    {
        last.insert("cache_control".into(), marker.clone());
    }
    let system = body
        .get("system")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(|text| json!([{"type": "text", "text": text, "cache_control": marker}]));
    if let (Some(system), Some(body)) = (system, body.as_object_mut()) {
        body.insert("system".into(), system);
    }
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
        upstream_status: None,
    })
}

fn status_error(status: reqwest::StatusCode, body: &[u8]) -> BackendError {
    let code = status.as_u16();
    let error = upstream_error(body);
    let reason = error
        .as_ref()
        .and_then(upstream_reason)
        .unwrap_or(status_reason(code));
    match code {
        429 => BackendError::upstream(FailureKind::Limit, reason, code),
        529 => BackendError::upstream(FailureKind::Unavailable, reason, code),
        400 | 402 | 403 if error.as_ref().is_some_and(is_billing_error) => {
            BackendError::upstream(FailureKind::Limit, BILLING, code)
        }
        400 | 404 | 413 | 422 => BackendError::upstream(FailureKind::InvalidRequest, reason, code),
        _ => BackendError::upstream(FailureKind::Unavailable, reason, code),
    }
}

fn upstream_error(body: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("error")
        .filter(|error| error.is_object())
        .cloned()
}

fn upstream_reason(error: &Value) -> Option<&'static str> {
    Some(match error["type"].as_str()? {
        "invalid_request_error" => "invalid_request_error",
        "authentication_error" => "authentication_error",
        "permission_error" => "permission_error",
        "not_found_error" => "not_found_error",
        "request_too_large" => "request_too_large",
        "rate_limit_error" => "rate_limit_error",
        "api_error" => "api_error",
        "overloaded_error" => "overloaded_error",
        "timeout_error" => "timeout_error",
        _ => return None,
    })
}

fn is_billing_error(error: &Value) -> bool {
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
        parse_response(&body).map(|response| ToolChatResponse {
            upstream_status: Some(status.as_u16()),
            ..response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{Message, Role, ToolSpec};
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

    #[test]
    fn billing_and_rate_limit_refusals_keep_distinct_reasons_and_upstream_status() {
        let billing = status_error(
            reqwest::StatusCode::BAD_REQUEST,
            br#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#,
        );
        let limited = status_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            br#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#,
        );
        let bare = status_error(reqwest::StatusCode::TOO_MANY_REQUESTS, b"");
        let auth = status_error(
            reqwest::StatusCode::UNAUTHORIZED,
            br#"{"error":{"type":"authentication_error","message":"x"}}"#,
        );
        assert_eq!(
            (billing.kind, billing.reason, billing.status),
            (FailureKind::Limit, "billing", Some(400))
        );
        assert_eq!(
            (limited.kind, limited.reason, limited.status),
            (FailureKind::Limit, "rate_limit_error", Some(429))
        );
        assert_eq!((bare.reason, bare.status), ("http_429", Some(429)));
        assert_eq!(
            (auth.kind, auth.reason, auth.status),
            (FailureKind::Unavailable, "authentication_error", Some(401))
        );
    }

    #[test]
    fn error_bodies_that_are_not_objects_fall_back_to_the_status_reason() {
        for body in [
            &b"[1,2]"[..],
            b"\"text\"",
            b"{\"error\":\"flat\"}",
            b"null",
            b"<html>",
        ] {
            let failure = status_error(reqwest::StatusCode::SERVICE_UNAVAILABLE, body);
            assert_eq!(
                (failure.kind, failure.reason, failure.status),
                (FailureKind::Unavailable, "http_5xx", Some(503))
            );
        }
        let unknown = status_error(
            reqwest::StatusCode::UNAUTHORIZED,
            br#"{"error":{"type":"new_kind","message":"x"}}"#,
        );
        assert_eq!((unknown.reason, unknown.status), ("http_401", Some(401)));
    }

    #[test]
    fn a_toolless_round_marks_only_the_system_prefix_and_never_adds_tools() {
        let mut final_round = wire_request(
            &ToolChatRequest {
                tools: Vec::new(),
                ..round_request(vec![user_text("question")])
            },
            &CallTarget {
                model: "m".into(),
                max_tokens: 1,
            },
        );
        mark_prompt_cache(&mut final_round);
        assert!(final_round.get("tools").is_none());
        assert_eq!(
            final_round["system"][0]["cache_control"]["type"],
            "ephemeral"
        );
        let mut bare = wire_request(
            &plain_request(None),
            &CallTarget {
                model: "m".into(),
                max_tokens: 1,
            },
        );
        bare.as_object_mut().map(|body| body.remove("system"));
        mark_prompt_cache(&mut bare);
        assert!(bare.get("system").is_none());
        assert!(bare.get("tools").is_none());
    }

    fn user_text(text: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    fn round_request(messages: Vec<Message>) -> ToolChatRequest {
        ToolChatRequest {
            system: "fixed system prefix".into(),
            messages,
            tools: vec![
                ToolSpec {
                    name: "find_entity".into(),
                    description: "a".into(),
                    input_schema: json!({"type": "object"}),
                },
                ToolSpec {
                    name: "get_hero".into(),
                    description: "b".into(),
                    input_schema: json!({"type": "object"}),
                },
            ],
            max_tokens: None,
            output_schema: None,
        }
    }

    #[test]
    fn api_key_path_marks_the_tools_and_system_prefix_identically_in_every_round() {
        let backend = AnthropicHttpBackend::with_api_key(
            "https://api.example/v1",
            Duration::from_secs(5),
            "sk-test",
            None,
        )
        .expect("backend");
        let target = CallTarget {
            model: "m".into(),
            max_tokens: 1,
        };
        let body = |request: &ToolChatRequest| -> Value {
            let built = backend.request(request, &target).build().expect("request");
            serde_json::from_slice(
                built
                    .body()
                    .and_then(reqwest::Body::as_bytes)
                    .expect("body"),
            )
            .expect("json")
        };
        let first = body(&round_request(vec![user_text("question")]));
        let second = body(&round_request(vec![
            user_text("question"),
            user_text("tool results"),
        ]));
        assert_eq!(first["tools"][1]["cache_control"]["type"], "ephemeral");
        assert!(first["tools"][0].get("cache_control").is_none());
        assert_eq!(first["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(first["system"][0]["text"], "fixed system prefix");
        assert_eq!(first["tools"].to_string(), second["tools"].to_string());
        assert_eq!(first["system"].to_string(), second["system"].to_string());
        let plan = AnthropicHttpBackend::new("http://127.0.0.1:1/v1", Duration::from_secs(5));
        let built = plan
            .request(&round_request(Vec::new()), &target)
            .build()
            .expect("request");
        let plain: Value = serde_json::from_slice(
            built
                .body()
                .and_then(reqwest::Body::as_bytes)
                .expect("body"),
        )
        .expect("json");
        assert_eq!(plain["system"], "fixed system prefix");
        assert!(!plain.to_string().contains("cache_control"));
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
