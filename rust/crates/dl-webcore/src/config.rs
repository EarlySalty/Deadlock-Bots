//! Web-spezifische Konfiguration. ENV-Namen identisch zum Python-Original
//! (public_stats.py / tierlist_public.py), damit dieselbe systemd-Umgebung
//! beide Welten versorgt.

#[derive(Clone)]
pub struct WebConfig {
    /// Session-Secret: PUBLIC_STATS_SESSION_SECRET → SESSIONS_ENCRYPTION_KEY
    pub session_secret: Option<String>,
    /// Secure-Flag für Cookies (PUBLIC_STATS_INSECURE_COOKIE / _COOKIE_SECURE)
    pub cookie_secure: bool,
    /// CORS-Allowlist für /api/public/* und /auth/* (PUBLIC_STATS_CORS_ORIGINS)
    pub cors_origins: Vec<String>,
    /// Interne Dashboard-Basis-URL (DASHBOARD_INTERNAL_API_BASE)
    pub dashboard_base: String,
    /// Token fürs OAuth-Relay: erster Wert aus `internal_tokens::DASHBOARD_INTERNAL_RECEIVER`
    pub relay_token: Option<String>,
    /// Token für validate-session: `internal_tokens::TWITCH_DASHBOARD_RECEIVER`
    pub twitch_token: Option<String>,
    /// Öffentliche Callback-URL des Stats-Auth-Flows (PUBLIC_STATS_CALLBACK_URL)
    pub stats_callback_url: String,
    /// Bind-Hosts (Python: PUBLIC_STATS_HOST env; Tierlist hardcoded 127.0.0.1)
    pub stats_host: String,
    pub tierlist_host: String,
    /// Verzeichnis mit activity_stats.html + rank_icons/ (DL_STATIC_DIR;
    /// Default zeigt auf das Bestands-Verzeichnis des Python-Originals)
    pub static_dir: String,
    /// Tierlist-Refresh-Loop aktiv? (DL_TIERLIST_REFRESH; beim Parallel-Test
    /// gegen die Prod-DB ausschalten, sonst schreiben zwei Welten Snapshots)
    pub tierlist_refresh_enabled: bool,
}

impl WebConfig {
    pub fn from_env() -> Self {
        Self::from_lookup(dl_core::runtime_config::lookup)
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let get = |key: &str| -> Option<String> {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };

        // Semantik wie public_stats._env_flag: gesetzt → Wert ∈ {1,true,yes,on};
        // nicht gesetzt → Default.
        let flag = |key: &str, default: bool| -> bool {
            match lookup(key) {
                None => default,
                Some(raw) => matches!(
                    raw.trim().to_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                ),
            }
        };

        let cookie_secure = if flag("PUBLIC_STATS_INSECURE_COOKIE", false) {
            false
        } else {
            flag("PUBLIC_STATS_COOKIE_SECURE", true)
        };

        let cors_origins = match get("PUBLIC_STATS_CORS_ORIGINS") {
            Some(csv) => csv
                .split(',')
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            None => default_cors_origins(),
        };

        Self {
            session_secret: get("PUBLIC_STATS_SESSION_SECRET")
                .or_else(|| get("SESSIONS_ENCRYPTION_KEY")),
            cookie_secure,
            cors_origins,
            dashboard_base: get("DASHBOARD_INTERNAL_API_BASE")
                .unwrap_or_else(|| "http://127.0.0.1:8766".to_string()),
            relay_token: dl_core::internal_tokens::first_token(
                &lookup,
                dl_core::internal_tokens::DASHBOARD_INTERNAL_RECEIVER,
            ),
            twitch_token: dl_core::internal_tokens::first_token(
                &lookup,
                dl_core::internal_tokens::TWITCH_DASHBOARD_RECEIVER,
            ),
            stats_callback_url: get("PUBLIC_STATS_CALLBACK_URL").unwrap_or_else(|| {
                "https://deutsche-deadlock-community.de/aktivitaet/auth/discord/complete"
                    .to_string()
            }),
            stats_host: get("PUBLIC_STATS_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            tierlist_host: get("TIERLIST_PUBLIC_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            static_dir: get("DL_STATIC_DIR").unwrap_or_else(|| "service/static".to_string()),
            tierlist_refresh_enabled: flag("DL_TIERLIST_REFRESH", true),
        }
    }
}

impl std::fmt::Debug for WebConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebConfig")
            .field(
                "session_secret",
                &secret_state(self.session_secret.as_deref()),
            )
            .field("cookie_secure", &self.cookie_secure)
            .field("cors_origins", &self.cors_origins)
            .field("dashboard_base", &self.dashboard_base)
            .field("relay_token", &secret_state(self.relay_token.as_deref()))
            .field("twitch_token", &secret_state(self.twitch_token.as_deref()))
            .field("stats_callback_url", &self.stats_callback_url)
            .field("stats_host", &self.stats_host)
            .field("tierlist_host", &self.tierlist_host)
            .field("static_dir", &self.static_dir)
            .field("tierlist_refresh_enabled", &self.tierlist_refresh_enabled)
            .finish()
    }
}

