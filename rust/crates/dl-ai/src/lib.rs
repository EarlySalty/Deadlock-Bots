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

/// Multimodal-Anfrage: Text + bis zu 4 Bild-URLs (Port von
/// `generate_multimodal` aus `cogs/ai_connector.py`). Default-Temperatur im
/// Original ist 0.2 — Konsumenten setzen sie explizit.
#[derive(Debug, Clone)]
pub struct GenerateMultimodalRequest {
    pub prompt: String,
    pub image_urls: Vec<String>,
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

/// Bild-/Multimodal-Generierung (analog `TextGenerator`). Eigener Trait, damit
/// die Moderation gezielt nur diese Fähigkeit mocken/injizieren kann.
#[async_trait::async_trait]
pub trait VisionGenerator: Send + Sync {
    async fn generate_multimodal(&self, request: GenerateMultimodalRequest) -> Option<String>;
}

/// Gültige Bild-URL-Präfixe (Original: `valid_prefixes` in `generate_multimodal`).
const VALID_IMAGE_PREFIXES: [&str; 3] = ["http://", "https://", "data:image/"];
/// Maximale Bildanzahl pro Request (Original: `valid_images[:4]`).
const MAX_MULTIMODAL_IMAGES: usize = 4;

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

impl MiniMaxClient {
    /// `data:image/...;base64,<data>` → (media_type, base64-data). Sonst None.
    /// Spiegelt `_parse_data_image_uri` aus dem Original 1:1.
    fn parse_data_image_uri(image_url: &str) -> Option<(String, String)> {
        if !image_url.starts_with("data:image/") {
            return None;
        }
        let (header, data) = image_url.split_once(',')?;
        if !header.contains(";base64") {
            return None;
        }
        // header[5:] → ab "data:" alles bis zum ersten ';' ist der Media-Type.
        let after_prefix = &header[5..];
        let media_type = after_prefix
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let media_type = if media_type.is_empty() {
            "image/png".to_string()
        } else {
            media_type
        };
        if !media_type.starts_with("image/") {
            return None;
        }
        Some((media_type, data.to_string()))
    }

    /// Anthropic-`/v1`-Content-Blöcke: Text zuerst, dann Bilder als
    /// base64- oder url-`source` (Original: `_build_token_plan_content`).
    fn build_token_plan_content(prompt: &str, image_urls: &[String]) -> Value {
        let mut content = vec![json!({ "type": "text", "text": prompt })];
        for image_url in image_urls {
            if let Some((media_type, data)) = Self::parse_data_image_uri(image_url) {
                content.push(json!({
                    "type": "image",
                    "source": { "type": "base64", "media_type": media_type, "data": data },
                }));
            } else {
                content.push(json!({
                    "type": "image",
                    "source": { "type": "url", "url": image_url },
                }));
            }
        }
        Value::Array(content)
    }

    /// Standard-`chatcompletion_v2`-User-Content: ohne Bilder reiner Text,
    /// sonst Text + `image_url`-Blöcke (Original: `_build_standard_user_content`).
    fn build_standard_user_content(prompt: &str, image_urls: &[String]) -> Value {
        if image_urls.is_empty() {
            return Value::String(prompt.to_string());
        }
        let mut content = vec![json!({ "type": "text", "text": prompt })];
        for image_url in image_urls {
            content.push(json!({
                "type": "image_url",
                "image_url": { "url": image_url },
            }));
        }
        Value::Array(content)
    }
}

#[async_trait::async_trait]
impl VisionGenerator for MiniMaxClient {
    async fn generate_multimodal(&self, request: GenerateMultimodalRequest) -> Option<String> {
        let model = request.model.unwrap_or_else(|| self.model.clone());
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);

        // Wie das Original: nur valide Präfixe, auf die ersten 4 kappen.
        let images: Vec<String> = request
            .image_urls
            .iter()
            .filter(|url| VALID_IMAGE_PREFIXES.iter().any(|p| url.starts_with(p)))
            .take(MAX_MULTIMODAL_IMAGES)
            .cloned()
            .collect();

        let response = if self.token_plan {
            self.http
                .post(format!("{}/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": model,
                    "system": request.system_prompt.unwrap_or_default(),
                    "messages": [{
                        "role": "user",
                        "content": Self::build_token_plan_content(&request.prompt, &images),
                    }],
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
            messages.push(json!({
                "role": "user",
                "content": Self::build_standard_user_content(&request.prompt, &images),
            }));
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
                tracing::warn!(%err, "MiniMax-Multimodal-Request fehlgeschlagen");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "MiniMax-Multimodal-API-Fehler");
            return None;
        }
        let data: Value = response.json().await.ok()?;

        if self.token_plan {
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

    #[test]
    fn multimodal_content_format() {
        // Token-Plan: data:-URI → base64-source, http → url-source.
        let images = vec![
            "data:image/jpeg;base64,QUJD".to_string(),
            "https://example.com/a.png".to_string(),
        ];
        let content = MiniMaxClient::build_token_plan_content("frage", &images);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "frage");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(content[1]["source"]["data"], "QUJD");
        assert_eq!(content[2]["source"]["type"], "url");
        assert_eq!(content[2]["source"]["url"], "https://example.com/a.png");

        // Standard ohne Bilder → reiner String; mit Bildern → image_url-Blöcke.
        assert_eq!(
            MiniMaxClient::build_standard_user_content("frage", &[]),
            serde_json::Value::String("frage".to_string())
        );
        let std_content = MiniMaxClient::build_standard_user_content("frage", &images);
        assert_eq!(std_content[1]["type"], "image_url");
        assert_eq!(std_content[1]["image_url"]["url"], "data:image/jpeg;base64,QUJD");
    }

    /// Filter (valide Präfixe) + Kappung auf 4 gegen einen Mock beweisen.
    #[tokio::test]
    async fn multimodal_filtert_und_kappt() {
        use axum::{routing::post, Json, Router};
        let captured: Arc<std::sync::Mutex<Vec<Value>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
        let app = Router::new().route(
            "/messages",
            post(move |Json(body): Json<Value>| {
                let cap = cap.clone();
                async move {
                    cap.lock().expect("lock").push(body);
                    Json(json!({ "content": [ { "type": "text", "text": "Scam" } ]}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        let base = format!("http://{addr}");

        let client = MiniMaxClient::new(&base, "tp-key", true, "MiniMax-M3");
        let text = client
            .generate_multimodal(GenerateMultimodalRequest {
                prompt: "p".into(),
                image_urls: vec![
                    "ftp://nope".into(), // ungültiges Präfix → gefiltert
                    "https://1".into(),
                    "https://2".into(),
                    "https://3".into(),
                    "https://4".into(),
                    "https://5".into(), // > 4 → gekappt
                ],
                system_prompt: Some("sys".into()),
                model: None,
                max_output_tokens: Some(300),
                temperature: 0.2,
            })
            .await;
        assert_eq!(text.as_deref(), Some("Scam"));
        let captured = captured.lock().expect("lock");
        let content = captured[0]["messages"][0]["content"]
            .as_array()
            .expect("array");
        // 1 Text-Block + genau 4 Bild-Blöcke (5./ungültige gefiltert).
        assert_eq!(content.len(), 5);
        assert_eq!(content[1]["source"]["url"], "https://1");
        assert_eq!(content[4]["source"]["url"], "https://4");
    }
}
