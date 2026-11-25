//! Zentrale KI-Anbindung für Bot-Flows.
//!
//! Produktive Text- und Bildfactorys nutzen Fireworks mit der gemeinsamen,
//! täglich geprüften DeepSeek-Flash-Auswahl. Der Reader wird vor jedem Aufruf
//! erneut gelesen; Zugangsdaten bleiben im bestehenden Secret-Speicher.

use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose, Engine as _};
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};

mod chat_provider;
mod chat_text;
mod configured_chat;
mod selected_chat;
mod transparency;
mod transparency_log;
pub use chat_provider::*;
pub use chat_text::*;
pub use fireworks_model_selection::selected_model as selected_flash_model;
pub use transparency::*;
pub use transparency_log::*;

pub const DEFAULT_MODEL: &str = "MiniMax-M3";
/// Freigegebene Ausgangsversion; produktive Textaufrufe lesen die tägliche
/// gemeinsame Flash-Auswahl und ignorieren frühere Modell-Pins.
pub const DEFAULT_FIREWORKS_MODEL: &str = "accounts/fireworks/models/deepseek-v4p1-flash";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-5.4-nano";
pub const DEFAULT_OPENAI_TEXT_MODEL: &str = "gpt-4o-mini";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.0-flash";
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 800;
const DEFAULT_BASE_URL: &str = "https://api.minimax.chat/v1";
const DEFAULT_TOKEN_PLAN_BASE_URL: &str = "https://api.minimax.io/anthropic/v1";
const DEFAULT_FIREWORKS_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const MAX_OPENAI_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

/// Content-Length is advisory; enforce the cap while receiving every chunk.
async fn bounded_response(mut response: reqwest::Response, limit: u64) -> Option<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() as u64 > limit.saturating_sub(bytes.len() as u64) {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some(bytes)
}

fn trusted_image_url(raw: &str) -> bool {
    if raw.starts_with("data:image/") {
        return true;
    }
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some("cdn.discordapp.com" | "media.discordapp.net")
        )
}

