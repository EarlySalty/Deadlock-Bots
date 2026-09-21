//! Startkonfiguration von dl-bot und dl-web aus der zentralen TOML.
//!
//! `Config::from_env` bleibt als Name für bestehende Aufrufstellen erhalten,
//! liest aber keine Konfigurationswerte aus ENV. Der Prozess lädt
//! `config/bot.toml` oder den expliziten CLI-Pfad `--config PFAD`.
//! Änderungen werden beim nächsten Prozessstart wirksam.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use crate::bot_config::{BotConfig, BotConfigError, DEFAULT_CONFIG_PATH};

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error(transparent)]
    File(#[from] BotConfigError),
    #[error(
        "Config-Pfad ungültig: --config PFAD darf einmal mit einem nichtleeren Pfad vorkommen"
    )]
    Arguments,
}

/// Gemeinsame Ports der Discord-Dienste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ports {
    pub dashboard: u16,
    pub public_stats: u16,
    pub master_broker: u16,
    pub tierlist_public: u16,
    pub changelog_api: u16,
}

/// Projektion der zentralen TOML für die bestehenden Startaufrufe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Bisheriger Snapshot-Pfadvertrag, kein Postgres-Zugang.
    pub db_path: PathBuf,
    pub ports: Ports,
}

/// Eine feste, validierte Momentaufnahme für die Laufzeit dieses Prozesses.
/// Ein Config-Reload darf nicht versehentlich Ports eines laufenden Servers ändern.
pub struct ProcessBotConfig {
    source: PathBuf,
    config: Arc<BotConfig>,
    fingerprint: String,
}

impl ProcessBotConfig {
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn snapshot(&self) -> Arc<BotConfig> {
        Arc::clone(&self.config)
    }

    fn load(path: PathBuf) -> Result<Self, ConfigError> {
        let source = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|_| BotConfigError::Read)?
                .join(path)
        };
        let config = Arc::new(BotConfig::load(&source)?);
        let fingerprint = crate::operating_config::fingerprint(&config)?;
        Ok(Self {
            source,
            config,
            fingerprint,
        })
    }
}

static PROCESS_CONFIG: OnceLock<Result<ProcessBotConfig, ConfigError>> = OnceLock::new();

/// CLI-Pfadauswahl ohne ENV-Overrides.
///
/// Andere Argumente gehören dem aufrufenden Binary und werden hier nicht
/// interpretiert. Nach `--` werden auch keine Config-Optionen mehr ausgewertet.
/// Fehlermeldungen geben weder den Pfad noch sonstige Argumentwerte wieder.
pub fn config_path_from_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<PathBuf, ConfigError> {
    let mut args = args.into_iter();
    let mut selected = None;
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        let candidate = if arg == "--config" {
            let path = args.next().ok_or(ConfigError::Arguments)?;
            if path.is_empty() || path.to_string_lossy().starts_with('-') {
                return Err(ConfigError::Arguments);
            }
            Some(PathBuf::from(path))
        } else if arg.to_string_lossy().starts_with("--config=") {
            let value = arg
                .to_str()
                .and_then(|text| text.strip_prefix("--config="))
                .filter(|value| !value.is_empty())
                .ok_or(ConfigError::Arguments)?;
            Some(PathBuf::from(value))
        } else {
            None
        };
        match candidate {
            Some(_) if selected.is_some() => return Err(ConfigError::Arguments),
            Some(path) => selected = Some(path),
            None => {}
        }
    }
    Ok(selected.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH)))
}

/// Prozessweite Dateiquelle. Auch ein Ladefehler bleibt für diesen Prozess
/// unverändert, statt beim nächsten Modul einen anderen Stand zu aktivieren.
pub fn process_bot_config() -> Result<&'static ProcessBotConfig, ConfigError> {
    match PROCESS_CONFIG.get_or_init(|| {
        let path = config_path_from_args(std::env::args_os().skip(1))?;
        let loaded = ProcessBotConfig::load(path)?;
        tracing::info!(
            schema_version = loaded.config.schema_version,
            fingerprint = loaded.fingerprint(),
            "DL_BOT_TOML_STARTUP_V1"
        );
        Ok(loaded)
    }) {
        Ok(loaded) => Ok(loaded),
        Err(error) => Err(error.clone()),
    }
}

