//! Typisierte Betriebswerte für bestehende Konstruktoren. Die Schlüssel der
//! Projektion sind ausschließlich interne Kompatibilitätsnamen, keine ENV-Map.
use crate::bot_config::{BotConfig, BotConfigError};
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

trait LookupValue {
    fn lookup_value(&self) -> String;
}
macro_rules! scalar { ($($ty:ty),*) => { $(impl LookupValue for $ty { fn lookup_value(&self) -> String { self.to_string() } })* }; }
scalar!(String, u64, i64, usize, u16, f64);
impl LookupValue for IpAddr {
    fn lookup_value(&self) -> String {
        match self {
            IpAddr::V4(value) => value.to_string(),
            IpAddr::V6(value) => format!("[{value}]"),
        }
    }
}
impl LookupValue for bool {
    fn lookup_value(&self) -> String {
        if *self { "1" } else { "0" }.into()
    }
}
impl LookupValue for PathBuf {
    fn lookup_value(&self) -> String {
        self.to_string_lossy().into_owned()
    }
}
impl<T: LookupValue> LookupValue for Vec<T> {
    fn lookup_value(&self) -> String {
        self.iter()
            .map(LookupValue::lookup_value)
            .collect::<Vec<_>>()
            .join(",")
    }
}

macro_rules! section {
    ($name:ident { $($field:ident : $ty:ty => $($key:literal)|+),* $(,)? }) => {
        #[derive(Clone, Default, Deserialize, Serialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct $name { $(pub $field: Option<$ty>,)* }
        impl $name { fn lookup(&self, key: &str) -> Option<String> { match key { $($($key)|+ => self.$field.as_ref().map(LookupValue::lookup_value),)* _ => None } } }
    };
}

section!(StartOptions {
    owner_id: u64 => "OWNER_ID",
    command_prefix: String => "COMMAND_PREFIX",
    command_sync: bool => "DL_BOT_COMMAND_SYNC" | "COMMAND_SYNC_ON_START",
    command_scope: String => "DL_BOT_COMMAND_START_SCOPE" | "DL_BOT_COMMAND_SCOPE" | "COMMAND_SYNC_START_SCOPE",
    command_guild_id: u64 => "DL_BOT_COMMAND_GUILD_ID" | "GUILD_ID",
    presence_intent: bool => "DL_ENABLE_PRESENCE_INTENT",
    pid_file: PathBuf => "DL_BOT_PID_FILE",
    broker_host: IpAddr => "MASTER_BROKER_HOST",
    broker_channels: Vec<u64> => "MASTER_BROKER_ALLOWED_CHANNEL_IDS" | "MASTER_BROKER_ALLOW_CHANNEL_IDS" | "MASTER_BROKER_CHANNEL_ALLOWLIST_IDS",
    broker_guilds: Vec<u64> => "MASTER_BROKER_ALLOWED_GUILD_IDS" | "MASTER_BROKER_ALLOW_GUILD_IDS" | "MASTER_BROKER_GUILD_ALLOWLIST_IDS",
    broker_roles: Vec<u64> => "MASTER_BROKER_ALLOWED_ROLE_IDS" | "MASTER_BROKER_ALLOW_ROLE_IDS" | "MASTER_BROKER_ROLE_ALLOWLIST_IDS",
    broker_idempotency_ttl_seconds: f64 => "MASTER_BROKER_IDEMPOTENCY_TTL_SECONDS",
    broker_inflight_ttl_seconds: f64 => "MASTER_BROKER_IDEMPOTENCY_INFLIGHT_TTL_SECONDS",
    broker_waiter_timeout_seconds: f64 => "MASTER_BROKER_IDEMPOTENCY_WAITER_TIMEOUT_SECONDS",
    broker_idempotency_max_entries: usize => "MASTER_BROKER_IDEMPOTENCY_MAX_ENTRIES",
    mcp_host: IpAddr => "MCP_CONNECTOR_HOST",
    mcp_port: u16 => "MCP_CONNECTOR_PORT",
    mcp_guild_id: u64 => "MCP_DEFAULT_GUILD_ID",
    mcp_export_dir: PathBuf => "MCP_EXPORT_DIR",
});

