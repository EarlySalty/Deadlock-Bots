use std::collections::HashSet;
use std::path::{Component, Path};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, SemaphorePermit};

const ABSENT: &str = "absent";
const STATEFUL_SUPPORT_TURN_LIMIT: usize = 4;

struct StatefulSupportAdmission {
    semaphore: Semaphore,
}

impl StatefulSupportAdmission {
    const fn new() -> Self {
        Self {
            semaphore: Semaphore::const_new(STATEFUL_SUPPORT_TURN_LIMIT),
        }
    }

    async fn acquire(&self) -> SemaphorePermit<'_> {
        self.semaphore
            .acquire()
            .await
            .expect("stateful support admission is never closed")
    }
}

static STATEFUL_SUPPORT_ADMISSION: StatefulSupportAdmission = StatefulSupportAdmission::new();

pub(crate) async fn acquire_stateful_support_turn() -> SemaphorePermit<'static> {
    STATEFUL_SUPPORT_ADMISSION.acquire().await
}

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

fn safe_source_paths(sources: &[KnowledgeSource]) -> String {
    let mut seen = HashSet::new();
    let paths = sources
        .iter()
        .filter_map(|source| safe_source_path(&source.path).filter(|path| seen.insert(*path)))
        .collect::<Vec<_>>()
        .join(",");
    if paths.is_empty() {
        ABSENT.to_string()
    } else {
        paths
    }
}

