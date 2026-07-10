use std::time::Duration;

use serde::{Deserialize, Serialize};

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

pub(crate) async fn ask(base_url: &str, question: &str, timeout: Duration) -> KnowledgeLookup {
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(client) => client,
        Err(_) => return KnowledgeLookup::Transport,
    };
    let url = format!("{}/public/v1/ask", base_url.trim_end_matches('/'));
    let response = match client
        .post(url)
        .json(&KnowledgeQuestion { question })
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return KnowledgeLookup::Timeout,
        Err(_) => return KnowledgeLookup::Transport,
    };
    if !response.status().is_success() {
        return KnowledgeLookup::Transport;
    }
    match response.json::<KnowledgeAnswer>().await {
        Ok(answer) if answer.answerable => KnowledgeLookup::Answer(answer),
        Ok(_) => KnowledgeLookup::Unanswerable,
        Err(error) if error.is_timeout() => KnowledgeLookup::Timeout,
        Err(error) if error.is_decode() => KnowledgeLookup::InvalidResponse,
        Err(_) => KnowledgeLookup::Transport,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
            r#"{"answerable":true,"answer":"Ja.","sources":[{"title":"T","path":"p.md"}]}"#,
            Duration::ZERO,
        )
        .await;

        let result = ask(&url, "Frage?", Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(
            result,
            KnowledgeLookup::Answer(KnowledgeAnswer {
                answerable: true,
                answer: Some("Ja.".to_string()),
                sources: vec![KnowledgeSource {
                    title: "T".to_string(),
                    path: "p.md".to_string(),
                }],
            })
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

        let result = ask(&url, "Frage?", Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::Unanswerable);
    }

    #[tokio::test]
    async fn delayed_response_is_timeout() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"zu spaet","sources":[]}"#,
            Duration::from_millis(100),
        )
        .await;

        let result = ask(&url, "Frage?", Duration::from_millis(10)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::Timeout);
    }

    #[tokio::test]
    async fn unsuccessful_status_is_transport() {
        let (url, handle) = knowledge_server(500, "{}", Duration::ZERO).await;

        let result = ask(&url, "Frage?", Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::Transport);
    }

    #[tokio::test]
    async fn invalid_json_is_invalid_response() {
        let (url, handle) = knowledge_server(200, "kein json", Duration::ZERO).await;

        let result = ask(&url, "Frage?", Duration::from_secs(1)).await;
        let _ = handle.await;

        assert_eq!(result, KnowledgeLookup::InvalidResponse);
    }
}