#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub max_output_tokens: Option<u32>,
    /// Optionaler OpenAI-kompatibler Reasoning-Modus; derzeit nur Fireworks.
    pub reasoning_effort: Option<String>,
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

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl ToolDefinition {
    pub fn as_wire_value(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.input_schema,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ToolUseRequest {
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub temperature: f64,
    pub tools: Vec<ToolDefinition>,
    pub max_tool_calls: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolGeneration {
    pub text: Option<String>,
    pub tool_calls: Vec<String>,
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, tool_name: &str, tool_input: &Value) -> Result<Value, String>;
}

/// Text-Generierung — Konsumenten hängen am Trait (Tests mocken ihn).
#[async_trait::async_trait]
pub trait TextGenerator: Send + Sync {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String>;
}

/// Anthropic-kompatibler Tool-Use-Loop für Text-Generierung.
#[async_trait::async_trait]
pub trait ToolTextGenerator: Send + Sync {
    async fn generate_text_with_tools(
        &self,
        request: ToolUseRequest,
        tool_executor: Arc<dyn ToolExecutor>,
    ) -> ToolGeneration;
}

/// Bild-/Multimodal-Generierung (analog `TextGenerator`). Eigener Trait, damit
/// die Moderation gezielt nur diese Fähigkeit mocken/injizieren kann.
#[async_trait::async_trait]
pub trait VisionGenerator: Send + Sync {
    async fn generate_multimodal(&self, request: GenerateMultimodalRequest) -> Option<String>;
}

/// Zentraler Bildpfad mit demselben Zugang und Modellvertrag wie Textaufrufe.
pub fn fireworks_vision_from_lookup(
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<(Arc<dyn VisionGenerator>, String)> {
    let model = match selected_flash_model() {
        Ok(model) => model,
        Err(error) => {
            tracing::error!(%error, "Gemeinsame Modellauswahl für die Bildanalyse fehlt");
            return None;
        }
    };
    Some((FireworksClient::from_env(lookup)?, model))
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

pub mod discord_image;

pub struct OpenAiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

pub struct FireworksClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    shared_selection: bool,
}

impl OpenAiClient {
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Arc<Self>> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let api_key = get("OPENAI_API_KEY").or_else(|| get("DEADLOCK_OPENAI_KEY"))?;
        let base_url =
            get("OPENAI_BASE_URL").unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
        let model = get("AI_IMAGE_MODEL")
            .or_else(|| get("OPENAI_MODEL"))
            .or_else(|| get("AI_OPENAI_MODEL"))
            .unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string());
        tracing::info!(%base_url, %model, "OpenAI-Vision-Client initialisiert");
        Some(Self::new(base_url, api_key, model))
    }

    pub fn text_from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Arc<Self>> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let api_key = get("OPENAI_API_KEY").or_else(|| get("DEADLOCK_OPENAI_KEY"))?;
        let base_url =
            get("OPENAI_BASE_URL").unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
        let model = get("AI_OPENAI_MODEL").unwrap_or_else(|| DEFAULT_OPENAI_TEXT_MODEL.to_string());
        tracing::info!(%base_url, %model, "OpenAI-Text-Client initialisiert");
        Some(Self::new(base_url, api_key, model))
    }

    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
        })
    }

    async fn image_data_uri(http: &reqwest::Client, image_url: &str) -> Option<String> {
        if image_url.starts_with("data:image/") {
            if image_url.len() as u64 > MAX_OPENAI_IMAGE_BYTES * 4 / 3 + 1024 {
                return None;
            }
            return Some(image_url.to_string());
        }
        if !(image_url.starts_with("http://") || image_url.starts_with("https://")) {
            return None;
        }
        let response = match http.get(image_url).send().await {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(%err, "OpenAI-Vision: Bilddownload fehlgeschlagen");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "OpenAI-Vision: Bilddownload-HTTP-Fehler");
            return None;
        }
        if response
            .content_length()
            .map(|len| len > MAX_OPENAI_IMAGE_BYTES)
            .unwrap_or(false)
        {
            tracing::warn!("OpenAI-Vision: Bild zu gross, uebersprungen");
            return None;
        }
        let media_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(Self::clean_image_media_type)
            .unwrap_or_else(|| Self::infer_image_media_type(image_url));
        let bytes = bounded_response(response, MAX_OPENAI_IMAGE_BYTES).await?;
        if bytes.is_empty() {
            return None;
        }
        let encoded = general_purpose::STANDARD.encode(bytes);
        Some(format!("data:{media_type};base64,{encoded}"))
    }

    fn clean_image_media_type(raw: &str) -> Option<String> {
        let media_type = raw.split(';').next()?.trim().to_lowercase();
        if media_type.starts_with("image/") {
            Some(media_type)
        } else {
            None
        }
    }

    fn infer_image_media_type(image_url: &str) -> String {
        let lower = image_url
            .split(['?', '#'])
            .next()
            .unwrap_or(image_url)
            .to_lowercase();
        if lower.ends_with(".png") {
            "image/png".to_string()
        } else if lower.ends_with(".gif") {
            "image/gif".to_string()
        } else if lower.ends_with(".webp") {
            "image/webp".to_string()
        } else {
            "image/jpeg".to_string()
        }
    }

    fn extract_openai_text(data: &Value) -> Option<String> {
        if let Some(text) = data.get("output_text").and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }

        let mut fragments = Vec::new();
        if let Some(items) = data
            .get("output")
            .or_else(|| data.get("outputs"))
            .and_then(Value::as_array)
        {
            for item in items {
                if item.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                if let Some(parts) = item.get("content").and_then(Value::as_array) {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            fragments.push(text);
                        }
                    }
                }
            }
        }

        if let Some(choices) = data.get("choices").and_then(Value::as_array) {
            for choice in choices {
                let Some(content) = choice.get("message").and_then(|m| m.get("content")) else {
                    continue;
                };
                if let Some(text) = content.as_str() {
                    fragments.push(text);
                    continue;
                }
                if let Some(parts) = content.as_array() {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            fragments.push(text);
                        }
                    }
                }
            }
        }

        let text = fragments.concat().trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn normalize_scam_json(text: &str) -> Option<String> {
        let cleaned = strip_think(text);
        let (Some(open), Some(close)) = (cleaned.find('{'), cleaned.rfind('}')) else {
            return None;
        };
        if close <= open {
            return None;
        }
        let Ok(payload) = serde_json::from_str::<Value>(&cleaned[open..=close]) else {
            return None;
        };
        if !payload.is_object() {
            return None;
        }
        let is_scam = payload
            .get("is_scam")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let confidence = payload
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let reason = payload
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(300)
            .collect::<String>();
        Some(
            json!({
                "is_scam": is_scam,
                "confidence": confidence,
                "reason": reason,
            })
            .to_string(),
        )
    }
}

