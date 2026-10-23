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
        let sequence = next_sequence(&self.sequence).ok_or_else(backend_error)?;
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
        max_units: usize,
    ) -> Result<BrainOutcome, BrainError> {
        if question.trim().is_empty() || question.chars().count() > 4000 || user_id == Some(0) {
            return Err(classified_backend_error("anfrage"));
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
            project_bounded(response, max_units)
        })
        .await
        .map_err(|_| classified_backend_error("transport"))?
    }
}

fn backend_error() -> BrainError {
    BrainError::Backend("Brain API nicht verfügbar oder Vertrag ungültig".into())
}

fn next_sequence(counter: &AtomicU64) -> Option<u64> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1)?;
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(previous) => return Some(previous),
            Err(actual) => current = actual,
        }
    }
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

#[cfg(test)]
fn project(response: PublicAnswerResponse) -> Result<BrainOutcome, BrainError> {
    project_bounded(response, 3800)
}

fn project_bounded(
    response: PublicAnswerResponse,
    max_units: usize,
) -> Result<BrainOutcome, BrainError> {
    match response.status {
        AnswerStatus::InsufficientEvidence | AnswerStatus::BuildRejected => {
            Ok(BrainOutcome::NoAnswer)
        }
        AnswerStatus::Answered => {
            let (text, had_links) = without_links(&response.text);
            if had_links {
                tracing::warn!(klasse = "link", "Links aus Brain-Antwort entfernt");
            }
            let text = text.replace("[[coaching]]", COACHING_CHANNEL_REFERENCE);
            let text = text.trim();
            if text.is_empty() {
                return Ok(BrainOutcome::NoAnswer);
            }
            let first_units = text.chars().next().map_or(0, char::len_utf16);
            let links = public_links(&response, max_units.saturating_sub(first_units));
            let budget = max_units.saturating_sub(links.encode_utf16().count());
            let mut units = 0;
            let end = text.char_indices().find_map(|(index, character)| {
                units += character.len_utf16();
                (units > budget).then_some(index)
            });
            let text = if let Some(end) = end {
                tracing::warn!(klasse = "laenge", "Brain-Antwort gekürzt");
                &text[..end]
            } else {
                text
            };
            if text.is_empty() {
                return Ok(BrainOutcome::NoAnswer);
            }
            Ok(BrainOutcome::Answer(format!("{text}{links}")))
        }
        AnswerStatus::Unverified | AnswerStatus::UnauthorizedEvidence => {
            tracing::warn!(status = ?response.status, "Brain hat keine verwendbare Antwort geliefert");
            Ok(BrainOutcome::NoAnswer)
        }
        AnswerStatus::Unavailable | AnswerStatus::ProviderError | AnswerStatus::BudgetExceeded => {
            Err(classified_backend_error("vertrag"))
        }
    }
}

fn public_links(response: &PublicAnswerResponse, max_units: usize) -> String {
    if response.status != AnswerStatus::Answered {
        return String::new();
    }
    let mut seen = BTreeSet::new();
    let docs = response
        .citations
        .iter()
        .filter(|citation| citation.label.len() <= 256)
        .filter_map(|citation| {
            citation
                .label
                .split_whitespace()
                .find(|url| valid_docs_url(url))
        })
        .filter(|url| seen.insert(*url))
        .map(|url| format!("\n\nQuelle: <{url}>"));
    let artifacts = response
        .artifacts
        .iter()
        .filter(|artifact| artifact.validate().is_ok())
        .take(brain_client::MAX_PUBLIC_ARTIFACTS)
        .map(|artifact| {
            let (title, had_links) = without_links(&artifact.title);
            if had_links {
                tracing::warn!(klasse = "link", "Links aus Brain-Artefakttitel entfernt");
            }
            format!("\n\nGrafik: {}\n<{}>", title.trim(), artifact.page_url)
        });
    let mut links = String::new();
    let mut units = 0;
    for link in docs.chain(artifacts) {
        let added = link.encode_utf16().count();
        if added <= max_units.saturating_sub(units) {
            links.push_str(&link);
            units += added;
        }
    }
    links
}

