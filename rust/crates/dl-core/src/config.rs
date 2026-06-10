//! Zentrale Laufzeit-Konfiguration.
//!
//! Wird beim Prozessstart genau EINMAL aus der Umgebung gelesen, validiert
//! und danach unveränderlich durchgereicht. Die ENV-Namen sind bewusst
//! identisch zum Python-Original, damit beide Welten während der
//! Strangler-Fig-Migration mit derselben systemd-Umgebung laufen.

use std::path::PathBuf;

/// ENV-Variablen — Namen sind Vertrag mit dem Python-Original (service/db.py u. a.).
const ENV_DB_PATH: &str = "DEADLOCK_DB_PATH";
const ENV_DB_DIR: &str = "DEADLOCK_DB_DIR";
const DB_FILE_NAME: &str = "deadlock.sqlite3";
/// Default relativ zum Arbeitsverzeichnis (systemd: WorkingDirectory = Repo-Root).
const DB_DEFAULT_RELATIVE: &str = "data/deadlock.sqlite3";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("ungültiger Wert für {var}: {value:?} ({reason})")]
    Invalid {
        var: &'static str,
        value: String,
        reason: String,
    },
}

/// Alle Ports der Bestandsdienste. Die Defaults sind die heute live
/// vergebenen Ports — sie bleiben beim Cutover identisch, damit externe
/// Konsumenten (Twitch-Bot → Broker, Caddy → Websites) nichts merken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ports {
    /// Admin-Dashboard (Python: DASHBOARD_PORT)
    pub dashboard: u16,
    /// Öffentliche Turnier-Website (Python: TURNIER_PUBLIC_PORT)
    pub turnier_public: u16,
    /// Öffentliche Aktivitäts-Statistiken (Python: PUBLIC_STATS_PORT)
    pub public_stats: u16,
    /// Master-Broker, interne Discord-Aktions-API (Python: MASTER_BROKER_PORT)
    pub master_broker: u16,
    /// Öffentliche Tierlist (Python: TIERLIST_PUBLIC_PORT)
    pub tierlist_public: u16,
    /// Changelog-Empfänger des Changelog-Publisher-Cogs (Python: CHANGELOG_API_PORT)
    pub changelog_api: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Pfad zur gemeinsamen SQLite-DB — der zentrale Vertrag mit dem
    /// Python-Bot (siehe rust/docs/01-db-contract.md).
    pub db_path: PathBuf,
    pub ports: Ports,
}

impl Config {
    /// Liest die Konfiguration aus der Prozess-Umgebung.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Testbare Variante: Werte kommen aus einer beliebigen Lookup-Funktion.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        // DB-Pfad-Auflösung exakt wie service/db.py:
        // 1) DEADLOCK_DB_PATH (kompletter Pfad, höchste Priorität)
        // 2) DEADLOCK_DB_DIR  (Verzeichnis; Datei heißt deadlock.sqlite3)
        // 3) Default data/deadlock.sqlite3 relativ zum Arbeitsverzeichnis
        let db_path = if let Some(path) = non_empty(lookup(ENV_DB_PATH)) {
            PathBuf::from(path)
        } else if let Some(dir) = non_empty(lookup(ENV_DB_DIR)) {
            PathBuf::from(dir).join(DB_FILE_NAME)
        } else {
            PathBuf::from(DB_DEFAULT_RELATIVE)
        };

        let ports = Ports {
            dashboard: port(&lookup, "DASHBOARD_PORT", 8766)?,
            turnier_public: port(&lookup, "TURNIER_PUBLIC_PORT", 8767)?,
            public_stats: port(&lookup, "PUBLIC_STATS_PORT", 8768)?,
            master_broker: port(&lookup, "MASTER_BROKER_PORT", 8770)?,
            tierlist_public: port(&lookup, "TIERLIST_PUBLIC_PORT", 8771)?,
            changelog_api: port(&lookup, "CHANGELOG_API_PORT", 8899)?,
        };

        Ok(Self { db_path, ports })
    }
}

/// Leere Strings zählen als "nicht gesetzt" (Verhalten wie im Python-Loader).
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn port(
    lookup: &impl Fn(&str) -> Option<String>,
    var: &'static str,
    default: u16,
) -> Result<u16, ConfigError> {
    match non_empty(lookup(var)) {
        None => Ok(default),
        Some(raw) => raw.trim().parse::<u16>().map_err(|e| ConfigError::Invalid {
            var,
            value: raw,
            reason: e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from<'a>(map: &'a HashMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| map.get(key).map(|v| (*v).to_string())
    }

    #[test]
    fn defaults_ohne_env() {
        let cfg = Config::from_lookup(|_| None).expect("Defaults müssen gültig sein");
        assert_eq!(cfg.db_path, PathBuf::from("data/deadlock.sqlite3"));
        assert_eq!(cfg.ports.dashboard, 8766);
        assert_eq!(cfg.ports.turnier_public, 8767);
        assert_eq!(cfg.ports.public_stats, 8768);
        assert_eq!(cfg.ports.master_broker, 8770);
        assert_eq!(cfg.ports.tierlist_public, 8771);
        assert_eq!(cfg.ports.changelog_api, 8899);
    }

    #[test]
    fn db_path_hat_vorrang_vor_db_dir() {
        let map = HashMap::from([
            ("DEADLOCK_DB_PATH", "/srv/spezial.sqlite3"),
            ("DEADLOCK_DB_DIR", "/srv/anders"),
        ]);
        let cfg = Config::from_lookup(lookup_from(&map)).expect("gültig");
        assert_eq!(cfg.db_path, PathBuf::from("/srv/spezial.sqlite3"));
    }

    #[test]
    fn db_dir_ergaenzt_dateinamen() {
        let map = HashMap::from([("DEADLOCK_DB_DIR", "/srv/deadlock")]);
        let cfg = Config::from_lookup(lookup_from(&map)).expect("gültig");
        assert_eq!(cfg.db_path, PathBuf::from("/srv/deadlock/deadlock.sqlite3"));
    }

    #[test]
    fn leerer_string_zaehlt_als_nicht_gesetzt() {
        let map = HashMap::from([("DEADLOCK_DB_PATH", "  "), ("DASHBOARD_PORT", "")]);
        let cfg = Config::from_lookup(lookup_from(&map)).expect("gültig");
        assert_eq!(cfg.db_path, PathBuf::from("data/deadlock.sqlite3"));
        assert_eq!(cfg.ports.dashboard, 8766);
    }

    #[test]
    fn ungueltiger_port_ist_fehler() {
        let map = HashMap::from([("MASTER_BROKER_PORT", "keine-zahl")]);
        let err = Config::from_lookup(lookup_from(&map)).expect_err("muss fehlschlagen");
        let ConfigError::Invalid { var, value, .. } = err;
        assert_eq!(var, "MASTER_BROKER_PORT");
        assert_eq!(value, "keine-zahl");
    }

    #[test]
    fn port_override_wird_uebernommen() {
        let map = HashMap::from([("PUBLIC_STATS_PORT", "9000")]);
        let cfg = Config::from_lookup(lookup_from(&map)).expect("gültig");
        assert_eq!(cfg.ports.public_stats, 9000);
    }
}