impl FireworksClient {
    fn http_client() -> Option<reqwest::Client> {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                tracing::warn!("Sicherer Fireworks-HTTP-Client konnte nicht erstellt werden")
            })
            .ok()
    }

    fn complete_text(data: &Value) -> Option<String> {
        if data
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|choices| {
                choices.iter().any(|choice| {
                    choice.get("finish_reason").and_then(Value::as_str) == Some("length")
                })
            })
        {
            return None;
        }
        OpenAiClient::extract_openai_text(data)
    }

    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Arc<Self>> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let api_key = get("FIREWORK_API_KEY").or_else(|| get("FIREWORKS_API_KEY"))?;
        let base_url = get("FIREWORK_BASE_URL")
            .or_else(|| get("FIREWORKS_BASE_URL"))
            .unwrap_or_else(|| DEFAULT_FIREWORKS_BASE_URL.to_string());
        let model = DEFAULT_FIREWORKS_MODEL;
        tracing::info!(%base_url, "Fireworks-Text-Client mit gemeinsamer Flash-Auswahl initialisiert");
        let mut client = Self::new(base_url, api_key, model)?;
        Arc::get_mut(&mut client)?.shared_selection = true;
        Some(client)
    }

    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Option<Arc<Self>> {
        Some(Arc::new(Self {
            http: Self::http_client()?,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            shared_selection: false,
        }))
    }
}

pub struct GeminiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl GeminiClient {
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Arc<Self>> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let api_key = get("GOOGLE_API_KEY").or_else(|| get("GEMINI_API_KEY"))?;
        let base_url =
            get("GEMINI_BASE_URL").unwrap_or_else(|| DEFAULT_GEMINI_BASE_URL.to_string());
        let model = get("AI_GEMINI_MODEL").unwrap_or_else(|| DEFAULT_GEMINI_MODEL.to_string());
        tracing::info!(%base_url, %model, "Gemini-Text-Client initialisiert");
        Some(Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
        }))
    }
}

#[async_trait::async_trait]
impl TextGenerator for FireworksClient {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String> {
        let model = self.selected_request_model(request.model)?;
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
        let mut messages = Vec::new();
        if let Some(system) = &request.system_prompt {
            messages.push(json!({ "role": "system", "content": system }));
        }
        messages.push(json!({ "role": "user", "content": request.prompt }));

        let body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": request.temperature,
            "response_format": { "type": "json_object" },
            "reasoning_effort": request.reasoning_effort.as_deref().unwrap_or("none"),
        });

        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(%err, "Fireworks-Text-Request fehlgeschlagen");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Fireworks-Text-API-Fehler");
            return None;
        }
        let data: Value =
            serde_json::from_slice(&bounded_response(response, MAX_API_RESPONSE_BYTES).await?)
                .ok()?;
        Self::complete_text(&data)
    }
}

impl FireworksClient {
    fn selected_request_model(&self, requested: Option<String>) -> Option<String> {
        if self.shared_selection {
            match selected_flash_model() {
                Ok(model) => Some(model),
                Err(error) => {
                    tracing::warn!(%error, "Gemeinsame Flash-Modellauswahl nicht verfügbar");
                    None
                }
            }
        } else {
            Some(requested.unwrap_or_else(|| self.model.clone()))
        }
    }

    async fn vision_messages(&self, request: &GenerateMultimodalRequest) -> Option<Vec<Value>> {
        let mut content = vec![json!({ "type": "text", "text": request.prompt })];
        for image_url in request
            .image_urls
            .iter()
            .filter(|url| trusted_image_url(url))
            .take(MAX_MULTIMODAL_IMAGES)
        {
            if let Some(data_uri) = OpenAiClient::image_data_uri(&self.http, image_url).await {
                content.push(json!({"type": "image_url", "image_url": {"url": data_uri}}));
            }
        }
        if content.len() == 1 {
            return None;
        }
        let mut messages = Vec::new();
        if let Some(system) = &request.system_prompt {
            messages.push(json!({ "role": "system", "content": system }));
        }
        messages.push(json!({ "role": "user", "content": content }));
        Some(messages)
    }
}

#[async_trait::async_trait]
impl VisionGenerator for FireworksClient {
    async fn generate_multimodal(&self, request: GenerateMultimodalRequest) -> Option<String> {
        let model = self.selected_request_model(request.model.clone())?;
        let messages = self.vision_messages(&request).await?;
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&json!({
                "model": model,
                "messages": messages,
                "max_tokens": request.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
                "temperature": request.temperature,
                "response_format": {"type": "json_object"},
                "reasoning_effort": "none",
            }))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Flash-Bildanalyse fehlgeschlagen");
            return None;
        }
        let data: Value =
            serde_json::from_slice(&bounded_response(response, MAX_API_RESPONSE_BYTES).await?)
                .ok()?;
        let text = Self::complete_text(&data)?;
        Some(OpenAiClient::normalize_scam_json(&text).unwrap_or(text))
    }
}

