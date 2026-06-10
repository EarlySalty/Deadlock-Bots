//! dl-ai — MiniMax-Anbindung (Port des MiniMax-Pfads aus
//! `cogs/ai_connector.py`).
//!
//! Zwei Modi wie das Original:
//! - **Token-Plan** (`MINIMAX_TOKEN_PLAN_KEY`): Anthropic-kompatible API
//!   (`x-api-key` + `POST /messages`, content-Fragmente).
//! - **Standard** (`MINIMAX_API_KEY`/`MINMAX`): Bearer +
//!   `POST /text/chatcompletion_v2` (choices/message/content).
//!
//! Bewusst NICHT portiert: die OpenAI-/Gemini-Pfade — auf diesem System ist
//! nur MiniMax konfiguriert; alle Konsumenten (Matcher, LFG, Moderation)
//! nutzen Provider `minimax`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

pub const DEFAULT_MODEL: &str = "MiniMax-M3";
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 800;
const DEFAULT_BASE_URL: &str = "https://api.minimax.chat/v1";
const DEFAULT_TOKEN_PLAN_BASE_URL: &str = "https://api.minimax.io/anthropic/v1";

#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub temperature: f64,
}

/// Text-Generierung — Konsumenten hängen am Trait (Tests mocken ihn).
#[async_trait::async_trait]
pub trait TextGenerator: Send + Sync {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String>;
}

pub struct MiniMaxClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    token_plan: bool,
    model: String,
}

impl MiniMaxClient {
    /// ENV-Kette wie das Original: MINIMAX_TOKEN_PLAN_KEY → MINIMAX_API_KEY
    /// → MINMAX; Token-Plan-Key schaltet die Anthropic-API um.
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Arc<Self>> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let token_plan_key = get("MINIMAX_TOKEN_PLAN_KEY");
        let api_key = token_plan_key
            .clone()
            .or_else(|| get("MINIMAX_API_KEY"))
            .or_else(|| get("MINMAX"))?;
        let token_plan = token_plan_key.is_some();
        let base_url = if token_plan {
            get("MINIMAX_TOKEN_PLAN_BASE_URL")
                .unwrap_or_else(|| DEFAULT_TOKEN_PLAN_BASE_URL.to_string())
        } else {
            get("MINIMAX_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
        };
        let model = get("MINIMAX_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        tracing::info!(
            mode = if token_plan { "token_plan" } else { "standard" },
            %base_url,
            %model,
            "MiniMax-Client initialisiert"
        );
        Some(Self::new(base_url, api_key, token_plan, model))
    }

    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        token_plan: bool,
        model: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            token_plan,
            model: model.into(),
        })
    }
}

#[async_trait::async_trait]
impl TextGenerator for MiniMaxClient {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String> {
        let model = request.model.unwrap_or_else(|| self.model.clone());
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);

        let response = if self.token_plan {
            self.http
                .post(format!("{}/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": model,
                    "system": request.system_prompt.unwrap_or_default(),
                    "messages": [{ "role": "user", "content": request.prompt }],
                    "max_tokens": max_tokens,
                    "temperature": request.temperature,
                }))
                .send()
                .await
        } else {
            let mut messages = Vec::new();
            if let Some(system) = &request.system_prompt {
                messages.push(json!({ "role": "system", "content": system }));
            }
            messages.push(json!({ "role": "user", "content": request.prompt }));
            self.http
                .post(format!("{}/text/chatcompletion_v2", self.base_url))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&json!({
                    "model": model,
                    "messages": messages,
                    "max_tokens": max_tokens,
                    "temperature": request.temperature,
                }))
                .send()
                .await
        };
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(%err, "MiniMax-Request fehlgeschlagen");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "MiniMax-API-Fehler");
            return None;
        }
        let data: Value = response.json().await.ok()?;

        if self.token_plan {
            // Anthropic-Format: content-Fragmente vom Typ "text"
            let fragments: Vec<&str> = data
                .get("content")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            if fragments.is_empty() {
                return None;
            }
            Some(fragments.concat().trim().to_string())
        } else {
            data.get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }
    }
}

/// `<think>…</think>`-Blöcke entfernen (MiniMax-M3-Eigenheit).
pub fn strip_think(text: &str) -> String {
    let lower = text.to_lowercase();
    match (lower.find("<think>"), lower.find("</think>")) {
        (Some(start), Some(end)) if end > start => {
            format!("{}{}", &text[..start], &text[end + "</think>".len()..])
                .trim()
                .to_string()
        }
        _ => text.trim().to_string(),
    }
}

