use crate::{AiAnswerer, BrainError, BrainOutcome};
use brain_client::{
    AnswerProfile, AnswerStatus, AsyncBrainClient, ClientError, PublicAnswerResponse, Query,
};
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
            .map_err(client_error)?;
            project(response)
        })
        .await
        .map_err(|_| classified_backend_error("transport"))?
    }
}

fn backend_error() -> BrainError {
    BrainError::Backend("Brain API nicht verfügbar oder Vertrag ungültig".into())
}

fn classified_backend_error(class: &'static str) -> BrainError {
    tracing::warn!(klasse = class, "Brain-Anfrage fehlgeschlagen");
    backend_error()
}

fn client_error(error: ClientError) -> BrainError {
    let class = match error {
        ClientError::Http(_) | ClientError::BodyRead(_) | ClientError::HttpStatus { .. } => {
            "transport"
        }
        _ => "vertrag",
    };
    classified_backend_error(class)
}

fn project(response: PublicAnswerResponse) -> Result<BrainOutcome, BrainError> {
    match response.status {
        AnswerStatus::Answered | AnswerStatus::BuildRejected => {
            let (text, had_links) = without_links(&response.text);
            if had_links {
                tracing::warn!(klasse = "link", "Links aus Brain-Antwort entfernt");
            }
            let text = text.trim();
            if text.is_empty() {
                return Ok(BrainOutcome::NoAnswer);
            }
            let mut units = 0;
            let end = text.char_indices().find_map(|(index, character)| {
                units += character.len_utf16();
                (units > 3800).then_some(index)
            });
            let text = if let Some(end) = end {
                tracing::warn!(klasse = "laenge", "Brain-Antwort gekürzt");
                &text[..end]
            } else {
                text
            };
            Ok(BrainOutcome::Answer(text.to_owned()))
        }
        AnswerStatus::InsufficientEvidence => Ok(BrainOutcome::NoAnswer),
        _ => Err(classified_backend_error("vertrag")),
    }
}

