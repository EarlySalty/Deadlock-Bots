//! Explizite Betriebs-Pins gelten auch dann, wenn ein älterer Konsument pro
//! Anfrage noch sein bisheriges Standardmodell mitschickt. Ohne Pin bleibt der
//! gesamte bisherige Aufrufvertrag erhalten (insbesondere JSON und Denk-Aus).
use crate::chat_provider::{
    ChatMessage, ChatParams, ChatProvider, ChatProviderError, ChatResponse, LlmProviderKind,
    LlmUseCase,
};
use std::{sync::Arc, time::Duration};

#[derive(Default)]
pub(crate) struct Overrides {
    model: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f64>,
    reasoning_effort: Option<String>,
    pub request_timeout: Option<Duration>,
}
impl Overrides {
    pub fn from_lookup(
        use_case: LlmUseCase,
        provider: LlmProviderKind,
        lookup: &impl Fn(&str) -> Option<String>,
    ) -> Self {
        let get = |prefix: &str| lookup(&format!("{prefix}{}", use_case.env_suffix()));
        let model = get("DL_LLM_MODEL_")
            .or_else(|| match provider {
                LlmProviderKind::Fireworks => {
                    lookup("FIREWORK_MODEL").or_else(|| lookup("FIREWORKS_MODEL"))
                }
                LlmProviderKind::OpenAi => {
                    lookup("OPENAI_MODEL").or_else(|| lookup("AI_OPENAI_MODEL"))
                }
                _ => None,
            })
            .filter(|value| !value.trim().is_empty());
        Self {
            model,
            max_tokens: get("DL_LLM_MAX_OUTPUT_TOKENS_").and_then(|value| value.parse().ok()),
            temperature: get("DL_LLM_TEMPERATURE_").and_then(|value| value.parse().ok()),
            reasoning_effort: get("DL_LLM_REASONING_EFFORT_"),
            request_timeout: get("DL_LLM_REQUEST_TIMEOUT_SECONDS_")
                .and_then(|value| value.parse().ok())
                .map(Duration::from_secs),
        }
    }
    pub fn wrap(self, inner: Arc<dyn ChatProvider>) -> Arc<dyn ChatProvider> {
        if self.model.is_none()
            && self.max_tokens.is_none()
            && self.temperature.is_none()
            && self.reasoning_effort.is_none()
        {
            inner
        } else {
            Arc::new(Configured {
                inner,
                overrides: self,
            })
        }
    }
}
struct Configured {
    inner: Arc<dyn ChatProvider>,
    overrides: Overrides,
}
#[async_trait::async_trait]
impl ChatProvider for Configured {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        mut params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        if let Some(model) = &self.overrides.model {
            params.model = Some(model.clone());
        }
        if let Some(tokens) = self.overrides.max_tokens {
            params.max_tokens = Some(tokens);
        }
        if let Some(temperature) = self.overrides.temperature {
            params.temperature = temperature;
        }
        if let Some(effort) = &self.overrides.reasoning_effort {
            params.reasoning_effort = Some(effort.clone());
        }
        self.inner.chat(messages, params).await
    }
    fn effective_model(&self, params: &ChatParams) -> Option<String> {
        self.overrides
            .model
            .clone()
            .or_else(|| self.inner.effective_model(params))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Spy(Mutex<Option<ChatParams>>);
    #[async_trait::async_trait]
    impl ChatProvider for Spy {
        async fn chat(
            &self,
            _messages: &[ChatMessage],
            params: ChatParams,
        ) -> Result<ChatResponse, ChatProviderError> {
            *self.0.lock().expect("Spy") = Some(params);
            Ok(ChatResponse::text("ok"))
        }
    }
    #[tokio::test]
    async fn explicit_config_beats_legacy_request_and_keeps_json_contract() {
        let lookup = |key: &str| match key {
            "DL_LLM_MODEL_BOT_PATE" => Some("accounts/fireworks/models/deepseek-v4p1-flash".into()),
            "FIREWORK_MODEL" => Some("accounts/fireworks/models/deepseek-v4-flash-0731".into()),
            "DL_LLM_MAX_OUTPUT_TOKENS_BOT_PATE" => Some("2048".into()),
            "DL_LLM_TEMPERATURE_BOT_PATE" => Some("0.4".into()),
            "DL_LLM_REASONING_EFFORT_BOT_PATE" => Some("none".into()),
            "DL_LLM_REQUEST_TIMEOUT_SECONDS_BOT_PATE" => Some("45".into()),
            _ => None,
        };
        let overrides =
            Overrides::from_lookup(LlmUseCase::BotPate, LlmProviderKind::Fireworks, &lookup);
        assert_eq!(overrides.request_timeout, Some(Duration::from_secs(45)));
        let spy = Arc::new(Spy::default());
        let provider = overrides.wrap(spy.clone());
        let params = ChatParams {
            model: Some("legacy-model".into()),
            json_mode: true,
            ..Default::default()
        };
        assert_eq!(
            provider.effective_model(&params).as_deref(),
            Some("accounts/fireworks/models/deepseek-v4p1-flash")
        );
        provider
            .chat(&[ChatMessage::user("synthetischer Test")], params)
            .await
            .expect("Aufruf");
        let observed = spy.0.lock().expect("Spy").clone().expect("Parameter");
        assert_eq!(
            observed.model.as_deref(),
            Some("accounts/fireworks/models/deepseek-v4p1-flash")
        );
        assert_eq!(observed.max_tokens, Some(2048));
        assert_eq!(observed.temperature, 0.4);
        assert_eq!(observed.reasoning_effort.as_deref(), Some("none"));
        assert!(observed.json_mode);
    }
    #[tokio::test]
    async fn provider_default_beats_legacy_request_without_crossing_providers() {
        for (provider, expected) in [
            (LlmProviderKind::Fireworks, "fireworks-default"),
            (LlmProviderKind::OpenAi, "openai-default"),
        ] {
            let lookup = |key: &str| match key {
                "FIREWORK_MODEL" => Some("fireworks-default".into()),
                "OPENAI_MODEL" => Some("openai-default".into()),
                _ => None,
            };
            let spy = Arc::new(Spy::default());
            let configured =
                Overrides::from_lookup(LlmUseCase::Faq, provider, &lookup).wrap(spy.clone());
            let params = ChatParams {
                model: Some("legacy-model".into()),
                reasoning_effort: Some("none".into()),
                json_mode: true,
                ..Default::default()
            };
            assert_eq!(
                configured.effective_model(&params).as_deref(),
                Some(expected)
            );
            configured.chat(&[], params).await.expect("Aufruf");
            let observed = spy.0.lock().expect("Spy").clone().expect("Parameter");
            assert_eq!(observed.model.as_deref(), Some(expected));
            assert_eq!(observed.reasoning_effort.as_deref(), Some("none"));
            assert!(observed.json_mode);
        }
    }

    #[tokio::test]
    async fn no_override_preserves_existing_per_call_options() {
        let spy = Arc::new(Spy::default());
        let provider =
            Overrides::from_lookup(LlmUseCase::Faq, LlmProviderKind::Fireworks, &|_| None)
                .wrap(spy.clone());
        let params = ChatParams {
            model: Some("existing-model".into()),
            reasoning_effort: Some("none".into()),
            json_mode: true,
            ..Default::default()
        };
        provider.chat(&[], params.clone()).await.expect("Aufruf");
        assert_eq!(
            spy.0.lock().expect("Spy").clone().expect("Parameter"),
            params
        );
    }
}
