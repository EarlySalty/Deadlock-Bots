use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::emulation;
use super::types::{ToolChatRequest, ToolChatResponse, Usage};
use crate::chat_provider::PROVIDER_ERROR;
use crate::upstream::limit_refusal;
use crate::{ChatMessage, ChatParams, ChatProvider, ChatProviderError};

pub struct ChatProviderBackend {
    inner: Arc<dyn ChatProvider>,
    records_own_usage: bool,
}

impl ChatProviderBackend {
    pub fn new(inner: Arc<dyn ChatProvider>, records_own_usage: bool) -> Self {
        Self {
            inner,
            records_own_usage,
        }
    }
}

fn provider_error(error: ChatProviderError) -> BackendError {
    if error == ChatProviderError::Timeout {
        return BackendError::new(FailureKind::Timeout, "provider_timeout");
    }
    let (Some(status), Some(reason)) = (error.upstream_status(), error.upstream_reason()) else {
        return BackendError::new(FailureKind::Unavailable, PROVIDER_ERROR);
    };
    let kind = if reason == PROVIDER_ERROR {
        FailureKind::InvalidOutput
    } else if limit_refusal(Some(status), reason) {
        FailureKind::Limit
    } else {
        FailureKind::Unavailable
    };
    BackendError::upstream(kind, reason, status)
}

fn json_payload(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    serde_json::from_str(trimmed.get(start..=end)?).ok()
}

#[async_trait]
impl ToolBackend for ChatProviderBackend {
    async fn complete(
        &self,
        request: &ToolChatRequest,
        target: &CallTarget,
    ) -> Result<ToolChatResponse, BackendError> {
        let prompt = emulation::prompt(request)?;
        if !prompt.images.is_empty() {
            return Err(BackendError::new(
                FailureKind::Unavailable,
                "image_input_unsupported",
            ));
        }
        let system = format!(
            "{}\n\nReturn exactly one JSON object that matches this JSON schema: {}",
            prompt.system, prompt.schema
        );
        let params = ChatParams {
            model: Some(target.model.clone()),
            max_tokens: Some(target.max_tokens),
            json_mode: true,
            system_prompt: Some(system),
            ..ChatParams::default()
        };
        let (response, upstream_status) = self
            .inner
            .chat_with_status(&[ChatMessage::user(prompt.user)], params)
            .await
            .map_err(provider_error)?;
        let answered = |error: BackendError| BackendError {
            status: upstream_status,
            ..error
        };
        let output = json_payload(&crate::strip_think(&response.content)).ok_or_else(|| {
            answered(BackendError::new(
                FailureKind::InvalidOutput,
                crate::usage::code::INVALID_JSON,
            ))
        })?;
        let (content, stop_reason) =
            emulation::decision(&output, &request.tools).map_err(answered)?;
        let usage = match (
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
        ) {
            (Some(prompt_tokens), Some(output_tokens)) => {
                let cache_read = response.usage.cached_tokens;
                Some(Usage {
                    input_tokens: prompt_tokens.saturating_sub(cache_read.unwrap_or(0)),
                    output_tokens,
                    cache_read_input_tokens: cache_read,
                    ..Usage::default()
                })
            }
            _ => None,
        };
        Ok(ToolChatResponse {
            model: response.model.unwrap_or_else(|| target.model.clone()),
            content,
            stop_reason,
            usage,
            request_id: None,
            upstream_status,
        })
    }