fn without_links(text: &str) -> (String, bool) {
    let lower = text.to_ascii_lowercase();
    let mut rest = text;
    let mut offset = 0;
    let mut result = String::with_capacity(text.len());
    let mut had_links = false;
    while let Some(start) = lower[offset..]
        .find("http://")
        .into_iter()
        .chain(lower[offset..].find("https://"))
        .min()
    {
        had_links = true;
        let prefix = &rest[..start];
        let markdown = prefix.ends_with("](") && prefix.rfind('[').is_some();
        if markdown {
            let label_start = prefix.rfind('[').expect("Linkbeschriftung");
            result.push_str(&prefix[..label_start]);
            result.push_str(&prefix[label_start + 1..prefix.len() - 2]);
        } else {
            result.push_str(prefix.strip_suffix(['<', '[', '(']).unwrap_or(prefix));
        }
        let link = &rest[start..];
        let mut parentheses = 0usize;
        let end = link
            .char_indices()
            .find_map(|(index, character)| match character {
                '(' => {
                    parentheses += 1;
                    None
                }
                ')' if parentheses > 0 => {
                    parentheses -= 1;
                    None
                }
                ')' | '<' | '>' | '[' | ']' | '"' | '\'' | '`' => Some(index),
                _ if character.is_whitespace() => Some(index),
                _ => None,
            })
            .unwrap_or(link.len());
        let url_end = link[..end]
            .trim_end_matches(['.', ',', ';', ':', '!', '?'])
            .len();
        let mut consumed = end;
        if (markdown && link[end..].starts_with(')'))
            || (prefix.ends_with('<') && link[end..].starts_with('>'))
            || (prefix.ends_with('[') && link[end..].starts_with(']'))
            || (prefix.ends_with('(') && link[end..].starts_with(')'))
        {
            consumed += 1;
        }
        result.push_str(&link[url_end..end]);
        offset += start + consumed;
        rest = &text[offset..];
    }
    result.push_str(rest);
    (result, had_links)
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
        sync::{Arc, Mutex},
        thread,
    };

    async fn fixture_answer(
        status: &str,
        text: &str,
        truncated_body: bool,
        invalid_contract: bool,
    ) -> Result<BrainOutcome, BrainError> {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Lokaler Testport");
        let endpoint = format!("http://{}", listener.local_addr().expect("Testadresse"));
        let status = status.to_owned();
        let text = text.to_owned();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("Testverbindung");
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("Testzeitlimit");
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            let query: Query = loop {
                let count = stream.read(&mut buffer).expect("Testanfrage");
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().expect("Inhaltslänge"))
                        })
                        .expect("Inhaltslängenheader");
                    if request.len() >= end + 4 + length {
                        break serde_json::from_slice(&request[end + 4..end + 4 + length])
                            .expect("Anfragevertrag");
                    }
                }
            };
            let mut response = json!({
                "contract_version": brain_client::PUBLIC_API_VERSION,
                "request_id": query.request_id,
                "knowledge_release": "test-release",
                "status": status,
                "text": text,
                "citations": [{"citation_id": "test", "label": "Serverwissen"}],
            });
            if invalid_contract {
                response["contract_version"] = json!("falscher-vertrag");
            }
            let response = response.to_string();
            let body = if truncated_body {
                &response[..response.len() / 2]
            } else {
                &response
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", response.len()).expect("Testantwort");
        });
        let answerer = BrainApiAnswerer::new(
            &endpoint,
            "fixture-brain-bearer",
            Duration::from_secs(3),
            "discord-fixture".into(),
            BTreeSet::from(["bot.public".into()]),
        )
        .expect("Testconsumer");
        let result = answerer
            .answer_for_discord("Sinclairs letzter Patch?", 3)
            .await;
        server.join().expect("Testserver");
        result
    }

    #[tokio::test]
    async fn fixture_antworten_bleiben_trotz_links_und_ueberlaenge_erhalten() {
        for status in ["answered", "build_rejected"] {
            let answer = fixture_answer(
                status,
                "Sinclair wurde angepasst. https://example.invalid/patch",
                false,
                false,
            )
            .await
            .expect("Antwort trotz Link");
            assert_eq!(
                answer,
                BrainOutcome::Answer("Sinclair wurde angepasst.".into())
            );

            let text = format!("{}🧠Rest", "ä".repeat(3799));
            let answer = fixture_answer(status, &text, false, false)
                .await
                .expect("Gekürzte Antwort");
            assert_eq!(answer, BrainOutcome::Answer("ä".repeat(3799)));

            assert_eq!(
                fixture_answer(status, "https://example.invalid/patch", false, false)
                    .await
                    .expect("Leere Antwort nach Linkentfernung"),
                BrainOutcome::NoAnswer
            );
        }
        assert_eq!(
            fixture_answer("insufficient_evidence", "Kein Beleg", false, false)
                .await
                .expect("Fehlendes Wissen"),
            BrainOutcome::NoAnswer
        );
        assert!(matches!(
            fixture_answer("answered", "Antwort", true, false).await,
            Err(BrainError::Backend(_))
        ));
        assert!(matches!(
            fixture_answer("answered", "Antwort", false, true).await,
            Err(BrainError::Backend(_))
        ));
    }

    #[test]
    fn links_werden_auch_aus_markdown_und_autolinks_entfernt() {
        for (text, expected) in [
            (
                "Siehe [Patch](https://example.invalid/a_(b)).",
                "Siehe Patch.",
            ),
            ("Siehe <http://example.invalid>. Ende", "Siehe . Ende"),
            ("HTTPS://example.invalid", ""),
            ("[https://example.invalid](https://example.invalid)", ""),
            (
                "Vorher https://example.invalid danach http://example.invalid",
                "Vorher  danach ",
            ),
            ("Antwort ohne Link", "Antwort ohne Link"),
        ] {
            let (clean, had_links) = without_links(text);
            assert_eq!(clean, expected);
            assert_eq!(had_links, text != expected);
        }
    }

    #[test]
    fn warnungen_nennen_nur_die_klasse_und_kuerzung_beachtet_utf16() {
        #[derive(Clone)]
        struct LogBuffer(Arc<Mutex<Vec<u8>>>);
        impl Write for LogBuffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .expect("Testprotokoll")
                    .extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = LogBuffer(Arc::new(Mutex::new(Vec::new())));
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let response = |text: String| PublicAnswerResponse {
                contract_version: brain_client::PUBLIC_API_VERSION.into(),
                request_id: "private-request-id".into(),
                knowledge_release: "test-release".into(),
                status: AnswerStatus::Answered,
                text,
                citations: vec![],
            };
            assert_eq!(
                project(response("https://example.invalid/private-token".into()))
                    .expect("Entfernter Link"),
                BrainOutcome::NoAnswer
            );
            let full = format!("{}🧠", "ä".repeat(3798));
            assert_eq!(
                project(response(full.clone())).expect("Antwort an der Grenze"),
                BrainOutcome::Answer(full.clone())
            );
            assert_eq!(
                project(response(format!("{full}weiter"))).expect("Gekürzte Antwort"),
                BrainOutcome::Answer(full)
            );
            client_error(ClientError::InvalidResponse);
            client_error(ClientError::HttpStatus {
                status: reqwest::StatusCode::BAD_GATEWAY,
                body: "privater-fragetext private-token private-request-id".into(),
            });
        });
        let log = String::from_utf8(buffer.0.lock().expect("Testprotokoll").clone())
            .expect("UTF-8-Protokoll");
        for class in ["link", "laenge", "vertrag", "transport"] {
            assert!(log.contains(&format!("klasse=\"{class}\"")), "{class}");
        }
        for private in ["privater-fragetext", "private-token", "private-request-id"] {
            assert!(!log.contains(private));
        }
    }

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
