use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::time::sleep;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const DEFAULT_MAX_RETRIES: usize = 2;
const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_OPENAI_CHAT_MODEL: &str = "gpt-5.4-mini";
const DEFAULT_FIREWORKS_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
const DEFAULT_MINIMAX_BASE_URL: &str = "https://api.minimax.chat/v1";
const DEFAULT_MINIMAX_TOKEN_PLAN_BASE_URL: &str = "https://api.minimax.io/anthropic/v1";
const DEFAULT_MINIMAX_MODEL: &str = "MiniMax-M3";
const DEFAULT_MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";
pub const DEFAULT_MISTRAL_MODEL: &str = "mistral-small-2603";
const DEFAULT_CHAT_MAX_TOKENS: u32 = 800;
const MINIMAX_USER_CONTENT_DEV_ESCAPE_ENV: &str = "DL_LLM_ALLOW_MINIMAX_USER_CONTENT_DEV_ONLY";
const MINIMAX_USER_CONTENT_DEV_ESCAPE_VALUE: &str = "ich-weiss-was-ich-tue";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

impl ChatRole {
    fn as_openai_role(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    fn as_anthropic_role(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::User => Some("user"),
            Self::Assistant => Some("assistant"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatParams {
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub json_mode: bool,
    pub temperature: f64,
    pub system_prompt: Option<String>,
}

impl Default for ChatParams {
    fn default() -> Self {
        Self {
            model: None,
            max_tokens: Some(DEFAULT_CHAT_MAX_TOKENS),
            json_mode: false,
            temperature: 0.2,
            system_prompt: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatResponse {
    pub content: String,
    pub model: Option<String>,
    pub usage: TokenUsage,
}

impl ChatResponse {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            model: None,
            usage: TokenUsage::default(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ChatProviderError {
    #[error("LLM provider request timed out")]
    Timeout,
    #[error("LLM provider rate limited")]
    RateLimit,
    #[error("LLM provider authentication failed")]
    Auth,
    #[error("LLM provider error: {0}")]
    Provider(String),
}

#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmUseCase {
    BotPate,
    CockpitVorschlag,
    Faq,
    LfgFreitext,
    ScrimLagebild,
}

impl LlmUseCase {
    pub fn all() -> &'static [Self] {
        &[
            Self::BotPate,
            Self::CockpitVorschlag,
            Self::Faq,
            Self::LfgFreitext,
            Self::ScrimLagebild,
        ]
    }

    pub fn env_suffix(self) -> &'static str {
        match self {
            Self::BotPate => "BOT_PATE",
            Self::CockpitVorschlag => "COCKPIT_VORSCHLAG",
            Self::Faq => "FAQ",
            Self::LfgFreitext => "LFG_FREITEXT",
            Self::ScrimLagebild => "SCRIM_LAGEBILD",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BotPate => "bot_pate",
            Self::CockpitVorschlag => "cockpit_vorschlag",
            Self::Faq => "faq",
            Self::LfgFreitext => "lfg_freitext",
            Self::ScrimLagebild => "scrim_lagebild",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmDataClass {
    UserContent,
    Synthetic,
}

impl LlmDataClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserContent => "user_content",
            Self::Synthetic => "synthetic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmProviderKind {
    OpenAi,
    Fireworks,
    MiniMax,
    Mistral,
    Mock,
}

impl LlmProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Fireworks => "fireworks",
            Self::MiniMax => "minimax",
            Self::Mistral => "mistral",
            Self::Mock => "mock",
        }
    }
}

impl std::str::FromStr for LlmProviderKind {
    type Err = LlmProviderConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_provider_name(value).as_str() {
            "openai" => Ok(Self::OpenAi),
            "fireworks" => Ok(Self::Fireworks),
            "minimax" => Ok(Self::MiniMax),
            "mistral" => Ok(Self::Mistral),
            "mock" => Ok(Self::Mock),
            _ => Err(LlmProviderConfigError::UnknownProvider(value.to_string())),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LlmProviderConfigError {
    #[error("unknown LLM provider '{0}'")]
    UnknownProvider(String),
    #[error("LLM compliance violation: provider {provider:?} is blocked for {data_class:?} use case {use_case:?}")]
    ComplianceViolation {
        use_case: LlmUseCase,
        data_class: LlmDataClass,
        provider: LlmProviderKind,
    },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ChatProviderInitError {
    #[error("missing required env var {env_key} for {provider}")]
    MissingApiKey {
        provider: &'static str,
        env_key: &'static str,
    },
    #[error("mock provider must be injected by tests")]
    MockProviderMustBeInjected,
    #[error(transparent)]
    Config(#[from] LlmProviderConfigError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProviderConfig {
    providers: HashMap<LlmUseCase, LlmProviderKind>,
    data_classes: HashMap<LlmUseCase, LlmDataClass>,
}

impl Default for LlmProviderConfig {
    fn default() -> Self {
        let providers = LlmUseCase::all()
            .iter()
            .map(|use_case| (*use_case, default_provider_for(*use_case)))
            .collect();
        Self {
            providers,
            data_classes: default_data_classes(),
        }
    }
}

impl LlmProviderConfig {
    pub fn from_env(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, LlmProviderConfigError> {
        let default_override = read_env(&lookup, "DL_LLM_PROVIDER_DEFAULT")
            .map(|value| value.parse::<LlmProviderKind>())
            .transpose()?;
        let mut providers = HashMap::new();
        for use_case in LlmUseCase::all() {
            let key = format!("DL_LLM_PROVIDER_{}", use_case.env_suffix());
            let provider = read_env(&lookup, &key)
                .map(|value| value.parse::<LlmProviderKind>())
                .transpose()?
                .or(default_override)
                .unwrap_or_else(|| default_provider_for(*use_case));
            providers.insert(*use_case, provider);
        }
        let config = Self {
            providers,
            data_classes: default_data_classes(),
        };
        config.validate_compliance(&lookup)?;
        Ok(config)
    }

    pub fn provider_for(
        &self,
        use_case: LlmUseCase,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<LlmProviderKind, LlmProviderConfigError> {
        let provider = self.configured_provider_for(use_case);
        self.enforce_compliance(use_case, provider, &lookup)?;
        Ok(provider)
    }

    fn configured_provider_for(&self, use_case: LlmUseCase) -> LlmProviderKind {
        self.providers
            .get(&use_case)
            .copied()
            .unwrap_or_else(|| default_provider_for(use_case))
    }

    fn data_class_for(&self, use_case: LlmUseCase) -> LlmDataClass {
        self.data_classes
            .get(&use_case)
            .copied()
            .unwrap_or_else(|| default_data_class_for(use_case))
    }

    fn validate_compliance(
        &self,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<(), LlmProviderConfigError> {
        for use_case in LlmUseCase::all() {
            self.enforce_compliance(*use_case, self.configured_provider_for(*use_case), &lookup)?;
        }
        Ok(())
    }

    fn enforce_compliance(
        &self,
        use_case: LlmUseCase,
        provider: LlmProviderKind,
        lookup: &impl Fn(&str) -> Option<String>,
    ) -> Result<(), LlmProviderConfigError> {
        let data_class = self.data_class_for(use_case);
        if provider != LlmProviderKind::MiniMax || data_class != LlmDataClass::UserContent {
            return Ok(());
        }
        if minimax_user_content_dev_escape_enabled(lookup) {
            tracing::warn!(
                provider = provider.as_str(),
                use_case = use_case.as_str(),
                data_class = data_class.as_str(),
                escape_env = MINIMAX_USER_CONTENT_DEV_ESCAPE_ENV,
                "MiniMax fuer User-Content per Dev-Escape-Hatch erlaubt"
            );
            return Ok(());
        }
        Err(LlmProviderConfigError::ComplianceViolation {
            use_case,
            data_class,
            provider,
        })
    }

    pub fn build_provider_for_env(
        &self,
        use_case: LlmUseCase,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<dyn ChatProvider>, ChatProviderInitError> {
        match self.provider_for(use_case, &lookup)? {
            LlmProviderKind::OpenAi => OpenAiChatProvider::from_env(lookup)
                .map(|provider| provider as Arc<dyn ChatProvider>),
            LlmProviderKind::Fireworks => OpenAiChatProvider::from_fireworks_env(lookup)
                .map(|provider| provider as Arc<dyn ChatProvider>),
            LlmProviderKind::MiniMax => MiniMaxChatProvider::from_env(lookup)
                .map(|provider| provider as Arc<dyn ChatProvider>),
            LlmProviderKind::Mistral => MistralChatProvider::from_env(lookup)
                .map(|provider| provider as Arc<dyn ChatProvider>),
            LlmProviderKind::Mock => Err(ChatProviderInitError::MockProviderMustBeInjected),
        }
    }
}

// KI-Compliance-Gate (§5.6): MiniMax darf nur per explizitem Dev-Override und
// ausschliesslich mit synthetischen Daten genutzt werden.
fn default_provider_for(use_case: LlmUseCase) -> LlmProviderKind {
    match use_case {
        LlmUseCase::BotPate => LlmProviderKind::Fireworks,
        LlmUseCase::CockpitVorschlag
        | LlmUseCase::Faq
        | LlmUseCase::LfgFreitext
        | LlmUseCase::ScrimLagebild => LlmProviderKind::Mistral,
    }
}

fn default_data_classes() -> HashMap<LlmUseCase, LlmDataClass> {
    LlmUseCase::all()
        .iter()
        .map(|use_case| (*use_case, default_data_class_for(*use_case)))
        .collect()
}

fn default_data_class_for(_use_case: LlmUseCase) -> LlmDataClass {
    LlmDataClass::UserContent
}

fn minimax_user_content_dev_escape_enabled(lookup: &impl Fn(&str) -> Option<String>) -> bool {
    read_env(lookup, MINIMAX_USER_CONTENT_DEV_ESCAPE_ENV).as_deref()
        == Some(MINIMAX_USER_CONTENT_DEV_ESCAPE_VALUE)
}

fn normalize_provider_name(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn read_env(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub struct MistralChatProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    retry: RetryConfig,
}

pub struct OpenAiChatProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    retry: RetryConfig,
}

impl OpenAiChatProvider {
    pub fn from_env(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<Self>, ChatProviderInitError> {
        let api_key = read_env(&lookup, "OPENAI_API_KEY")
            .or_else(|| read_env(&lookup, "DEADLOCK_OPENAI_KEY"))
            .ok_or(ChatProviderInitError::MissingApiKey {
                provider: "openai",
                env_key: "OPENAI_API_KEY or DEADLOCK_OPENAI_KEY",
            })?;
        let base_url =
            read_env(&lookup, "OPENAI_BASE_URL").unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.into());
        let model = read_env(&lookup, "DL_LLM_MODEL_BOT_PATE")
            .or_else(|| read_env(&lookup, "OPENAI_MODEL"))
            .or_else(|| read_env(&lookup, "AI_OPENAI_MODEL"))
            .unwrap_or_else(|| DEFAULT_OPENAI_CHAT_MODEL.into());
        tracing::info!(provider = "openai", %model, "LLM-Chat-Provider initialisiert");
        Ok(Self::new(base_url, api_key, model))
    }

    pub fn from_fireworks_env(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<Self>, ChatProviderInitError> {
        let api_key = read_env(&lookup, "FIREWORK_API_KEY")
            .or_else(|| read_env(&lookup, "FIREWORKS_API_KEY"))
            .ok_or(ChatProviderInitError::MissingApiKey {
                provider: "fireworks",
                env_key: "FIREWORK_API_KEY or FIREWORKS_API_KEY",
            })?;
        let base_url = read_env(&lookup, "FIREWORK_BASE_URL")
            .or_else(|| read_env(&lookup, "FIREWORKS_BASE_URL"))
            .unwrap_or_else(|| DEFAULT_FIREWORKS_BASE_URL.into());
        let model = read_env(&lookup, "DL_LLM_MODEL_BOT_PATE")
            .or_else(|| read_env(&lookup, "FIREWORK_MODEL"))
            .or_else(|| read_env(&lookup, "FIREWORKS_MODEL"))
            .unwrap_or_else(|| crate::DEFAULT_FIREWORKS_MODEL.into());
        tracing::info!(provider = "fireworks", %model, "LLM-Chat-Provider initialisiert");
        Ok(Self::new(base_url, api_key, model))
    }

    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Arc<Self> {
        Self::new_with_retry(base_url, api_key, default_model, RetryConfig::default())
    }

    pub fn new_with_retry(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
        retry: RetryConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            http: retry.http_client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            default_model: default_model.into(),
            retry,
        })
    }
}

#[async_trait]
impl ChatProvider for OpenAiChatProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        let model = params
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let wire_messages = openai_messages(messages, params.system_prompt.as_deref())?;
        let mut payload = json!({
            "model": model,
            "messages": wire_messages,
            "temperature": params.temperature,
        });
        // max_tokens: None = Feld weglassen, das Modell-Maximum gilt (uncapped)
        if let Some(max_tokens) = params.max_tokens {
            payload["max_tokens"] = json!(max_tokens);
        }
        if params.json_mode {
            payload["response_format"] = json!({ "type": "json_object" });
        }

        let url = format!("{}/chat/completions", self.base_url);
        let result = send_json_with_retry("openai", &self.retry, || {
            self.http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&payload)
        })
        .await?;
        let response = parse_openai_chat_response(&result.body)?;
        log_chat_success("openai", result.status, result.elapsed, &response.usage);
        Ok(response)
    }
}

impl MistralChatProvider {
    pub fn from_env(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<Self>, ChatProviderInitError> {
        let api_key =
            read_env(&lookup, "MISTRAL_API_KEY").ok_or(ChatProviderInitError::MissingApiKey {
                provider: "mistral",
                env_key: "MISTRAL_API_KEY",
            })?;
        let base_url = read_env(&lookup, "MISTRAL_BASE_URL")
            .unwrap_or_else(|| DEFAULT_MISTRAL_BASE_URL.into());
        let model =
            read_env(&lookup, "MISTRAL_MODEL").unwrap_or_else(|| DEFAULT_MISTRAL_MODEL.into());
        tracing::info!(provider = "mistral", %model, "LLM-Chat-Provider initialisiert");
        Ok(Self::new(base_url, api_key, model))
    }

    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Arc<Self> {
        Self::new_with_retry(base_url, api_key, default_model, RetryConfig::default())
    }

    pub fn new_with_retry(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
        retry: RetryConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            http: retry.http_client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            default_model: default_model.into(),
            retry,
        })
    }
}

#[async_trait]
impl ChatProvider for MistralChatProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        let model = params
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let wire_messages = openai_messages(messages, params.system_prompt.as_deref())?;
        let mut payload = json!({
            "model": model,
            "messages": wire_messages,
            "temperature": params.temperature,
        });
        payload["max_tokens"] = json!(params.max_tokens.unwrap_or(DEFAULT_CHAT_MAX_TOKENS));

        let url = format!("{}/chat/completions", self.base_url);
        let result = send_json_with_retry("mistral", &self.retry, || {
            self.http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&payload)
        })
        .await?;
        let response = parse_openai_chat_response(&result.body)?;
        log_chat_success("mistral", result.status, result.elapsed, &response.usage);
        Ok(response)
    }
}

pub struct MiniMaxChatProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    token_plan: bool,
    default_model: String,
    retry: RetryConfig,
}

impl MiniMaxChatProvider {
    pub fn from_env(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<Self>, ChatProviderInitError> {
        let token_plan_key = read_env(&lookup, "MINIMAX_TOKEN_PLAN_KEY");
        let api_key = token_plan_key
            .clone()
            .or_else(|| read_env(&lookup, "MINIMAX_API_KEY"))
            .or_else(|| read_env(&lookup, "MINMAX"))
            .ok_or(ChatProviderInitError::MissingApiKey {
                provider: "minimax",
                env_key: "MINIMAX_TOKEN_PLAN_KEY or MINIMAX_API_KEY",
            })?;
        let token_plan = token_plan_key.is_some();
        let base_url = if token_plan {
            read_env(&lookup, "MINIMAX_TOKEN_PLAN_BASE_URL")
                .unwrap_or_else(|| DEFAULT_MINIMAX_TOKEN_PLAN_BASE_URL.into())
        } else {
            read_env(&lookup, "MINIMAX_BASE_URL").unwrap_or_else(|| DEFAULT_MINIMAX_BASE_URL.into())
        };
        let model =
            read_env(&lookup, "MINIMAX_MODEL").unwrap_or_else(|| DEFAULT_MINIMAX_MODEL.into());
        tracing::info!(
            provider = "minimax",
            mode = if token_plan { "token_plan" } else { "standard" },
            %model,
            "LLM-Chat-Provider initialisiert"
        );
        Ok(Self::new(base_url, api_key, token_plan, model))
    }

    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        token_plan: bool,
        default_model: impl Into<String>,
    ) -> Arc<Self> {
        Self::new_with_retry(
            base_url,
            api_key,
            token_plan,
            default_model,
            RetryConfig::default(),
        )
    }

    pub fn new_with_retry(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        token_plan: bool,
        default_model: impl Into<String>,
        retry: RetryConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            http: retry.http_client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            token_plan,
            default_model: default_model.into(),
            retry,
        })
    }

    fn normalize_model(model: String) -> String {
        if model == "MiniMax-Text-01" {
            DEFAULT_MINIMAX_MODEL.to_string()
        } else {
            model
        }
    }
}

#[async_trait]
impl ChatProvider for MiniMaxChatProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        let model = Self::normalize_model(
            params
                .model
                .clone()
                .unwrap_or_else(|| self.default_model.clone()),
        );
        let max_tokens = params.max_tokens.unwrap_or(DEFAULT_CHAT_MAX_TOKENS);

        let (url, payload, auth_header) = if self.token_plan {
            let (system, wire_messages) =
                anthropic_messages(messages, params.system_prompt.as_deref())?;
            (
                format!("{}/messages", self.base_url),
                json!({
                    "model": model,
                    "system": system,
                    "messages": wire_messages,
                    "max_tokens": max_tokens,
                    "temperature": params.temperature,
                }),
                MiniMaxAuthHeader::TokenPlan,
            )
        } else {
            (
                format!("{}/text/chatcompletion_v2", self.base_url),
                json!({
                    "model": model,
                    "messages": openai_messages(messages, params.system_prompt.as_deref())?,
                    "max_tokens": max_tokens,
                    "temperature": params.temperature,
                }),
                MiniMaxAuthHeader::Standard,
            )
        };

        let result = send_json_with_retry("minimax", &self.retry, || {
            let request = self.http.post(&url).json(&payload);
            match auth_header {
                MiniMaxAuthHeader::TokenPlan => request
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01"),
                MiniMaxAuthHeader::Standard => {
                    request.header("Authorization", format!("Bearer {}", self.api_key))
                }
            }
        })
        .await?;

        let response = if self.token_plan {
            parse_anthropic_chat_response(&result.body)?
        } else {
            parse_openai_chat_response(&result.body)?
        };
        log_chat_success("minimax", result.status, result.elapsed, &response.usage);
        Ok(response)
    }
}

