//! Konfiguration des Master-Dashboards — ENV-Namen und Defaults exakt wie
//! `service/dashboard.py` (Owner-ID, Rollen-IDs, TTLs, OAuth-Redirect).
//!
//! Im Python-Original liegen Client-ID/-Secret im OS-Keyring mit ENV-
//! Fallback. dl-web ist ein reiner Web-Prozess ohne Keyring-Anbindung; wir
//! lesen ausschließlich die ENV-Variablen, die der Infisical-Loader ohnehin
//! setzt. Werte werden über eine Lookup-Funktion bezogen, damit Tests die
//! Umgebung nicht anfassen müssen.

/// Discord-Server-Owner mit Voll-Zugriff (DEFAULT_DASHBOARD_OWNER_USER_ID).
pub const DEFAULT_OWNER_USER_ID: u64 = 662995601738170389;
/// Moderator-Rolle → Voll-Zugriff (DEFAULT_DASHBOARD_MODERATOR_ROLE_ID).
pub const DEFAULT_MODERATOR_ROLE_ID: u64 = 1337518124647579661;

const DEFAULT_SESSION_TTL_SECONDS: i64 = 1_209_600; // 14 Tage
const DEFAULT_OAUTH_STATE_TTL_SECONDS: i64 = 21_600; // 6 Stunden
const DEFAULT_DISCORD_REDIRECT_URI: &str =
    "https://deutsche-deadlock-community.de/callback/discord";
const DEFAULT_PUBLIC_BASE_URL: &str = "https://admin.deutsche-deadlock-community.de";
const DEFAULT_LISTEN_BASE_URL: &str = "http://127.0.0.1:8766";
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const DEFAULT_BROKER_BASE: &str = "http://127.0.0.1:8770";

/// Zugriffsstufe einer Dashboard-Session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessLevel {
    /// Voller Zugriff (Owner, Admin, Moderator-Rolle).
    Full,
    /// Gültige Session ohne Vollzugriff (z. B. importierte turnier_only-Session).
    TurnierOnly,
}

impl AccessLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            AccessLevel::Full => "full",
            AccessLevel::TurnierOnly => "turnier_only",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DashboardConfig {
    pub discord_client_id: Option<String>,
    pub discord_client_secret: Option<String>,
    pub discord_redirect_uri: String,
    pub owner_user_id: u64,
    pub moderator_role_id: u64,
    /// Gilden, in denen Admin-/Rollen-Status geprüft wird. Leer = alle
    /// Bot-Gilden (Broker entscheidet).
    pub auth_guild_ids: Vec<u64>,
    pub session_ttl_secs: i64,
    /// TTL der DB-gestützten delegierten OAuth-States (initiate/consume).
    pub oauth_state_ttl_secs: i64,
    /// Erlaubte Request-Origins für CSRF-/Origin-Prüfung (leer = keine
    /// Einschränkung, plus die Basis-URLs unten).
    pub allowed_origins: Vec<String>,
    pub public_base_url: Option<String>,
    pub listen_base_url: String,
    pub discord_api_base: String,
    /// Basis-URL des Master-Brokers (Member-Access-Lookup für den Login).
    pub broker_base: String,
    /// Lokales Datenverzeichnis fuer Datei-Artefakte neben der frueheren
    /// SQLite-DB (`repo/data`).
    pub data_dir: PathBuf,
    /// Tokens für die turnier-/allgemeinen internen Routen
    /// (initiate/consume/authorize-url/session).
    pub turnier_tokens: Vec<String>,
    /// Tokens für die Twitch-internen Routen (validate/import-session).
    pub twitch_tokens: Vec<String>,
}

impl DashboardConfig {
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let get = |key: &str| -> Option<String> {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };

        let turnier_tokens = collect_tokens(
            &get,
            &[
                "TURNIER_INTERNAL_API_TOKEN",
                "MASTER_BROKER_TOKEN",
                "MAIN_BOT_INTERNAL_TOKEN",
                "TWITCH_INTERNAL_API_TOKEN",
            ],
        );
        let twitch_tokens = collect_tokens(&get, &["TWITCH_INTERNAL_API_TOKEN"]);

