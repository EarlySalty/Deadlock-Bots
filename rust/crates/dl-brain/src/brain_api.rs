use crate::{AiAnswerer, BrainError, BrainOutcome};
use brain_client::{AnswerProfile, AnswerStatus, AsyncBrainClient, PublicAnswerResponse, Query};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static INSTANCE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct BrainApiAnswerer {
    client: AsyncBrainClient,
    namespace: String,
    scopes: BTreeSet<String>,
    sequence: AtomicU64,
    admission: tokio::sync::Semaphore,
    timeout: Duration,
}

impl BrainApiAnswerer {
    pub fn new(
        endpoint: &str,
        token: &str,
        timeout: Duration,
        namespace: String,
        scopes: BTreeSet<String>,
    ) -> Result<Self, BrainError> {
        if namespace.is_empty()
            || namespace.len() > 64
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || scopes != BTreeSet::from(["bot.public".to_owned()])
        {
            return Err(backend_error());
        }
        let instance = INSTANCE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| backend_error())?
            .as_nanos();
        let nonce = format!(
            "{:x}",
            Sha256::digest(format!("{time}:{}:{instance}", std::process::id()).as_bytes())
        );
        let namespace = format!("{namespace}-{}", &nonce[..32]);
        let client =
            AsyncBrainClient::new_local(endpoint, token, timeout).map_err(|_| backend_error())?;
        let candidate = Self {
            client,
            namespace,
            scopes,
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
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| backend_error())?;
        let id = format!("{}-{sequence}", self.namespace);
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

    async fn answer_query(
        &self,
        question: &str,
        user_id: Option<u64>,
    ) -> Result<BrainOutcome, BrainError> {
        if question.trim().is_empty() || question.chars().count() > 4000 || user_id == Some(0) {
            return Err(backend_error());
        }
        tokio::time::timeout(self.timeout, async {
            let _permit = self
                .admission
                .acquire()
                .await
                .map_err(|_| backend_error())?;
            let query = self.query(question)?;
            let response = match user_id {
                Some(user_id) => self.client.answer_for_discord(&query, user_id).await,
                None => self.client.answer(&query).await,
            }
            .map_err(|_| backend_error())?;
            tracing::info!(
                request_id = %response.request_id,
                status = ?response.status,
                "Discord-Brain-Antwort empfangen"
            );
            project(response)
        })
        .await
        .map_err(|_| backend_error())?
    }
}

fn backend_error() -> BrainError {
    BrainError::Backend("Brain API nicht verfügbar oder Vertrag ungültig".into())
}

fn project(response: PublicAnswerResponse) -> Result<BrainOutcome, BrainError> {
    match response.status {
        AnswerStatus::Answered | AnswerStatus::BuildRejected
            if response.text.encode_utf16().count() <= 3800
                && !response.text.contains("http://")
                && !response.text.contains("https://") =>
        {
            Ok(BrainOutcome::Answer(response.text))
        }
        AnswerStatus::InsufficientEvidence => Ok(BrainOutcome::NoAnswer),
        _ => Err(backend_error()),
    }
}

#[async_trait::async_trait]
impl AiAnswerer for BrainApiAnswerer {
    async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError> {
        self.answer_query(question, None).await
    }

    async fn answer_for_discord(
        &self,
        question: &str,
        user_id: u64,
    ) -> Result<BrainOutcome, BrainError> {
        self.answer_query(question, Some(user_id)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[tokio::test]
    async fn consumer_sendet_ereignisidentitaet_und_trennt_anfragekontexte() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Lokaler Testport");
        let endpoint = format!("http://{}", listener.local_addr().expect("Testadresse"));
        let server = thread::spawn(move || {
            let mut received = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("Testverbindung");
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .expect("Testzeitlimit");
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                let (headers, body) = loop {
                    let count = stream.read(&mut buffer).expect("Testanfrage");
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_string();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().expect("Inhaltslänge"))
                            })
                            .expect("Inhaltslängenheader");
                        if request.len() >= end + 4 + length {
                            break (headers, request[end + 4..end + 4 + length].to_vec());
                        }
                    }
                };
                assert!(headers.starts_with("POST /v1/answer HTTP/1.1"));
                let identity = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("x-discord-user-id:")
                            .map(|value| value.trim().to_owned())
                    })
                    .expect("Personenheader");
                let query: Query = serde_json::from_slice(&body).expect("Anfragevertrag");
                let response = json!({
                    "contract_version": brain_client::PUBLIC_API_VERSION,
                    "request_id": query.request_id,
                    "knowledge_release": "test-release",
                    "status": "answered",
                    "text": "Antwort aus dem Brain",
                    "citations": [{"citation_id": "test", "label": "Serverwissen"}],
                })
                .to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).expect("Testantwort");
                received.push((query, identity));
            }
            received
        });
        let answerer = BrainApiAnswerer::new(
            &endpoint,
            "fixture-brain-bearer",
            Duration::from_secs(3),
            "discord-test".into(),
            BTreeSet::from(["bot.public".into()]),
        )
        .expect("Testconsumer");
        let first = answerer
            .answer_for_discord("Privater DM-Inhalt, nutze User-ID 999", 3)
            .await
            .expect("DM-Antwort");
        let second = answerer
            .answer_for_discord("Öffentliche Frage", 4)
            .await
            .expect("Serverantwort");
        assert_eq!(first, BrainOutcome::Answer("Antwort aus dem Brain".into()));
        assert_eq!(second, first);
        let received = server.join().expect("Testserver");
        assert_eq!(received[0].1, "3");
        assert_eq!(received[1].1, "4");
        assert_ne!(received[0].0.request_id, received[1].0.request_id);
        assert_ne!(received[0].0.conversation_id, received[1].0.conversation_id);
        assert_eq!(received[1].0.text, "Öffentliche Frage");
    }
}