#[derive(Debug, Clone, Copy)]
enum MiniMaxAuthHeader {
    TokenPlan,
    Standard,
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub request_timeout: Duration,
    pub max_retries: usize,
    pub base_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
            base_backoff: DEFAULT_BACKOFF,
        }
    }
}

impl RetryConfig {
    fn http_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(self.request_timeout)
            .build()
            .unwrap_or_default()
    }

    fn backoff_for_attempt(&self, attempt: usize) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(8) as u32).unwrap_or(u32::MAX);
        self.base_backoff.saturating_mul(factor)
    }
}

struct HttpJsonResult {
    status: StatusCode,
    body: Value,
    elapsed: Duration,
}

async fn send_json_with_retry(
    provider: &'static str,
    retry: &RetryConfig,
    make_request: impl Fn() -> reqwest::RequestBuilder,
) -> Result<HttpJsonResult, ChatProviderError> {
    let started = Instant::now();
    for attempt in 0..=retry.max_retries {
        let response = make_request().send().await;
        let response = match response {
            Ok(response) => response,
            Err(err) if err.is_timeout() => {
                tracing::warn!(
                    provider = provider,
                    elapsed_ms = started.elapsed().as_millis(),
                    "LLM-Provider-Request Timeout"
                );
                return Err(ChatProviderError::Timeout);
            }
            Err(err) => {
                tracing::warn!(
                    provider = provider,
                    elapsed_ms = started.elapsed().as_millis(),
                    error_kind = %request_error_kind(&err),
                    "LLM-Provider-Request fehlgeschlagen"
                );
                return Err(ChatProviderError::Provider(request_error_kind(&err)));
            }
        };
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            tracing::warn!(
                provider = provider,
                status = status.as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                "LLM-Provider-Auth-Fehler"
            );
            return Err(ChatProviderError::Auth);
        }
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            if attempt < retry.max_retries {
                let backoff = retry.backoff_for_attempt(attempt);
                tracing::warn!(
                    provider = provider,
                    status = status.as_u16(),
                    attempt = attempt + 1,
                    retry_in_ms = backoff.as_millis(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "LLM-Provider-Request wird wiederholt"
                );
                sleep(backoff).await;
                continue;
            }
            tracing::warn!(
                provider = provider,
                status = status.as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                "LLM-Provider-Request nach Retries fehlgeschlagen"
            );
            return if status == StatusCode::TOO_MANY_REQUESTS {
                Err(ChatProviderError::RateLimit)
            } else {
                Err(ChatProviderError::Provider(format!(
                    "HTTP {}",
                    status.as_u16()
                )))
            };
        }
        if !status.is_success() {
            tracing::warn!(
                provider = provider,
                status = status.as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                "LLM-Provider-API-Fehler"
            );
            return Err(ChatProviderError::Provider(format!(
                "HTTP {}",
                status.as_u16()
            )));
        }
        let body = response
            .json::<Value>()
            .await
            .map_err(|_| ChatProviderError::Provider("invalid JSON response".to_string()))?;
        return Ok(HttpJsonResult {
            status,
            body,
            elapsed: started.elapsed(),
        });
    }

    Err(ChatProviderError::Provider(
        "retry loop exhausted unexpectedly".to_string(),
    ))
}