section!(BridgeOptions {
    twitch_api_url: String => "TWITCH_INTERNAL_API_BASE_URL",
    twitch_host: IpAddr => "TWITCH_INTERNAL_API_HOST",
    twitch_port: u16 => "TWITCH_INTERNAL_API_PORT",
    twitch_timeout_seconds: f64 => "TWITCH_INTERNAL_API_TIMEOUT_SEC",
    twitch_allow_non_loopback: bool => "TWITCH_INTERNAL_API_ALLOW_NON_LOOPBACK",
    turnier_api_url: String => "TURNIER_INTERNAL_API_BASE_URL",
    website_api_url: String => "WEBSITE_API_BASE",
    matcher_enabled: bool => "STREAMER_LINK_ENABLED",
    matcher_guild_id: u64 => "STREAMER_GUILD_ID",
    matcher_notify_channel_id: u64 => "STREAMER_LINK_NOTIFY_CHANNEL_ID",
    matcher_role_id: u64 => "STREAMER_ROLE_ID",
    matcher_auto_threshold: i64 => "STREAMER_LINK_AUTO_THRESHOLD",
    matcher_review_threshold: i64 => "STREAMER_LINK_REVIEW_THRESHOLD",
    matcher_fuzzy_floor: f64 => "STREAMER_LINK_FUZZY_FLOOR",
    matcher_max_ai_per_scan: u64 => "STREAMER_LINK_MAX_AI_PER_SCAN",
    matcher_scan_interval_hours: u64 => "STREAMER_LINK_SCAN_INTERVAL_HOURS",
    matcher_state_path: PathBuf => "STREAMER_LINK_STATE_PATH",
    matcher_ai_provider: String => "STREAMER_LINK_AI_PROVIDER",
});

section!(DashboardOptions {
    discord_client_id: String => "DISCORD_OAUTH_CLIENT_ID",
    host: IpAddr => "DASHBOARD_HOST",
    public_url: String => "MASTER_DASHBOARD_PUBLIC_URL",
    listen_url: String => "MASTER_DASHBOARD_LISTEN_URL",
    broker_url: String => "MASTER_BROKER_BASE_URL",
    allowed_origins: Vec<String> => "MASTER_DASHBOARD_ALLOWED_ORIGINS",
    auth_guild_ids: Vec<u64> => "MASTER_DASHBOARD_AUTH_GUILD_IDS",
    owner_user_id: u64 => "MASTER_DASHBOARD_OWNER_USER_ID",
    moderator_role_id: u64 => "MASTER_DASHBOARD_MODERATOR_ROLE_ID",
    audit_bot_user_id: u64 => "DISCORD_BOT_USER_ID",
    session_ttl_seconds: i64 => "MASTER_DASHBOARD_SESSION_TTL_SEC",
    oauth_state_ttl_seconds: i64 => "MASTER_DASHBOARD_OAUTH_STATE_TTL_SEC" | "DEADLOCK_OAUTH_STATE_TTL_SECONDS",
    discord_redirect_uri: String => "MASTER_DASHBOARD_DISCORD_REDIRECT_URI",
    discord_api_base: String => "DISCORD_API_BASE",
    data_dir: PathBuf => "DEADLOCK_DB_DIR",
    wiki_root: PathBuf => "DL_BRAIN_WIKI_ROOT",
    insights_guild_id: u64 => "DISCORD_INSIGHTS_GUILD_ID",
    insights_archive_dir: PathBuf => "INSIGHTS_ARCHIVE_DIR",
});

section!(WebOptions {
    cookie_secure: bool => "PUBLIC_STATS_COOKIE_SECURE",
    insecure_cookie: bool => "PUBLIC_STATS_INSECURE_COOKIE",
    cors_origins: Vec<String> => "PUBLIC_STATS_CORS_ORIGINS",
    dashboard_base: String => "DASHBOARD_INTERNAL_API_BASE",
    callback_url: String => "PUBLIC_STATS_CALLBACK_URL",
    stats_host: IpAddr => "PUBLIC_STATS_HOST",
    tierlist_host: IpAddr => "TIERLIST_PUBLIC_HOST",
    static_dir: PathBuf => "DL_STATIC_DIR",
    tierlist_refresh: bool => "DL_TIERLIST_REFRESH",
});

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub start: StartOptions,
    pub bridges: BridgeOptions,
    pub dashboard: DashboardOptions,
    pub web: WebOptions,
}

impl RuntimeConfig {
    pub fn lookup(&self, key: &str) -> Option<String> {
        self.start
            .lookup(key)
            .or_else(|| self.bridges.lookup(key))
            .or_else(|| self.dashboard.lookup(key))
            .or_else(|| self.web.lookup(key))
    }