fn secret_state(value: Option<&str>) -> &'static str {
    if value.is_some() {
        "<gesetzt>"
    } else {
        "<fehlt>"
    }
}

fn default_cors_origins() -> Vec<String> {
    let mut origins = Vec::new();
    for port in 5173..=5175 {
        origins.push(format!("http://localhost:{port}"));
        origins.push(format!("http://127.0.0.1:{port}"));
    }
    origins
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn debug_zeigt_keine_geheimnisse() {
        let map = HashMap::from([
            ("PUBLIC_STATS_SESSION_SECRET", "sitzung-wert-31a"),
            ("TURNIER_INTERNAL_API_TOKEN", "relay-wert-8b2"),
            ("TWITCH_INTERNAL_API_TOKEN", " twitch-wert-5e7 "),
        ]);
        let cfg = WebConfig::from_lookup(|key| map.get(key).map(|v| v.to_string()));
        assert_eq!(cfg.twitch_token.as_deref(), Some("twitch-wert-5e7"));
        let debug = format!("{cfg:?}");
        for secret in ["sitzung-wert-31a", "relay-wert-8b2", "twitch-wert-5e7"] {
            assert!(!debug.contains(secret), "{debug}");
        }
    }

    #[test]
    fn defaults() {
        let cfg = WebConfig::from_lookup(|_| None);
        assert!(cfg.session_secret.is_none());
        assert!(cfg.cookie_secure);
        assert_eq!(cfg.cors_origins.len(), 6);
        assert_eq!(cfg.dashboard_base, "http://127.0.0.1:8766");
        assert_eq!(cfg.stats_host, "127.0.0.1");
        assert!(cfg.tierlist_refresh_enabled);
    }

    #[test]
    fn insecure_cookie_flag_gewinnt() {
        let map = HashMap::from([
            ("PUBLIC_STATS_INSECURE_COOKIE", "1"),
            ("PUBLIC_STATS_COOKIE_SECURE", "1"),
        ]);
        let cfg = WebConfig::from_lookup(|k| map.get(k).map(|v| v.to_string()));
        assert!(!cfg.cookie_secure);
    }

    #[test]
    fn token_fallback_kette() {
        let map = HashMap::from([("TWITCH_INTERNAL_API_TOKEN", "tw-token")]);
        let cfg = WebConfig::from_lookup(|k| map.get(k).map(|v| v.to_string()));
        assert_eq!(cfg.relay_token.as_deref(), Some("tw-token"));
        assert_eq!(cfg.twitch_token.as_deref(), Some("tw-token"));

        let map = HashMap::from([
            ("TURNIER_INTERNAL_API_TOKEN", "turnier"),
            ("TWITCH_INTERNAL_API_TOKEN", "tw"),
        ]);
        let cfg = WebConfig::from_lookup(|k| map.get(k).map(|v| v.to_string()));
        assert_eq!(cfg.relay_token.as_deref(), Some("turnier"));
        assert_eq!(cfg.twitch_token.as_deref(), Some("tw"));

        let map = HashMap::from([("MASTER_BROKER_TOKEN", " brk ")]);
        let cfg = WebConfig::from_lookup(|k| map.get(k).map(|v| v.to_string()));
        assert_eq!(cfg.relay_token.as_deref(), Some("brk"));
        assert_eq!(cfg.twitch_token, None);
    }

    #[test]
    fn cors_csv_wird_geparst() {
        let map = HashMap::from([(
            "PUBLIC_STATS_CORS_ORIGINS",
            "https://a.example, https://b.example/ ,",
        )]);
        let cfg = WebConfig::from_lookup(|k| map.get(k).map(|v| v.to_string()));
        assert_eq!(
            cfg.cors_origins,
            vec!["https://a.example", "https://b.example"]
        );
    }
}
