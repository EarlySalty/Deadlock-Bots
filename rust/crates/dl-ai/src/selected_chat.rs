//! Gemeinsame Flash-Auswahl vor jedem Textaufruf; keine eigene Katalogabfrage.
use crate::{ChatMessage, ChatParams, ChatProvider, ChatProviderError, ChatResponse};
use std::sync::Arc;

pub(crate) struct SelectedChat {
    inner: Arc<dyn ChatProvider>,
    select: fn() -> Result<String, fireworks_model_selection::SelectionError>,
}

impl SelectedChat {
    pub fn new(inner: Arc<dyn ChatProvider>) -> Self {
        Self {
            inner,
            select: fireworks_model_selection::selected_model,
        }
    }
}

#[async_trait::async_trait]
impl ChatProvider for SelectedChat {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        mut params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        params.model = Some((self.select)().map_err(|error| {
            ChatProviderError::Provider(format!("Gemeinsame Flash-Modellauswahl: {error}"))
        })?);
        params.reasoning_effort.get_or_insert_with(|| "none".into());
        self.inner.chat(messages, params).await
    }

    fn effective_model(&self, _params: &ChatParams) -> Option<String> {
        (self.select)().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static REVISION: AtomicUsize = AtomicUsize::new(0);
    fn selected() -> Result<String, fireworks_model_selection::SelectionError> {
        match REVISION.load(Ordering::SeqCst) {
            0 => Ok("accounts/fireworks/models/deepseek-v4p1-flash".into()),
            1 => Ok("accounts/fireworks/models/deepseek-v4p2-flash".into()),
            _ => Err(fireworks_model_selection::SelectionError::InvalidState),
        }
    }

    #[tokio::test]
    async fn requests_follow_shared_updates_and_invalid_state_never_calls_provider() {
        let mock = crate::MockChatProvider::new(vec![
            Ok(ChatResponse::text("eins")),
            Ok(ChatResponse::text("zwei")),
            Ok(ChatResponse::text("drei")),
        ]);
        let provider = SelectedChat {
            inner: mock.clone(),
            select: selected,
        };
        let params = ChatParams {
            model: Some("legacy-model".into()),
            json_mode: true,
            max_tokens: Some(100),
            reasoning_effort: None,
            ..Default::default()
        };
        for revision in 0..2 {
            REVISION.store(revision, Ordering::SeqCst);
            provider.chat(&[], params.clone()).await.expect("Aufruf");
        }
        let requests = mock.requests();
        assert_eq!(
            requests[0].1.model.as_deref(),
            Some("accounts/fireworks/models/deepseek-v4p1-flash")
        );
        assert_eq!(
            requests[1].1.model.as_deref(),
            Some("accounts/fireworks/models/deepseek-v4p2-flash")
        );
        assert!(requests
            .iter()
            .all(|(_, p)| p.json_mode && p.reasoning_effort.as_deref() == Some("none")));
        assert!(requests.iter().all(|(_, p)| p.max_tokens == Some(100)));
        let mut thinking = params.clone();
        thinking.reasoning_effort = Some("high".into());
        provider
            .chat(&[], thinking)
            .await
            .expect("Explizites Denken");
        assert_eq!(
            mock.requests()[2].1.reasoning_effort.as_deref(),
            Some("high")
        );
        REVISION.store(2, Ordering::SeqCst);
        assert!(provider.chat(&[], params).await.is_err());
        assert_eq!(mock.requests().len(), 3);
    }
}