    pub fn resolve_paths(&mut self, base: &Path) {
        for path in [
            &mut self.start.pid_file,
            &mut self.start.mcp_export_dir,
            &mut self.bridges.matcher_state_path,
            &mut self.dashboard.data_dir,
            &mut self.dashboard.wiki_root,
            &mut self.dashboard.insights_archive_dir,
            &mut self.web.static_dir,
        ]
        .into_iter()
        .flatten()
        {
            if path.is_relative() {
                *path = base.join(&*path);
            }
        }
    }

    pub fn validate(&self) -> Result<(), BotConfigError> {
        let invalid = || BotConfigError::Validation("Ungültiger Betriebswert in runtime");
        if self
            .dashboard
            .discord_client_id
            .as_deref()
            .is_some_and(|value| {
                value
                    .parse::<u64>()
                    .map_or(true, |id| id == 0 || id > i64::MAX as u64)
            })
            || self
                .start
                .command_prefix
                .as_deref()
                .is_some_and(|value| value.trim().is_empty() || value.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        for seconds in [
            self.start.broker_idempotency_ttl_seconds,
            self.start.broker_inflight_ttl_seconds,
            self.start.broker_waiter_timeout_seconds,
        ]
        .into_iter()
        .flatten()
        {
            if !seconds.is_finite() || !(0.001..=31_536_000.0).contains(&seconds) {
                return Err(invalid());
            }
        }
        if self
            .start
            .broker_idempotency_max_entries
            .is_some_and(|value| value == 0 || value > 1_000_000)
        {
            return Err(invalid());
        }
        if self
            .start
            .command_scope
            .as_deref()
            .is_some_and(|value| !matches!(value, "guild" | "global" | "both"))
        {
            return Err(invalid());
        }
        for port in [self.start.mcp_port, self.bridges.twitch_port]
            .into_iter()
            .flatten()
        {
            if port == 0 {
                return Err(invalid());
            }
        }
        for host in [
            self.start.broker_host,
            self.start.mcp_host,
            self.dashboard.host,
            self.web.stats_host,
            self.web.tierlist_host,
        ]
        .into_iter()
        .flatten()
        {
            if !host.is_loopback() {
                return Err(invalid());
            }
        }
        for ttl in [
            self.dashboard.session_ttl_seconds,
            self.dashboard.oauth_state_ttl_seconds,
        ]
        .into_iter()
        .flatten()
        {
            if !(1..=31_536_000).contains(&ttl) {
                return Err(invalid());
            }
        }
        if self
            .bridges
            .twitch_timeout_seconds
            .is_some_and(|value| !value.is_finite() || !(0.5..=3600.0).contains(&value))
            || self
                .bridges
                .matcher_fuzzy_floor
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(invalid());
        }
        for threshold in [
            self.bridges.matcher_auto_threshold,
            self.bridges.matcher_review_threshold,
        ]
        .into_iter()
        .flatten()
        {
            if !(0..=100).contains(&threshold) {
                return Err(invalid());
            }
        }
        for id in [
            self.start.owner_id,
            self.start.command_guild_id,
            self.start.mcp_guild_id,
            self.bridges.matcher_guild_id,
            self.bridges.matcher_notify_channel_id,
            self.bridges.matcher_role_id,
            self.dashboard.owner_user_id,
            self.dashboard.moderator_role_id,
            self.dashboard.audit_bot_user_id,
            self.dashboard.insights_guild_id,
        ]
        .into_iter()
        .flatten()
        {
            if id == 0 || id > i64::MAX as u64 {
                return Err(invalid());
            }
        }
        for list in [
            &self.start.broker_channels,
            &self.start.broker_guilds,
            &self.start.broker_roles,
            &self.dashboard.auth_guild_ids,
        ]
        .into_iter()
        .flatten()
        {
            if list.iter().any(|id| *id == 0 || *id > i64::MAX as u64) {
                return Err(invalid());
            }
        }
        for path in [
            &self.start.pid_file,
            &self.start.mcp_export_dir,
            &self.bridges.matcher_state_path,
            &self.dashboard.data_dir,
            &self.dashboard.wiki_root,
            &self.dashboard.insights_archive_dir,
            &self.web.static_dir,
        ]
        .into_iter()
        .flatten()
        {
            if path.as_os_str().is_empty() || path.to_string_lossy().chars().any(char::is_control) {
                return Err(invalid());
            }
        }
        for (address, local) in [
            (
                &self.bridges.twitch_api_url,
                !self.bridges.twitch_allow_non_loopback.unwrap_or(false),
            ),
            (&self.bridges.turnier_api_url, true),
            (&self.bridges.website_api_url, false),
            (&self.dashboard.public_url, false),
            (&self.dashboard.listen_url, true),
            (&self.dashboard.broker_url, true),
            (&self.dashboard.discord_redirect_uri, false),
            (&self.dashboard.discord_api_base, false),
            (&self.web.dashboard_base, true),
            (&self.web.callback_url, false),
        ] {
            if let Some(address) = address {
                validate_url(address, local)?;
            }
        }
        for origins in [&self.dashboard.allowed_origins, &self.web.cors_origins]
            .into_iter()
            .flatten()
        {
            for origin in origins {
                validate_url(origin, false)?;
            }
        }
        Ok(())
    }
}

