//! Web-spezifische Konfiguration. ENV-Namen identisch zum Python-Original
//! (public_stats.py / tierlist_public.py), damit dieselbe systemd-Umgebung
//! beide Welten versorgt.

#[derive(Debug, Clone)]
pub struct WebConfig {
    /// Session-Secret: PUBLIC_STATS_SESSION_SECRET → SESSIONS_ENCRYPTION_KEY
    pub session_secret: Option<String>,
    /// Secure-Flag für Cookies (PUBLIC_STATS_INSECURE_COOKIE / _COOKIE_SECURE)
    pub cookie_secure: bool,
    /// CORS-Allowlist für /api/public/* und /auth/* (PUBLIC_STATS_CORS_ORIGINS)
    pub cors_origins: Vec<String>,
    /// Interne Dashboard-Basis-URL (DASHBOARD_INTERNAL_API_BASE)
    pub dashboard_base: String,
    /// Token fürs OAuth-Relay: TURNIER_… → MAIN_BOT_… → TWITCH_…
    pub relay_token: Option<String>,
    /// Token für validate-session (der Endpunkt prüft spezifisch das Twitch-Token)
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
            relay_token: get("TURNIER_INTERNAL_API_TOKEN")
                .or_else(|| get("MAIN_BOT_INTERNAL_TOKEN"))
                .or_else(|| get("TWITCH_INTERNAL_API_TOKEN")),
            twitch_token: get("TWITCH_INTERNAL_API_TOKEN"),
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
