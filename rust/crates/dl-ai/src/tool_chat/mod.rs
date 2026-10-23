mod anthropic_http;
mod backend;
mod chat_backend;
mod claude_plan;
mod config;
mod emulation;
mod router;
mod types;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub use anthropic_http::{parse_response, wire_request, AnthropicHttpBackend};
pub use backend::{BackendError, CallTarget, FailureKind, ToolBackend};
pub use chat_backend::ChatProviderBackend;
pub use claude_plan::ClaudePlanBackend;
pub use config::{
    ConfigError, FallbackConfig, GatewayConfig, ProviderConfig, ProviderKind, SlotConfig, Target,
};
pub use router::{
    LedgerEntry, LedgerOutcome, PostgresLedger, ProviderHandle, RouteError, Routed, SlotRouter,
    UsageLedger,
};
pub use types::{
    ContentBlock, Message, Role, StopReason, ToolChatRequest, ToolChatResponse, ToolSpec, Usage,
};

use crate::{ChatProvider, OpenAiChatProvider, RetryConfig};

fn retry(timeout_ms: Option<u64>) -> RetryConfig {
    RetryConfig {
        request_timeout: Duration::from_millis(timeout_ms.unwrap_or(60_000)),
        max_retries: 0,
        ..RetryConfig::default()
    }
}

fn backend(
    name: &str,
    provider: &ProviderConfig,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Arc<dyn ToolBackend>, ConfigError> {
    let missing = |field: &str| ConfigError::Invalid(format!("provider {name} needs {field}"));
    Ok(match provider.kind {
        ProviderKind::ClaudePlan => Arc::new(ClaudePlanBackend::new(
            provider
                .claude_bin
                .clone()
                .ok_or_else(|| missing("claude_bin"))?,
            provider
                .work_dir
                .clone()
                .ok_or_else(|| missing("work_dir"))?,
            provider
                .max_output_bytes
                .ok_or_else(|| missing("max_output_bytes"))?,
        )),
        ProviderKind::CodexPlan => Arc::new(AnthropicHttpBackend::new(
            provider
                .base_url
                .as_deref()
                .ok_or_else(|| missing("base_url"))?,
            Duration::from_millis(provider.request_timeout_ms.unwrap_or(60_000)),
        )),
        ProviderKind::Fireworks => {
            let chat =
                OpenAiChatProvider::from_fireworks_env(lookup, retry(provider.request_timeout_ms))
                    .map_err(|_| missing("a Fireworks API key in the environment"))?;
            Arc::new(ChatProviderBackend::new(
                chat as Arc<dyn ChatProvider>,
                true,
            ))
        }
        ProviderKind::OpenaiCompatible => {
            let key_name = provider
                .api_key_env
                .as_deref()
                .ok_or_else(|| missing("api_key_env"))?;
            let key = lookup(key_name)
                .filter(|key| !key.trim().is_empty())
                .ok_or_else(|| missing("its API key in the environment"))?;
            let chat = OpenAiChatProvider::new_with_retry(
                provider
                    .base_url
                    .clone()
                    .ok_or_else(|| missing("base_url"))?,
                key,
                String::new(),
                retry(provider.request_timeout_ms),
            );
            Arc::new(ChatProviderBackend::new(
                chat as Arc<dyn ChatProvider>,
                false,
            ))
        }
    })
}

pub fn build_router(
    config: &GatewayConfig,
    lookup: impl Fn(&str) -> Option<String>,
    ledger: Arc<dyn UsageLedger>,
) -> Result<SlotRouter, ConfigError> {
    let mut providers = BTreeMap::new();
    for (name, provider) in &config.providers {
        providers.insert(
            name.clone(),
            ProviderHandle::new(
                backend(name, provider, &lookup)?,
                provider.concurrency,
                Duration::from_millis(provider.queue_timeout_ms),
            ),
        );
    }
    let slots = config
        .slots
        .iter()
        .map(|(name, slot)| (name.clone(), config.targets(slot)))
        .collect();
    Ok(SlotRouter::new(slots, providers, ledger))
}
