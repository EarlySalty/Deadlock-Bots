use std::collections::HashSet;
use std::path::{Component, Path};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const LOG_QUESTION_MAX_CHARS: usize = 240;
const ABSENT: &str = "absent";

#[cfg(test)]
pub(crate) mod test_logging {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub(crate) struct LogCapture {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl LogCapture {
        pub(crate) fn text(&self) -> String {
            String::from_utf8_lossy(&self.bytes.lock().expect("log capture")).to_string()
        }
    }

    pub(crate) struct LogCaptureWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for LogCaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("log capture")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            LogCaptureWriter {
                bytes: self.bytes.clone(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KnowledgeAnswer {
    pub answerable: bool,
    pub answer: Option<String>,
    #[serde(default)]
    pub sources: Vec<KnowledgeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KnowledgeSource {
    pub title: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
struct KnowledgeQuestion<'a> {
    question: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KnowledgeLookup {
    Answer(KnowledgeAnswer),
    Unanswerable,
    Timeout,
    Transport,
    InvalidResponse,
}

pub(crate) fn safe_log_question(question: &str) -> String {
    question
        .chars()
        .take(LOG_QUESTION_MAX_CHARS)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn safe_source_paths(sources: &[KnowledgeSource]) -> String {
    let mut seen = HashSet::new();
    let paths = sources
        .iter()
        .filter_map(|source| {
            let path = source.path.trim();
            let relative = !path.is_empty()
                && !path.starts_with('/')
                && !path.starts_with('\\')
                && !path.contains(':')
                && !path.chars().any(char::is_control)
                && Path::new(path)
                    .components()
                    .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
            (relative && seen.insert(path)).then_some(path)
        })
        .collect::<Vec<_>>()
        .join(",");
    if paths.is_empty() {
        ABSENT.to_string()
    } else {
        paths
    }
}

fn logged_lookup(
    question: &str,
    lookup: KnowledgeLookup,
    reason: &'static str,
    sources: &str,
    error_class: Option<&'static str>,
) -> KnowledgeLookup {
    let (verdict, confidence) = match &lookup {
        KnowledgeLookup::Answer(_) => ("yes", "source_grounded"),
        KnowledgeLookup::Unanswerable => ("no", "none"),
        KnowledgeLookup::Timeout => ("timeout", "none"),
        KnowledgeLookup::Transport | KnowledgeLookup::InvalidResponse => ("error", "none"),
    };
    let question = safe_log_question(question);
    tracing::info!(
        question = %question,
        verdict = %verdict,
        confidence = %confidence,
        retrieval_score = %ABSENT,
        reason = %reason,
        sources = %sources,
        error_class = %error_class.unwrap_or(ABSENT),
        "dl-community knowledge-client decision"
    );
    lookup
}

pub(crate) async fn ask(base_url: &str, question: &str, timeout: Duration) -> KnowledgeLookup {
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(client) => client,
        Err(_) => {
            return logged_lookup(
                question,
                KnowledgeLookup::Transport,
                "client_build",
                ABSENT,
                Some("client_build"),
            )
        }
    };
    let url = format!("{}/public/v1/ask", base_url.trim_end_matches('/'));
    let response = match client
        .post(url)
        .json(&KnowledgeQuestion { question })
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            return logged_lookup(
                question,
                KnowledgeLookup::Timeout,
                "request_timeout",
                ABSENT,
                Some("timeout"),
            )
        }
        Err(_) => {
            return logged_lookup(
                question,
                KnowledgeLookup::Transport,
                "request_transport",
                ABSENT,
                Some("transport"),
            )
        }
    };
    if !response.status().is_success() {
        return logged_lookup(
            question,
            KnowledgeLookup::Transport,
            "http_status",
            ABSENT,
            Some("http_status"),
        );
    }
    match response.json::<KnowledgeAnswer>().await {
        Ok(answer) if answer.answerable => {
            let sources = safe_source_paths(&answer.sources);
            logged_lookup(
                question,
                KnowledgeLookup::Answer(answer),
                "response_answerable",
                &sources,
                None,
            )
        }
        Ok(answer) => {
            let sources = safe_source_paths(&answer.sources);
            logged_lookup(
                question,
                KnowledgeLookup::Unanswerable,
                "response_unanswerable",
                &sources,
                None,
            )
        }
        Err(error) if error.is_timeout() => logged_lookup(
            question,
            KnowledgeLookup::Timeout,
            "response_timeout",
            ABSENT,
            Some("timeout"),
        ),
        Err(error) if error.is_decode() => logged_lookup(
            question,
            KnowledgeLookup::InvalidResponse,
            "invalid_response",
            ABSENT,
            Some("decode"),
        ),
        Err(_) => logged_lookup(
            question,
            KnowledgeLookup::Transport,
            "response_transport",
            ABSENT,
            Some("transport"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::test_logging::LogCapture;
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn ask_with_logs(
        base_url: &str,
        question: &str,
        timeout: Duration,
    ) -> (KnowledgeLookup, String) {
        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let result = ask(base_url, question, timeout).await;
        drop(guard);
        (result, capture.text())
    }

    fn assert_client_decision(
        logs: &str,
        verdict: &str,
        confidence: &str,
        reason: &str,
        sources: &str,
        error_class: &str,
    ) {
        assert_eq!(
            logs.matches("dl-community knowledge-client decision")
                .count(),
            1,
            "{logs}"
        );
        for field in [
            format!("verdict={verdict}"),
            format!("confidence={confidence}"),
            "retrieval_score=absent".to_string(),
            format!("reason={reason}"),
            format!("sources={sources}"),
            format!("error_class={error_class}"),
        ] {
            assert!(logs.contains(&field), "missing {field}: {logs}");
        }
    }

    async fn knowledge_server(
        status: u16,
        body: &'static str,
        delay: Duration,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await;
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let status_line = match status {
                200 => "200 OK",
                500 => "500 Internal Server Error",
                _ => "400 Bad Request",
            };
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn answerable_response_is_answer() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"Ja.","sources":[{"title":"T","path":"public/p.md"},{"title":"Duplikat","path":"public/p.md"},{"title":"Absolut","path":"/intern/secret.md"},{"title":"URL","path":"https://example.invalid/leak"}]}"#,
            Duration::ZERO,
        )
        .await;
        let question = format!("{}\nNICHT_LOGGEN", "ä".repeat(239));

        let (result, logs) = ask_with_logs(&url, &question, Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(
            result,
            KnowledgeLookup::Answer(KnowledgeAnswer {
                answerable: true,
                answer: Some("Ja.".to_string()),
                sources: vec![
                    KnowledgeSource {
                        title: "T".to_string(),
                        path: "public/p.md".to_string(),
                    },
                    KnowledgeSource {
                        title: "Duplikat".to_string(),
                        path: "public/p.md".to_string(),
                    },
                    KnowledgeSource {
                        title: "Absolut".to_string(),
                        path: "/intern/secret.md".to_string(),
                    },
                    KnowledgeSource {
                        title: "URL".to_string(),
                        path: "https://example.invalid/leak".to_string(),
                    },
                ],
            })
        );
        assert_client_decision(
            &logs,
            "yes",
            "source_grounded",
            "response_answerable",
            "public/p.md",
            "absent",
        );
        assert!(logs.contains(&format!("question={} ", "ä".repeat(239))));
        assert!(!logs.contains("NICHT_LOGGEN"));
        assert!(!logs.contains("Ja."));
        assert_eq!(logs.matches("public/p.md").count(), 1, "{logs}");
        assert!(!logs.contains("/intern/secret.md"));
        assert!(!logs.contains("https://example.invalid/leak"));
        assert!(!logs.contains(&url));
    }

    #[tokio::test]
    async fn unanswerable_response_is_unanswerable() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":false,"answer":null,"sources":[]}"#,
            Duration::ZERO,
        )
        .await;

        let (result, logs) = ask_with_logs(&url, "Frage?", Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::Unanswerable);
        assert_client_decision(
            &logs,
            "no",
            "none",
            "response_unanswerable",
            "absent",
            "absent",
        );
    }

    #[tokio::test]
    async fn delayed_response_is_timeout() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"zu spaet","sources":[]}"#,
            Duration::from_millis(100),
        )
        .await;