fn safe_source_path(path: &str) -> Option<&str> {
    let safe = !path.is_empty()
        && path.trim() == path
        && path.ends_with(".html")
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && Path::new(path)
            .components()
            .all(|component| match component {
                Component::Normal(segment) => {
                    !segment.to_string_lossy().eq_ignore_ascii_case("internal")
                }
                _ => false,
            });
    safe.then_some(path)
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
    let question_chars = question.chars().count();
    tracing::info!(
        question_chars,
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
    let mut url = match reqwest::Url::parse(base_url) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|address| address.is_loopback())
                }) =>
        {
            url
        }
        _ => {
            return logged_lookup(
                question,
                KnowledgeLookup::InvalidResponse,
                "invalid_target",
                ABSENT,
                Some("target"),
            )
        }
    };
    let path = format!("{}/public/v1/ask", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
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
        Ok(answer)
            if answer.answerable
                && answer
                    .answer
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
                && !answer.sources.is_empty()
                && answer
                    .sources
                    .iter()
                    .all(|source| safe_source_path(&source.path).is_some()) =>
        {
            let sources = safe_source_paths(&answer.sources);
            logged_lookup(
                question,
                KnowledgeLookup::Answer(answer),
                "response_answerable",
                &sources,
                None,
            )
        }
        Ok(answer) if answer.answerable => logged_lookup(
            question,
            KnowledgeLookup::InvalidResponse,
            "invalid_response",
            ABSENT,
            Some("validation"),
        ),
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
    use std::io::{Read as _, Write as _};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PROXY_TEST_CHILD: &str = "DL_KNOWLEDGE_PROXY_TEST_CHILD";
    const PROXY_TEST_TARGET: &str = "DL_KNOWLEDGE_PROXY_TEST_TARGET";

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

    #[test]
    fn decision_logs_speichern_bei_keinem_ergebnis_frageinhalt() {
        let cases = [
            (
                "SUCCESS_MARKER_7F",
                format!("SUCCESS_MARKER_7F{}", '\u{7}'),
                KnowledgeLookup::Answer(KnowledgeAnswer {
                    answerable: true,
                    answer: Some("Antwort".to_string()),
                    sources: Vec::new(),
                }),
                "yes",
                "source_grounded",
                None,
            ),
            (
                "NO_MARKER_0C",
                format!("NO_MARKER_0C{}", '\u{c}'),
                KnowledgeLookup::Unanswerable,
                "no",
                "none",
                None,
            ),
            (
                "TIMEOUT_MARKER_1F",
                format!("TIMEOUT_MARKER_1F{}", '\u{1f}'),
                KnowledgeLookup::Timeout,
                "timeout",
                "none",
                Some("timeout"),
            ),
            (
                "ERROR_MARKER_00",
                format!("ERROR_MARKER_00{}", '\0'),
                KnowledgeLookup::Transport,
                "error",
                "none",
                Some("transport"),
            ),
        ];
        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);

        for (_, question, lookup, _, _, error_class) in &cases {
            logged_lookup(
                question,
                lookup.clone(),
                "contract_test",
                ABSENT,
                *error_class,
            );
        }
        drop(guard);

        let logs = capture.text();
        assert_eq!(
            logs.matches("dl-community knowledge-client decision")
                .count(),
            4,
            "{logs}"
        );
        assert!(!logs.contains("question="), "{logs}");
        for control in ['\u{7}', '\u{c}', '\u{1f}', '\0'] {
            assert!(!logs.contains(control), "{logs}");
        }
        for (marker, question, _, verdict, confidence, _) in &cases {
            assert!(!logs.contains(marker), "{logs}");
            assert!(
                logs.contains(&format!("question_chars={}", question.chars().count())),
                "{logs}"
            );
            assert!(logs.contains(&format!("verdict={verdict}")), "{logs}");
            assert!(logs.contains(&format!("confidence={confidence}")), "{logs}");
        }
    }

    #[tokio::test]
    async fn stateful_support_admission_teilt_exakt_vier_db_pfade() {
        let admission = Arc::new(StatefulSupportAdmission::new());
        let mut held = Vec::new();
        for _ in 0..STATEFUL_SUPPORT_TURN_LIMIT {
            held.push(
                admission
                    .semaphore
                    .try_acquire()
                    .expect("vier gemeinsame FAQ-/Concierge-Slots"),
            );
        }
        assert!(admission.semaphore.try_acquire().is_err());

        let attempted = Arc::new(tokio::sync::Notify::new());
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let db_stateful_path_claimed = Arc::new(AtomicBool::new(false));
        let attempted_wait = attempted.notified();
        let fifth = tokio::spawn({
            let admission = admission.clone();
            let attempted = attempted.clone();
            let entered = entered.clone();
            let release = release.clone();
            let db_stateful_path_claimed = db_stateful_path_claimed.clone();
            async move {
                attempted.notify_one();
                let _permit = admission.acquire().await;
                db_stateful_path_claimed.store(true, Ordering::SeqCst);
                entered.notify_one();
                release.notified().await;
            }
        });

        attempted_wait.await;
        assert!(
            !db_stateful_path_claimed.load(Ordering::SeqCst),
            "fuenfter Turn darf vor Permit keinen DB-Stateful-Pfad beanspruchen"
        );
        drop(held.pop());
        entered.notified().await;
        assert!(db_stateful_path_claimed.load(Ordering::SeqCst));

        release.notify_one();
        fifth.await.expect("fifth stateful turn");
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

    fn child_process_http_server(
        status: &'static str,
        body: &'static str,
    ) -> (
        String,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind child server");
        listener
            .set_nonblocking(true)
            .expect("set child server nonblocking");
        let address = listener.local_addr().expect("child server address");
        let hit = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let hit_for_server = hit.clone();
        let stop_for_server = stop.clone();
        let handle = std::thread::spawn(move || {
            while !stop_for_server.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        hit_for_server.store(true, Ordering::SeqCst);
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .expect("set child server read timeout");
                        let mut request = [0_u8; 4096];
                        let _ = stream.read(&mut request);
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("child server accept: {error}"),
                }
            }
        });
        (format!("http://{address}"), hit, stop, handle)
    }

    #[tokio::test]
    async fn proxy_child_nutzt_loopback_direkt() {
        let Some(target) = std::env::var_os(PROXY_TEST_TARGET) else {
            return;
        };
        assert!(std::env::var_os(PROXY_TEST_CHILD).is_some());

        let result = ask(
            &target.to_string_lossy(),
            "PROXY_EXFILTRATION_MARKER",
            Duration::from_secs(1),
        )
        .await;

        assert!(
            matches!(result, KnowledgeLookup::Transport),
            "Loopback-Anfrage wurde nicht direkt ausgeführt"
        );
    }

    #[test]
    fn loopback_anfrage_ignoriert_systemproxy() {
        let (target_url, target_hit, target_stop, target_server) =
            child_process_http_server("503 Service Unavailable", "{}");
        let proxy_answer = r#"{"answerable":true,"answer":"Proxy-Antwort","sources":[{"title":"T","path":"public/p.html"}]}"#;
        let (proxy_url, proxy_hit, proxy_stop, proxy_server) =
            child_process_http_server("200 OK", proxy_answer);

        let output = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "knowledge_client::tests::proxy_child_nutzt_loopback_direkt",
                "--test-threads=1",
            ])
            .env(PROXY_TEST_CHILD, "1")
            .env(PROXY_TEST_TARGET, &target_url)
            .env("HTTP_PROXY", &proxy_url)
            .env("HTTPS_PROXY", &proxy_url)
            .env("ALL_PROXY", &proxy_url)
            .env("http_proxy", &proxy_url)
            .env("https_proxy", &proxy_url)
            .env("all_proxy", &proxy_url)
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .output()
            .expect("run isolated proxy child test");

        target_stop.store(true, Ordering::SeqCst);
        proxy_stop.store(true, Ordering::SeqCst);
        target_server.join().expect("join target server");
        proxy_server.join().expect("join proxy server");

        assert!(
            !proxy_hit.load(Ordering::SeqCst),
            "Loopback-Anfrage erreichte den Systemproxy"
        );
        assert!(
            target_hit.load(Ordering::SeqCst),
            "Loopback-Anfrage erreichte das direkte Ziel nicht"
        );
        assert!(output.status.success(), "isolierter Proxy-Test schlug fehl");
    }

    #[tokio::test]
    async fn answerable_response_is_answer() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"Ja.","sources":[{"title":"T","path":"public/p.html"},{"title":"Duplikat","path":"public/p.html"},{"title":"Guide","path":"guides/faq.html"}]}"#,
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
                        path: "public/p.html".to_string(),
                    },
                    KnowledgeSource {
                        title: "Duplikat".to_string(),
                        path: "public/p.html".to_string(),
                    },
                    KnowledgeSource {
                        title: "Guide".to_string(),
                        path: "guides/faq.html".to_string(),
                    },
                ],
            })
        );
        assert_client_decision(
            &logs,
            "yes",
            "source_grounded",
            "response_answerable",
            "public/p.html,guides/faq.html",
            "absent",
        );
        assert!(logs.contains(&format!("question_chars={}", question.chars().count())));
        assert!(!logs.contains("question="));
        assert!(!logs.contains('ä'));
        assert!(!logs.contains("NICHT_LOGGEN"));
        assert!(!logs.contains("Ja."));
        assert_eq!(logs.matches("public/p.html").count(), 1, "{logs}");
        assert!(!logs.contains(&url));
    }

    #[tokio::test]
    async fn answerable_response_braucht_text_und_ausschliesslich_sichere_html_quellen() {
        for (case, body) in [
            (
                "leerer Text",
                r#"{"answerable":true,"answer":"  ","sources":[{"title":"T","path":"public/p.html"}]}"#,
            ),
            (
                "keine Quelle",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[]}"#,
            ),
            (
                "falsche Endung",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[{"title":"T","path":"public/p.md"}]}"#,
            ),
            (
                "internal Segment",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[{"title":"T","path":"public/Internal/secret.html"}]}"#,
            ),
            (
                "Parent Segment",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[{"title":"T","path":"../public/p.html"}]}"#,
            ),
            (
                "absolut",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[{"title":"T","path":"/public/p.html"}]}"#,
            ),
            (
                "Doppelpunkt",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[{"title":"T","path":"https://example.invalid/p.html"}]}"#,
            ),
            (
                "Steuerzeichen",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[{"title":"T","path":"public/\u000asecret.html"}]}"#,
            ),
            (
                "gemischt",
                r#"{"answerable":true,"answer":"NICHT_LOGGEN","sources":[{"title":"T","path":"public/p.html"},{"title":"X","path":"../internal/x.html"}]}"#,
            ),
        ] {
            let (url, handle) = knowledge_server(200, body, Duration::ZERO).await;

            let (result, logs) = ask_with_logs(&url, "Frage?", Duration::from_secs(1)).await;
            let _ = handle.await;

            assert_eq!(result, KnowledgeLookup::InvalidResponse, "{case}");
            assert_client_decision(
                &logs,
                "error",
                "none",
                "invalid_response",
                "absent",
                "validation",
            );
            assert!(!logs.contains("NICHT_LOGGEN"), "{case}: {logs}");
        }
    }

    #[tokio::test]
    async fn non_loopback_target_wird_vor_request_abgelehnt() {
        let base_url = "http://192.0.2.1:8896";

        let (result, logs) = ask_with_logs(base_url, "Frage?", Duration::from_millis(10)).await;

        assert_eq!(result, KnowledgeLookup::InvalidResponse);
        assert_client_decision(&logs, "error", "none", "invalid_target", "absent", "target");
        assert!(!logs.contains(base_url));
    }

    #[tokio::test]
    async fn loopback_redirect_leitet_frage_nicht_an_zweites_ziel_weiter() {
        let target = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect target");
        let target_addr = target.local_addr().expect("redirect target addr");
        let followed = Arc::new(AtomicBool::new(false));
        let followed_for_server = followed.clone();
        let target_server = tokio::spawn(async move {
            let accepted = tokio::time::timeout(Duration::from_millis(500), target.accept()).await;
            if let Ok(Ok((mut stream, _))) = accepted {
                followed_for_server.store(true, Ordering::SeqCst);
                let mut request = [0_u8; 2048];
                let _ = stream.read(&mut request).await;
                let body = r#"{"answerable":true,"answer":"Darf nicht folgen.","sources":[{"title":"T","path":"public/p.html"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        let redirect = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect source");
        let redirect_addr = redirect.local_addr().expect("redirect source addr");
        let redirect_server = tokio::spawn(async move {
            let (mut stream, _) = redirect.accept().await.expect("redirect request");
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{target_addr}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("redirect response");
        });

        let (result, logs) = ask_with_logs(
            &format!("http://{redirect_addr}"),
            "Private Frage?",
            Duration::from_secs(1),
        )
        .await;
        redirect_server.await.expect("redirect server");
        target_server.await.expect("target server");

        assert_eq!(result, KnowledgeLookup::Transport);
        assert!(!followed.load(Ordering::SeqCst));
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

/// Reiner Abruf öffentlicher Passagen für die gemeinsame Antwortinstanz.
pub struct CommunityRetriever {
    pub base_url: String,
    pub timeout: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrievalWire {
    status: String,
    evidence: Vec<dl_answer::Evidence>,
    truncated: bool,
}

#[async_trait::async_trait]
impl dl_answer::Retriever for CommunityRetriever {
    async fn retrieve(
        &self,
        question: &str,
    ) -> Result<dl_answer::Retrieved, dl_answer::AnswerError> {
        use dl_answer::AnswerError;
        let mut url =
            reqwest::Url::parse(&self.base_url).map_err(|_| AnswerError::InvalidEvidence)?;
        let loopback = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !loopback
            || !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AnswerError::InvalidEvidence);
        }
        url.set_path(&format!(
            "{}/public/v1/retrieve",
            url.path().trim_end_matches('/')
        ));
        url.set_query(None);
        url.set_fragment(None);
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout)
            .build()
            .map_err(|_| AnswerError::Retrieval)?;
        let mut response = client
            .post(url)
            .json(&KnowledgeQuestion { question })
            .send()
            .await
            .map_err(map_retrieval_error)?;
        if !response.status().is_success() {
            return Err(AnswerError::Retrieval);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_retrieval_error)? {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err(AnswerError::InvalidEvidence);
            }
            bytes.extend_from_slice(&chunk);
        }
        let wire: RetrievalWire =
            serde_json::from_slice(&bytes).map_err(|_| AnswerError::InvalidEvidence)?;
        validate_retrieval(wire)
    }
}

fn map_retrieval_error(error: reqwest::Error) -> dl_answer::AnswerError {
    if error.is_timeout() {
        dl_answer::AnswerError::Timeout
    } else {
        dl_answer::AnswerError::Retrieval
    }
}

fn validate_retrieval(wire: RetrievalWire) -> Result<dl_answer::Retrieved, dl_answer::AnswerError> {
    use dl_answer::{AnswerError, Source};
    let mut ids = HashSet::new();
    let units: usize = wire
        .evidence
        .iter()
        .map(|item| item.text.encode_utf16().count())
        .sum();
    let valid = match wire.status.as_str() {
        "ready" => !wire.evidence.is_empty(),
        "no_evidence" => wire.evidence.is_empty(),
        _ => false,
    };
    if !valid
        || wire.evidence.len() > 12
        || units > 12_000
        || wire.evidence.iter().any(|item| {
            !matches!(item.source, Source::CommunityPage { .. })
                || !item.source.valid()
                || item.text.trim().is_empty()
                || !item
                    .id
                    .strip_prefix('C')
                    .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                || !ids.insert(item.id.clone())
        })
    {
        return Err(AnswerError::InvalidEvidence);
    }
    Ok(dl_answer::Retrieved {
        evidence: wire.evidence,
        truncated: wire.truncated,
        out_of_domain: false,
    })
}