impl Config {
    /// Historischer Methodenname für die vorhandenen Binary-Einstiegspunkte.
    /// Keine Abfrage von MASTER_BROKER_PORT, DEADLOCK_DB_PATH oder anderer ENV.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_process()
    }

    pub fn from_process() -> Result<Self, ConfigError> {
        let snapshot = process_bot_config()?.snapshot();
        Self::from_bot_config(&snapshot)
    }

    /// Expliziter Dateieinstieg für Werkzeuge und isolierte Tests.
    /// Setzt oder verändert keine prozessweite Momentaufnahme.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::from_bot_config(&BotConfig::load(path)?)
    }

    pub fn from_bot_config(config: &BotConfig) -> Result<Self, ConfigError> {
        // Die Struct-Felder sind öffentlich: auch direkt konstruierte Werte prüfen.
        config.validate()?;
        Ok(Self {
            db_path: config.storage.legacy_snapshot_path.clone(),
            ports: Ports {
                dashboard: config.services.dashboard_port,
                public_stats: config.services.public_stats_port,
                master_broker: config.services.master_broker_port,
                tierlist_public: config.services.tierlist_public_port,
                changelog_api: config.services.changelog_port,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(|value| OsString::from(*value)).collect()
    }

    #[test]
    fn default_path_does_not_need_configuration_env() {
        assert_eq!(
            config_path_from_args(Vec::<OsString>::new()).expect("default path"),
            PathBuf::from(DEFAULT_CONFIG_PATH)
        );
    }

    #[test]
    fn explicit_path_and_equals_form_are_supported() {
        assert_eq!(
            config_path_from_args(args(&["--config", "/srv/bot.toml"])).expect("path"),
            PathBuf::from("/srv/bot.toml")
        );
        assert_eq!(
            config_path_from_args(args(&["--config=config/custom.toml"])).expect("path"),
            PathBuf::from("config/custom.toml")
        );
    }

    #[test]
    fn duplicate_empty_and_missing_path_are_rejected() {
        for values in [
            vec!["--config"],
            vec!["--config", ""],
            vec!["--config="],
            vec!["--config", "--other"],
            vec!["--config", "one.toml", "--config", "two.toml"],
            vec!["--config=one.toml", "--config=two.toml"],
        ] {
            assert_eq!(
                config_path_from_args(args(&values)),
                Err(ConfigError::Arguments)
            );
        }
    }

    #[test]
    fn other_binary_arguments_and_option_terminator_are_preserved() {
        assert_eq!(
            config_path_from_args(args(&["serve", "--port", "9000"])).expect("default"),
            PathBuf::from(DEFAULT_CONFIG_PATH)
        );
        assert_eq!(
            config_path_from_args(args(&["--", "--config", "not-an-option.toml"]))
                .expect("default"),
            PathBuf::from(DEFAULT_CONFIG_PATH)
        );
    }

    #[cfg(unix)]
    #[test]
    fn separate_path_argument_can_be_non_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"/tmp/config-\xff.toml".to_vec());
        assert_eq!(
            config_path_from_args(vec![OsString::from("--config"), path.clone()])
                .expect("non-UTF-8 path"),
            PathBuf::from(path)
        );
    }

    #[test]
    fn defaults_preserve_existing_port_contract() {
        let bot = BotConfig::parse("schema_version = 1").expect("valid");
        let config = Config::from_bot_config(&bot).expect("project");
        assert_eq!(config.db_path, PathBuf::from("data/deadlock.sqlite3"));
        assert_eq!(config.ports.dashboard, 8766);
        assert_eq!(config.ports.public_stats, 8768);
        assert_eq!(config.ports.master_broker, 8770);
        assert_eq!(config.ports.tierlist_public, 8771);
        assert_eq!(config.ports.changelog_api, 8899);
    }

    #[test]
    fn file_values_reach_the_existing_runtime_projection() {
        let bot = BotConfig::parse(
            "schema_version = 1\n\
             [services]\n\
             dashboard_port = 9001\n\
             public_stats_port = 9002\n\
             master_broker_port = 9003\n\
             tierlist_public_port = 9004\n\
             changelog_port = 9005\n\
             [storage]\n\
             legacy_snapshot_path = '/srv/snapshots/legacy.sqlite3'\n",
        )
        .expect("valid");
        let config = Config::from_bot_config(&bot).expect("project");
        assert_eq!(
            config.ports,
            Ports {
                dashboard: 9001,
                public_stats: 9002,
                master_broker: 9003,
                tierlist_public: 9004,
                changelog_api: 9005,
            }
        );
        assert_eq!(
            config.db_path,
            PathBuf::from("/srv/snapshots/legacy.sqlite3")
        );
    }

    #[test]
    fn missing_file_is_an_error_not_a_default_configuration() {
        let directory = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            Config::from_file(directory.path().join("absent.toml")),
            Err(ConfigError::File(BotConfigError::Read))
        ));
    }

    #[test]
    fn invalid_directly_constructed_values_are_rejected() {
        let mut bot = BotConfig::parse("schema_version = 1").expect("valid");
        bot.services.public_stats_port = bot.services.master_broker_port;
        assert!(Config::from_bot_config(&bot).is_err());
    }

    #[test]
    fn snapshot_does_not_change_after_file_edit() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("bot.toml");
        std::fs::write(&path, "schema_version = 1").expect("write");
        let process = ProcessBotConfig::load(path.clone()).expect("load");
        let before = process.snapshot();
        std::fs::write(
            &path,
            "schema_version = 1\n[services]\nmaster_broker_port = 9001",
        )
        .expect("edit");
        let unchanged = process.snapshot();
        assert!(Arc::ptr_eq(&before, &unchanged));
        assert_eq!(unchanged.services.master_broker_port, 8770);
        assert_eq!(
            Config::from_file(&path)
                .expect("new startup")
                .ports
                .master_broker,
            9001
        );
    }

    #[test]
    fn error_display_and_debug_do_not_contain_source_values() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("bot.toml");
        std::fs::write(
            &path,
            "schema_version = 1\n[services]\nmaster_broker_port = 'SYNTHETIC_MARKER'",
        )
        .expect("write");
        let error = Config::from_file(&path).expect_err("invalid");
        assert!(!format!("{error:?} {error}").contains("SYNTHETIC_MARKER"));
    }
}
