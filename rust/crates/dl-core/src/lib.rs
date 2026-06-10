//! Fundament-Crate des Rust-Rewrites.
//!
//! Enthält ausschließlich Querschnitts-Bausteine ohne Domänenlogik:
//! - [`config`]: die EINE typisierte Laufzeit-Konfiguration (ersetzt die
//!   166 verstreuten `os.getenv`-Aufrufe des Python-Originals)
//! - [`observability`]: Tracing-Initialisierung für die Binaries

pub mod config;
pub mod observability;

pub use config::{Config, ConfigError, Ports};