    fn records_own_usage(&self) -> bool {
        self.records_own_usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_chat::types::{ContentBlock, Message, Role, StopReason, ToolSpec};
    use crate::upstream::Account;
    use crate::{ChatResponse, MockChatProvider, TokenUsage};
    use serde_json::json;

    const CREDIT: &str =
        r#"{"error":{"message":"Your credit balance is too low to access the API."}}"#;
    const USAGE: &str = r#"{"error":{"message":"The usage limit has been reached."}}"#;

    struct Answered(Result<ChatResponse, ChatProviderError>);

    #[async_trait]
    impl ChatProvider for Answered {
        async fn chat(
            &self,
            _messages: &[ChatMessage],
            _params: ChatParams,
        ) -> Result<ChatResponse, ChatProviderError> {
            self.0.clone()
        }

        async fn chat_with_status(
            &self,
            _messages: &[ChatMessage],
            _params: ChatParams,
        ) -> Result<(ChatResponse, Option<u16>), ChatProviderError> {
            Ok((self.0.clone()?, Some(200)))
        }
    }

    fn question() -> ToolChatRequest {
        ToolChatRequest {
            system: String::new(),
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: "q".into() }],
            }],
            tools: Vec::new(),
            max_tokens: None,
            output_schema: None,
        }
    }

    fn served(content: &str, cached_tokens: Option<u64>) -> ChatResponse {
        ChatResponse {
            content: content.into(),
            model: None,
            usage: TokenUsage {
                prompt_tokens: Some(100),
                completion_tokens: Some(7),
                total_tokens: Some(107),
                cached_tokens,
            },
        }
    }

    async fn completed(
        provider: Arc<dyn ChatProvider>,
    ) -> Result<ToolChatResponse, (FailureKind, &'static str, Option<u16>)> {
        ChatProviderBackend::new(provider, false)
            .complete(
                &question(),
                &CallTarget {
                    model: "configured-model".into(),
                    max_tokens: 99,
                },
            )
            .await
            .map_err(|error| (error.kind, error.reason, error.status))
    }

    #[tokio::test]
    async fn answers_after_a_2xx_keep_the_status_and_split_off_cached_tokens() {
        let answer = r#"{"answer":"ok"}"#;
        for (cached, input_tokens, cache_read) in [
            (Some(60), 40, Some(60)),
            (Some(0), 100, Some(0)),
            (None, 100, None),
            (Some(100), 0, Some(100)),
        ] {
            let response = completed(Arc::new(Answered(Ok(served(answer, cached)))))
                .await
                .expect("served");
            assert_eq!(response.upstream_status, Some(200));
            let usage = response.usage.expect("usage");
            assert_eq!(
                (usage.input_tokens, usage.cache_read_input_tokens),
                (input_tokens, cache_read),
                "{cached:?}"
            );
            assert_eq!(usage.total_input(), 100, "{cached:?}");
            assert_eq!(usage.cache_creation_input_tokens, None);
        }
        assert_eq!(
            completed(Arc::new(Answered(Ok(served("no json", None))))).await,
            Err((FailureKind::InvalidOutput, "invalid_json", Some(200)))
        );
        let undecided = completed(Arc::new(Answered(Ok(served("{}", None)))))
            .await
            .expect_err("no decision");
        assert_eq!(
            (undecided.0, undecided.2),
            (FailureKind::InvalidOutput, Some(200))
        );
        let unusable = ChatProviderError::Upstream {
            status: 200,
            hinweis: "invalid JSON response".into(),
            reason: PROVIDER_ERROR,
        };
        assert_eq!(
            completed(Arc::new(Answered(Err(unusable)))).await,
            Err((FailureKind::InvalidOutput, "provider_error", Some(200)))
        );
        let unreported = completed(MockChatProvider::new(vec![Ok(served(answer, None))]))
            .await
            .expect("served without a status");
        assert_eq!(unreported.upstream_status, None);
        assert_eq!(
            completed(MockChatProvider::new(vec![Ok(served("no json", None))])).await,
            Err((FailureKind::InvalidOutput, "invalid_json", None))
        );
    }

    #[tokio::test]
    async fn chat_provider_rejects_image_input_instead_of_dropping_it() {
        let mock = MockChatProvider::new(Vec::new());
        let backend = ChatProviderBackend::new(mock.clone(), true);
        let request = ToolChatRequest {
            system: String::new(),
            messages: vec![Message {
                role: Role::User,
                content: vec![
                    ContentBlock::Image {
                        source: crate::tool_chat::types::ImageSource {
                            kind: crate::tool_chat::types::ImageSourceKind::Base64,
                            media_type: "image/png".into(),
                            data: "iVBORw0KGgo=".into(),
                        },
                    },
                    ContentBlock::Text { text: "q".into() },
                ],
            }],
            tools: Vec::new(),
            max_tokens: None,
            output_schema: None,
        };
        let error = backend
            .complete(
                &request,
                &CallTarget {
                    model: "configured-model".into(),
                    max_tokens: 99,
                },
            )
            .await
            .expect_err("image input must fail");
        assert_eq!(error.kind, FailureKind::Unavailable);
        assert!(error.kind.allows_fallback());
        assert_eq!(error.reason, "image_input_unsupported");
        assert!(mock.requests().is_empty());
    }

    #[test]
    fn provider_failures_keep_their_kind_and_carry_the_upstream_status() {
        let mapped = |error| {
            let mapped = provider_error(error);
            (mapped.kind, mapped.reason, mapped.status)
        };
        assert_eq!(
            mapped(ChatProviderError::RateLimit),
            (FailureKind::Limit, "http_429", Some(429))
        );
        let metered =
            |status, body| mapped(ChatProviderError::upstream(status, body, Account::Metered));
        let plan = |status, body| mapped(ChatProviderError::upstream(status, body, Account::Plan));
        assert_eq!(
            metered(403, ""),
            (FailureKind::Unavailable, "http_403", Some(403))
        );
        assert_eq!(
            metered(401, CREDIT),
            (FailureKind::Unavailable, "http_401", Some(401))
        );
        assert_eq!(
            metered(412, "{}"),
            (FailureKind::Limit, "billing", Some(412))
        );
        assert_eq!(
            metered(503, CREDIT),
            (FailureKind::Unavailable, "http_5xx", Some(503))
        );
        assert_eq!(
            metered(400, "{}"),
            (FailureKind::Unavailable, "http_4xx", Some(400))
        );
        for status in [400, 403] {
            for body in [CREDIT, USAGE] {
                assert_eq!(
                    metered(status, body),
                    (FailureKind::Limit, "billing", Some(status))
                );
            }
            assert_eq!(
                plan(status, CREDIT),
                (FailureKind::Limit, "billing", Some(status))
            );
            assert_eq!(
                plan(status, USAGE),
                (FailureKind::Limit, "usage_limit", Some(status))
            );
        }
        assert_eq!(
            metered(429, USAGE),
            (FailureKind::Limit, "http_429", Some(429))
        );
        assert_eq!(
            plan(429, USAGE),
            (FailureKind::Limit, "http_429", Some(429))
        );
        assert_eq!(
            plan(500, USAGE),
            (FailureKind::Limit, "usage_limit", Some(500))
        );
        assert_eq!(
            mapped(ChatProviderError::Provider("invalid JSON response".into())),
            (FailureKind::Unavailable, "provider_error", None)
        );
        assert_eq!(
            mapped(ChatProviderError::Timeout),
            (FailureKind::Timeout, "provider_timeout", None)
        );
    }

    #[tokio::test]
    async fn existing_chat_provider_emulates_a_tool_round_with_the_slot_model() {
        let mock = MockChatProvider::new(vec![Ok(ChatResponse {
            content: "```json\n{\"tool_calls\":[{\"name\":\"patch\",\"input\":{\"v\":1}}]}\n```"
                .into(),
            model: Some("served".into()),
            usage: TokenUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(2),
                total_tokens: Some(12),
                cached_tokens: None,
            },
        })]);
        let backend = ChatProviderBackend::new(mock.clone(), true);
        let request = ToolChatRequest {
            system: "sys".into(),
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: "q".into() }],
            }],
            tools: vec![ToolSpec {
                name: "patch".into(),
                description: String::new(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: None,
            output_schema: None,
        };
        let response = backend
            .complete(
                &request,
                &CallTarget {
                    model: "configured-model".into(),
                    max_tokens: 99,
                },
            )
            .await
            .expect("tool round");
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.usage.map(|u| u.input_tokens), Some(10));
        let requests = mock.requests();
        assert_eq!(requests[0].1.model.as_deref(), Some("configured-model"));
        assert_eq!(requests[0].1.max_tokens, Some(99));
        assert!(requests[0].1.json_mode);
        assert!(backend.records_own_usage());
    }
}