#[async_trait::async_trait]
impl VisionGenerator for OpenAiClient {
    async fn generate_multimodal(&self, request: GenerateMultimodalRequest) -> Option<String> {
        let model = request.model.unwrap_or_else(|| self.model.clone());
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
        let images: Vec<String> = request
            .image_urls
            .iter()
            .filter(|url| VALID_IMAGE_PREFIXES.iter().any(|p| url.starts_with(p)))
            .take(MAX_MULTIMODAL_IMAGES)
            .cloned()
            .collect();

        let mut content = vec![json!({ "type": "text", "text": request.prompt })];
        for image_url in images {
            if let Some(data_uri) = Self::image_data_uri(&self.http, &image_url).await {
                content.push(json!({
                    "type": "image_url",
                    "image_url": { "url": data_uri },
                }));
            }
        }
        if content.len() == 1 {
            return None;
        }

        let mut messages = Vec::new();
        if let Some(system) = request.system_prompt {
            messages.push(json!({ "role": "system", "content": system }));
        }
        messages.push(json!({ "role": "user", "content": content }));

        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({
                "model": model,
                "messages": messages,
                "max_completion_tokens": max_tokens,
            }))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(%err, "OpenAI-Vision-Request fehlgeschlagen");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "OpenAI-Vision-API-Fehler");
            return None;
        }
        let data: Value = response.json().await.ok()?;
        let text = Self::extract_openai_text(&data)?;
        Some(Self::normalize_scam_json(&text).unwrap_or(text))
    }
}

#[async_trait::async_trait]
impl TextGenerator for OpenAiClient {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String> {
        let model = request.model.unwrap_or_else(|| self.model.clone());
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
        let response = self
            .http
            .post(format!("{}/responses", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({
                "model": model,
                "input": request.prompt,
                "instructions": request.system_prompt,
                "max_output_tokens": max_tokens,
                "temperature": request.temperature,
            }))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(%err, "OpenAI-Text-Request fehlgeschlagen");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "OpenAI-Text-API-Fehler");
            return None;
        }
        let data: Value = response.json().await.ok()?;
        Self::extract_openai_text(&data)
    }
}

#[async_trait::async_trait]
impl TextGenerator for GeminiClient {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String> {
        let model = request.model.unwrap_or_else(|| self.model.clone());
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
        let contents = match request.system_prompt {
            Some(system) if !system.trim().is_empty() => format!("{system}\n\n{}", request.prompt),
            _ => request.prompt,
        };
        let response = self
            .http
            .post(format!(
                "{}/models/{}:generateContent?key={}",
                self.base_url, model, self.api_key
            ))
            .json(&json!({
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": contents }],
                }],
                "generationConfig": {
                    "temperature": request.temperature,
                    "maxOutputTokens": max_tokens,
                },
            }))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(%err, "Gemini-Request fehlgeschlagen");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Gemini-API-Fehler");
            return None;
        }
        let data: Value = response.json().await.ok()?;
        let mut fragments = Vec::new();
        for candidate in data
            .get("candidates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for part in candidate
                .get("content")
                .and_then(|content| content.get("parts"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    fragments.push(text);
                }
            }
        }
        let text = fragments.concat().trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