        let (result, logs) = ask_with_logs(&url, "Frage?", Duration::from_millis(10)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::Timeout);
        assert_client_decision(
            &logs,
            "timeout",
            "none",
            "request_timeout",
            "absent",
            "timeout",
        );
    }

    #[tokio::test]
    async fn unsuccessful_status_is_transport() {
        let (url, handle) = knowledge_server(500, "{}", Duration::ZERO).await;

        let (result, logs) = ask_with_logs(&url, "Frage?", Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::Transport);
        assert_client_decision(
            &logs,
            "error",
            "none",
            "http_status",
            "absent",
            "http_status",
        );
    }

    #[tokio::test]
    async fn unavailable_service_is_transport_and_logged() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve unavailable address");
        let url = format!("http://{}", listener.local_addr().expect("local addr"));
        drop(listener);

        let (result, logs) = ask_with_logs(&url, "Frage?", Duration::from_secs(1)).await;

        assert_eq!(result, KnowledgeLookup::Transport);
        assert_client_decision(
            &logs,
            "error",
            "none",
            "request_transport",
            "absent",
            "transport",
        );
        assert!(!logs.contains(&url));
    }

    #[tokio::test]
    async fn invalid_json_is_invalid_response() {
        let (url, handle) = knowledge_server(200, "kein json", Duration::ZERO).await;

        let (result, logs) = ask_with_logs(&url, "Frage?", Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::InvalidResponse);
        assert_client_decision(
            &logs,
            "error",
            "none",
            "invalid_response",
            "absent",
            "decode",
        );
    }
}