fn request_error_kind(err: &reqwest::Error) -> String {
    if err.is_connect() {
        "connect".to_string()
    } else if err.is_request() {
        "request".to_string()
    } else if err.is_body() {
        "body".to_string()
    } else if err.is_decode() {
        "decode".to_string()
    } else {
        "transport".to_string()
    }
}

fn log_chat_success(
    provider: &'static str,
    status: StatusCode,
    elapsed: Duration,
    usage: &TokenUsage,
) {
    tracing::info!(
        provider = provider,
        status = status.as_u16(),
        elapsed_ms = elapsed.as_millis(),
        prompt_tokens = usage.prompt_tokens,
        completion_tokens = usage.completion_tokens,
        total_tokens = usage.total_tokens,
        "LLM-Provider-Chat abgeschlossen"
    );
}

fn openai_messages(
    messages: &[ChatMessage],
    system_prompt: Option<&str>,
) -> Result<Vec<Value>, ChatProviderError> {
    let mut wire = Vec::new();
    if let Some(system_prompt) = non_empty(system_prompt) {
        wire.push(json!({ "role": "system", "content": system_prompt }));
    }
    for message in messages {
        let Some(content) = non_empty(Some(&message.content)) else {
            continue;
        };
        wire.push(json!({
            "role": message.role.as_openai_role(),
            "content": content,
        }));
    }
    if wire.is_empty() {
        return Err(ChatProviderError::Provider(
            "chat request requires at least one message".to_string(),
        ));
    }
    Ok(wire)
}