        let public_base_url = Some(
            get("MASTER_DASHBOARD_PUBLIC_URL")
                .unwrap_or_else(|| DEFAULT_PUBLIC_BASE_URL.to_string()),
        );
        let listen_base_url = get("MASTER_DASHBOARD_LISTEN_URL")
            .unwrap_or_else(|| DEFAULT_LISTEN_BASE_URL.to_string());
        let allowed_origins = build_allowed_origins(
            public_base_url.as_deref(),
            &listen_base_url,
            get("MASTER_DASHBOARD_ALLOWED_ORIGINS").as_deref(),
        );
        let data_dir = resolve_data_dir(&get);

        Self {
            discord_client_id: get("DISCORD_OAUTH_CLIENT_ID"),
            discord_client_secret: get("DISCORD_OAUTH_CLIENT_SECRET"),
            discord_redirect_uri: get("MASTER_DASHBOARD_DISCORD_REDIRECT_URI")
                .unwrap_or_else(|| DEFAULT_DISCORD_REDIRECT_URI.to_string()),
            owner_user_id: get("MASTER_DASHBOARD_OWNER_USER_ID")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_OWNER_USER_ID),
            moderator_role_id: get("MASTER_DASHBOARD_MODERATOR_ROLE_ID")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MODERATOR_ROLE_ID),
            auth_guild_ids: get("MASTER_DASHBOARD_AUTH_GUILD_IDS")
                .map(|v| parse_id_list(&v))
                .unwrap_or_default(),
            session_ttl_secs: get("MASTER_DASHBOARD_SESSION_TTL_SEC")
                .and_then(|v| v.parse().ok())
                .filter(|v: &i64| *v > 0)
                .unwrap_or(DEFAULT_SESSION_TTL_SECONDS),
            oauth_state_ttl_secs: get("MASTER_DASHBOARD_OAUTH_STATE_TTL_SEC")
                .or_else(|| get("DEADLOCK_OAUTH_STATE_TTL_SECONDS"))
                .and_then(|v| v.parse().ok())
                .filter(|v: &i64| *v > 0)
                .unwrap_or(DEFAULT_OAUTH_STATE_TTL_SECONDS),
            allowed_origins,
            public_base_url,
            listen_base_url,
            discord_api_base: get("DISCORD_API_BASE")
                .unwrap_or_else(|| DISCORD_API_BASE.to_string()),
            broker_base: get("MASTER_BROKER_BASE_URL")
                .unwrap_or_else(|| DEFAULT_BROKER_BASE.to_string()),
            data_dir,
            turnier_tokens,
            twitch_tokens,
        }
    }

    /// Discord OAuth ist nur mit beiden Credentials benutzbar.
    pub fn discord_oauth_configured(&self) -> bool {
        self.discord_client_id.is_some() && self.discord_client_secret.is_some()
    }

    /// Python setzt `_discord_auth_enabled = True`: fehlende Credentials sind
    /// deshalb keine Deaktivierung, sondern eine Fehlkonfiguration.
    pub fn auth_misconfigured(&self) -> bool {
        !self.discord_oauth_configured()
    }

    /// Auth bleibt erzwungen, auch wenn OAuth falsch konfiguriert ist. Die
    /// Handler liefern dann 503 statt das Dashboard offen auszuliefern.
    pub fn auth_enforced(&self) -> bool {
        true
    }

    /// Prüft ein Token gegen die turnier-/allgemeinen internen Routen.
    pub fn accepts_turnier_token(&self, token: &str) -> bool {
        token_matches(&self.turnier_tokens, token)
    }

    /// Prüft ein Token gegen die Twitch-internen Routen.
    pub fn accepts_twitch_token(&self, token: &str) -> bool {
        token_matches(&self.twitch_tokens, token)
    }
}

fn collect_tokens(get: &impl Fn(&str) -> Option<String>, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        if let Some(value) = get(key) {
            if !out.contains(&value) {
                out.push(value);
            }
        }
    }
    out
}

fn token_matches(tokens: &[String], candidate: &str) -> bool {
    let candidate = candidate.trim();
    !candidate.is_empty() && tokens.iter().any(|t| t == candidate)
}

fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn normalize_origin(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let mut origin = format!("{}://{}", parsed.scheme(), host);
    if let Some(port) = parsed.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Some(origin)
}

fn build_allowed_origins(
    public_base_url: Option<&str>,
    listen_base_url: &str,
    extra_raw: Option<&str>,
) -> Vec<String> {
    let mut origins = Vec::new();
    for raw in [public_base_url, Some(listen_base_url)]
        .into_iter()
        .flatten()
    {
        if let Some(origin) = normalize_origin(raw) {
            push_unique(&mut origins, origin);
        }
    }
    for raw in extra_raw.into_iter().flat_map(split_csv) {
        if let Some(origin) = normalize_origin(&raw) {
            push_unique(&mut origins, origin);
        }
    }
    origins
}

