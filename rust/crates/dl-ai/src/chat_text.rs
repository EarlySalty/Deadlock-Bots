//! Bruecke vom Compliance-geprueften [`ChatProvider`] auf den
//! [`TextGenerator`]-Trait, an dem die bestehenden Konsumenten haengen.
//!
//! Ohne diese Bruecke gab es zwei Wege zum Modell: den geprueften ueber
//! [`crate::LlmProviderConfig::build_provider_for_env`] und einen zweiten, der
//! die Clients direkt aus der Umgebung baute und dabei am Gate vorbeilief.

use std::sync::Arc;

use crate::chat_provider::{ChatMessage, ChatParams, ChatProvider, LlmUseCase};
use crate::{GenerateRequest, TextGenerator, DEFAULT_MAX_OUTPUT_TOKENS};

/// Verpackt einen [`ChatProvider`] als [`TextGenerator`].
///
/// `json_mode` bildet ab, dass der frueher direkt verdrahtete Fireworks-Client
/// bei jeder Anfrage `response_format=json_object` gesetzt hat; Konsumenten,
/// die JSON parsen, brauchen das weiterhin.
pub struct ChatTextGenerator {
    provider: Arc<dyn ChatProvider>,
    use_case: LlmUseCase,
    json_mode: bool,
}

impl ChatTextGenerator {
    /// Freitext-Antworten (kein erzwungenes JSON).
    pub fn new(provider: Arc<dyn ChatProvider>, use_case: LlmUseCase) -> Arc<Self> {
        Arc::new(Self {
            provider,
            use_case,
            json_mode: false,
        })
    }

    /// Antworten, die der Konsument als JSON-Objekt parst.
    pub fn new_json(provider: Arc<dyn ChatProvider>, use_case: LlmUseCase) -> Arc<Self> {
        Arc::new(Self {
            provider,
            use_case,
            json_mode: true,
        })
    }

    fn params_for(&self, request: &GenerateRequest) -> ChatParams {
        ChatParams {
            model: request.model.clone(),
            max_tokens: Some(
                request
                    .max_output_tokens
                    .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
            ),
            json_mode: self.json_mode,
            temperature: request.temperature,
            system_prompt: request.system_prompt.clone(),
        }
    }
}

#[async_trait::async_trait]
impl TextGenerator for ChatTextGenerator {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String> {
        if let Some(effort) = request.reasoning_effort.as_deref() {
            tracing::warn!(
                use_case = self.use_case.as_str(),
                reasoning_effort = effort,
                "reasoning_effort wird vom ChatProvider-Weg nicht durchgereicht"
            );
        }
        let params = self.params_for(&request);
        let model = params.model.clone();
        let messages = [ChatMessage::user(request.prompt)];
        match self.provider.chat(&messages, params).await {
            Ok(response) if response.content.trim().is_empty() => {
                tracing::warn!(
                    use_case = self.use_case.as_str(),
                    model = response
                        .model
                        .as_deref()
                        .or(model.as_deref())
                        .unwrap_or("default"),
                    "LLM-Antwort war leer"
                );
                None
            }
            Ok(response) => {
                tracing::debug!(
                    use_case = self.use_case.as_str(),
                    model = response
                        .model
                        .as_deref()
                        .or(model.as_deref())
                        .unwrap_or("default"),
                    chars = response.content.len(),
                    "LLM-Antwort erhalten"
                );
                Some(response.content)
            }
            Err(error) => {
                tracing::warn!(
                    use_case = self.use_case.as_str(),
                    model = model.as_deref().unwrap_or("default"),
                    %error,
                    "LLM-Anfrage fehlgeschlagen"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::chat_provider::{ChatProviderError, ChatResponse};

    #[derive(Default)]
    struct SpyProvider {
        seen: Mutex<Vec<(Vec<ChatMessage>, ChatParams)>>,
        answer: Option<String>,
    }

    #[async_trait::async_trait]
    impl ChatProvider for SpyProvider {
        async fn chat(
            &self,
            messages: &[ChatMessage],
            params: ChatParams,
        ) -> Result<ChatResponse, ChatProviderError> {
            if let Ok(mut seen) = self.seen.lock() {
                seen.push((messages.to_vec(), params));
            }
            match &self.answer {
                Some(answer) => Ok(ChatResponse::text(answer.clone())),
                None => Err(ChatProviderError::RateLimit),
            }
        }
    }

    fn request(model: Option<&str>) -> GenerateRequest {
        GenerateRequest {
            prompt: "Frage".to_string(),
            system_prompt: Some("System".to_string()),
            model: model.map(str::to_string),
            max_output_tokens: Some(123),
            reasoning_effort: None,
            temperature: 0.7,
        }
    }

    #[tokio::test]
    async fn modellwahl_aus_der_anfrage_erreicht_den_provider() {
        let provider = Arc::new(SpyProvider {
            seen: Mutex::new(Vec::new()),
            answer: Some("Antwort".to_string()),
        });
        let generator = ChatTextGenerator::new(provider.clone(), LlmUseCase::TurnierVorschlag);

        let answer = generator
            .generate_text(request(Some("gpt-eigenes-modell")))
            .await;

        assert_eq!(answer.as_deref(), Some("Antwort"));
        let seen = provider.seen.lock().expect("Spy-Aufzeichnung");
        let (messages, params) = seen.first().expect("genau ein Aufruf");
        assert_eq!(params.model.as_deref(), Some("gpt-eigenes-modell"));
        assert_eq!(params.max_tokens, Some(123));
        assert_eq!(params.system_prompt.as_deref(), Some("System"));
        assert!(!params.json_mode);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Frage");
    }

    #[tokio::test]
    async fn json_variante_setzt_den_json_modus() {
        let provider = Arc::new(SpyProvider {
            seen: Mutex::new(Vec::new()),
            answer: Some("{}".to_string()),
        });
        let generator = ChatTextGenerator::new_json(provider.clone(), LlmUseCase::ModerationText);

        let answer = generator.generate_text(request(None)).await;

        assert_eq!(answer.as_deref(), Some("{}"));
        let seen = provider.seen.lock().expect("Spy-Aufzeichnung");
        let (_, params) = seen.first().expect("genau ein Aufruf");
        assert!(params.json_mode);
    }

    #[tokio::test]
    async fn provider_fehler_liefert_none_statt_leerer_antwort() {
        let provider = Arc::new(SpyProvider {
            seen: Mutex::new(Vec::new()),
            answer: None,
        });
        let generator = ChatTextGenerator::new(provider, LlmUseCase::Faq);

        assert_eq!(generator.generate_text(request(None)).await, None);
    }

    #[tokio::test]
    async fn ohne_token_grenze_gilt_der_bisherige_standardwert() {
        let provider = Arc::new(SpyProvider {
            seen: Mutex::new(Vec::new()),
            answer: Some("Antwort".to_string()),
        });
        let generator = ChatTextGenerator::new(provider.clone(), LlmUseCase::Faq);
        let mut req = request(None);
        req.max_output_tokens = None;

        let _ = generator.generate_text(req).await;

        let seen = provider.seen.lock().expect("Spy-Aufzeichnung");
        let (_, params) = seen.first().expect("genau ein Aufruf");
        assert_eq!(params.max_tokens, Some(DEFAULT_MAX_OUTPUT_TOKENS));
    }
}
