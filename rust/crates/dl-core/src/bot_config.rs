//! Dateibasierte Discord-Konfiguration ohne ENV-Overrides.
//!
//! Der Lader verändert weder Prozess-ENV noch Dateien. Die Port-Konfiguration
//! von dl-bot und dl-web verwendet ihn über dl-core::Config. Weitere
//! Modulkonfigurationen sind noch nicht vollständig migriert.

use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use serde::{Deserialize, Serialize};

const MAX_CONFIG_BYTES: u64 = 256 * 1024;
pub const DEFAULT_CONFIG_PATH: &str = "config/bot.toml";

/// Fehler enthalten keine TOML-Werte und keinen Parser-Quelltext.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BotConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub runtime: crate::runtime_config::RuntimeConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub services: ServiceConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub features: FeatureConfig,
    #[serde(default)]
    pub moderation: ModerationConfig,
    #[serde(default)]
    pub concierge: ConciergeConfig,
    #[serde(default)]
    pub llm: LlmConfig,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiscordConfig {
    /// Snowflakes als Dezimalstrings, damit der gesamte u64-Bereich nutzbar ist.
    pub guild_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceConfig {
    pub steam_api_url: String,
    pub dashboard_port: u16,
    pub public_stats_port: u16,
    pub master_broker_port: u16,
    pub tierlist_public_port: u16,
    pub changelog_port: u16,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            steam_api_url: "http://127.0.0.1:8783".into(),
            dashboard_port: 8766,
            public_stats_port: 8768,
            master_broker_port: 8770,
            tierlist_public_port: 8771,
            changelog_port: 8899,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// Kompatibilitätsfeld für den bisherigen Config::db_path-Vertrag.
    /// Kein Postgres-Zugang und keine Umstellung der produktiven Persistenz.
    /// Relative Pfade beziehen sich wie zuvor auf das Arbeitsverzeichnis.
    pub legacy_snapshot_path: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            legacy_snapshot_path: PathBuf::from("data/deadlock.sqlite3"),
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeatureConfig {
    pub gateway: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModerationConfig {
    pub enforce: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConciergeConfig {
    pub timeout_seconds: u64,
}

impl Default for ConciergeConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 100,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Fireworks,
    Openai,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum UseCase {
    BotPate,
    Faq,
    LfgFreitext,
    ScrimLagebild,
    VerbinderMatch,
    VerbinderKritik,
    AiOnboarding,
    BrainAntwort,
    CoachingAnfrage,
    ModerationText,
    ModerationVerify,
    StreamerMatcher,
    TurnierVorschlag,
    VoiceHint,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlmConfig {
    /// Nur ein belegter bisheriger DL_LLM_PROVIDER_DEFAULT-Override.
    pub default_provider: Option<Provider>,
    pub fireworks: FireworksConfig,
    pub use_cases: BTreeMap<UseCase, UseCaseConfig>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UseCaseConfig {
    pub provider: Provider,
    /// Bewusster Pin für diesen Anwendungsfall.
    pub model: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FireworksConfig {
    /// Ausschließlich ein bewusster Pin; keine automatische Modellwahl.
    pub model: Option<String>,
}

fn snowflake(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|id| id != 0)
}

/// Eng begrenzte Namensfamilie. Preview, Pro, Distill und fremde Konten passen nicht.
fn flash_version(model: &str) -> Option<(u32, u32, u32)> {
    let name = model.strip_prefix("accounts/fireworks/models/deepseek-v")?;
    let (version, suffix) = name.split_once("-flash")?;
    let release = if suffix.is_empty() {
        0
    } else {
        let date = suffix.strip_prefix('-')?;
        if !matches!(date.len(), 4 | 8) || !date.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        date.parse::<u32>().ok()?
    };
    let (major, minor) = version.split_once('p').unwrap_or((version, "0"));
    if major.is_empty()
        || minor.is_empty()
        || !major
            .bytes()
            .chain(minor.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let major = major.parse::<u32>().ok()?;
    if major == 0 {
        return None;
    }
    Some((major, minor.parse::<u32>().ok()?, release))
}

impl BotConfig {
    pub fn parse(text: &str) -> Result<Self, BotConfigError> {
        if text.len() as u64 > MAX_CONFIG_BYTES {
            return Err(BotConfigError::TooLarge);
        }
        let config: Self =
            toml::from_str(text).map_err(|error: toml::de::Error| BotConfigError::Parse {
                offset: error.span().map_or(0, |span| span.start),
            })?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, BotConfigError> {
        Self::load_document(path).map(|(config, _)| config)
    }

    pub fn load_document(path: impl AsRef<Path>) -> Result<(Self, String), BotConfigError> {
        let mut bytes = Vec::new();
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = options
            .open(path.as_ref())
            .map_err(|_| BotConfigError::Read)?;
        if !file.metadata().map_err(|_| BotConfigError::Read)?.is_file() {
            return Err(BotConfigError::Read);
        }
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| BotConfigError::Read)?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(BotConfigError::TooLarge);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| BotConfigError::Encoding)?;
        let mut config = Self::parse(text)?;
        if let Some(base) = path.as_ref().parent() {
            config.runtime.resolve_paths(base);
        }
        Ok((config, text.to_owned()))
    }

    pub fn validate(&self) -> Result<(), BotConfigError> {
        self.runtime.validate()?;
        let invalid = BotConfigError::Validation;
        let steam = url::Url::parse(&self.services.steam_api_url)
            .map_err(|_| invalid("services.steam_api_url ist ungültig"))?;
        if !matches!(steam.scheme(), "http" | "https")
            || !steam
                .host_str()
                .and_then(|host| {
                    host.trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .ok()
                })
                .is_some_and(|ip| ip.is_loopback())
            || !steam.username().is_empty()
            || steam.password().is_some()
            || steam.query().is_some()
            || steam.fragment().is_some()
            || steam.path() != "/"
            || steam.port_or_known_default() == Some(0)
        {
            return Err(invalid(
                "services.steam_api_url benötigt eine Loopback-Adresse ohne Zugangsdaten oder Pfad",
            ));
        }
        if self.schema_version != 1 {
            return Err(invalid("schema_version muss 1 sein"));
        }
        if self
            .discord
            .guild_id
            .as_deref()
            .is_some_and(|id| !snowflake(id))
        {
            return Err(invalid(
                "Discord-IDs müssen positive u64-Dezimalstrings sein",
            ));
        }
        if self.features.gateway && self.discord.guild_id.is_none() {
            return Err(invalid("discord.guild_id fehlt bei aktiviertem Gateway"));
        }
        let ports = [
            self.services.dashboard_port,
            self.services.public_stats_port,
            self.services.master_broker_port,
            self.services.tierlist_public_port,
            self.services.changelog_port,
        ];
        if ports.contains(&0)
            || ports
                .iter()
                .enumerate()
                .any(|(i, port)| ports[..i].contains(port))
        {
            return Err(invalid("Dienstports müssen positiv und verschieden sein"));
        }
        let snapshot = self.storage.legacy_snapshot_path.to_string_lossy();
        if snapshot.trim().is_empty()
            || snapshot.chars().any(char::is_control)
            || snapshot.contains("://")
        {
            return Err(invalid(
                "storage.legacy_snapshot_path muss ein lokaler Dateipfad sein",
            ));
        }
        if self.concierge.timeout_seconds == 0 || self.concierge.timeout_seconds > 110 {
            return Err(invalid(
                "concierge.timeout_seconds muss zwischen 1 und 110 liegen",
            ));
        }
        let fireworks = &self.llm.fireworks;
        if fireworks
            .model
            .as_deref()
            .is_some_and(|id| flash_version(id).is_none())
        {
            return Err(invalid(
                "Fireworks-Pin liegt außerhalb der freigegebenen DeepSeek-Flash-Familie",
            ));
        }
        for cfg in self.llm.use_cases.values() {
            if cfg.model.as_deref().is_some_and(|id| {
                id.is_empty()
                    || id.trim() != id
                    || id.chars().any(char::is_control)
                    || (cfg.provider == Provider::Fireworks && flash_version(id).is_none())
            }) {
                return Err(invalid("Modell-Pin für Anwendungsfall ist ungültig"));
            }
        }
        Ok(())
    }

    /// Die Ausnahme-Defaults entsprechen der bestehenden Provider-Fabrik.
    pub fn provider_for(&self, use_case: UseCase) -> Provider {
        self.llm
            .use_cases
            .get(&use_case)
            .map(|cfg| cfg.provider)
            .or(self.llm.default_provider)
            .unwrap_or(match use_case {
                UseCase::ModerationVerify | UseCase::TurnierVorschlag | UseCase::VoiceHint => {
                    Provider::Openai
                }
                _ => Provider::Fireworks,
            })
    }
}

/// Validierte Momentaufnahme. Ein fehlerhaftes Neuladen ersetzt den Stand nicht.
/// Ein Aufrufer muss Änderungen danach auch an seine Dienste weiterreichen.
/// Der produktive Startpfad verwendet eine feste Momentaufnahme bis zum Neustart.
pub struct BotConfigStore {
    path: PathBuf,
    current: RwLock<Arc<BotConfig>>,
}

impl BotConfigStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BotConfigError> {
        let requested = path.as_ref();
        // Den vom Betreiber gewählten Dateinamen erhalten. Das Auflösen der
        // Datei selbst würde einen später umgeschalteten Symlink festhalten.
        let path = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|_| BotConfigError::Read)?
                .join(requested)
        };
        let config = BotConfig::load(&path)?;
        Ok(Self {
            path,
            current: RwLock::new(Arc::new(config)),
        })
    }

    pub fn snapshot(&self) -> Result<Arc<BotConfig>, BotConfigError> {
        self.current
            .read()
            .map(|value| Arc::clone(&value))
            .map_err(|_| BotConfigError::Lock)
    }

    pub fn reload(&self) -> Result<Arc<BotConfig>, BotConfigError> {
        // Unter derselben Sperre laden und tauschen, damit parallele Reloads
        // keine ältere Momentaufnahme nach einer neueren veröffentlichen.
        let mut current = self.current.write().map_err(|_| BotConfigError::Lock)?;
        let replacement = Arc::new(BotConfig::load(&self.path)?);
        *current = Arc::clone(&replacement);
        Ok(replacement)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MINIMAL: &str = "schema_version = 1\n";

    fn rejected(text: &str) {
        assert!(BotConfig::parse(text).is_err());
    }

    #[test]
    fn minimal_config_has_safe_defaults() {
        let c = BotConfig::parse(MINIMAL).expect("valid");
        assert!(!c.features.gateway);
        assert!(c.llm.fireworks.model.is_none());
    }

    #[test]
    fn schema_version_is_required() {
        rejected("");
    }

    #[test]
    fn unknown_schema_is_rejected() {
        rejected("schema_version = 2");
    }

    #[test]
    fn unknown_top_level_is_rejected() {
        rejected("schema_version = 1\nllmm = {}");
    }

    #[test]
    fn unknown_nested_field_is_rejected() {
        rejected("schema_version = 1\n[llm.fireworks]\nrefesh_seconds = 10");
    }

    #[test]
    fn secret_fields_are_rejected_without_echo() {
        for text in [
            "schema_version = 1\napi_key = 'SYNTHETIC_SENSITIVE_VALUE'",
            "schema_version = 1\n[llm.fireworks]\nmodel = 'SYNTHETIC_SENSITIVE_VALUE'",
        ] {
            let error = BotConfig::parse(text).err().expect("invalid");
            assert!(!format!("{error:?} {error}").contains("SYNTHETIC_SENSITIVE_VALUE"));
        }
    }

    #[test]
    fn malformed_toml_does_not_echo_input() {
        let error = BotConfig::parse("schema_version = 'SYNTHETIC_SENSITIVE_VALUE")
            .err()
            .expect("invalid");
        assert!(!format!("{error:?} {error}").contains("SYNTHETIC_SENSITIVE_VALUE"));
    }

    #[test]
    fn snowflake_supports_full_u64_range() {
        assert!(BotConfig::parse(
            "schema_version = 1\n[discord]\nguild_id = '18446744073709551615'"
        )
        .is_ok());
    }

    #[test]
    fn zero_negative_and_overflow_ids_are_rejected() {
        for id in ["0", "-1", "18446744073709551616", " 123", ""] {
            rejected(&format!("schema_version = 1\n[discord]\nguild_id = '{id}'"));
        }
    }

    #[test]
    fn gateway_requires_guild_id() {
        rejected("schema_version = 1\n[features]\ngateway = true");
    }

    #[test]
    fn duplicate_or_zero_ports_are_rejected() {
        rejected("schema_version = 1\n[services]\nmaster_broker_port = 0");
        rejected("schema_version = 1\n[services]\nmaster_broker_port = 8899");
    }

    #[test]
    fn every_service_port_is_validated() {
        for key in [
            "dashboard_port",
            "public_stats_port",
            "master_broker_port",
            "tierlist_public_port",
            "changelog_port",
        ] {
            for invalid in [0, -1, 65536] {
                rejected(&format!(
                    "schema_version = 1\n[services]\n{key} = {invalid}"
                ));
            }
        }
        rejected("schema_version = 1\n[services]\ndashboard_port = 8770");
        rejected("schema_version = 1\n[services]\npublic_stats_port = 8771");
    }

    #[test]
    fn invalid_storage_paths_do_not_echo_values() {
        for value in [
            "",
            "   ",
            "postgres://SYNTHETIC_SENSITIVE_VALUE@localhost/db",
        ] {
            let text = format!("schema_version = 1\n[storage]\nlegacy_snapshot_path = '{value}'");
            let error = BotConfig::parse(&text).err().expect("invalid");
            assert!(!format!("{error:?} {error}").contains("SYNTHETIC_SENSITIVE_VALUE"));
        }
        rejected("schema_version = 1\n[storage]\nlegacy_snapshot_path = \"bad\\u0000path\"");
        rejected("schema_version = 1\n[storage]\nlegacy_snaphot_path = 'data/db'");
    }

    #[test]
    fn pro_preview_and_foreign_accounts_are_rejected() {
        for name in [
            "accounts/fireworks/models/deepseek-v4-pro",
            "accounts/fireworks/models/deepseek-v4-flash-preview",
            "accounts/other/models/deepseek-v4-flash",
            "accounts/fireworks/models/deepseek-v4-flash-vision-exp",
        ] {
            rejected(&format!(
                "schema_version = 1\n[llm.fireworks]\nmodel = '{name}'"
            ));
        }
    }

    #[test]
    fn unknown_use_case_is_rejected() {
        rejected("schema_version = 1\n[llm.use_cases.bot_pat]\nprovider = 'fireworks'");
    }

    #[test]
    fn existing_provider_exceptions_are_preserved() {
        let c = BotConfig::parse(MINIMAL).expect("valid");
        assert!(c.provider_for(UseCase::VoiceHint) == Provider::Openai);
        assert!(c.provider_for(UseCase::BotPate) == Provider::Fireworks);
    }

    #[test]
    fn absent_file_is_not_silently_defaulted() {
        let directory = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            BotConfig::load(directory.path().join("absent.toml")),
            Err(BotConfigError::Read)
        ));
    }

    #[test]
    fn oversized_input_is_rejected() {
        assert!(matches!(
            BotConfig::parse(&"x".repeat(MAX_CONFIG_BYTES as usize + 1)),
            Err(BotConfigError::TooLarge)
        ));
    }

    #[test]
    fn bad_reload_keeps_previous_snapshot() {
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

    #[cfg(unix)]
    #[test]
    fn reload_follows_replaced_symlink() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().expect("tempdir");
        let first = directory.path().join("first.toml");
        let second = directory.path().join("second.toml");
        let link = directory.path().join("bot.toml");
        std::fs::write(&first, MINIMAL).expect("write first");
        std::fs::write(&second, "schema_version = 1\n[moderation]\nenforce = true")
            .expect("write second");
        symlink(&first, &link).expect("first link");
        let store = BotConfigStore::open(&link).expect("open");
        assert!(!store.snapshot().expect("snapshot").moderation.enforce);
        let replacement = directory.path().join("next-link");
        symlink(&second, &replacement).expect("second link");
        std::fs::rename(&replacement, &link).expect("replace link");
        assert!(store.reload().expect("reload").moderation.enforce);
    }
}
