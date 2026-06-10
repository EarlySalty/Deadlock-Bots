//! Gemeinsame Web-Infrastruktur für dl-web (und später den Broker).
//!
//! Im Python-Original ist diese Schicht 4× kopiert (Dashboard, Public-Stats,
//! Tierlist, Turnier) — hier existiert sie genau einmal:
//! - [`session`] — HMAC-signierte Session-Cookies, byte-kompatibel zu
//!   public_stats.py (`<b64url-payload>.<hmac-sha256-hex>`)
//! - [`dashboard`] — Client für die interne Dashboard-API (OAuth-Relay,
//!   Session-Validierung), solange das Python-Dashboard der OAuth-Besitzer ist
//! - [`envelope`] — einheitliche JSON-Fehlerantworten
//! - [`client_ip`] — Client-IP-Ermittlung (X-Forwarded-For)
//! - [`config`] — Web-spezifische ENV-Konfiguration (Namen wie im Original)

pub mod client_ip;
pub mod config;
pub mod dashboard;
pub mod envelope;
pub mod session;

pub use config::WebConfig;
pub use dashboard::DashboardClient;
pub use session::SessionCodec;
