//! Dateibasierte Discord-Konfiguration, ohne ENV-Overrides.
//!
//! Dieser Lader verändert weder Prozess-ENV noch Dateien. Die produktiven
//! Aufrufstellen müssen vor dem Cutover ausdrücklich auf ihn umgestellt werden.

use std::{collections::BTreeMap, fs::File, io::Read, path::{Path, PathBuf}, sync::{Arc, RwLock}};
use serde::Deserialize;

const MAX_CONFIG_BYTES: u64 = 256 * 1024;
pub const DEFAULT_CONFIG_PATH: &str = "config/bot.toml";

/// Fehler enthalten keine TOML-Werte und keinen Parser-Quelltext.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BotConfigError {
    #[error("Config-Datei konnte nicht gelesen werden")]
    Read,
    #[error("Config-Datei überschreitet 256 KiB")]
    TooLarge,
    #[error("Config-Datei enthält kein gültiges UTF-8")]
    Encoding,
    #[error("Config-Syntax oder unbekanntes Feld, Byteposition {offset}")]
    Parse { offset: usize },
    #[error("Ungültige Konfiguration: {0}")]
    Validation(&'static str),
    #[error("Config-Zustand konnte nicht gesperrt werden")]
    Lock,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub services: ServiceConfig,
    #[serde(default)]
    pub features: FeatureConfig,
    #[serde(default)]
    pub moderation: ModerationConfig,
    #[serde(default)]
    pub concierge: ConciergeConfig,
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
    #[serde(default)]
    pub llm: LlmConfig,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiscordConfig {
    /// Snowflakes als Dezimalstrings, damit der gesamte u64-Bereich nutzbar ist.
    pub guild_id: Option<String>,
    pub channels: BTreeMap<String, String>,
    pub roles: BTreeMap<String, String>,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceConfig {
    pub master_broker_port: u16,
    pub changelog_port: u16,
}
impl Default for ServiceConfig {
    fn default() -> Self { Self { master_broker_port: 8770, changelog_port: 8899 } }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeatureConfig {
    pub gateway: bool,
    pub onboarding: bool,
    pub concierge: bool,
    pub lfg: bool,
    pub voice: bool,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModerationConfig {
    pub enforce: bool,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConciergeConfig {
    pub timeout_seconds: u64,
}
impl Default for ConciergeConfig {
    fn default() -> Self { Self { timeout_seconds: 100 } }
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode { #[default] Bm25, Hybrid }

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KnowledgeConfig {
    pub ask_url: String,
    pub timeout_seconds: u64,
    pub retrieval: RetrievalMode,
}
impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            ask_url: "http://127.0.0.1:8896/public/v1/ask".into(),
            timeout_seconds: 7,
            retrieval: RetrievalMode::Bm25,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Provider { #[default] Fireworks, Openai }

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum UseCase {
    BotPate, Faq, LfgFreitext, ScrimLagebild, VerbinderMatch, VerbinderKritik,
    AiOnboarding, BrainAntwort, CoachingAnfrage, ModerationText,
    ModerationVerify, StreamerMatcher, TurnierVorschlag, VoiceHint,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlmConfig {
    pub fireworks: FireworksConfig,
    pub use_cases: BTreeMap<UseCase, UseCaseConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UseCaseConfig {
    pub provider: Provider,
    /// Bewusster Pin für diesen Anwendungsfall.
    pub model: Option<String>,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelSelection { #[default] LatestStable, Pinned }

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily { #[default] DeepseekFlash }

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FireworksConfig {
    pub selection: ModelSelection,
    pub family: ModelFamily,
    /// Ein eingetragener Pin deaktiviert die automatische Auswahl.
    pub model: Option<String>,
    pub refresh_seconds: u64,
    pub fallback_max_age_seconds: u64,
}
impl Default for FireworksConfig {
    fn default() -> Self {
        Self {
            selection: ModelSelection::LatestStable,
            family: ModelFamily::DeepseekFlash,
            model: None,
            refresh_seconds: 3600,
            fallback_max_age_seconds: 86400,
        }
    }
}
impl FireworksConfig {
    pub fn automatic(&self) -> bool {
        self.selection == ModelSelection::LatestStable && self.model.is_none()
    }
}

fn snowflake(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|id| id != 0)
}

/// Eng begrenzte Namensfamilie. Preview, Pro, Distill und fremde Konten passen nicht.
fn flash_version(model: &str) -> Option<(u32, u32, u32)> {
    let name = model.strip_prefix("accounts/fireworks/models/deepseek-v")?;
    let (version, suffix) = name.split_once("-flash")?;
    let release = if suffix.is_empty() { 0 } else {
        let date = suffix.strip_prefix('-')?;
        if !matches!(date.len(), 4 | 8) || !date.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        date.parse::<u32>().ok()?
    };
    let (major, minor) = version.split_once('p').unwrap_or((version, "0"));
    if major.is_empty() || minor.is_empty()
        || !major.bytes().chain(minor.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let major = major.parse::<u32>().ok()?;
    if major == 0 { return None; }
    Some((major, minor.parse::<u32>().ok()?, release))
}

impl BotConfig {
    pub fn parse(text: &str) -> Result<Self, BotConfigError> {
        if text.len() as u64 > MAX_CONFIG_BYTES { return Err(BotConfigError::TooLarge); }
        let config: Self = toml::from_str(text).map_err(|error: toml::de::Error| {
            BotConfigError::Parse { offset: error.span().map_or(0, |span| span.start) }
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, BotConfigError> {
        let mut bytes = Vec::new();
        File::open(path).map_err(|_| BotConfigError::Read)?
            .take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)
            .map_err(|_| BotConfigError::Read)?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES { return Err(BotConfigError::TooLarge); }
        let text = std::str::from_utf8(&bytes).map_err(|_| BotConfigError::Encoding)?;
        Self::parse(text)
    }

    pub fn validate(&self) -> Result<(), BotConfigError> {
        let invalid = BotConfigError::Validation;
        if self.schema_version != 1 { return Err(invalid("schema_version muss 1 sein")); }
        if self.discord.guild_id.as_deref().is_some_and(|id| !snowflake(id))
            || self.discord.channels.values().chain(self.discord.roles.values()).any(|id| !snowflake(id)) {
            return Err(invalid("Discord-IDs müssen positive u64-Dezimalstrings sein"));
        }
        if self.features.gateway && self.discord.guild_id.is_none() {
            return Err(invalid("discord.guild_id fehlt bei aktiviertem Gateway"));
        }
        if self.services.master_broker_port == 0 || self.services.changelog_port == 0
            || self.services.master_broker_port == self.services.changelog_port {
            return Err(invalid("Dienstports müssen positiv und verschieden sein"));
        }
        if self.concierge.timeout_seconds == 0 || self.concierge.timeout_seconds > 110 {
            return Err(invalid("concierge.timeout_seconds muss zwischen 1 und 110 liegen"));
        }
        if self.knowledge.timeout_seconds == 0
            || self.knowledge.timeout_seconds >= self.concierge.timeout_seconds {
            return Err(invalid("Knowledge-Timeout muss positiv und kleiner als Concierge-Timeout sein"));
        }
        let url = url::Url::parse(&self.knowledge.ask_url)
            .map_err(|_| invalid("knowledge.ask_url ist ungültig"))?;
        let loopback = url.host_str().and_then(|s| s.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if !loopback || !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty() || url.password().is_some()
            || url.query().is_some() || url.fragment().is_some() {
            return Err(invalid("knowledge.ask_url benötigt eine Loopback-IP ohne Zugangsdaten, Query oder Fragment"));
        }
        let fireworks = &self.llm.fireworks;
        if fireworks.refresh_seconds == 0 || fireworks.refresh_seconds > fireworks.fallback_max_age_seconds {
            return Err(invalid("Modell-Prüfintervall muss positiv und höchstens so lang wie die Rückfallfrist sein"));
        }
        if fireworks.selection == ModelSelection::Pinned && fireworks.model.is_none() {
            return Err(invalid("llm.fireworks.model fehlt für pinned"));
        }
        if fireworks.model.as_deref().is_some_and(|id| flash_version(id).is_none()) {
            return Err(invalid("Fireworks-Pin liegt außerhalb der freigegebenen DeepSeek-Flash-Familie"));
        }
        for cfg in self.llm.use_cases.values() {
            if cfg.model.as_deref().is_some_and(|id| {
                id.is_empty() || id.trim() != id || id.chars().any(char::is_control)
                    || (cfg.provider == Provider::Fireworks && flash_version(id).is_none())
            }) {
                return Err(invalid("Modell-Pin für Anwendungsfall ist ungültig"));
            }
        }
        Ok(())
    }

    /// Die Ausnahme-Defaults entsprechen der bestehenden Provider-Fabrik.
    pub fn provider_for(&self, use_case: UseCase) -> Provider {
        self.llm.use_cases.get(&use_case).map(|cfg| cfg.provider).unwrap_or_else(|| {
            match use_case {
                UseCase::ModerationVerify | UseCase::TurnierVorschlag | UseCase::VoiceHint => Provider::Openai,
                _ => Provider::Fireworks,
            }
        })
    }
}

/// Validierte Momentaufnahme. Ein fehlerhaftes Neuladen ersetzt den Stand nicht.
/// Ein Aufrufer muss Änderungen danach auch an seine Dienste weiterreichen.
pub struct BotConfigStore {
    path: PathBuf,
    current: RwLock<Arc<BotConfig>>,
}
impl BotConfigStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BotConfigError> {
        let path = path.as_ref().canonicalize().map_err(|_| BotConfigError::Read)?;
        let config = BotConfig::load(&path)?;
        Ok(Self { path, current: RwLock::new(Arc::new(config)) })
    }
    pub fn snapshot(&self) -> Result<Arc<BotConfig>, BotConfigError> {
        self.current.read().map(|value| Arc::clone(&value)).map_err(|_| BotConfigError::Lock)
    }
    pub fn reload(&self) -> Result<Arc<BotConfig>, BotConfigError> {
        // Unter derselben Sperre laden und tauschen, damit parallele Reloads
        // keine ältere Momentaufnahme nach einer neueren veröffentlichen.
        let mut current = self.current.write().map_err(|_| BotConfigError::Lock)?;
        let replacement = Arc::new(BotConfig::load(&self.path)?);
        *current = Arc::clone(&replacement);
        Ok(replacement)
    }
    pub fn path(&self) -> &Path { &self.path }
}

/// Vom Provider-Adapter gelieferte Metadaten. Erfolgreiche Probe ist gesondert
/// zu belegen; ein Katalogeintrag ist kein Funktionsnachweis.
pub struct ModelCandidate {
    pub id: String,
    pub published_at: u64,
    pub ready: bool,
    pub serverless: bool,
    pub probe_passed: bool,
}

/// Reine Auswahlfunktion, kein Scheduler und kein Netzwerkaufruf.
/// Pins benötigen keinen Katalog und deaktivieren die automatische Auswahl.
pub fn select_model<'a>(policy: &'a FireworksConfig, candidates: &'a [ModelCandidate], now: u64) -> Option<&'a str> {
    if let Some(pin) = policy.model.as_deref() { return flash_version(pin).map(|_| pin); }
    if !policy.automatic() { return None; }
    candidates.iter().filter(|c| {
        c.ready && c.serverless && c.probe_passed && c.published_at > 0 && c.published_at <= now
            && flash_version(&c.id).is_some()
    }).max_by_key(|c| (flash_version(&c.id).map(|v| (v.0, v.1)), c.published_at, flash_version(&c.id).map(|v| v.2), c.id.as_str()))
        .map(|c| c.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    const MINIMAL: &str = "schema_version = 1\n";
    fn rejected(text: &str) { assert!(BotConfig::parse(text).is_err()); }
    fn candidate(name: &str, at: u64) -> ModelCandidate {
        ModelCandidate { id: format!("accounts/fireworks/models/{name}"), published_at: at, ready: true, serverless: true, probe_passed: true }
    }
    #[test] fn minimal_config_has_safe_defaults() {
        let c = BotConfig::parse(MINIMAL).expect("valid");
        assert!(!c.features.gateway);
        assert!(c.knowledge.retrieval == RetrievalMode::Bm25);
        assert!(c.llm.fireworks.automatic());
    }
    #[test] fn schema_version_is_required() { rejected(""); }
    #[test] fn unknown_schema_is_rejected() { rejected("schema_version = 2"); }
    #[test] fn unknown_top_level_is_rejected() { rejected("schema_version = 1\nllmm = {}"); }
    #[test] fn unknown_nested_field_is_rejected() { rejected("schema_version = 1\n[llm.fireworks]\nrefesh_seconds = 10"); }
    #[test] fn secret_fields_are_rejected_without_echo() {
        for text in ["schema_version = 1\napi_key = 'SYNTHETIC_SENSITIVE_VALUE'", "schema_version = 1\n[llm.fireworks]\nmodel = 'SYNTHETIC_SENSITIVE_VALUE'"] {
            let error = BotConfig::parse(text).err().expect("invalid");
            assert!(!format!("{error:?} {error}").contains("SYNTHETIC_SENSITIVE_VALUE"));
        }
    }
    #[test] fn malformed_toml_does_not_echo_input() {
        let error = BotConfig::parse("schema_version = 'SYNTHETIC_SENSITIVE_VALUE").err().expect("invalid");
        assert!(!format!("{error:?} {error}").contains("SYNTHETIC_SENSITIVE_VALUE"));
    }
    #[test] fn snowflake_supports_full_u64_range() {
        assert!(BotConfig::parse("schema_version = 1\n[discord]\nguild_id = '18446744073709551615'").is_ok());
    }
    #[test] fn zero_negative_and_overflow_ids_are_rejected() {
        for id in ["0", "-1", "18446744073709551616", " 123", ""] {
            rejected(&format!("schema_version = 1\n[discord]\nguild_id = '{id}'"));
        }
    }
    #[test] fn gateway_requires_guild_id() { rejected("schema_version = 1\n[features]\ngateway = true"); }
    #[test] fn duplicate_or_zero_ports_are_rejected() {
        rejected("schema_version = 1\n[services]\nmaster_broker_port = 0");
        rejected("schema_version = 1\n[services]\nmaster_broker_port = 8899");
    }
    #[test] fn external_and_credential_urls_are_rejected() {
        for endpoint in ["http://example.com/ask", "http://localhost/ask", "http://user:pass@127.0.0.1/ask", "http://127.0.0.1/ask?key=value", "http://127.0.0.1/ask#fragment"] {
            rejected(&format!("schema_version = 1\n[knowledge]\nask_url = '{endpoint}'"));
        }
    }
    #[test] fn ipv6_loopback_is_supported() {
        assert!(BotConfig::parse("schema_version = 1\n[knowledge]\nask_url = 'http://[::1]:8896/public/v1/ask'").is_ok());
    }
    #[test] fn timeout_order_is_checked() { rejected("schema_version = 1\n[knowledge]\ntimeout_seconds = 100"); }
    #[test] fn pin_disables_auto_selection() {
        let c = BotConfig::parse("schema_version = 1\n[llm.fireworks]\nmodel = 'accounts/fireworks/models/deepseek-v4p1-flash'").expect("valid");
        assert!(!c.llm.fireworks.automatic());
        assert!(select_model(&c.llm.fireworks, &[], 100).is_some());
    }
    #[test] fn pinned_without_model_is_rejected() { rejected("schema_version = 1\n[llm.fireworks]\nselection = 'pinned'"); }
    #[test] fn pro_preview_and_foreign_accounts_are_rejected() {
        for name in ["accounts/fireworks/models/deepseek-v4-pro", "accounts/fireworks/models/deepseek-v4-flash-preview", "accounts/other/models/deepseek-v4-flash", "accounts/fireworks/models/deepseek-v4-flash-vision-exp"] {
            rejected(&format!("schema_version = 1\n[llm.fireworks]\nmodel = '{name}'"));
        }
    }
    #[test] fn unknown_use_case_is_rejected() { rejected("schema_version = 1\n[llm.use_cases.bot_pat]\nprovider = 'fireworks'"); }
    #[test] fn existing_provider_exceptions_are_preserved() {
        let c = BotConfig::parse(MINIMAL).expect("valid");
        assert!(c.provider_for(UseCase::VoiceHint) == Provider::Openai);
        assert!(c.provider_for(UseCase::BotPate) == Provider::Fireworks);
    }
    #[test] fn numeric_versions_are_not_lexically_sorted() {
        let models = [candidate("deepseek-v9-flash", 50), candidate("deepseek-v10-flash", 60)];
        assert_eq!(select_model(&FireworksConfig::default(), &models, 100), Some(models[1].id.as_str()));
    }
    #[test] fn newer_upload_of_old_version_does_not_downgrade() {
        let models = [candidate("deepseek-v4p1-flash", 50), candidate("deepseek-v4-flash-0731", 60)];
        assert_eq!(select_model(&FireworksConfig::default(), &models, 100), Some(models[0].id.as_str()));
    }
    #[test] fn candidate_must_be_ready_serverless_probed_and_not_future() {
        let mut models = [candidate("deepseek-v4-flash", 50), candidate("deepseek-v5-flash", 60), candidate("deepseek-v6-flash", 70), candidate("deepseek-v7-flash", 80), candidate("deepseek-v8-flash", 200)];
        models[1].ready = false;
        models[2].serverless = false;
        models[3].probe_passed = false;
        assert_eq!(select_model(&FireworksConfig::default(), &models, 100), Some(models[0].id.as_str()));
    }
    #[test] fn absent_file_is_not_silently_defaulted() {
        let directory = tempfile::tempdir().expect("tempdir");
        assert!(matches!(BotConfig::load(directory.path().join("absent.toml")), Err(BotConfigError::Read)));
    }
    #[test] fn oversized_input_is_rejected() {
        assert!(matches!(BotConfig::parse(&"x".repeat(MAX_CONFIG_BYTES as usize + 1)), Err(BotConfigError::TooLarge)));
    }
    #[test] fn bad_reload_keeps_previous_snapshot() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("bot.toml");
        std::fs::write(&path, MINIMAL).expect("write");
        let store = BotConfigStore::open(&path).expect("open");
        let before = store.snapshot().expect("snapshot");
        std::fs::write(&path, "broken = [").expect("write");
        assert!(store.reload().is_err());
        assert!(Arc::ptr_eq(&before, &store.snapshot().expect("snapshot")));
        std::fs::write(&path, "schema_version = 1\n[moderation]\nenforce = true").expect("write");
        let after = store.reload().expect("reload");
        assert!(after.moderation.enforce);
        assert!(!before.moderation.enforce);
    }
}
