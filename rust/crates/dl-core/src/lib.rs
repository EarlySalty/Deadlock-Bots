//! Gemeinsame Bausteine des Rust-Workspace.
//!
//! `bot_config` stellt den TOML-Lader für den Discord-Config-Umbau bereit.
//! `config` lädt den unveränderlichen Prozessstand aus TOML. Weitere
//! Betriebsverbraucher werden über die typisierte Runtimeprojektion angebunden.

pub mod admin_settings;
pub mod bot_config;
pub mod config;
pub mod observability;
pub mod operating_config;
pub mod pyfloat;
pub mod runtime_config;
pub mod settings_catalog;

pub use config::{Config, ConfigError, Ports};
