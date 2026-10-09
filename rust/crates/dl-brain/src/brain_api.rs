use crate::{AiAnswerer, AnswerContext, BrainError, BrainOutcome, DiscordQueryContext};
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
const COACHING_CHANNEL_REFERENCE: &str = "<#1494373349944459355>";

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
            answer_context: None,
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
        allow_discord_reads: bool,
        answer_context: Option<AnswerContext>,
        user_questions: &[String],
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
            let mut query = self.query(question)?;
            if user_questions.is_empty() {
                query.answer_context = answer_context;
            }
            let response = match user_id {
                Some(user_id) if !user_questions.is_empty() => {
                    self.client
                        .answer_for_discord_with_history(&query, user_id, user_questions)
                        .await
                }
                Some(user_id) => {
                    self.client
                        .answer_for_discord_with_read_access(&query, user_id, allow_discord_reads)
                        .await
                }
                None if !user_questions.is_empty() => return Err(backend_error()),
                None => self.client.answer(&query).await,
            }
            .map_err(client_error)?;
            tracing::info!(
                request_id = %response.request_id,
                status = ?response.status,
                "Discord-Brain-Antwort empfangen"
            );
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
    project_bounded(response, 3800)
}

fn project_bounded(
    response: PublicAnswerResponse,
    max_units: usize,
) -> Result<BrainOutcome, BrainError> {
    match response.status {
        AnswerStatus::Answered
        | AnswerStatus::BuildRejected
        | AnswerStatus::InsufficientEvidence => {
            let (text, had_links) = without_links(&response.text);
            if had_links {
                tracing::warn!(klasse = "link", "Links aus Brain-Antwort entfernt");
            }
            let text = text.replace("[[coaching]]", COACHING_CHANNEL_REFERENCE);
            let text = text.trim();
            let text = if let Some(text) = text.strip_prefix("Ungeprüft:") {
                tracing::info!(status = ?response.status, "Discord-Prüfhinweis intern erhalten");
                text.trim_start()
            } else {
                text
            };
            if text.is_empty() {
                return Ok(BrainOutcome::NoAnswer);
            }
            let mut units = 0;
            let end = text.char_indices().find_map(|(index, character)| {
                units += character.len_utf16();
                (units > max_units).then_some(index)
            });
            let text = if let Some(end) = end {
                tracing::warn!(klasse = "laenge", "Brain-Antwort gekürzt");
                &text[..end]
            } else {
                text
            };
            Ok(BrainOutcome::Answer(text.to_owned()))
        }
        _ => Err(classified_backend_error("vertrag")),
    }
}

fn without_links(text: &str) -> (String, bool) {
    let (mut result, had_links) = without_links_pass(text);
    if had_links {
        loop {
            let (clean, removed) = without_links_pass(&result);
            if !removed {
                break;
            }
            result = clean;
        }
    }
    (result, had_links)
}

