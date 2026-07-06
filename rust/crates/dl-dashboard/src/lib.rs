//! Master-Dashboard (:8766) — Port von `service/dashboard.py`.
//!
//! Das Dashboard ist während der Migration der **OAuth-Besitzer**: Stats
//! (:8768) und Tierlist (:8771) delegieren ihre
//! Anmeldung hierher (siehe `dl-webcore::DashboardClient`). Darum wird es als
//! Erstes der Phase 9 portiert — ohne kompatiblen Auth-Provider kann der
//! Cutover der bereits portierten Webs nicht erfolgen.
//!
//! Architektur (kleine, klar getrennte Bausteine statt eines 7600-Zeilen-
//! Monolithen):
//! - [`config`] — ENV-Konfiguration (Rollen-IDs, TTLs, Tokens)
//! - [`token`] — URL-sichere Zufallstokens (`secrets.token_urlsafe`)
//! - [`oauth`] — Discord-OAuth2-Client (Authorize/Token/Profil/Connections)
//! - [`oauth_state`] — DB-gestützte delegierte States (initiate/consume)
//! - [`session`] — In-Memory-Store der `master_dash_session`-Cookies
//!
//! Die Routen-Schicht (Auth-Provider, Analytics, Steuerung)
//! baut auf diesen Bausteinen auf und kommt schrittweise dazu.

// axum-Response als Err-Variante ist groß, aber bewusst: die Guards geben die
// fertige Fehlerantwort zurück (wie in dl-broker/dl-stats).
#![allow(clippy::result_large_err)]

pub mod analytics;
pub mod auth;
pub mod authority;
pub mod config;
pub mod db;
pub mod deadlock;
pub mod insights;
pub mod names;
pub mod oauth;
pub mod oauth_state;
pub mod public;
pub mod reaction_roles;
pub mod repo_activity;
pub mod server_stats;
pub mod session;
pub mod survey;
pub mod token;
pub mod web;

pub use authority::{decide_access, AccessOutcome, MemberAccessInfo, MemberLookup};
pub use config::{AccessLevel, DashboardConfig};
pub use names::{display_name_or_default, BrokerNameResolver, NameResolver};
pub use oauth::{DiscordUser, OAuthClient, TokenResponse};
pub use oauth_state::{NewOAuthState, OAuthState, OAuthStateStore};
pub use session::{NewSession, Session, SessionStore};
pub use web::{router, DashboardApp, SESSION_COOKIE};

/// Aktuelle Unix-Zeit in ganzen Sekunden (für `oauth_states`-TTLs).
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Aktuelle Unix-Zeit als Sekundenbruch (für Session-TTLs; wie `time.time()`).
pub fn now_unix_f64() -> f64 {
    let now = chrono::Utc::now();
    now.timestamp() as f64 + f64::from(now.timestamp_subsec_micros()) / 1_000_000.0
}