fn anthropic_messages(
    messages: &[ChatMessage],
    system_prompt: Option<&str>,
) -> Result<(String, Vec<Value>), ChatProviderError> {
    let mut system_parts = Vec::new();
    if let Some(system_prompt) = non_empty(system_prompt) {
        system_parts.push(system_prompt.to_string());
    }
    let mut wire = Vec::new();
    for message in messages {
        let Some(content) = non_empty(Some(&message.content)) else {
            continue;
        };
        if let Some(role) = message.role.as_anthropic_role() {
            wire.push(json!({
                "role": role,
                "content": [{ "type": "text", "text": content }],
            }));
        } else {
            system_parts.push(content.to_string());
        }
    }
    if wire.is_empty() {
        return Err(ChatProviderError::Provider(
            "chat request requires at least one user or assistant message".to_string(),
        ));
    }
    Ok((system_parts.join("\n\n"), wire))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_openai_chat_response(data: &Value) -> Result<ChatResponse, ChatProviderError> {
    let content = data
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(parse_openai_content)
        .ok_or_else(|| ChatProviderError::Provider("missing chat response content".to_string()))?;
    Ok(ChatResponse {
        content,
        model: data
            .get("model")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage: parse_usage(data),
    })
}

fn parse_openai_content(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => non_empty(Some(text)).map(ToString::to_string),
        Value::Array(items) => {
            let text = text_chunks(items).concat();
            non_empty(Some(&text)).map(ToString::to_string)
        }
        _ => None,
    }
}