fn valid_docs_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/EarlySalty/Deadlock-Docs/blob/") else {
        return false;
    };
    let Some((revision, section)) = rest.split_once("/public/") else {
        return false;
    };
    if revision.len() != 40
        || !revision
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return false;
    }
    let Some((path, lines)) = section.split_once("#L") else {
        return false;
    };
    if !path.ends_with(".html")
        || path.split('/').any(|part| {
            part.is_empty()
                || matches!(part, "." | "..")
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return false;
    }
    let Some((start, end)) = lines.split_once("-L") else {
        return false;
    };
    if !start.bytes().all(|byte| byte.is_ascii_digit())
        || !end.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    matches!((start.parse::<u32>(), end.parse::<u32>()), (Ok(start), Ok(end)) if start > 0 && end >= start)
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
        if !task.valid() {
            return Err(classified_backend_error("anfrage"));
        }
        self.answer_query(question, Some(user_id), false, None, user_questions, 1800)
            .await
    }

    async fn answer(&self, question: &str) -> Result<BrainOutcome, BrainError> {
        self.answer_query(question, None, true, None, &[], 3800)
            .await
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
            3800,
        )
        .await
    }

    async fn answer_for_discord_with_read_access(
        &self,
        question: &str,
        user_id: u64,
        allow_discord_reads: bool,
    ) -> Result<BrainOutcome, BrainError> {
        self.answer_query(
            question,
            Some(user_id),
            allow_discord_reads,
            None,
            &[],
            3800,
        )
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
        let task_request = task.is_some();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("Testverbindung");
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("Testzeitlimit");
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            let request_id = loop {
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
                        let body = &request[end + 4..end + 4 + length];
                        assert!(headers.starts_with("POST /v1/answer HTTP/1.1"));
                        assert!(!headers.contains("dl-knowledge"));
                        assert!(!headers
                            .to_ascii_lowercase()
                            .contains("x-discord-answer-task"));
                        assert!(headers
                            .to_ascii_lowercase()
                            .contains("x-discord-user-id: 3"));
                        if recorded_history.is_empty() {
                            assert_eq!(
                                headers
                                    .to_ascii_lowercase()
                                    .contains("x-discord-read-access: disabled"),
                                task_request
                            );
                            let query: Query =
                                serde_json::from_slice(body).expect("Anfragevertrag");
                            assert_eq!(
                                query.requested_scopes,
                                BTreeSet::from(["bot.public".into()])
                            );
                            assert_eq!(query.text, "Sinclairs letzter Patch?");
                            break query.request_id;
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
                        break query.request_id;
                    }
                }
            };
            let mut response = json!({
                "contract_version": brain_client::PUBLIC_API_VERSION,
                "request_id": request_id,
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
            for (status, expected) in [
                ("answered", BrainOutcome::Answer("Antwort vom Brain".into())),
                ("insufficient_evidence", BrainOutcome::NoAnswer),
            ] {
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
                    expected
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
            for text in ["Ungeprüft: Mir geht es gut, danke!", "Kein Beleg", ""] {
                assert_eq!(
                    fixture_answer_with_task(
                        "insufficient_evidence",
                        text,
                        false,
                        false,
                        Some(capability),
                    )
                    .await
                    .expect("Fehlendes Wissen"),
                    BrainOutcome::NoAnswer
                );
            }
            {
                let status = "answered";
                let text = format!("Ungeprüft: {}🧠Rest", "ä".repeat(1788));
                let outcome =
                    fixture_answer_with_task(status, &text, false, false, Some(capability))
                        .await
                        .expect("Brain-Text");
                assert_eq!(
                    outcome,
                    BrainOutcome::Answer(format!("Ungeprüft: {}", "ä".repeat(1788)))
                );
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
                assert!(matches!(empty, Err(BrainError::Backend(_))));
            }
            for status in [
                "unavailable",
                "provider_error",
                "budget_exceeded",
                "task_failed",
            ] {
                assert!(matches!(
                    fixture_answer_with_task(status, "Antwort", false, false, Some(capability))
                        .await,
                    Err(BrainError::Backend(_))
                ));
            }
            for status in ["unverified", "unauthorized_evidence"] {
                assert!(matches!(
                    fixture_answer_with_task(status, "Antwort", false, false, Some(capability))
                        .await,
                    Ok(BrainOutcome::NoAnswer)
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
        {
            let status = "answered";
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

    #[tokio::test]
    async fn coachingziel_wird_nach_dem_linkfilter_auf_discord_projiziert() {
        {
            let status = "answered";
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
        for status in ["unavailable", "provider_error", "budget_exceeded"] {
            assert!(matches!(
                fixture_answer(status, "[[coaching]]", false, false).await,
                Err(BrainError::Backend(_))
            ));
        }
        for status in ["unverified", "unauthorized_evidence"] {
            assert!(matches!(
                fixture_answer(status, "[[coaching]]", false, false).await,
                Ok(BrainOutcome::NoAnswer)
            ));
        }
    }

    #[tokio::test]
    async fn ungueltige_anfragen_scheitern_ohne_http() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Lokaler Testport");
        listener
            .set_nonblocking(true)
            .expect("Nichtblockierender Testport");
        let endpoint = format!("http://{}", listener.local_addr().expect("Testadresse"));
        let answerer = BrainApiAnswerer::new(
            &endpoint,
            "fixture-consumer",
            Duration::from_secs(3),
            "invalid-query".into(),
            BTreeSet::from(["bot.public".into()]),
        )
        .expect("Testadapter");
        for (question, user_id) in [("".into(), 3), ("ä".repeat(4001), 3), ("Frage".into(), 0)] {
            assert!(matches!(
                answerer.answer_for_discord(&question, user_id).await,
                Err(BrainError::Backend(_))
            ));
        }
        assert!(matches!(
            answerer
                .answer_discord_task(
                    "Frage",
                    3,
                    &crate::DiscordAnswerTask {
                        capability: crate::DiscordAnswerCapability::Faq,
                        channel_id: 0,
                    }
                )
                .await,
            Err(BrainError::Backend(_))
        ));
        assert_eq!(
            listener.accept().expect_err("Keine HTTP-Anfrage").kind(),
            std::io::ErrorKind::WouldBlock
        );
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
        {
            let status = "answered";
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
                artifacts: vec![],
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
            let id = "a".repeat(64);
            let graphic = brain_client::PublicArtifact::hero_compare(
                &id,
                "Haze gegen Infernus: Lebenspunkte nach Boons",
            )
            .expect("Dienstlink");
            let mut forged = graphic.clone();
            forged.page_url = "https://example.invalid/private-token".into();
            let mut linked = response("Haze hat weniger Lebenspunkte.".into());
            linked.artifacts = vec![graphic, forged];
            assert_eq!(
                project(linked.clone()).expect("Antwort mit Grafik"),
                BrainOutcome::Answer(format!(
                    "Haze hat weniger Lebenspunkte.\n\nGrafik: Haze gegen Infernus: Lebenspunkte nach Boons\n<https://deutsche-deadlock-community.de/brain/site/compare/{id}>"
                ))
            );
            linked.status = AnswerStatus::InsufficientEvidence;
            assert_eq!(
                project(linked).expect("Ohne Antwort"),
                BrainOutcome::NoAnswer
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
    async fn nicht_beantwortete_fragen_bleiben_auch_im_aufgabenadapter_still() {
        for task in [
            None,
            Some(crate::DiscordAnswerCapability::Faq),
            Some(crate::DiscordAnswerCapability::Concierge),
        ] {
            for status in ["insufficient_evidence", "build_rejected"] {
                assert_eq!(
                    fixture_answer_with_task(status, "Keine Sachantwort", false, false, task)
                        .await
                        .expect("Stille Projektion"),
                    BrainOutcome::NoAnswer
                );
            }
        }
    }

    #[test]
    fn artifact_titles_do_not_reintroduce_unvalidated_links() {
        let id = "a".repeat(64);
        let docs_url = "https://github.com/EarlySalty/Deadlock-Docs/blob/7b0f0e645f026214f28b99dddbb18bd477bc2819/public/deadlock-helden/billy.html#L35-L38";
        for (title, expected_title) in [
            ("Details http://127.0.0.1:9/private", "Details"),
            (
                "Details [intern](http://127.0.0.1:9/private)",
                "Details intern",
            ),
            ("Details <HTTPS://example.invalid/private>", "Details"),
            (
                "Details ht[https://example.invalid]tp://127.0.0.1:9/private",
                "Details",
            ),
            ("Größe 𐐷 https://example.invalid/private", "Größe 𐐷"),
            ("http://127.0.0.1:9/private", ""),
        ] {
            let artifact = brain_client::PublicArtifact::hero_compare(&id, title)
                .expect("Gültiges öffentliches Artefakt");
            assert!(artifact.validate().is_ok());
            let links = format!(
                "\n\nQuelle: <{docs_url}>\n\nGrafik: {expected_title}\n<{}>",
                artifact.page_url
            );
            let response = PublicAnswerResponse {
                contract_version: brain_client::PUBLIC_API_VERSION.into(),
                request_id: "artifact-title-projection".into(),
                knowledge_release: "test-release".into(),
                status: AnswerStatus::Answered,
                text: "𐐷".repeat(2000),
                citations: vec![
                    brain_client::PublicCitation {
                        citation_id: "docs".into(),
                        label: format!("Deadlock-Docs: {docs_url}"),
                    },
                    brain_client::PublicCitation {
                        citation_id: "duplicate".into(),
                        label: docs_url.into(),
                    },
                ],
                artifacts: vec![artifact],
            };
            for max_units in [1800, 3800] {
                let BrainOutcome::Answer(text) =
                    project_bounded(response.clone(), max_units).expect("Linkfreier Artefakttitel")
                else {
                    panic!("Beantwortete Frage");
                };
                let body_units = max_units - links.encode_utf16().count();
                assert_eq!(text, format!("{}{links}", "𐐷".repeat(body_units / 2)));
                assert!(text.encode_utf16().count() <= max_units);
                assert_eq!(text.matches(docs_url).count(), 1);
                assert!(!text.contains("dl-knowledge"));
            }
            let mut unanswered = response;
            for status in [
                AnswerStatus::InsufficientEvidence,
                AnswerStatus::BuildRejected,
            ] {
                unanswered.status = status;
                assert!(public_links(&unanswered, 3800).is_empty());
                assert_eq!(
                    project_bounded(unanswered.clone(), 1800).expect("Stille Projektion"),
                    BrainOutcome::NoAnswer
                );
            }
        }
    }

    #[test]
    fn projection_without_body_budget_is_silent() {
        let response = PublicAnswerResponse {
            contract_version: brain_client::PUBLIC_API_VERSION.into(),
            request_id: "bounded-projection".into(),
            knowledge_release: "test-release".into(),
            status: AnswerStatus::Answered,
            text: "🧠".into(),
            citations: vec![],
            artifacts: vec![],
        };
        for max_units in [0, 1] {
            assert_eq!(
                project_bounded(response.clone(), max_units).expect("Leere Projektion"),
                BrainOutcome::NoAnswer
            );
        }
        assert_eq!(
            project_bounded(response, 2).expect("Vollständiges Zeichen"),
            BrainOutcome::Answer("🧠".into())
        );
    }

    #[test]
    fn nur_immutable_oeffentliche_docs_quellen_erreichen_discord() {
        let url = "https://github.com/EarlySalty/Deadlock-Docs/blob/7b0f0e645f026214f28b99dddbb18bd477bc2819/public/deadlock-helden/billy.html#L35-L38";
        assert!(valid_docs_url(url));
        for invalid in [
            url.replace("https://", "http://"),
            url.replace("github.com/", "github.com.evil.invalid/"),
            url.replace("EarlySalty/", "Andere/"),
            url.replace("/blob/", "/raw/"),
            url.replace("7b0f0e645f026214f28b99dddbb18bd477bc2819", "main"),
            url.replace("7b0f0e", "7B0F0E"),
            url.replace("/public/", "/internal/"),
            url.replace("/public/", "/public/../internal/"),
            url.replace("/public/", "/public/%2e%2e/"),
            url.replace(".html#", ".md#"),
            url.replace("#L35-L38", "?query=1#L35-L38"),
            url.replace("#L35-L38", "#early-game"),
            url.replace("#L35-L38", "#L0-L38"),
            url.replace("#L35-L38", "#L38-L35"),
            url.replace("github.com", "user@github.com"),
        ] {
            assert!(!valid_docs_url(&invalid), "{invalid}");
        }
        let response = PublicAnswerResponse {
            contract_version: brain_client::PUBLIC_API_VERSION.into(),
            request_id: "docs-projection".into(),
            knowledge_release: "test-release".into(),
            status: AnswerStatus::Answered,
            text: "ä".repeat(2000),
            citations: vec![
                brain_client::PublicCitation {
                    citation_id: "docs".into(),
                    label: format!("Deadlock-Docs: {url}"),
                },
                brain_client::PublicCitation {
                    citation_id: "duplicate".into(),
                    label: url.into(),
                },
                brain_client::PublicCitation {
                    citation_id: "internal".into(),
                    label: url.replace("/public/", "/internal/"),
                },
                brain_client::PublicCitation {
                    citation_id: "oversized".into(),
                    label: format!("{} {url}", "a".repeat(256)),
                },
            ],
            artifacts: vec![],
        };
        let BrainOutcome::Answer(text) =
            project_bounded(response.clone(), 1800).expect("Öffentliche Quelle")
        else {
            panic!("Beantwortete Frage");
        };
        assert_eq!(text.matches(url).count(), 1);
        assert!(!text.contains("/internal/"));
        assert_eq!(text.encode_utf16().count(), 1800);
        let mut unicode = response;
        unicode.text = "🧠".into();
        let budget = format!("\n\nQuelle: <{url}>").encode_utf16().count() + 1;
        assert_eq!(
            project_bounded(unicode, budget).expect("UTF-16-Textreserve"),
            BrainOutcome::Answer("🧠".into())
        );
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