// ── Matcher-Scoring (schließt die Phase-3c-Lücke) ──────────────────────────

/// AiScorer für den Streamer-Link-Matcher — Prompt/Parameter wortgleich
/// zum Original (temperature 0, max 160 Tokens, JSON-only-System-Prompt).
pub struct MatcherScorer {
    pub generator: Arc<dyn TextGenerator>,
}

#[async_trait::async_trait]
impl dl_bridges::matcher::AiScorer for MatcherScorer {
    async fn score(
        &self,
        login: &str,
        member: &dl_bridges::matcher::MemberLite,
        ratio: f64,
    ) -> (Option<i64>, String) {
        let system =
            "Du bewertest, ob ein Discord-Nutzer und ein Twitch-Streamer dieselbe Person sind, \
ausschließlich anhand der Namen. Antworte NUR mit JSON: \
{\"score\": <0-100>, \"reason\": \"<kurze Begründung>\"}. \
score ist die Wahrscheinlichkeit in Prozent. Kein weiterer Text.";
        let prompt = format!(
            "Twitch-Login: {login}\n\
             Discord-Username: {}\n\
             Discord-Anzeigename: {}\n\
             Discord-Servername: {}\n\
             String-Ähnlichkeit der normalisierten Namen (0-1): {ratio:.2}\n\
             Wie wahrscheinlich ist es dieselbe Person?",
            member.name,
            member.global_name.as_deref().unwrap_or("-"),
            member.nick.as_deref().unwrap_or("-"),
        );
        let text = self
            .generator
            .generate_text(GenerateRequest {
                prompt,
                system_prompt: Some(system.to_string()),
                model: None,
                max_output_tokens: Some(160),
                temperature: 0.0,
            })
            .await;
        dl_bridges::matcher::parse_ai_score(text.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn think_stripping() {
        assert_eq!(
            strip_think("<think>überlegung</think>  {\"score\": 90}"),
            "{\"score\": 90}"
        );
        assert_eq!(strip_think("plain"), "plain");
    }

    /// Wire-Format beider Modi gegen einen axum-Mock beweisen.
    #[tokio::test]
    async fn beide_modi_gegen_mock() {
        use axum::{routing::post, Json, Router};
        let captured: Arc<std::sync::Mutex<Vec<(String, Value)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        let cap1 = captured.clone();
        let cap2 = captured.clone();
        let app = Router::new()
            .route(
                "/messages",
                post(
                    move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                        let cap = cap1.clone();
                        async move {
                            assert_eq!(
                                headers.get("x-api-key").and_then(|v| v.to_str().ok()),
                                Some("tp-key")
                            );
                            cap.lock().expect("lock").push(("messages".into(), body));
                            Json(json!({ "content": [
                                { "type": "thinking", "text": "ignorieren" },
                                { "type": "text", "text": "Hallo " },
                                { "type": "text", "text": "Welt" },
                            ]}))
                        }
                    },
                ),
            )
            .route(
                "/text/chatcompletion_v2",
                post(
                    move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                        let cap = cap2.clone();
                        async move {
                            assert_eq!(
                                headers.get("authorization").and_then(|v| v.to_str().ok()),
                                Some("Bearer std-key")
                            );
                            cap.lock().expect("lock").push(("chat".into(), body));
                            Json(json!({ "choices": [
                                { "message": { "content": "Antwort" } }
                            ]}))
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        let base = format!("http://{addr}");

        let token_plan = MiniMaxClient::new(&base, "tp-key", true, "MiniMax-M3");
        let text = token_plan
            .generate_text(GenerateRequest {
                prompt: "ping".into(),
                system_prompt: Some("sys".into()),
                model: None,
                max_output_tokens: Some(50),
                temperature: 0.0,
            })
            .await;
        assert_eq!(text.as_deref(), Some("Hallo Welt"));

        let standard = MiniMaxClient::new(&base, "std-key", false, "MiniMax-M3");
        let text = standard
            .generate_text(GenerateRequest {
                prompt: "ping".into(),
                system_prompt: Some("sys".into()),
                model: None,
                max_output_tokens: Some(50),
                temperature: 0.6,
            })
            .await;
        assert_eq!(text.as_deref(), Some("Antwort"));

        let captured = captured.lock().expect("lock");
        let (_, token_plan_body) = &captured[0];
        assert_eq!(token_plan_body["system"], "sys");
        assert_eq!(token_plan_body["max_tokens"], 50);
        let (_, standard_body) = &captured[1];
        assert_eq!(standard_body["messages"][0]["role"], "system");
        assert_eq!(standard_body["messages"][1]["content"], "ping");
    }
}
