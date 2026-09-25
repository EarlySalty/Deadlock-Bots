//! Opt-in implementation of the existing command answer port. No runtime wiring.
//! Auth, allowlists, commands, cooldowns and Discord formatting stay in their current owners.
use crate::{AiAnswerer, BrainError, BrainOutcome};
use brain_client::{AnswerProfile, AnswerStatus, AsyncBrainClient, PublicAnswerResponse, Query};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub struct BrainApiAnswerer {
    client: AsyncBrainClient,
    namespace: String,
    scopes: BTreeSet<String>,
    sequence: AtomicU64,
    admission: tokio::sync::Semaphore,
    timeout: Duration,
}
impl BrainApiAnswerer {
    /// `namespace` must be a unique opaque invocation/process namespace, not a user name.
    /// Scopes come from trusted composition, never chat text. The server still authenticates
    /// the token and binds its principal. This constructor neither reads nor changes config.
    pub fn new(
        endpoint: &str,
        token: &str,
        timeout: Duration,
        namespace: String,
        game_scopes: BTreeSet<String>,
    ) -> Result<Self, BrainError> {
        if namespace.is_empty()
            || namespace.len() > 128
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || game_scopes.is_empty()
        {
            return Err(backend_error());
        }
        let client =
            AsyncBrainClient::new_local(endpoint, token, timeout).map_err(|_| backend_error())?;
        let candidate = Self {
            client,
            namespace,
            scopes: game_scopes,
            sequence: AtomicU64::new(0),
            admission: tokio::sync::Semaphore::new(4),
            timeout,
        };
        candidate
            .query("validation")?
            .validate()
            .map_err(|_| backend_error())?;
        Ok(candidate)
    }
    fn query(&self, question: &str) -> Result<Query, BrainError> {
        let sequence = self
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| backend_error())?;
        let id = format!("{}-{sequence}", self.namespace);
        // The legacy AiAnswerer port has no authenticated conversation identity.
        // Use a distinct one-shot conversation, never a shared cross-user cache scope.
        Ok(Query {
            request_id: id.clone(),
            conversation_id: id,
            text: question.to_owned(),
            requested_scopes: self.scopes.clone(),
            profile: AnswerProfile::Explain,
            patch: None,
            mode: None,
        })
    }
}
fn backend_error() -> BrainError {
    BrainError::Backend("Brain API nicht verfügbar oder Vertrag ungültig".into())
}
fn project(response: PublicAnswerResponse) -> Result<BrainOutcome, BrainError> {
    match response.status {
        AnswerStatus::Answered => {
            // Retain the existing GameOnly text budget and URL-free response convention.
            if response.text.encode_utf16().count() > 3800
                || response.text.contains("http://")
                || response.text.contains("https://")
            {
                return Err(backend_error());
            }
            Ok(BrainOutcome::Answer(response.text))
        }
        AnswerStatus::InsufficientEvidence => Ok(BrainOutcome::NoAnswer),
        AnswerStatus::UnauthorizedEvidence
        | AnswerStatus::ProviderError
        | AnswerStatus::BudgetExceeded => Err(backend_error()),
    }
}
#[async_trait::async_trait]
impl AiAnswerer for BrainApiAnswerer {
    async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError> {
        if question.trim().is_empty() || question.chars().count() > 4000 {
            return Err(backend_error());
        }
        tokio::time::timeout(self.timeout, async {
            let _permit = self
                .admission
                .acquire()
                .await
                .map_err(|_| backend_error())?;
            let query = self.query(question)?;
            let response = self
                .client
                .answer(&query)
                .await
                .map_err(|_| backend_error())?;
            project(response)
        })
        .await
        .map_err(|_| backend_error())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{handle_brain_query, BrainConfig, BrainCooldowns};
    use brain_client::{PublicCitation, PUBLIC_API_VERSION};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn fixture(statuses: Vec<AnswerStatus>) -> (String, thread::JoinHandle<Vec<Query>>) {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("offline fixture operation must succeed");
        let endpoint = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("offline fixture operation must succeed")
        );
        let thread = thread::spawn(move || {
            let mut queries = Vec::new();
            for status in statuses {
                let (mut stream, _) = listener
                    .accept()
                    .expect("offline fixture operation must succeed");
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .expect("offline fixture operation must succeed");
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                let body =
                    loop {
                        let n = stream
                            .read(&mut buffer)
                            .expect("offline fixture operation must succeed");
                        assert!(n > 0);
                        request.extend_from_slice(&buffer[..n]);
                        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&request[..end]);
                            assert!(headers
                                .lines()
                                .any(|l| l
                                    .eq_ignore_ascii_case("authorization: Bearer fixture-token")));
                            let len: usize = headers
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase().strip_prefix("content-length:").map(
                                        |s| {
                                            s.trim()
                                                .parse()
                                                .expect("offline fixture operation must succeed")
                                        },
                                    )
                                })
                                .expect("offline fixture operation must succeed");
                            if request.len() >= end + 4 + len {
                                break request[end + 4..].to_vec();
                            }
                        }
                        assert!(request.len() < 70 * 1024);
                    };
                let query: Query =
                    serde_json::from_slice(&body).expect("offline fixture operation must succeed");
                let response = PublicAnswerResponse {
                    contract_version: PUBLIC_API_VERSION.into(),
                    request_id: query.request_id.clone(),
                    knowledge_release: "fixture-release".into(),
                    status,
                    text: "Antwort äöü 🧪".into(),
                    citations: if status == AnswerStatus::Answered {
                        vec![PublicCitation {
                            citation_id: "opaque".into(),
                            label: "Beleg".into(),
                        }]
                    } else {
                        vec![]
                    },
                };
                queries.push(query);
                let body = serde_json::to_string(&response)
                    .expect("offline fixture operation must succeed");
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).expect("offline fixture operation must succeed");
            }
            queries
        });
        (endpoint, thread)
    }
    fn adapter(endpoint: &str) -> BrainApiAnswerer {
        BrainApiAnswerer::new(
            endpoint,
            "fixture-token",
            Duration::from_secs(2),
            "fixture-run".into(),
            BTreeSet::from(["fixture.game".into()]),
        )
        .expect("offline fixture operation must succeed")
    }
    #[tokio::test]
    async fn existing_usage_length_cooldown_and_text_semantics_are_preserved() {
        let (endpoint, server) = fixture(vec![AnswerStatus::Answered]);
        let backend = adapter(&endpoint);
        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        };
        let cooldowns = BrainCooldowns::default();
        assert_eq!(
            handle_brain_query("", 1, &config, &cooldowns, &backend).await,
            BrainOutcome::Usage
        );
        assert_eq!(
            handle_brain_query(&"x".repeat(21), 1, &config, &cooldowns, &backend).await,
            BrainOutcome::TooLong { len: 21 }
        );
        assert_eq!(
            handle_brain_query("Abrams", 1, &config, &cooldowns, &backend).await,
            BrainOutcome::Answer("Antwort äöü 🧪".into())
        );
        assert!(matches!(
            handle_brain_query("Abrams", 1, &config, &cooldowns, &backend).await,
            BrainOutcome::Cooldown { .. }
        ));
        let queries = server
            .join()
            .expect("offline fixture operation must succeed");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].text, "Abrams");
        assert_eq!(
            queries[0].requested_scopes,
            BTreeSet::from(["fixture.game".into()])
        );
    }
    #[tokio::test]
    async fn backend_failures_do_not_consume_cooldown_or_trigger_fallbacks() {
        let (endpoint, server) = fixture(vec![
            AnswerStatus::ProviderError,
            AnswerStatus::UnauthorizedEvidence,
        ]);
        let backend = adapter(&endpoint);
        let config = BrainConfig {
            max_question_len: 20,
            cooldown_secs: 20,
        };
        let cooldowns = BrainCooldowns::default();
        for _ in 0..2 {
            assert_eq!(
                handle_brain_query("Abrams", 1, &config, &cooldowns, &backend).await,
                BrainOutcome::BackendError
            );
            assert!(cooldowns.lock().await.is_empty());
        }
        let queries = server
            .join()
            .expect("offline fixture operation must succeed");
        assert_ne!(queries[0].conversation_id, queries[1].conversation_id);
        assert_ne!(queries[0].request_id, queries[1].request_id);
    }
    #[tokio::test]
    async fn missing_evidence_remains_no_answer() {
        let (endpoint, server) = fixture(vec![AnswerStatus::InsufficientEvidence]);
        assert_eq!(
            adapter(&endpoint)
                .answer("Abrams")
                .await
                .expect("offline fixture operation must succeed"),
            BrainOutcome::NoAnswer
        );
        server
            .join()
            .expect("offline fixture operation must succeed");
    }
    #[test]
    fn configuration_is_explicit_local_and_scope_bound() {
        assert!(BrainApiAnswerer::new(
            "https://example.invalid",
            "fixture-token",
            Duration::from_secs(1),
            "run".into(),
            BTreeSet::from(["fixture.game".into()])
        )
        .is_err());
        assert!(BrainApiAnswerer::new(
            "http://127.0.0.1:1",
            "fixture-token",
            Duration::from_secs(1),
            "run".into(),
            BTreeSet::new()
        )
        .is_err());
        let backend = adapter("http://127.0.0.1:1");
        assert_eq!(backend.admission.available_permits(), 4);
        assert_eq!(
            backend
                .query("ignore scopes")
                .expect("offline fixture operation must succeed")
                .requested_scopes,
            BTreeSet::from(["fixture.game".into()])
        );
    }
}