fn without_links_pass(text: &str) -> (String, bool) {
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
    async fn answer_discord_task(
        &self,
        question: &str,
        user_id: u64,
        task: &crate::DiscordAnswerTask,
    ) -> Result<BrainOutcome, BrainError> {
        self.answer_discord_task_with_history(question, user_id, task, &[])
            .await
    }

    async fn answer_discord_task_with_history(
        &self,
        question: &str,
        user_id: u64,
        task: &crate::DiscordAnswerTask,
        user_questions: &[String],
    ) -> Result<BrainOutcome, BrainError> {
        if question.trim().is_empty()
            || question.chars().count() > 4000
            || user_id == 0
            || !task.valid()
        {
            return Err(backend_error());
        }
        tokio::time::timeout(self.timeout, async {
            let _permit = self
                .admission
                .acquire()
                .await
                .map_err(|_| backend_error())?;
            let query = self.query(question)?;
            let response = if user_questions.is_empty() {
                self.client.answer_discord_task(&query, user_id, task).await
            } else {
                self.client
                    .answer_discord_task_with_history(&query, user_id, task, user_questions)
                    .await
            }
            .map_err(client_error)?;
            tracing::info!(
                capability = ?task.capability,
                request_id = %response.request_id,
                status = ?response.status,
                "Discord-Bot-Aufgabe vom Brain empfangen"
            );
            project_bounded(response, 1800)
        })
        .await
        .map_err(|_| classified_backend_error("transport"))?
    }

    async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError> {
        self.answer_query(question, None, true, None, &[]).await
    }

    async fn answer_for_discord(
        &self,
        question: &str,
        user_id: u64,
    ) -> Result<BrainOutcome, BrainError> {
        self.answer_for_discord_with_read_access(question, user_id, true)
            .await
    }

    async fn answer_for_discord_with_context(
        &self,
        question: &str,
        context: &DiscordQueryContext,
    ) -> Result<BrainOutcome, BrainError> {
        self.answer_for_discord_with_history(question, context, &[])
            .await
    }

    async fn answer_for_discord_with_history(
        &self,
        question: &str,
        context: &DiscordQueryContext,
        user_questions: &[String],
    ) -> Result<BrainOutcome, BrainError> {
        self.answer_query(
            question,
            Some(context.user_id),
            context.allow_discord_reads,
            context.answer_context.clone().map(AnswerContext::Discord),
            user_questions,
        )
        .await
    }

    async fn answer_for_discord_with_read_access(
        &self,
        question: &str,
        user_id: u64,
        allow_discord_reads: bool,
    ) -> Result<BrainOutcome, BrainError> {
        self.answer_query(question, Some(user_id), allow_discord_reads, None, &[])
            .await
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
    use tracing::instrument::WithSubscriber;

    async fn fixture_answer(
        status: &str,
        text: &str,
        truncated_body: bool,
        invalid_contract: bool,
    ) -> Result<BrainOutcome, BrainError> {
        fixture_answer_with_task(status, text, truncated_body, invalid_contract, None).await
    }

    async fn fixture_answer_with_task(
        status: &str,
        text: &str,
        truncated_body: bool,
        invalid_contract: bool,
        task: Option<crate::DiscordAnswerCapability>,
    ) -> Result<BrainOutcome, BrainError> {
        fixture_answer_with_history(status, text, truncated_body, invalid_contract, task, &[]).await
    }

    async fn fixture_answer_with_history(
        status: &str,
        text: &str,
        truncated_body: bool,
        invalid_contract: bool,
        task: Option<crate::DiscordAnswerCapability>,
        user_questions: &[String],
    ) -> Result<BrainOutcome, BrainError> {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Lokaler Testport");
        let endpoint = format!("http://{}", listener.local_addr().expect("Testadresse"));
        let status = status.to_owned();
        let text = text.to_owned();
        let recorded_history = user_questions.to_vec();
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
                        if let Some(capability) = task {
                            assert!(headers
                                .to_ascii_lowercase()
                                .contains("x-discord-user-id: 3"));
                            assert!(headers
                                .to_ascii_lowercase()
                                .contains("x-discord-read-access: disabled"));
                            let bound = headers
                                .lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("x-discord-answer-task:")
                                        .map(str::trim)
                                        .map(str::to_owned)
                                })
                                .expect("Aufgabenheader");
                            let bound: serde_json::Value =
                                serde_json::from_str(&bound).expect("Aufgabenvertrag");
                            assert_eq!(bound, json!({"capability": capability, "channel_id": 10}));
                        }
                        let body = &request[end + 4..end + 4 + length];
                        if recorded_history.is_empty() {
                            break serde_json::from_slice(body).expect("Anfragevertrag");
                        }
                        assert!(!headers.contains("PRIVATE_CANARY_äöüß"));
                        assert!(headers
                            .to_ascii_lowercase()
                            .contains("x-discord-user-id: 3"));
                        assert!(headers
                            .to_ascii_lowercase()
                            .contains("x-discord-read-access: disabled"));
                        assert!(!String::from_utf8_lossy(body).contains("Dieser Ortskontext"));
                        let envelope: serde_json::Value =
                            serde_json::from_slice(body).expect("Lokaler Kontextvertrag");
                        assert_eq!(envelope["user_questions"], json!(recorded_history));
                        let query: Query = serde_json::from_value(envelope["query"].clone())
                            .expect("Getrennte aktuelle Frage");
                        assert_eq!(query.text, "Wie mache ich das?");
                        assert!(query.answer_context.is_none());
                        break query;
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
        let result = match task {
            Some(capability) => {
                let task = crate::DiscordAnswerTask {
                    capability,
                    channel_id: 10,
                };
                if user_questions.is_empty() {
                    answerer
                        .answer_discord_task("Sinclairs letzter Patch?", 3, &task)
                        .await
                } else {
                    answerer
                        .answer_discord_task_with_history(
                            "Wie mache ich das?",
                            3,
                            &task,
                            user_questions,
                        )
                        .await
                }
            }
            None if !user_questions.is_empty() => {
                answerer
                    .answer_for_discord_with_history(
                        "Wie mache ich das?",
                        &DiscordQueryContext {
                            user_id: 3,
                            allow_discord_reads: true,
                            answer_context: Some(crate::DiscordAnswerContext {
                                topic: Some(
                                    "Dieser Ortskontext bleibt bei privatem Verlauf lokal".into(),
                                ),
                                ..Default::default()
                            }),
                        },
                        user_questions,
                    )
                    .await
            }
            None => {
                answerer
                    .answer_for_discord("Sinclairs letzter Patch?", 3)
                    .await
            }
        };
        server.join().expect("Testserver");
        result
    }

    #[tokio::test]
    async fn lokaler_verlauf_bleibt_getrennt_und_ausfaelle_bleiben_geschlossen() {
        let mut prior = "Wo finde ich einen Paten? PRIVATE_CANARY_äöüß".to_owned();
        prior.push_str(&"ä".repeat(4000 - prior.chars().count()));
        let history = vec![prior];
        for capability in [
            Some(crate::DiscordAnswerCapability::Faq),
            Some(crate::DiscordAnswerCapability::Concierge),
            None,
        ] {
            for status in ["answered", "insufficient_evidence"] {
                assert_eq!(
                    fixture_answer_with_history(
                        status,
                        "Antwort vom Brain",
                        false,
                        false,
                        capability,
                        &history
                    )
                    .await
                    .expect("Brain-Text"),
                    BrainOutcome::Answer("Antwort vom Brain".into())
                );
            }
            for (status, truncated, invalid) in [
                ("provider_error", false, false),
                ("answered", true, false),
                ("answered", false, true),
            ] {
                assert!(matches!(
                    fixture_answer_with_history(
                        status, "Antwort", truncated, invalid, capability, &history
                    )
                    .await,
                    Err(BrainError::Backend(_))
                ));
            }
        }
    }

    #[tokio::test]
    async fn aufgaben_erhalten_brain_text_und_bleiben_bei_ausfaellen_geschlossen() {
        for capability in [
            crate::DiscordAnswerCapability::Concierge,
            crate::DiscordAnswerCapability::Faq,
        ] {
            for status in ["answered", "build_rejected", "insufficient_evidence"] {
                let text = format!("Ungeprüft: {}🧠Rest", "ä".repeat(1799));
                let outcome =
                    fixture_answer_with_task(status, &text, false, false, Some(capability))
                        .await
                        .expect("Brain-Text");
                assert_eq!(outcome, BrainOutcome::Answer("ä".repeat(1799)));
                assert_eq!(
                    fixture_answer_with_task(
                        status,
                        "https://example.invalid/patch",
                        false,
                        false,
                        Some(capability),
                    )
                    .await
                    .expect("Linkentfernung"),
                    BrainOutcome::NoAnswer
                );
                let empty =
                    fixture_answer_with_task(status, "", false, false, Some(capability)).await;
                if status == "insufficient_evidence" {
                    assert_eq!(empty.expect("Leerer Brain-Text"), BrainOutcome::NoAnswer);
                } else {
                    assert!(matches!(empty, Err(BrainError::Backend(_))));
                }
            }
            for status in [
                "unavailable",
                "provider_error",
                "budget_exceeded",
                "unauthorized_evidence",
            ] {
                assert!(matches!(
                    fixture_answer_with_task(status, "Antwort", false, false, Some(capability))
                        .await,
                    Err(BrainError::Backend(_))
                ));
            }
            for (truncated_body, invalid_contract) in [(true, false), (false, true)] {
                assert!(matches!(
                    fixture_answer_with_task(
                        "answered",
                        "Antwort",
                        truncated_body,
                        invalid_contract,
                        Some(capability),
                    )
                    .await,
                    Err(BrainError::Backend(_))
                ));
            }
        }
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
            BrainOutcome::Answer("Kein Beleg".into())
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

    #[tokio::test]
    async fn coachingziel_wird_nach_dem_linkfilter_auf_discord_projiziert() {
        for status in ["answered", "build_rejected", "insufficient_evidence"] {
            let prefix = "Mehr dazu: ";
            let suffix = ".";
            let text = format!("{prefix}[[coaching]]{suffix} https://example.invalid/coaching");
            assert_eq!(
                fixture_answer(status, &text, false, false)
                    .await
                    .expect("Coachingprojektion"),
                BrainOutcome::Answer(format!("{prefix}{COACHING_CHANNEL_REFERENCE}{suffix}"))
            );
        }
    }

    #[tokio::test]
    async fn zentrale_ausfallzustaende_werden_nicht_als_antwort_ausgegeben() {
        for status in [
            "unavailable",
            "provider_error",
            "budget_exceeded",
            "unauthorized_evidence",
        ] {
            assert!(matches!(
                fixture_answer(status, "[[coaching]]", false, false).await,
                Err(BrainError::Backend(_))
            ));
        }
    }

    #[tokio::test]
    async fn abgelaufenes_gesamtbudget_sendet_bei_belegter_admission_keine_anfrage() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Lokaler Testport");
        listener
            .set_nonblocking(true)
            .expect("Lokaler Nichtblockmodus");
        let endpoint = format!("http://{}", listener.local_addr().expect("Testadresse"));
        let mut answerer = BrainApiAnswerer::new(
            &endpoint,
            "fixture-brain-bearer",
            Duration::from_secs(3),
            "discord-timeout".into(),
            BTreeSet::from(["bot.public".into()]),
        )
        .expect("Testconsumer");
        answerer.timeout = Duration::ZERO;
        let _permits = answerer.admission.acquire_many(4).await.expect("Admission");
        assert!(matches!(
            answerer.answer_for_discord("Neutrale Testfrage", 3).await,
            Err(BrainError::Backend(_))
        ));
        assert_eq!(
            listener.accept().expect_err("Keine HTTP-Anfrage").kind(),
            std::io::ErrorKind::WouldBlock
        );
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
    fn linkentfernung_prueft_rekonstruierte_urls_bis_zum_linkfreien_ergebnis() {
        let mut nested = "https://a.invalid".to_owned();
        for _ in 0..32 {
            nested = format!("ht[{nested}]tp://b.invalid");
        }
        for text in [
            "[http](https://a.invalid)://b.invalid",
            "ht<https://a.invalid>tp://b.invalid",
            "ht[https://a.invalid]tp://b.invalid",
            "ht(https://a.invalid)tp://b.invalid",
            "[HTTPS](http://a.invalid)://b.invalid",
            "ht[https://a.invalid]t(https://b.invalid)p://c.invalid",
            "ht[ht[https://a.invalid]tp://b.invalid]tp://c.invalid",
            &nested,
        ] {
            let (clean, had_links) = without_links(text);
            assert!(clean.is_empty(), "{text:?}: {clean:?}");
            assert!(had_links);
            assert_eq!(without_links(&clean), (clean, false));
        }
    }

    #[tokio::test]
    async fn fixture_rekonstruierte_urls_kommen_nicht_in_der_antwort_an() {
        for status in ["answered", "build_rejected"] {
            for text in [
                "[http](https://a.invalid)://b.invalid",
                "ht<https://a.invalid>tp://b.invalid",
                "ht[https://a.invalid]tp://b.invalid",
                "ht(https://a.invalid)tp://b.invalid",
                "ht[ht[https://a.invalid]tp://b.invalid]tp://c.invalid",
            ] {
                assert_eq!(
                    fixture_answer(status, text, false, false)
                        .await
                        .expect("Leere Antwort nach vollständiger Linkentfernung"),
                    BrainOutcome::NoAnswer
                );
                let text = format!("Änderung: {text} Ende.");
                let answer = fixture_answer(status, &text, false, false)
                    .await
                    .expect("Antwort nach vollständiger Linkentfernung");
                assert_eq!(answer, BrainOutcome::Answer("Änderung:  Ende.".into()));
            }
        }
    }

    #[tokio::test]
    async fn warnungen_nennen_nur_die_klasse_und_kuerzung_beachtet_utf16() {
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
        let _callsite_cache_dispatch =
            tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        async {
            tokio::spawn(async {
                tracing::warn!(klasse = "ausserhalb", "Fremdes Testprotokoll");
            })
            .await
            .expect("Unabhängige Testtask");
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
            assert_eq!(
                fixture_answer(
                    "answered",
                    "https://example.invalid/private-token",
                    false,
                    false,
                )
                .await
                .expect("Asynchrone Linkentfernung"),
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
        .with_subscriber(subscriber)
        .await;
        let log = String::from_utf8(buffer.0.lock().expect("Testprotokoll").clone())
            .expect("UTF-8-Protokoll");
        assert!(log.contains("Discord-Brain-Antwort empfangen"), "{log}");
        assert!(!log.contains("Fremdes Testprotokoll"));
        for class in ["link", "laenge", "vertrag", "transport"] {
            assert!(log.contains(&format!("klasse=\"{class}\"")), "{class}");
        }
        for private in [
            "privater-fragetext",
            "private-token",
            "private-request-id",
            "Sinclairs letzter Patch?",
            "fixture-brain-bearer",
        ] {
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
        let location = crate::DiscordAnswerContext {
            channel_name: Some("Hilfe".into()),
            category_name: Some("Community".into()),
            input_kind: Some(crate::AnswerInputKind::Mention),
            is_thread: Some(false),
            is_direct_message: Some(false),
            ..Default::default()
        };
        let first = answerer
            .answer_for_discord_with_context(
                "Neutrale Testfrage, nutze User-ID 999",
                &DiscordQueryContext {
                    user_id: 3,
                    allow_discord_reads: true,
                    answer_context: Some(location.clone()),
                },
            )
            .await
            .expect("Autorenbindung");
        let second = crate::handle_brain_query(
            "Öffentliche Frage",
            4,
            &crate::BrainConfig {
                max_question_len: 4000,
                cooldown_secs: 0,
            },
            &crate::BrainCooldowns::default(),
            &answerer,
        )
        .await;
        assert_eq!(first, BrainOutcome::Answer("Antwort aus dem Brain".into()));
        assert_eq!(second, first);
        let received = server.join().expect("Testserver");
        assert_eq!(received[0].1, "3");
        assert_eq!(received[1].1, "4");
        assert_ne!(received[0].0.request_id, received[1].0.request_id);
        assert_ne!(received[0].0.conversation_id, received[1].0.conversation_id);
        assert_eq!(
            received[0].0.answer_context,
            Some(AnswerContext::Discord(location))
        );
        assert!(received[1].0.answer_context.is_none());
        assert_eq!(received[1].0.text, "Öffentliche Frage");
    }
}