fn validate_url(raw: &str, local: bool) -> Result<(), BotConfigError> {
    let invalid =
        || BotConfigError::Validation("Betriebs-URL ist ungültig oder enthält Zugangsdaten");
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() == Some(0)
        || (local
            && !url
                .host_str()
                .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback()))
    {
        return Err(invalid());
    }
    Ok(())
}

impl BotConfig {
    pub fn runtime_value(&self, key: &str) -> Option<String> {
        match key {
            "MAIN_GUILD_ID" | "OUR_GUILD_ID" => self.discord.guild_id.clone(),
            "DL_BOT_GATEWAY" => Some(self.features.gateway.lookup_value()),
            "STEAM_BOT_API_URL" => Some(self.services.steam_api_url.clone()),
            "MASTER_BROKER_BASE_URL" => Some(
                self.runtime
                    .dashboard
                    .broker_url
                    .clone()
                    .unwrap_or_else(|| {
                        format!(
                            "http://{}:{}",
                            self.runtime
                                .start
                                .broker_host
                                .map(|host| host.lookup_value())
                                .unwrap_or_else(|| "127.0.0.1".into()),
                            self.services.master_broker_port
                        )
                    }),
            ),
            "DASHBOARD_INTERNAL_API_BASE" | "MASTER_DASHBOARD_LISTEN_URL" => {
                let explicit = if key == "DASHBOARD_INTERNAL_API_BASE" {
                    &self.runtime.web.dashboard_base
                } else {
                    &self.runtime.dashboard.listen_url
                };
                Some(explicit.clone().unwrap_or_else(|| {
                    format!(
                        "http://{}:{}",
                        self.runtime
                            .dashboard
                            .host
                            .map(|host| host.lookup_value())
                            .unwrap_or_else(|| "127.0.0.1".into()),
                        self.services.dashboard_port
                    )
                }))
            }
            "DEADLOCK_DB_PATH" => Some(self.storage.legacy_snapshot_path.lookup_value()),
            _ => self.runtime.lookup(key),
        }
    }
}

/// Nur bestehende Infisical-Zugangswerte dürfen aus dem Secret-Bootstrap kommen.
/// Unbekannte Namen sind keine heimlichen Betriebs-ENV-Overrides.
pub fn secret_value(key: &str) -> Option<String> {
    match key {
        "DISCORD_TOKEN"
        | "DISCORD_TOKEN_RANKED"
        | "CHANGELOG_API_TOKEN"
        | "MASTER_BROKER_TOKEN"
        | "MAIN_BOT_INTERNAL_TOKEN"
        | "TWITCH_INTERNAL_API_TOKEN"
        | "STEAM_INTERNAL_API_TOKEN"
        | "INTERNAL_API_TOKEN"
        | "TURNIER_INTERNAL_API_TOKEN"
        | "SERVERSYNC_INTERNAL_TOKEN"
        | "MCP_CONNECTOR_TOKEN"
        | "COACHING_BOT_TOKEN"
        | "DISCORD_OAUTH_CLIENT_SECRET"
        | "PUBLIC_STATS_SESSION_SECRET"
        | "SESSIONS_ENCRYPTION_KEY"
        | "DISCORD_INSIGHTS_USER_TOKEN"
        | "OPENAI_API_KEY"
        | "DEADLOCK_OPENAI_KEY"
        | "FIREWORK_API_KEY"
        | "FIREWORKS_API_KEY"
        | "MISTRAL_API_KEY"
        | "GEMINI_API_KEY"
        | "GOOGLE_API_KEY"
        | "MINIMAX_API_KEY"
        | "MINMAX"
        | "MINIMAX_TOKEN_PLAN_KEY" => std::env::var(key)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
        _ => None,
    }
}

pub fn lookup(key: &str) -> Option<String> {
    secret_value(key).or_else(|| {
        crate::config::process_bot_config()
            .ok()?
            .snapshot()
            .runtime_value(key)
    })
}
