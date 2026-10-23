use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub listen: SocketAddr,
    pub max_request_bytes: usize,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub slots: BTreeMap<String, SlotConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    ClaudePlan,
    CodexPlan,
    Fireworks,
    OpenaiCompatible,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudePlan => "claude_plan",
            Self::CodexPlan => "codex_plan",
            Self::Fireworks => "fireworks",
            Self::OpenaiCompatible => "openai_compatible",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub concurrency: usize,
    pub queue_timeout_ms: u64,
    pub claude_bin: Option<PathBuf>,
    pub work_dir: Option<PathBuf>,
    pub max_output_bytes: Option<usize>,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub request_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotConfig {
    pub provider: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_tokens: u32,
    #[serde(default)]
    pub fallback: Vec<FallbackConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackConfig {
    pub provider: String,
    pub model: String,
    pub timeout_ms: Option<u64>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub provider: String,
    pub model: String,
    pub timeout: Duration,
    pub max_tokens: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file unreadable: {0}")]
    Read(#[from] std::io::Error),
    #[error("config file invalid: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config value invalid: {0}")]
    Invalid(String),
}

impl GatewayConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: String| Err(ConfigError::Invalid(message));
        if !self.listen.ip().is_loopback() {
            return invalid("listen must be a loopback address".into());
        }
        if self.max_request_bytes == 0 {
            return invalid("max_request_bytes must be positive".into());
        }
        for (name, provider) in &self.providers {
            provider.validate(name)?;
        }
        if self.slots.is_empty() {
            return invalid("at least one slot is required".into());
        }
        for (name, slot) in &self.slots {
            if !valid_name(name) {
                return invalid(format!("slot name {name} is invalid"));
            }
            for target in self.targets(slot) {
                if !self.providers.contains_key(&target.provider) {
                    return invalid(format!(
                        "slot {name} uses unknown provider {}",
                        target.provider
                    ));
                }
                if !valid_model(&target.model) || target.timeout.is_zero() || target.max_tokens == 0
                {
                    return invalid(format!("slot {name} has an invalid target"));
                }
            }
        }
        Ok(())
    }

    pub fn targets(&self, slot: &SlotConfig) -> Vec<Target> {
        let primary = Target {
            provider: slot.provider.clone(),
            model: slot.model.clone(),
            timeout: Duration::from_millis(slot.timeout_ms),
            max_tokens: slot.max_tokens,
        };
        std::iter::once(primary)
            .chain(slot.fallback.iter().map(|fallback| Target {
                provider: fallback.provider.clone(),
                model: fallback.model.clone(),
                timeout: Duration::from_millis(fallback.timeout_ms.unwrap_or(slot.timeout_ms)),
                max_tokens: fallback.max_tokens.unwrap_or(slot.max_tokens),
            }))
            .collect()
    }
}

impl ProviderConfig {
    fn validate(&self, name: &str) -> Result<(), ConfigError> {
        let missing = |field: &str| {
            Err(ConfigError::Invalid(format!(
                "provider {name} of kind {} needs {field}",
                self.kind.as_str()
            )))
        };
        if !valid_name(name) || self.concurrency == 0 || self.queue_timeout_ms == 0 {
            return Err(ConfigError::Invalid(format!(
                "provider {name} needs a valid name, concurrency and queue_timeout_ms"
            )));
        }
        match self.kind {
            ProviderKind::ClaudePlan => {
                if self.claude_bin.is_none() {
                    return missing("claude_bin");
                }
                if self.work_dir.is_none() {
                    return missing("work_dir");
                }
                if self.max_output_bytes.is_none_or(|bytes| bytes == 0) {
                    return missing("max_output_bytes");
                }
            }
            ProviderKind::CodexPlan => {
                if self.base_url.is_none() {
                    return missing("base_url");
                }
                if self.request_timeout_ms.is_none() {
                    return missing("request_timeout_ms");
                }
            }
            ProviderKind::Fireworks => {
                if self.request_timeout_ms.is_none() {
                    return missing("request_timeout_ms");
                }
            }
            ProviderKind::OpenaiCompatible => {
                if self.base_url.is_none() {
                    return missing("base_url");
                }
                if self.api_key_env.is_none() {
                    return missing("api_key_env");
                }
                if self.request_timeout_ms.is_none() {
                    return missing("request_timeout_ms");
                }
            }
        }
        Ok(())
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 120
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._/:".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
listen = "127.0.0.1:18790"
max_request_bytes = 1048576

[providers.claude_plan]
kind = "claude_plan"
concurrency = 2
queue_timeout_ms = 1000
claude_bin = "/bin/claude"
work_dir = "/tmp"
max_output_bytes = 1048576

[providers.codex_plan]
kind = "codex_plan"
concurrency = 1
queue_timeout_ms = 1000
base_url = "http://127.0.0.1:18769/v1"
request_timeout_ms = 60000

[slots.brain_answer]
provider = "claude_plan"
model = "claude-haiku-5-5"
timeout_ms = 120000
max_tokens = 4096
fallback = [{ provider = "codex_plan", model = "gpt-6-luna", timeout_ms = 30000 }]
"#;

    #[test]
    fn slots_resolve_into_an_ordered_target_chain() {
        let config = GatewayConfig::parse(SAMPLE).expect("valid config");
        let targets = config.targets(&config.slots["brain_answer"]);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].model, "claude-haiku-5-5");
        assert_eq!(targets[1].provider, "codex_plan");
        assert_eq!(targets[1].timeout, Duration::from_secs(30));
        assert_eq!(targets[1].max_tokens, 4096);
    }

    #[test]
    fn unsafe_or_incomplete_configs_are_rejected() {
        for broken in [
            SAMPLE.replace("127.0.0.1:18790", "0.0.0.0:18790"),
            SAMPLE.replace("provider = \"codex_plan\"", "provider = \"nope\""),
            SAMPLE.replace("claude_bin = \"/bin/claude\"\n", ""),
            SAMPLE.replace("concurrency = 2", "concurrency = 0"),
            SAMPLE.replace("max_tokens = 4096", "max_tokens = 4096\nsurprise = 1"),
        ] {
            assert!(GatewayConfig::parse(&broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn shipped_gateway_config_keeps_brain_answer_on_haiku_without_fallback() {
        let shipped = include_str!("../../../../../config/dl-ai-gateway.toml");
        let config = GatewayConfig::parse(shipped).expect("shipped config");
        let slot = &config.slots["brain_answer"];
        assert_eq!(
            config.providers[&slot.provider].kind,
            ProviderKind::ClaudePlan
        );
        assert_eq!(slot.model, "claude-haiku-5-5");
        assert!(slot.fallback.is_empty());
    }
}
