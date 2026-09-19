//! Gemeinsame Bausteine des Rust-Workspace.
//!
//! `bot_config` stellt den TOML-Lader für den Discord-Config-Umbau bereit.
//! Die Umstellung der bestehenden Laufzeit-Aufrufstellen ist noch offen.
//! `config` bleibt bis dahin die bestehende ENV-Konfiguration.

pub mod bot_config;
pub mod config;
pub mod observability;
pub mod pyfloat;

pub use config::{Config, ConfigError, Ports};