fn resolve_data_dir(get: &impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(dir) = get("DEADLOCK_DB_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(path) = get("DEADLOCK_DB_PATH") {
        if let Some(parent) = PathBuf::from(path).parent() {
            return parent.to_path_buf();
        }
    }
    PathBuf::from("data")
}

fn parse_id_list(raw: &str) -> Vec<u64> {
    raw.split(&[',', ' '][..])
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup<'a>(map: &'a HashMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| map.get(key).map(|v| v.to_string())
    }

    #[test]
    fn defaults_ohne_env() {
        let cfg = DashboardConfig::from_lookup(|_| None);
        assert_eq!(cfg.owner_user_id, DEFAULT_OWNER_USER_ID);
        assert_eq!(cfg.moderator_role_id, DEFAULT_MODERATOR_ROLE_ID);
        assert_eq!(cfg.session_ttl_secs, DEFAULT_SESSION_TTL_SECONDS);
        assert_eq!(cfg.oauth_state_ttl_secs, DEFAULT_OAUTH_STATE_TTL_SECONDS);
        assert!(cfg.auth_enforced());
        assert!(cfg.auth_misconfigured());
        assert!(!cfg.discord_oauth_configured());
        assert_eq!(cfg.discord_redirect_uri, DEFAULT_DISCORD_REDIRECT_URI);
        assert_eq!(
            cfg.public_base_url.as_deref(),
            Some(DEFAULT_PUBLIC_BASE_URL)
        );
        assert!(cfg
            .allowed_origins
            .contains(&"https://admin.deutsche-deadlock-community.de".to_string()));
        assert!(cfg
            .allowed_origins
            .contains(&"http://127.0.0.1:8766".to_string()));
    }

    #[test]
    fn auth_erzwungen_wenn_creds_da() {
        let map = HashMap::from([
            ("DISCORD_OAUTH_CLIENT_ID", "123"),
            ("DISCORD_OAUTH_CLIENT_SECRET", "shh"),
        ]);
        let cfg = DashboardConfig::from_lookup(lookup(&map));
        assert!(cfg.auth_enforced());
        assert!(!cfg.auth_misconfigured());
        assert!(cfg.discord_oauth_configured());
    }

    #[test]
    fn auth_fail_closed_bei_unvollstaendigen_creds() {
        let map = HashMap::from([("DISCORD_OAUTH_CLIENT_ID", "123")]);
        let cfg = DashboardConfig::from_lookup(lookup(&map));
        assert!(cfg.auth_enforced());
        assert!(cfg.auth_misconfigured());
        assert!(!cfg.discord_oauth_configured());
    }

    #[test]
    fn turnier_token_fallbackkette_dedupliziert() {
        // Nur MASTER_BROKER_TOKEN gesetzt → landet in der Turnier-Kette.
        let map = HashMap::from([("MASTER_BROKER_TOKEN", "brk")]);
        let cfg = DashboardConfig::from_lookup(lookup(&map));
        assert!(cfg.accepts_turnier_token("brk"));
        assert!(!cfg.accepts_turnier_token(""));
        assert!(!cfg.accepts_twitch_token("brk"));

        // Gleicher Wert in mehreren Variablen → nur einmal.
        let map = HashMap::from([
            ("TURNIER_INTERNAL_API_TOKEN", "same"),
            ("TWITCH_INTERNAL_API_TOKEN", "same"),
        ]);
        let cfg = DashboardConfig::from_lookup(lookup(&map));
        assert_eq!(cfg.turnier_tokens.len(), 1);
        assert!(cfg.accepts_twitch_token("same"));
    }

    #[test]
    fn guild_und_origin_listen() {
        let map = HashMap::from([
            ("MASTER_DASHBOARD_AUTH_GUILD_IDS", "111, 222 333"),
            (
                "MASTER_DASHBOARD_ALLOWED_ORIGINS",
                "https://a.de, https://b.de",
            ),
        ]);
        let cfg = DashboardConfig::from_lookup(lookup(&map));
        assert_eq!(cfg.auth_guild_ids, vec![111, 222, 333]);
        assert!(cfg.allowed_origins.contains(&"https://a.de".to_string()));
        assert!(cfg.allowed_origins.contains(&"https://b.de".to_string()));
    }
}
use std::path::PathBuf;
