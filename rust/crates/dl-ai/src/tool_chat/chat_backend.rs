use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::emulation;
use super::types::{ToolChatRequest, ToolChatResponse, Usage};
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
    match error {
        ChatProviderError::Timeout => BackendError::new(FailureKind::Timeout, "provider_timeout"),
        ChatProviderError::RateLimit => {
            BackendError::new(FailureKind::Limit, "provider_rate_limit")
        }
        ChatProviderError::Auth => BackendError::new(FailureKind::Unavailable, "provider_auth"),
        ChatProviderError::Provider(_) => {
            BackendError::new(FailureKind::Unavailable, "provider_error")
        }
    }
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
        let response = self
            .inner
            .chat(&[ChatMessage::user(prompt.user)], params)
            .await
            .map_err(provider_error)?;
        let output = json_payload(&crate::strip_think(&response.content))
            .ok_or_else(|| BackendError::new(FailureKind::InvalidOutput, "invalid_json"))?;
        let (content, stop_reason) = emulation::decision(&output, &request.tools)?;
        let usage = match (
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
        ) {
            (Some(input_tokens), Some(output_tokens)) => Some(Usage {
                input_tokens,
                output_tokens,
                ..Usage::default()
            }),
            _ => None,
        };
        Ok(ToolChatResponse {
            model: response.model.unwrap_or_else(|| target.model.clone()),
            content,
            stop_reason,
            usage,
            request_id: None,
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
    use crate::{ChatResponse, MockChatProvider, TokenUsage};
    use serde_json::json;

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
