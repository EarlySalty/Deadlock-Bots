//! Optionaler typisierter Antwortpfad für den vorhandenen Brain-Befehl.
//! Authentifizierung, Kanalfreigaben und Ausgabe bleiben bei den bisherigen Komponenten.
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
    /// `namespace` ist ein technischer Aufrufbezeichner ohne Benutzernamen.
    /// Scopes stammen aus der geprüften Konfiguration. Der Server authentifiziert
    /// das Token und bindet den Principal; der Konstruktor lädt keine Konfiguration.
    pub fn new(
        endpoint: &str,
        token: &str,
        timeout: Duration,
        namespace: String,
        game_scopes: BTreeSet<String>,
    ) -> Result<Self, BrainError> {
        if namespace.is_empty()
            || namespace.len() > 64
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || game_scopes.is_empty()
        {
            return Err(backend_error());
        }
        // Container und Neustarts können dieselbe PID verwenden.
        // Jede Adapterinstanz erhält deshalb einen eigenen zufälligen 128-Bit-Bezeichner.
        let instance_nonce = rand::random::<u128>();
        let namespace = format!("{namespace}-{instance_nonce:032x}");
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
        let mut current = self.sequence.load(Ordering::Relaxed);
        let sequence = loop {
            let next = current.checked_add(1).ok_or_else(backend_error)?;
            match self.sequence.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(previous) => break previous,
                Err(actual) => current = actual,
            }
        };
        let id = format!("{}-{sequence}", self.namespace);
        // Der Antwortport liefert keine authentifizierte Gesprächsidentität.
        // Jede Anfrage bekommt ein eigenes Gespräch, damit kein nutzerübergreifender Cache entsteht.
        Ok(Query {
            request_id: id.clone(),
            conversation_id: id,
            text: question.to_owned(),
            domain: None,
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
fn validated_answer(text: String) -> Result<BrainOutcome, BrainError> {
    if text.encode_utf16().count() > 3800 || text.contains("http://") || text.contains("https://") {
        return Err(backend_error());
    }
    Ok(BrainOutcome::Answer(text))
}
fn project(response: PublicAnswerResponse) -> Result<BrainOutcome, BrainError> {
    match response.status {
        AnswerStatus::Answered | AnswerStatus::BuildRejected => validated_answer(response.text),
        AnswerStatus::InsufficientEvidence => Ok(BrainOutcome::NoAnswer),
        AnswerStatus::UnauthorizedEvidence
        | AnswerStatus::Unavailable
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

    const FIXTURE_BEARER: &str = "brain-fixture-bearer";

    fn fixture(statuses: Vec<AnswerStatus>) -> (String, thread::JoinHandle<Vec<Query>>) {
        fixture_with_text(statuses, "Antwort äöü 🧪".into())
    }

    fn fixture_with_text(
        statuses: Vec<AnswerStatus>,
        response_text: String,
    ) -> (String, thread::JoinHandle<Vec<Query>>) {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("Isolierte Testoperation muss gelingen");
        let endpoint = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("Isolierte Testoperation muss gelingen")
        );
        listener
            .set_nonblocking(true)
            .expect("Isolierte Testoperation muss gelingen");
        let thread = thread::spawn(move || {
            let mut queries = Vec::new();
            for status in statuses {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(accepted) => break accepted,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "Testclient verbindet nicht rechtzeitig"
                            );
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => panic!("Isolierte Testverbindung fehlgeschlagen"),
                    }
                };
                stream
                    .set_nonblocking(false)
                    .expect("Isolierte Testoperation muss gelingen");
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .expect("Isolierte Testoperation muss gelingen");
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .expect("Isolierte Testoperation muss gelingen");
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                let body = loop {
                    let n = stream
                        .read(&mut buffer)
                        .expect("Isolierte Testoperation muss gelingen");
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let expected_auth = format!("authorization: Bearer {FIXTURE_BEARER}");
                        assert!(headers
                            .lines()
                            .any(|line| line.eq_ignore_ascii_case(&expected_auth)));
                        let len: usize = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| {
                                        s.trim()
                                            .parse()
                                            .expect("Isolierte Testoperation muss gelingen")
                                    })
                            })
                            .expect("Isolierte Testoperation muss gelingen");
                        if request.len() >= end + 4 + len {
                            break request[end + 4..].to_vec();
                        }
                    }
                    assert!(request.len() < 70 * 1024);
                };
                let query: Query =
                    serde_json::from_slice(&body).expect("Isolierte Testoperation muss gelingen");
                let response = PublicAnswerResponse {
                    contract_version: PUBLIC_API_VERSION.into(),
                    request_id: query.request_id.clone(),
                    knowledge_release: "fixture-release".into(),
                    status,
                    text: response_text.clone(),
                    citations: if matches!(
                        status,
                        AnswerStatus::Answered | AnswerStatus::BuildRejected
                    ) {
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
                    .expect("Isolierte Testoperation muss gelingen");
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).expect("Isolierte Testoperation muss gelingen");
            }
            queries
        });
        (endpoint, thread)
    }
    fn adapter(endpoint: &str) -> BrainApiAnswerer {
        BrainApiAnswerer::new(
            endpoint,
            FIXTURE_BEARER,
            Duration::from_secs(2),
            "fixture-run".into(),
            BTreeSet::from(["fixture.game".into()]),
        )
        .expect("Isolierte Testoperation muss gelingen")
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
            .expect("Isolierte Testoperation muss gelingen");
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
            .expect("Isolierte Testoperation muss gelingen");
        assert_ne!(queries[0].conversation_id, queries[1].conversation_id);
        assert_ne!(queries[0].request_id, queries[1].request_id);
    }
    #[tokio::test]
    async fn build_rejected_is_visible_but_unavailable_is_backend_failure() {
        let (endpoint, server) =
            fixture(vec![AnswerStatus::BuildRejected, AnswerStatus::Unavailable]);
        let backend = adapter(&endpoint);
        assert_eq!(
            backend
                .answer("illegaler Build")
                .await
                .expect("domain rejection is a typed answer"),
            BrainOutcome::Answer("Antwort äöü 🧪".into())
        );
        assert!(backend.answer("Abrams").await.is_err());
        server
            .join()
            .expect("Isolierte Testoperation muss gelingen");
    }

    #[tokio::test]
    async fn build_rejected_rejects_urls_and_text_over_3800_utf16_units() {
        let (endpoint, server) = fixture_with_text(
            vec![AnswerStatus::BuildRejected],
            "Mehr unter https://example.invalid".into(),
        );
        assert!(matches!(
            adapter(&endpoint).answer("illegaler Build").await,
            Err(BrainError::Backend(_))
        ));
        server
            .join()
            .expect("Isolierte Testoperation muss gelingen");

        let (endpoint, server) =
            fixture_with_text(vec![AnswerStatus::BuildRejected], "🧪".repeat(1901));
        assert!(matches!(
            adapter(&endpoint).answer("illegaler Build").await,
            Err(BrainError::Backend(_))
        ));
        server
            .join()
            .expect("Isolierte Testoperation muss gelingen");
    }

    #[tokio::test]
    async fn missing_evidence_remains_no_answer() {
        let (endpoint, server) = fixture(vec![AnswerStatus::InsufficientEvidence]);
        assert_eq!(
            adapter(&endpoint)
                .answer("Abrams")
                .await
                .expect("Isolierte Testoperation muss gelingen"),
            BrainOutcome::NoAnswer
        );
        server
            .join()
            .expect("Isolierte Testoperation muss gelingen");
    }
    #[test]
    fn independent_instances_never_share_request_or_conversation_ids() {
        let first = BrainApiAnswerer::new(
            "http://127.0.0.1:1",
            "fixture-token",
            Duration::from_secs(1),
            "same-container-pid".into(),
            BTreeSet::from(["fixture.game".into()]),
        )
        .expect("offline construction must succeed");
        let second = BrainApiAnswerer::new(
            "http://127.0.0.1:1",
            "fixture-token",
            Duration::from_secs(1),
            "same-container-pid".into(),
            BTreeSet::from(["fixture.game".into()]),
        )
        .expect("offline construction must succeed");

        let first_query = first.query("Abrams").expect("query must be valid");
        let second_query = second.query("Abrams").expect("query must be valid");
        assert_ne!(first_query.request_id, second_query.request_id);
        assert_ne!(first_query.conversation_id, second_query.conversation_id);
    }

    #[test]
    fn sequenznummern_bleiben_parallel_eindeutig_und_ueberlaufen_nicht() {
        let backend = adapter("http://127.0.0.1:1");
        let ids = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..16)
                .map(|_| {
                    let backend = &backend;
                    scope.spawn(move || {
                        let query = backend
                            .query("Synthetische Anfrage")
                            .expect("Parallele synthetische Anfrage innerhalb des Zählerbereichs");
                        assert_eq!(query.request_id, query.conversation_id);
                        query.request_id
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().expect("Parallele Anfrage ohne Threadfehler"))
                .collect::<BTreeSet<_>>()
        });
        assert_eq!(ids.len(), 16);

        backend.sequence.store(u64::MAX - 1, Ordering::Relaxed);
        let last = backend
            .query("Letzte synthetische Anfrage")
            .expect("Letzte Anfrage vor dem Zählerüberlauf");
        assert!(last.request_id.ends_with(&format!("-{}", u64::MAX - 1)));
        assert_eq!(last.request_id, last.conversation_id);
        assert!(backend.query("Überlauf muss scheitern").is_err());
        assert!(backend.query("Erneuter Überlauf muss scheitern").is_err());
        assert_eq!(backend.sequence.load(Ordering::Relaxed), u64::MAX);
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
                .expect("Isolierte Testoperation muss gelingen")
                .requested_scopes,
            BTreeSet::from(["fixture.game".into()])
        );
    }
}
