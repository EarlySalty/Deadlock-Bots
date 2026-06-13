//! Master-Dashboard (:8766) — Port von `service/dashboard.py`.
//!
//! Das Dashboard ist während der Migration der **OAuth-Besitzer**: Stats
//! (:8768), Tierlist (:8771) und Turnier-Web (:8767) delegieren ihre
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
//! Die Routen-Schicht (Auth-Provider, Analytics, Turnier-Admin, Steuerung)
//! baut auf diesen Bausteinen auf und kommt schrittweise dazu.

pub mod config;
pub mod oauth;
pub mod oauth_state;
pub mod session;
pub mod token;

pub use config::{AccessLevel, DashboardConfig};
pub use oauth::{DiscordUser, OAuthClient, TokenResponse};
pub use oauth_state::{NewOAuthState, OAuthState, OAuthStateStore};
pub use session::{NewSession, Session, SessionStore};

/// Aktuelle Unix-Zeit in ganzen Sekunden (für `oauth_states`-TTLs).
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Aktuelle Unix-Zeit als Sekundenbruch (für Session-TTLs; wie `time.time()`).
pub fn now_unix_f64() -> f64 {
    let now = chrono::Utc::now();
    now.timestamp() as f64 + f64::from(now.timestamp_subsec_micros()) / 1_000_000.0
}