fn text_chunks(items: &[Value]) -> Vec<&str> {
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect()
}

fn parse_anthropic_chat_response(data: &Value) -> Result<ChatResponse, ChatProviderError> {
    let fragments: Vec<&str> = data
        .get("content")
        .and_then(Value::as_array)
        .map(|items| text_chunks(items))
        .unwrap_or_default();
    let content = fragments.concat().trim().to_string();
    if content.is_empty() {
        return Err(ChatProviderError::Provider(
            "missing chat response content".to_string(),
        ));
    }
    Ok(ChatResponse {
        content,
        model: data
            .get("model")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage: parse_usage(data),
    })
}

fn parse_usage(data: &Value) -> TokenUsage {
    let usage = data.get("usage");
    TokenUsage {
        prompt_tokens: usage
            .and_then(|usage| usage.get("prompt_tokens"))
            .or_else(|| usage.and_then(|usage| usage.get("input_tokens")))
            .and_then(Value::as_u64),
        completion_tokens: usage
            .and_then(|usage| usage.get("completion_tokens"))
            .or_else(|| usage.and_then(|usage| usage.get("output_tokens")))
            .and_then(Value::as_u64),
        total_tokens: usage
            .and_then(|usage| usage.get("total_tokens"))
            .and_then(Value::as_u64),
    }
}

pub struct MockChatProvider {
    responses: Mutex<Vec<Result<ChatResponse, ChatProviderError>>>,
    requests: Mutex<Vec<(Vec<ChatMessage>, ChatParams)>>,
}