#[async_trait::async_trait]
impl TextGenerator for MiniMaxClient {
    async fn generate_text(&self, request: GenerateRequest) -> Option<String> {
        let model = Self::normalize_model(request.model.unwrap_or_else(|| self.model.clone()));
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
                    "messages": [{
                        "role": "user",
                        "content": Self::build_token_plan_content(&request.prompt, &[]),
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
            Self::extract_token_plan_text(&data)
        } else {
            Self::extract_standard_text(&data)
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

    fn normalize_model(model: String) -> String {
        if model == "MiniMax-Text-01" {
            DEFAULT_MODEL.to_string()
        } else {
            model
        }
    }

    fn extract_token_plan_text(data: &Value) -> Option<String> {
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
        let text = fragments.concat().trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn extract_standard_text(data: &Value) -> Option<String> {
        data.get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

#[async_trait::async_trait]
impl ToolTextGenerator for MiniMaxClient {
    async fn generate_text_with_tools(
        &self,
        request: ToolUseRequest,
        tool_executor: Arc<dyn ToolExecutor>,
    ) -> ToolGeneration {
        if !self.token_plan {
            return ToolGeneration::default();
        }

        let model = Self::normalize_model(request.model.unwrap_or_else(|| self.model.clone()));
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
        let mut messages = vec![json!({
            "role": "user",
            "content": Self::build_token_plan_content(&request.prompt, &[]),
        })];
        let mut used_tool_names = Vec::new();
        let mut tool_rounds = 0usize;

        loop {
            let budget_left = tool_rounds < request.max_tool_calls;
            let mut payload = serde_json::Map::new();
            payload.insert("model".into(), json!(model));
            payload.insert(
                "system".into(),
                json!(request.system_prompt.clone().unwrap_or_default()),
            );
            payload.insert("messages".into(), Value::Array(messages.clone()));
            payload.insert("max_tokens".into(), json!(max_tokens));
            payload.insert("temperature".into(), json!(request.temperature));
            if budget_left && !request.tools.is_empty() {
                payload.insert(
                    "tools".into(),
                    Value::Array(
                        request
                            .tools
                            .iter()
                            .map(ToolDefinition::as_wire_value)
                            .collect(),
                    ),
                );
            }

            let response = self
                .http
                .post(format!("{}/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&Value::Object(payload))
                .send()
                .await;
            let response = match response {
                Ok(response) => response,
                Err(err) => {
                    tracing::warn!(%err, "MiniMax-Tool-Loop-Request fehlgeschlagen");
                    return ToolGeneration {
                        text: None,
                        tool_calls: used_tool_names,
                    };
                }
            };
            if !response.status().is_success() {
                tracing::warn!(status = %response.status(), "MiniMax-Tool-Loop-API-Fehler");
                return ToolGeneration {
                    text: None,
                    tool_calls: used_tool_names,
                };
            }
            let Ok(data) = response.json::<Value>().await else {
                return ToolGeneration {
                    text: None,
                    tool_calls: used_tool_names,
                };
            };
            let content_blocks = data
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            if data.get("stop_reason").and_then(Value::as_str) == Some("tool_use") && budget_left {
                let mut tool_result_blocks = Vec::new();
                for block in &content_blocks {
                    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let tool_name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let tool_input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    let result = match tool_executor.execute(tool_name, &tool_input).await {
                        Ok(result) => result,
                        Err(err) => {
                            tracing::warn!(%tool_name, %err, "MiniMax-Tool-Executor fehlgeschlagen");
                            json!({ "error": "tool_execution_failed" })
                        }
                    };
                    let content = serde_json::to_string(&result).unwrap_or_else(|_| {
                        "{\"error\":\"tool_result_serialization_failed\"}".to_string()
                    });
                    used_tool_names.push(tool_name.to_string());
                    tool_result_blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": block.get("id").cloned().unwrap_or(Value::Null),
                        "content": content,
                    }));
                }

                if tool_result_blocks.is_empty() {
                    return ToolGeneration {
                        text: Self::extract_token_plan_text(&data),
                        tool_calls: used_tool_names,
                    };
                }

                messages
                    .push(json!({ "role": "assistant", "content": Value::Array(content_blocks) }));
                messages
                    .push(json!({ "role": "user", "content": Value::Array(tool_result_blocks) }));
                tool_rounds += 1;
                continue;
            }

            return ToolGeneration {
                text: Self::extract_token_plan_text(&data),
                tool_calls: used_tool_names,
            };
        }
    }
}

#[async_trait::async_trait]
impl VisionGenerator for MiniMaxClient {
    async fn generate_multimodal(&self, request: GenerateMultimodalRequest) -> Option<String> {
        let model = Self::normalize_model(request.model.unwrap_or_else(|| self.model.clone()));
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
    fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
        haystack
            .as_bytes()
            .windows(needle.len())
            .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
    }

    let mut rest = text;
    let mut out = String::with_capacity(text.len());
    while let Some(start) = find_ascii_case_insensitive(rest, "<think>") {
        out.push_str(&rest[..start]);
        let after_open = start + "<think>".len();
        let Some(end_rel) = find_ascii_case_insensitive(&rest[after_open..], "</think>") else {
            rest = "";
            break;
        };
        rest = &rest[after_open + end_rel + "</think>".len()..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

// ── Matcher-Scoring (schließt die Phase-3c-Lücke) ──────────────────────────

/// AiScorer für den Streamer-Link-Matcher — Prompt wortgleich zum Original
/// (temperature 0, JSON-only-System-Prompt).
///
/// Token-Budget: 160 stammt aus der Python-Zeit. Das aktuelle Modell
/// ([`DEFAULT_FIREWORKS_MODEL`], der undatierte Name war unter diesem Konto
/// nicht ansprechbar) schreibt eine ausführliche `reason`-Begründung und
/// braucht für einen Standardfall gemessene 222 Completion-Tokens; bei 160
/// bricht die Antwort mitten im JSON ab, kommt mit `finish_reason=length`
/// zurück und der Provider meldet "response truncated (max_tokens)" — der
/// Aufruf fällt dann auf die reine String-Heuristik zurück.
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
                max_output_tokens: Some(DEFAULT_MAX_OUTPUT_TOKENS),
                reasoning_effort: None,
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
                reasoning_effort: None,
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
                reasoning_effort: None,
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
        assert_eq!(
            std_content[1]["image_url"]["url"],
            "data:image/jpeg;base64,QUJD"
        );
    }

    #[test]
    fn openai_text_from_env_nutzt_ai_openai_model_oder_python_fallback() {
        let explicit = OpenAiClient::text_from_env(|key| match key {
            "OPENAI_API_KEY" => Some("key".to_string()),
            "AI_OPENAI_MODEL" => Some("gpt-custom".to_string()),
            _ => None,
        })
        .expect("explicit text client");
        assert_eq!(explicit.model, "gpt-custom");

        let fallback = OpenAiClient::text_from_env(|key| match key {
            "OPENAI_API_KEY" => Some("key".to_string()),
            _ => None,
        })
        .expect("fallback text client");
        assert_eq!(fallback.model, "gpt-4o-mini");

        let legacy_openai_model_ignored = OpenAiClient::text_from_env(|key| match key {
            "OPENAI_API_KEY" => Some("key".to_string()),
            "OPENAI_MODEL" => Some("legacy-image-model".to_string()),
            _ => None,
        })
        .expect("legacy text client");
        assert_eq!(legacy_openai_model_ignored.model, "gpt-4o-mini");
    }

    #[test]
    fn fireworks_from_env_nutzt_singular_key_und_deepseek_flash_default() {
        let client = FireworksClient::from_env(|key| match key {
            "FIREWORK_API_KEY" => Some("fw-key".to_string()),
            _ => None,
        })
        .expect("fireworks client");

        assert_eq!(client.model, DEFAULT_FIREWORKS_MODEL);
        assert_eq!(client.base_url, DEFAULT_FIREWORKS_BASE_URL);
    }

    #[test]
    fn fireworks_from_env_ersetzt_veralteten_modellalias() {
        let client = FireworksClient::from_env(|key| match key {
            "FIREWORK_API_KEY" => Some("fw-key".to_string()),
            "FIREWORK_MODEL" => Some("accounts/fireworks/models/deepseek-v4-flash".into()),
            _ => None,
        })
        .expect("fireworks client");

        assert_eq!(client.model, DEFAULT_FIREWORKS_MODEL);
    }

    #[tokio::test]
    async fn bounded_reads_reject_unknown_length_and_keep_small_images() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn response(body: Vec<u8>) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("addr");
            tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                for chunk in body.chunks(4096) {
                    if stream.write_all(chunk).await.is_err() {
                        break;
                    }
                }
            });
            format!("http://{addr}/image.png")
        }
        let client = FireworksClient::http_client().expect("HTTP client");
        let url = response(vec![1; 1024]).await;
        let raw = client.get(url).send().await.expect("response");
        assert_eq!(raw.content_length(), None);
        assert!(bounded_response(raw, 512).await.is_none());
        let url = response(vec![1; MAX_OPENAI_IMAGE_BYTES as usize + 1]).await;
        assert!(OpenAiClient::image_data_uri(&client, &url).await.is_none());
        let url = response(vec![1, 2, 3]).await;
        assert_eq!(
            OpenAiClient::image_data_uri(&client, &url).await.as_deref(),
            Some("data:image/png;base64,AQID")
        );
        let url = response(Vec::new()).await;
        assert!(OpenAiClient::image_data_uri(&client, &url).await.is_none());
    }

    #[test]
    fn vision_urls_allow_only_exact_https_discord_cdn_or_data() {
        assert!(trusted_image_url(
            "https://cdn.discordapp.com/attachments/1/a.png?ex=123"
        ));
        assert!(trusted_image_url(
            "https://media.discordapp.net/attachments/1/a.png"
        ));
        assert!(trusted_image_url("data:image/png;base64,AQID"));
        for url in [
            "http://cdn.discordapp.com/a.png",
            "https://cdn.discordapp.com.evil.test/a.png",
            "https://localhost/a.png",
            "https://127.0.0.1/a.png",
            "https://user@cdn.discordapp.com/a.png",
            "https://cdn.discordapp.com:444/a.png",
        ] {
            assert!(!trusted_image_url(url), "{url}");
        }
    }

    #[tokio::test]
    async fn fireworks_transport_rejects_redirect_and_oversized_api_body() {
        use axum::{body::Body, http::Response, routing::post, Router};
        let app = Router::new()
            .route(
                "/redirect/chat/completions",
                post(|| async {
                    Response::builder()
                        .status(302)
                        .header("location", "/target")
                        .body(Body::empty())
                        .expect("redirect")
                }),
            )
            .route(
                "/large/chat/completions",
                post(|| async {
                    Response::builder()
                        .body(Body::from(vec![b' '; MAX_API_RESPONSE_BYTES as usize + 1]))
                        .expect("large")
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        for route in ["redirect", "large"] {
            let client = FireworksClient::new(
                format!("http://{addr}/{route}"),
                "synthetic-key",
                DEFAULT_FIREWORKS_MODEL,
            )
            .expect("HTTP client");
            assert!(client
                .generate_multimodal(GenerateMultimodalRequest {
                    prompt: "Prüfen".into(),
                    image_urls: vec!["data:image/png;base64,AQID".into()],
                    system_prompt: None,
                    model: None,
                    max_output_tokens: Some(30),
                    temperature: 0.0,
                })
                .await
                .is_none());
        }
    }

    #[tokio::test]
    async fn fireworks_vision_keeps_images_json_and_disables_reasoning() {
        use axum::{routing::post, Json, Router};
        let captured = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let cap = captured.clone();
        let app = Router::new().route("/chat/completions", post(move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
            let cap = cap.clone();
            async move {
                assert_eq!(headers.get("authorization").and_then(|v| v.to_str().ok()), Some("Bearer synthetic-key"));
                cap.lock().expect("capture").push(body);
                Json(json!({"choices": [{"finish_reason": "stop", "message": {"content": "{\"is_scam\":false,\"confidence\":0.9,\"reason\":\"klar\"}"}}]}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        let client = FireworksClient::new(
            format!("http://{addr}"),
            "synthetic-key",
            DEFAULT_FIREWORKS_MODEL,
        )
        .expect("HTTP client");
        let image = "data:image/png;base64,c3ludGhldGlj";
        let response = client
            .generate_multimodal(GenerateMultimodalRequest {
                prompt: "Bild prüfen".into(),
                image_urls: vec![image.into()],
                system_prompt: Some("JSON".into()),
                model: None,
                max_output_tokens: Some(300),
                temperature: 0.0,
            })
            .await
            .expect("Bildantwort");
        assert!(response.contains("klar"));
        let requests = captured.lock().expect("capture");
        assert_eq!(requests.len(), 1);
        let body = &requests[0];
        assert_eq!(body["model"], DEFAULT_FIREWORKS_MODEL);
        assert_eq!(body["max_tokens"], 300);
        assert_eq!(body["reasoning_effort"], "none");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["messages"][1]["content"][1]["image_url"]["url"], image);
        assert!(FireworksClient::complete_text(
            &json!({"choices":[{"finish_reason":"length", "message":{"content":"partial"}}]})
        )
        .is_none());
        assert!(
            FireworksClient::complete_text(&json!({"choices":[{"message":{"content":""}}]}))
                .is_none()
        );
    }

    #[tokio::test]
    async fn fireworks_chat_completion_wire_format() {
        use axum::{routing::post, Json, Router};
        let captured: Arc<std::sync::Mutex<Vec<Value>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let cap = cap.clone();
                    async move {
                        assert_eq!(
                            headers.get("authorization").and_then(|v| v.to_str().ok()),
                            Some("Bearer fw-key")
                        );
                        cap.lock().expect("lock").push(body);
                        Json(json!({ "choices": [
                            { "message": { "content": "{\"category\":\"game_related_ok\"}" } }
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
        let client =
            FireworksClient::new(&base, "fw-key", DEFAULT_FIREWORKS_MODEL).expect("HTTP client");

        let text = client
            .generate_text(GenerateRequest {
                prompt: "pruefe".to_string(),
                system_prompt: Some("system".to_string()),
                model: None,
                max_output_tokens: Some(300),
                reasoning_effort: Some("none".to_string()),
                temperature: 0.0,
            })
            .await;

        let normal_text = client
            .generate_text(GenerateRequest {
                prompt: "normal".to_string(),
                system_prompt: None,
                model: None,
                max_output_tokens: Some(300),
                reasoning_effort: None,
                temperature: 0.0,
            })
            .await;

        assert_eq!(text.as_deref(), Some("{\"category\":\"game_related_ok\"}"));
        assert_eq!(normal_text.as_deref(), text.as_deref());
        assert!(client
            .generate_text(GenerateRequest {
                prompt: "Explizites Denken".into(),
                system_prompt: None,
                model: None,
                max_output_tokens: Some(100),
                reasoning_effort: Some("high".into()),
                temperature: 0.0,
            })
            .await
            .is_some());
        let captured = captured.lock().expect("lock");
        assert_eq!(captured[0]["model"], DEFAULT_FIREWORKS_MODEL);
        assert_eq!(captured[0]["messages"][0]["role"], "system");
        assert_eq!(captured[0]["messages"][1]["content"], "pruefe");
        assert_eq!(captured[0]["max_tokens"], 300);
        assert_eq!(captured[0]["temperature"], 0.0);
        assert_eq!(captured[0]["response_format"]["type"], "json_object");
        assert_eq!(captured[0]["reasoning_effort"], "none");
        assert_eq!(captured[1]["reasoning_effort"], "none");
        assert_eq!(captured[2]["reasoning_effort"], "high");
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

    struct EchoToolExecutor;

    #[async_trait::async_trait]
    impl ToolExecutor for EchoToolExecutor {
        async fn execute(&self, tool_name: &str, tool_input: &Value) -> Result<Value, String> {
            Ok(json!({
                "tool": tool_name,
                "input": tool_input,
                "status": "ok",
            }))
        }
    }

    fn diagnose_tool() -> ToolDefinition {
        ToolDefinition {
            name: "twitch_diagnose".to_string(),
            description: "Diagnose".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
        }
    }

    #[tokio::test]
    async fn tool_loop_sendet_tool_result_turn_und_liefert_finalen_text() {
        use axum::{routing::post, Json, Router};
        let captured: Arc<std::sync::Mutex<Vec<Value>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
        let app = Router::new().route(
            "/messages",
            post(move |Json(body): Json<Value>| {
                let cap = cap.clone();
                async move {
                    let mut captured = cap.lock().expect("lock");
                    let idx = captured.len();
                    captured.push(body);
                    drop(captured);
                    if idx == 0 {
                        Json(json!({
                            "stop_reason": "tool_use",
                            "content": [
                                { "type": "text", "text": "Ich prüfe das." },
                                {
                                    "type": "tool_use",
                                    "id": "toolu_1",
                                    "name": "twitch_diagnose",
                                    "input": {}
                                }
                            ]
                        }))
                    } else {
                        Json(json!({
                            "stop_reason": "end_turn",
                            "content": [{ "type": "text", "text": "Fertig" }]
                        }))
                    }
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
        let result = client
            .generate_text_with_tools(
                ToolUseRequest {
                    prompt: "Ticket".to_string(),
                    system_prompt: Some("System".to_string()),
                    model: Some("MiniMax-Text-01".to_string()),
                    max_output_tokens: Some(200),
                    temperature: 0.2,
                    tools: vec![diagnose_tool()],
                    max_tool_calls: 4,
                },
                Arc::new(EchoToolExecutor),
            )
            .await;

        assert_eq!(result.text.as_deref(), Some("Fertig"));
        assert_eq!(result.tool_calls, vec!["twitch_diagnose"]);
        let captured = captured.lock().expect("lock");
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0]["model"], "MiniMax-M3");
        assert_eq!(captured[0]["tools"][0]["name"], "twitch_diagnose");
        assert_eq!(captured[1]["messages"][1]["role"], "assistant");
        assert_eq!(captured[1]["messages"][2]["role"], "user");
        assert_eq!(
            captured[1]["messages"][2]["content"][0]["type"],
            "tool_result"
        );
        assert_eq!(
            captured[1]["messages"][2]["content"][0]["tool_use_id"],
            "toolu_1"
        );
        let content = captured[1]["messages"][2]["content"][0]["content"]
            .as_str()
            .expect("tool result string");
        assert!(content.contains("\"status\":\"ok\""));
    }

    #[tokio::test]
    async fn tool_loop_stoppt_nach_max_tool_calls_und_bietet_keine_tools_mehr_an() {
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
                    Json(json!({
                        "stop_reason": "tool_use",
                        "content": [{
                            "type": "tool_use",
                            "id": "toolu_repeat",
                            "name": "twitch_diagnose",
                            "input": {}
                        }]
                    }))
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
        let result = client
            .generate_text_with_tools(
                ToolUseRequest {
                    prompt: "Ticket".to_string(),
                    system_prompt: None,
                    model: None,
                    max_output_tokens: Some(200),
                    temperature: 0.2,
                    tools: vec![diagnose_tool()],
                    max_tool_calls: 2,
                },
                Arc::new(EchoToolExecutor),
            )
            .await;

        assert_eq!(result.text, None);
        assert_eq!(result.tool_calls.len(), 2);
        let captured = captured.lock().expect("lock");
        assert_eq!(captured.len(), 3);
        assert!(captured[0].get("tools").is_some());
        assert!(captured[1].get("tools").is_some());
        assert!(captured[2].get("tools").is_none());
    }
}