impl MockChatProvider {
    pub fn new(responses: Vec<Result<ChatResponse, ChatProviderError>>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub fn single(content: impl Into<String>) -> Arc<Self> {
        Self::new(vec![Ok(ChatResponse::text(content))])
    }

    pub fn requests(&self) -> Vec<(Vec<ChatMessage>, ChatParams)> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl ChatProvider for MockChatProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        self.requests
            .lock()
            .map_err(|_| ChatProviderError::Provider("mock request lock poisoned".to_string()))?
            .push((messages.to_vec(), params));
        let mut responses = self
            .responses
            .lock()
            .map_err(|_| ChatProviderError::Provider("mock response lock poisoned".to_string()))?;
        if responses.is_empty() {
            return Err(ChatProviderError::Provider(
                "mock response exhausted".to_string(),
            ));
        }
        responses.remove(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct LogCapture {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl LogCapture {
        fn text(&self) -> String {
            let bytes = self.bytes.lock().expect("log capture").clone();
            String::from_utf8_lossy(&bytes).to_string()
        }
    }

    struct LogCaptureWriter {
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

    fn fast_retry(max_retries: usize) -> RetryConfig {
        RetryConfig {
            request_timeout: Duration::from_secs(2),
            max_retries,
            base_backoff: Duration::from_millis(1),
        }
    }

    fn config_for_provider_and_data_class(
        use_case: LlmUseCase,
        provider: LlmProviderKind,
        data_class: LlmDataClass,
    ) -> LlmProviderConfig {
        let mut providers = std::collections::HashMap::new();
        providers.insert(use_case, provider);
        let mut data_classes = default_data_classes();
        data_classes.insert(use_case, data_class);
        LlmProviderConfig {
            providers,
            data_classes,
        }
    }

    async fn spawn_json_server(app: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        format!("http://{addr}")
    }

    #[test]
    fn config_default_ist_fireworks_fuer_bot_pate_und_env_override_pro_use_case() {
        let cfg = LlmProviderConfig::from_env(|key| match key {
            "DL_LLM_PROVIDER_DEFAULT" => Some("mistral".to_string()),
            "DL_LLM_PROVIDER_BOT_PATE" => Some("mock".to_string()),
            "DL_LLM_PROVIDER_FAQ" => Some("mock".to_string()),
            _ => None,
        })
        .expect("config");

        assert_eq!(
            cfg.provider_for(LlmUseCase::BotPate, |_| None)
                .expect("provider"),
            LlmProviderKind::Mock
        );
        assert_eq!(
            cfg.provider_for(LlmUseCase::CockpitVorschlag, |_| None)
                .expect("provider"),
            LlmProviderKind::Mistral
        );
        assert_eq!(
            cfg.provider_for(LlmUseCase::Faq, |_| None)
                .expect("provider"),
            LlmProviderKind::Mock
        );

        let defaults = LlmProviderConfig::default();
        for use_case in LlmUseCase::all() {
            let expected = match use_case {
                LlmUseCase::BotPate => LlmProviderKind::Fireworks,
                LlmUseCase::CockpitVorschlag
                | LlmUseCase::Faq
                | LlmUseCase::LfgFreitext
                | LlmUseCase::ScrimLagebild => LlmProviderKind::Mistral,
            };
            assert_eq!(
                defaults
                    .provider_for(*use_case, |_| None)
                    .expect("default provider"),
                expected
            );
            assert_eq!(
                defaults.data_class_for(*use_case),
                LlmDataClass::UserContent
            );
        }
    }

    #[test]
    fn lfg_freitext_hat_eigenen_user_content_provider_pfad() {
        let cfg = LlmProviderConfig::from_env(|key| {
            (key == "DL_LLM_PROVIDER_LFG_FREITEXT").then(|| "openai".to_string())
        })
        .expect("config");

        assert_eq!(
            cfg.provider_for(LlmUseCase::LfgFreitext, |_| None)
                .expect("provider"),
            LlmProviderKind::OpenAi
        );
        assert_eq!(
            cfg.data_class_for(LlmUseCase::LfgFreitext),
            LlmDataClass::UserContent
        );
    }

    #[test]
    fn fireworks_from_env_nutzt_singular_keys_und_provider_default() {
        let provider = OpenAiChatProvider::from_fireworks_env(|key| match key {
            "FIREWORK_API_KEY" => Some("fw-key".to_string()),
            _ => None,
        })
        .expect("fireworks provider");

        assert_eq!(provider.base_url, DEFAULT_FIREWORKS_BASE_URL);
        assert_eq!(provider.default_model, crate::DEFAULT_FIREWORKS_MODEL);
    }

    #[test]
    fn fireworks_from_env_nimmt_bot_pate_model_vor_fireworks_model() {
        let provider = OpenAiChatProvider::from_fireworks_env(|key| match key {
            "FIREWORKS_API_KEY" => Some("fw-key".to_string()),
            "DL_LLM_MODEL_BOT_PATE" => Some("bot-pate-model".to_string()),
            "FIREWORK_MODEL" => Some("fireworks-model".to_string()),
            _ => None,
        })
        .expect("fireworks provider");

        assert_eq!(provider.default_model, "bot-pate-model");
    }

    #[test]
    fn compliance_blockiert_minimax_fuer_user_content_use_case() {
        let err = LlmProviderConfig::from_env(|key| match key {
            "DL_LLM_PROVIDER_DEFAULT" => Some("minimax".to_string()),
            _ => None,
        })
        .expect_err("minimax must be blocked for user content");

        assert_eq!(
            err,
            LlmProviderConfigError::ComplianceViolation {
                use_case: LlmUseCase::BotPate,
                data_class: LlmDataClass::UserContent,
                provider: LlmProviderKind::MiniMax,
            }
        );
    }

    #[test]
    fn compliance_erlaubt_minimax_fuer_synthetische_daten() {
        let cfg = config_for_provider_and_data_class(
            LlmUseCase::Faq,
            LlmProviderKind::MiniMax,
            LlmDataClass::Synthetic,
        );

        assert_eq!(
            cfg.provider_for(LlmUseCase::Faq, |_| None)
                .expect("synthetic minimax"),
            LlmProviderKind::MiniMax
        );
    }

    #[test]
    fn compliance_dev_escape_erlaubt_minimax_user_content_mit_warnung() {
        fn lookup(key: &str) -> Option<String> {
            match key {
                "DL_LLM_PROVIDER_FAQ" => Some("minimax".to_string()),
                MINIMAX_USER_CONTENT_DEV_ESCAPE_ENV => {
                    Some(MINIMAX_USER_CONTENT_DEV_ESCAPE_VALUE.to_string())
                }
                _ => None,
            }
        }

        let log_capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(log_capture.clone())
            .with_ansi(false)
            .without_time()
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let cfg = LlmProviderConfig::from_env(lookup).expect("dev escape config");
        assert_eq!(
            cfg.provider_for(LlmUseCase::Faq, lookup)
                .expect("dev escape provider"),
            LlmProviderKind::MiniMax
        );
        drop(guard);

        let logs = log_capture.text();
        assert!(logs.contains("MiniMax fuer User-Content per Dev-Escape-Hatch erlaubt"));
        assert!(logs.contains(MINIMAX_USER_CONTENT_DEV_ESCAPE_ENV));
    }

    #[test]
    fn config_lehnt_unbekannten_provider_ab() {
        let err = LlmProviderConfig::from_env(|key| {
            (key == "DL_LLM_PROVIDER_FAQ").then(|| "unknown".to_string())
        })
        .expect_err("unknown provider");
        assert_eq!(
            err,
            LlmProviderConfigError::UnknownProvider("unknown".to_string())
        );
    }

    #[tokio::test]
    async fn openai_sendet_response_format_bei_json_mode() {
        use axum::{routing::post, Json, Router};
        let captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(move |Json(body): Json<Value>| {
                let cap = cap.clone();
                async move {
                    cap.lock().expect("lock").push(body);
                    Json(json!({
                        "model": "gpt-test",
                        "choices": [{ "message": { "content": "{}" } }]
                    }))
                }
            }),
        );
        let base = spawn_json_server(app).await;
        let provider =
            OpenAiChatProvider::new_with_retry(base, "openai-key", "gpt-test", fast_retry(0));

        provider
            .chat(
                &[ChatMessage::user("Hallo")],
                ChatParams {
                    json_mode: true,
                    ..ChatParams::default()
                },
            )
            .await
            .expect("chat response");
        provider
            .chat(
                &[ChatMessage::user("Hallo")],
                ChatParams {
                    json_mode: true,
                    max_tokens: None,
                    ..ChatParams::default()
                },
            )
            .await
            .expect("chat response");

        let captured = captured.lock().expect("lock");
        assert_eq!(captured[0]["response_format"]["type"], "json_object");
        assert!(captured[0].get("max_tokens").is_some());
        assert!(
            captured[1].get("max_tokens").is_none(),
            "max_tokens: None muss das Feld komplett weglassen (uncapped)"
        );
    }

    #[tokio::test]
    async fn mistral_sendet_openai_kompatibles_chat_completion_wire_format() {
        use axum::{routing::post, Json, Router};
        let captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let cap = cap.clone();
                    async move {
                        assert_eq!(
                            headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok()),
                            Some("Bearer mistral-key")
                        );
                        cap.lock().expect("lock").push(body);
                        Json(json!({
                            "model": "mistral-small-2603",
                            "choices": [{ "message": { "content": "Antwort" } }],
                            "usage": {
                                "prompt_tokens": 11,
                                "completion_tokens": 5,
                                "total_tokens": 16
                            }
                        }))
                    }
                },
            ),
        );
        let base = spawn_json_server(app).await;
        let provider = MistralChatProvider::new_with_retry(
            base,
            "mistral-key",
            DEFAULT_MISTRAL_MODEL,
            fast_retry(0),
        );

        let response = provider
            .chat(
                &[ChatMessage::user("Hallo")],
                ChatParams {
                    model: None,
                    max_tokens: Some(123),
                    json_mode: false,
                    temperature: 0.3,
                    system_prompt: Some("System".to_string()),
                },
            )
            .await
            .expect("chat response");

        assert_eq!(response.content, "Antwort");
        assert_eq!(response.model.as_deref(), Some("mistral-small-2603"));
        assert_eq!(response.usage.total_tokens, Some(16));
        let captured = captured.lock().expect("lock");
        assert_eq!(captured[0]["model"], DEFAULT_MISTRAL_MODEL);
        assert_eq!(captured[0]["max_tokens"], 123);
        assert_eq!(captured[0]["temperature"], 0.3);
        assert_eq!(captured[0]["messages"][0]["role"], "system");
        assert_eq!(captured[0]["messages"][0]["content"], "System");
        assert_eq!(captured[0]["messages"][1]["role"], "user");
        assert_eq!(captured[0]["messages"][1]["content"], "Hallo");
    }

    #[test]
    fn openai_response_parser_akzeptiert_string_und_chunk_content() {
        let string_response = parse_openai_chat_response(&json!({
            "model": "mistral-small-2603",
            "choices": [{ "message": { "content": " Antwort " } }],
        }))
        .expect("string content");
        assert_eq!(string_response.content, "Antwort");
        assert_eq!(string_response.model.as_deref(), Some("mistral-small-2603"));

        let chunk_response = parse_openai_chat_response(&json!({
            "choices": [{
                "message": {
                    "content": [
                        { "type": "text", "text": "Ant" },
                        { "type": "text", "text": "wort" }
                    ]
                }
            }]
        }))
        .expect("chunk content");
        assert_eq!(chunk_response.content, "Antwort");

        let mixed_response = parse_openai_chat_response(&json!({
            "choices": [{
                "message": {
                    "content": [
                        { "type": "image_url", "image_url": "https://example.invalid/a.png" },
                        { "type": "text", "text": "Gemischt" },
                        { "type": "tool_use", "text": "ignoriert" },
                        { "text": "auch ignoriert" }
                    ]
                }
            }]
        }))
        .expect("mixed content");
        assert_eq!(mixed_response.content, "Gemischt");
    }

    #[test]
    fn openai_response_parser_lehnt_null_und_leere_choices_sauber_ab() {
        for data in [
            json!({ "choices": [{ "message": { "content": null } }] }),
            json!({ "choices": [] }),
        ] {
            let err = parse_openai_chat_response(&data).expect_err("missing content");
            assert_eq!(
                err,
                ChatProviderError::Provider("missing chat response content".to_string())
            );
        }
    }

    #[tokio::test]
    async fn provider_retryt_429_begrenzt_ohne_prompt_logging() {
        use axum::{routing::post, Json, Router};
        let attempts = Arc::new(Mutex::new(0usize));
        let seen = attempts.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(move |Json(_body): Json<Value>| {
                let seen = seen.clone();
                async move {
                    let mut attempts = seen.lock().expect("lock");
                    *attempts += 1;
                    if *attempts == 1 {
                        (
                            axum::http::StatusCode::TOO_MANY_REQUESTS,
                            Json(json!({ "error": "rate_limit" })),
                        )
                    } else {
                        (
                            axum::http::StatusCode::OK,
                            Json(json!({
                                "choices": [{ "message": { "content": "ok" } }]
                            })),
                        )
                    }
                }
            }),
        );
        let base = spawn_json_server(app).await;
        let provider = MistralChatProvider::new_with_retry(base, "key", "model", fast_retry(2));

        let response = provider
            .chat(&[ChatMessage::user("synthetisch")], ChatParams::default())
            .await
            .expect("retry succeeds");

        assert_eq!(response.content, "ok");
        assert_eq!(*attempts.lock().expect("lock"), 2);
    }

    #[tokio::test]
    async fn provider_retryt_5xx_bis_zur_obergrenze() {
        use axum::{routing::post, Json, Router};
        let attempts = Arc::new(Mutex::new(0usize));
        let seen = attempts.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(move || {
                let seen = seen.clone();
                async move {
                    *seen.lock().expect("lock") += 1;
                    (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "server" })),
                    )
                }
            }),
        );
        let base = spawn_json_server(app).await;
        let provider = MistralChatProvider::new_with_retry(base, "key", "model", fast_retry(2));

        let err = provider
            .chat(&[ChatMessage::user("synthetisch")], ChatParams::default())
            .await
            .expect_err("server error");

        assert_eq!(err, ChatProviderError::Provider("HTTP 500".to_string()));
        assert_eq!(*attempts.lock().expect("lock"), 3);
    }

    #[tokio::test]
    async fn provider_retryt_400_und_401_nicht() {
        async fn error_for_status(status: axum::http::StatusCode) -> (ChatProviderError, usize) {
            use axum::{routing::post, Json, Router};
            let attempts = Arc::new(Mutex::new(0usize));
            let seen = attempts.clone();
            let app = Router::new().route(
                "/chat/completions",
                post(move || {
                    let seen = seen.clone();
                    async move {
                        *seen.lock().expect("lock") += 1;
                        (status, Json(json!({ "error": status.as_u16() })))
                    }
                }),
            );
            let base = spawn_json_server(app).await;
            let provider = MistralChatProvider::new_with_retry(base, "key", "model", fast_retry(2));
            let err = provider
                .chat(&[ChatMessage::user("synthetisch")], ChatParams::default())
                .await
                .expect_err("non-retryable error");
            let attempts = *attempts.lock().expect("lock");
            (err, attempts)
        }

        let (bad_request, bad_request_attempts) =
            error_for_status(axum::http::StatusCode::BAD_REQUEST).await;
        assert_eq!(
            bad_request,
            ChatProviderError::Provider("HTTP 400".to_string())
        );
        assert_eq!(bad_request_attempts, 1);

        let (auth, auth_attempts) = error_for_status(axum::http::StatusCode::UNAUTHORIZED).await;
        assert_eq!(auth, ChatProviderError::Auth);
        assert_eq!(auth_attempts, 1);
    }

    #[tokio::test]
    async fn auth_und_finales_rate_limit_werden_typisiert() {
        use axum::{routing::post, Json, Router};
        let auth_app = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "auth" })),
                )
            }),
        );
        let auth_base = spawn_json_server(auth_app).await;
        let auth_provider =
            MistralChatProvider::new_with_retry(auth_base, "key", "model", fast_retry(0));
        let err = auth_provider
            .chat(&[ChatMessage::user("synthetisch")], ChatParams::default())
            .await
            .expect_err("auth error");
        assert_eq!(err, ChatProviderError::Auth);

        let rate_app = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({ "error": "rate_limit" })),
                )
            }),
        );
        let rate_base = spawn_json_server(rate_app).await;
        let rate_provider =
            MistralChatProvider::new_with_retry(rate_base, "key", "model", fast_retry(0));
        let err = rate_provider
            .chat(&[ChatMessage::user("synthetisch")], ChatParams::default())
            .await
            .expect_err("rate limit");
        assert_eq!(err, ChatProviderError::RateLimit);
    }

    #[tokio::test]
    async fn minimax_standard_und_token_plan_wire_format() {
        use axum::{routing::post, Json, Router};
        let captured: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let standard_cap = captured.clone();
        let token_cap = captured.clone();
        let app = Router::new()
            .route(
                "/text/chatcompletion_v2",
                post(
                    move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                        let standard_cap = standard_cap.clone();
                        async move {
                            assert_eq!(
                                headers
                                    .get("authorization")
                                    .and_then(|value| value.to_str().ok()),
                                Some("Bearer std-key")
                            );
                            standard_cap
                                .lock()
                                .expect("lock")
                                .push(("standard".to_string(), body));
                            Json(json!({
                                "choices": [{ "message": { "content": "std" } }],
                                "usage": { "prompt_tokens": 1, "completion_tokens": 2 }
                            }))
                        }
                    },
                ),
            )
            .route(
                "/messages",
                post(
                    move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                        let token_cap = token_cap.clone();
                        async move {
                            assert_eq!(
                                headers
                                    .get("x-api-key")
                                    .and_then(|value| value.to_str().ok()),
                                Some("tp-key")
                            );
                            token_cap
                                .lock()
                                .expect("lock")
                                .push(("token".to_string(), body));
                            Json(json!({
                                "model": "MiniMax-M3",
                                "content": [{ "type": "text", "text": "tp" }],
                                "usage": { "input_tokens": 3, "output_tokens": 4 }
                            }))
                        }
                    },
                ),
            );
        let base = spawn_json_server(app).await;

        let standard = MiniMaxChatProvider::new_with_retry(
            &base,
            "std-key",
            false,
            "MiniMax-M3",
            fast_retry(0),
        );
        let standard_response = standard
            .chat(
                &[ChatMessage::user("Hallo")],
                ChatParams {
                    system_prompt: Some("System".to_string()),
                    ..ChatParams::default()
                },
            )
            .await
            .expect("standard");
        assert_eq!(standard_response.content, "std");

        let token = MiniMaxChatProvider::new_with_retry(
            &base,
            "tp-key",
            true,
            "MiniMax-Text-01",
            fast_retry(0),
        );
        let token_response = token
            .chat(
                &[
                    ChatMessage::system("zusatz"),
                    ChatMessage::user("Hallo"),
                    ChatMessage::assistant("Vorantwort"),
                ],
                ChatParams {
                    system_prompt: Some("System".to_string()),
                    max_tokens: Some(77),
                    ..ChatParams::default()
                },
            )
            .await
            .expect("token");
        assert_eq!(token_response.content, "tp");
        assert_eq!(token_response.usage.prompt_tokens, Some(3));

        let captured = captured.lock().expect("lock");
        let standard_body = &captured
            .iter()
            .find(|(kind, _)| kind == "standard")
            .expect("standard body")
            .1;
        assert_eq!(standard_body["messages"][0]["role"], "system");
        assert_eq!(standard_body["messages"][1]["content"], "Hallo");
        let token_body = &captured
            .iter()
            .find(|(kind, _)| kind == "token")
            .expect("token body")
            .1;
        assert_eq!(token_body["model"], "MiniMax-M3");
        assert_eq!(token_body["system"], "System\n\nzusatz");
        assert_eq!(token_body["max_tokens"], 77);
        assert_eq!(token_body["messages"][0]["role"], "user");
        assert_eq!(token_body["messages"][1]["role"], "assistant");
    }

    #[tokio::test]
    async fn mock_provider_liefert_responses_und_zeichnet_requests_auf() {
        let provider = MockChatProvider::single("mocked");
        let params = ChatParams {
            model: Some("test-model".to_string()),
            ..ChatParams::default()
        };
        let response = provider
            .chat(&[ChatMessage::user("synthetisch")], params.clone())
            .await
            .expect("mock response");

        assert_eq!(response.content, "mocked");
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, vec![ChatMessage::user("synthetisch")]);
        assert_eq!(requests[0].1, params);
    }

    #[test]
    fn factory_liest_env_ohne_mock_auto_build() {
        let cfg = LlmProviderConfig::from_env(|key| match key {
            "DL_LLM_PROVIDER_FAQ" => Some("mistral".to_string()),
            _ => None,
        })
        .expect("config");
        let provider = cfg
            .build_provider_for_env(LlmUseCase::Faq, |key| match key {
                "MISTRAL_API_KEY" => Some("secret".to_string()),
                _ => None,
            })
            .expect("mistral provider");
        drop(provider);

        let missing = match cfg.build_provider_for_env(LlmUseCase::Faq, |_key| None) {
            Ok(_) => panic!("expected missing api key"),
            Err(err) => err,
        };
        assert_eq!(
            missing,
            ChatProviderInitError::MissingApiKey {
                provider: "mistral",
                env_key: "MISTRAL_API_KEY"
            }
        );
    }
}
