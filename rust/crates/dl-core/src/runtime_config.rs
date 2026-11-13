//! Typisierte Betriebswerte für bestehende Konstruktoren. Die Schlüssel der
//! Projektion sind ausschließlich interne Kompatibilitätsnamen, keine ENV-Map.
use crate::bot_config::{BotConfig, BotConfigError};
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

pub const STRIPE_MIN_CHARGE_EUR_CENTS: i64 = 50;
pub const STRIPE_MAX_UNIT_AMOUNT_CENTS: i64 = 99_999_999;
pub const SUPPORTER_MIN_CENTS: i64 = 500;
pub const SUPPORTER_TWITCH_CHANNELS: &[&str] = &["earlysalty", "dach_lock"];
pub const SUPPORTER_TIER_MIN_EUR: u64 = SUPPORTER_MIN_CENTS as u64 / 100;
pub const SUPPORTER_TIER_MAX_EUR: u64 = STRIPE_MAX_UNIT_AMOUNT_CENTS as u64 / 100;
pub const SUPPORTER_MAX_TIERS: usize = 5;
pub const SUPPORTER_CUTOVER_GRACE_DAYS: i32 = 31;
pub const MINUTE_SECONDS: u64 = 60;
pub const HOUR_SECONDS: u64 = 3_600;
pub const DAY_SECONDS: u64 = 24 * HOUR_SECONDS;
pub const YEAR_SECONDS: u64 = 365 * DAY_SECONDS;
pub const SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE: RangeInclusive<u64> = MINUTE_SECONDS..=DAY_SECONDS;
pub const SUPPORTER_MAX_REVOKES_RANGE: RangeInclusive<u64> = 0..=100;
pub const SUPPORTER_FUNDING_GOAL_CENTS_RANGE: RangeInclusive<i64> =
    SUPPORTER_MIN_CENTS..=STRIPE_MAX_UNIT_AMOUNT_CENTS;
pub const HTTP_TIMEOUT_MS_RANGE: RangeInclusive<u64> = 1..=120_000;
pub const HTTP_DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;
pub const STEAM_BOT_STRIPE_TIMEOUT_MS: u64 = 10_000;
pub const HTTP_DEFAULT_REQUEST_TIMEOUT_MS: u64 =
    STEAM_BOT_STRIPE_TIMEOUT_MS + HTTP_DEFAULT_CONNECT_TIMEOUT_MS;
const _: () = assert!(HTTP_DEFAULT_REQUEST_TIMEOUT_MS > STEAM_BOT_STRIPE_TIMEOUT_MS);
const _: () = assert!(*HTTP_TIMEOUT_MS_RANGE.end() >= HTTP_DEFAULT_REQUEST_TIMEOUT_MS);
pub const MODERATION_THRESHOLD_RANGE: RangeInclusive<f64> = 0.0..=1.0;
pub const MATCHER_FUZZY_FLOOR_RANGE: RangeInclusive<f64> = 0.0..=1.0;
pub const MATCHER_THRESHOLD_RANGE: RangeInclusive<i64> = 0..=100;
pub const MATCHER_MAX_AI_PER_SCAN_RANGE: RangeInclusive<u64> = 0..=1_000;
pub const MATCHER_SCAN_INTERVAL_HOURS_RANGE: RangeInclusive<u64> = 1..=YEAR_SECONDS / HOUR_SECONDS;
pub const BRAIN_MAX_QUESTION_LEN_RANGE: RangeInclusive<usize> = 1..=65_536;
pub const BRAIN_IMAGE_COUNT_RANGE: RangeInclusive<usize> = 1..=4;
pub const BRAIN_IMAGE_DOWNLOAD_BYTES_RANGE: RangeInclusive<u64> = 1..=512 * 1024 * 1024;
pub const BRAIN_IMAGE_EDGE_RANGE: RangeInclusive<u64> = 384..=12_000;
pub const BRAIN_IMAGE_JPEG_QUALITY_RANGE: RangeInclusive<u64> = 40..=95;
pub const DISCORD_INTERACTION_TOKEN_MS: u64 = 15 * MINUTE_SECONDS * 1_000;
pub const STEAM_FORWARD_TIMEOUT_MS_RANGE: RangeInclusive<u64> = 1..=DISCORD_INTERACTION_TOKEN_MS;
pub const STEAM_INVITE_TASK_WAIT_MS: u64 = 45_000;
pub const STEAM_INVITE_WORST_CASE_MS: u64 = 2 * STEAM_INVITE_TASK_WAIT_MS;
pub const STEAM_INVITE_TIMEOUT_MS_RANGE: RangeInclusive<u64> =
    STEAM_INVITE_WORST_CASE_MS + 1..=DISCORD_INTERACTION_TOKEN_MS;
pub const STEAM_RANK_DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const STEAM_RANKCHECK_DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const STEAM_SYNC_DEFAULT_TIMEOUT_MS: u64 = 180_000;
pub const STEAM_INVITE_DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const DURATION_SECONDS_RANGE: RangeInclusive<u64> = 0..=YEAR_SECONDS;
pub const RUNTIME_TTL_RANGE: RangeInclusive<i64> = 1..=YEAR_SECONDS as i64;
pub const BROKER_IDEMPOTENCY_ENTRIES_RANGE: RangeInclusive<usize> = 1..=1_000_000;
pub const DISCORD_ID_RANGE: RangeInclusive<u64> = 1..=i64::MAX as u64;
pub const PORT_RANGE: RangeInclusive<u16> = 1..=u16::MAX;
pub const BRAIN_DAILY_USER_LIMIT_RANGE: RangeInclusive<usize> = 1..=usize::MAX;
pub const PRESENCE_STATUS_MAX_CHARS: usize = 128;
pub const BROKER_SECONDS_RANGE: RangeInclusive<f64> = 0.001..=YEAR_SECONDS as f64;
pub const TWITCH_TIMEOUT_SECONDS_RANGE: RangeInclusive<f64> = 0.5..=HOUR_SECONDS as f64;
pub const SUPPORTER_DEFAULT_MAX_REVOKES: u64 = 10;
pub const SUPPORTER_DEFAULT_SYNC_INTERVAL_SECONDS: u64 = 300;
pub const SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS: i64 = 25_000;
pub const SUPPORTER_DEFAULT_TIERS_EUR: &[u64] = &[5, 10, 20, 50];

pub fn supporter_tiers_valid(tiers: &[u64]) -> bool {
    !tiers.is_empty()
        && tiers.len() <= SUPPORTER_MAX_TIERS
        && tiers
            .iter()
            .all(|eur| (SUPPORTER_TIER_MIN_EUR..=SUPPORTER_TIER_MAX_EUR).contains(eur))
        && tiers.windows(2).all(|pair| pair[0] < pair[1])
}

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
    ($name:ident { $($field:ident : $ty:ty => $($key:literal)|+),* $(,)? } $(internal { $($inner:ident : $inner_ty:ty),* $(,)? })?) => {
        #[derive(Clone, Default, Deserialize, Serialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct $name { $(pub $field: Option<$ty>,)* $($(pub $inner: Option<$inner_ty>,)*)? }
        impl $name { fn lookup(&self, key: &str) -> Option<String> { match key { $($($key)|+ => self.$field.as_ref().map(LookupValue::lookup_value),)* _ => None } } }
    };
}

section!(StartOptions {
    log_filter: String => "RUST_LOG",
    owner_id: u64 => "OWNER_ID",
    command_prefix: String => "COMMAND_PREFIX",
    presence_status: String => "DL_BOT_PRESENCE_STATUS",
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
    mcp_verified_role_id: u64 => "MCP_VERIFIED_ROLE_ID",
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
} internal {
    http_connect_timeout_ms: u64,
    http_request_timeout_ms: u64,
    steam_rank_timeout_ms: u64,
    steam_rankcheck_timeout_ms: u64,
    steam_sync_timeout_ms: u64,
    steam_invite_timeout_ms: u64,
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

section!(CommunityOptions {
    steam_voice_nudge: bool => "DL_STEAM_VOICE_NUDGE_ENABLED",
    lfg_forum_cutover: bool => "DL_LFG_FORUM_CUTOVER",
    lfg_panel_channel_id: u64 => "DL_LFG_PANEL_CHANNEL_ID",
    lfg_forum_channel_id: u64 => "DL_LFG_FORUM_CHANNEL_ID",
    router_auto_move: bool => "DL_ROUTER_AUTO_MOVE_ENABLED",
    router_auto_move_delay_seconds: u64 => "DL_ROUTER_AUTO_MOVE_DELAY_SECONDS",
    lane_pairing: bool => "DL_LANE_PAIRING_ENABLED",
    survey_pulse: bool => "SURVEY_PULSE_ENABLED",
    survey_interval_days: i64 => "SURVEY_PULSE_INTERVAL_DAYS",
    scrim_visible_vcs: bool => "DL_SCRIM_VISIBLE_VCS_ENABLED",
    scrim_category_id: u64 => "DL_SCRIM_VISIBLE_VCS_CATEGORY_ID",
    scrim_lagebild: bool => "DL_SCRIM_LAGEBILD_ENABLED",
    player_finder: bool => "PLAYER_FINDER_ENABLED",
    lfg_freetext: bool => "DL_LFG_FREITEXT_ENABLED",
    lfg_freetext_channel_id: u64 => "DL_LFG_FREITEXT_CHANNEL_ID",
    voice_hint: bool => "DL_VOICE_HINT_ENABLED",
    brain_help_channel_id: u64 => "DL_BRAIN_HELP_CHANNEL_ID",
    concierge_enabled: bool => "DL_CONCIERGE_ENABLED",
    concierge_test_users: Vec<u64> => "DL_CONCIERGE_TEST_USER_ALLOWLIST",
    concierge_fallback_category_id: u64 => "DL_CONCIERGE_FALLBACK_CATEGORY_ID",
    concierge_pate_category_id: u64 => "DL_CONCIERGE_PATE_CATEGORY_ID",
    concierge_pate_channel_id: u64 => "DL_CONCIERGE_PATE_CHANNEL_ID",
    concierge_pate_panel_channel_id: u64 => "DL_CONCIERGE_PATE_PANEL_CHANNEL_ID",
    concierge_pate_help_channel_id: u64 => "DL_CONCIERGE_PATE_HELP_CHANNEL_ID",
    concierge_mod_role_id: u64 => "DL_CONCIERGE_MOD_PING_ROLE_ID",
    concierge_active_minutes: i64 => "DL_CONCIERGE_ACTIVE_THRESHOLD_MINUTES",
    concierge_brand_emoji: String => "DL_CONCIERGE_BRAND_EMOJI",
    concierge_free_voice: bool => "CONCIERGE_FREE_VOICE",
    concierge_proactive: bool => "DL_CONCIERGE_PROACTIVE",
    recording_state_dir: PathBuf => "SCRIM_RECORD_STATE_DIR",
    recording_rclone_path: PathBuf => "SCRIM_RECORD_RCLONE_PATH",
    recording_archive_base: String => "SCRIM_RECORD_ARCHIVE_BASE",
} internal {
    supporter_role_grant_apply: bool,
    supporter_role_sync_apply: bool,
    supporter_role_sync_max_revokes: u64,
    supporter_sync_interval_seconds: u64,
    supporter_funding_goal_cents: i64,
    supporter_twitch_api_base: String,
    supporter_tiers_eur: Vec<u64>,
});

section!(ModerationOptions {
    channel_id: u64 => "MODERATION_CHANNEL_ID",
    scan_channel_ids: Vec<u64> => "MOD_SCAN_CHANNEL_IDS" | "AI_MODERATOR_SCAN_CHANNEL_IDS",
    invite_allowlist: Vec<String> => "INVITE_ALLOWLIST_FALLBACK",
    analyze_flag_threshold: f64 => "MOD_ANALYZE_FLAG_THRESHOLD",
    auto_verify_threshold: f64 => "MOD_AUTO_VERIFY_THRESHOLD",
    propose_verify_threshold: f64 => "MOD_PROPOSE_VERIFY_THRESHOLD",
    timeout_minutes: i64 => "MOD_TIMEOUT_MINUTES",
    behavior_proposal_timeout_minutes: i64 => "MOD_BEHAVIOR_PROPOSAL_TIMEOUT_MINUTES",
    ai_moderator: bool => "AI_MODERATOR_ENABLE",
});

section!(AiOptions {
    openai_base_url: String => "OPENAI_BASE_URL",
    openai_model: String => "OPENAI_MODEL" | "AI_OPENAI_MODEL",
    fireworks_base_url: String => "FIREWORK_BASE_URL" | "FIREWORKS_BASE_URL",
    turnier_model: String => "TURNIER_AI_MODEL",
    moderation_text_model: String => "MOD_TEXT_ANALYZE_MODEL",
    moderation_image_model: String => "MOD_IMAGE_ANALYZE_MODEL",
    moderation_verify_model: String => "MOD_VERIFY_MODEL",
    concierge_model: String => "DL_CONCIERGE_MODEL",
    transparency_enabled: bool => "DL_AI_TRANSPARENCY_ENABLED",
    transparency_channel_id: u64 => "DL_AI_TRANSPARENCY_CHANNEL_ID",
    transparency_include_moderation: bool => "DL_AI_TRANSPARENCY_INCLUDE_MODERATION",
    transparency_error_repeat_seconds: u64 => "DL_AI_TRANSPARENCY_ERROR_REPEAT_SECONDS",
    transparency_error_followup_seconds: u64 => "DL_AI_TRANSPARENCY_ERROR_FOLLOWUP_SECONDS",
    brain_bin: PathBuf => "BRAIN_BIN",
    brain_retrieval_url: String => "DL_BRAIN_RETRIEVAL_URL",
    brain_command_enabled: bool => "BRAIN_CMD_ENABLED",
    brain_open_test_mode: bool => "BRAIN_OPEN_TEST_MODE",
    brain_channels: Vec<u64> => "BRAIN_CHANNEL_ALLOWLIST",
    brain_cooldown_seconds: u64 => "BRAIN_COOLDOWN_SECS",
    brain_daily_user_limit: usize => "BRAIN_DAILY_USER_LIMIT",
    brain_max_question_len: usize => "BRAIN_MAX_QUESTION_LEN",
    brain_api_endpoint: String => "BRAIN_API_ENDPOINT",
    brain_api_timeout_ms: u64 => "BRAIN_API_TIMEOUT_MS",
    community_brain_answers: bool => "DL_COMMUNITY_BRAIN_ANSWERS",
    brain_image_max_count: usize => "BRAIN_IMAGE_MAX_COUNT",
    brain_image_max_download_bytes: u64 => "BRAIN_IMAGE_MAX_DOWNLOAD_BYTES",
    brain_image_max_edge: u64 => "BRAIN_IMAGE_MAX_EDGE",
    brain_image_jpeg_quality: u64 => "BRAIN_IMAGE_JPEG_QUALITY",
    brain_image_followup_seconds: u64 => "BRAIN_IMAGE_FOLLOWUP_SECONDS",
});

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub start: StartOptions,
    pub bridges: BridgeOptions,
    pub dashboard: DashboardOptions,
    pub web: WebOptions,
    pub community: CommunityOptions,
    pub moderation: ModerationOptions,
    pub ai: AiOptions,
}

impl RuntimeConfig {
    pub fn lookup(&self, key: &str) -> Option<String> {
        self.start
            .lookup(key)
            .or_else(|| self.bridges.lookup(key))
            .or_else(|| self.dashboard.lookup(key))
            .or_else(|| self.web.lookup(key))
            .or_else(|| self.community.lookup(key))
            .or_else(|| self.moderation.lookup(key))
            .or_else(|| self.ai.lookup(key))
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
            &mut self.community.recording_state_dir,
            &mut self.community.recording_rclone_path,
            &mut self.ai.brain_bin,
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
        for seconds in [
            self.community.router_auto_move_delay_seconds,
            self.ai.transparency_error_repeat_seconds,
            self.ai.transparency_error_followup_seconds,
            self.ai.brain_cooldown_seconds,
            self.ai.brain_image_followup_seconds,
        ]
        .into_iter()
        .flatten()
        {
            if !DURATION_SECONDS_RANGE.contains(&seconds) {
                return Err(invalid());
            }
        }
        if self
            .ai
            .brain_daily_user_limit
            .is_some_and(|limit| !BRAIN_DAILY_USER_LIMIT_RANGE.contains(&limit))
        {
            return Err(invalid());
        }
        if self
            .ai
            .brain_max_question_len
            .is_some_and(|length| !BRAIN_MAX_QUESTION_LEN_RANGE.contains(&length))
            || self
                .ai
                .brain_image_max_count
                .is_some_and(|count| !BRAIN_IMAGE_COUNT_RANGE.contains(&count))
            || self
                .ai
                .brain_image_max_download_bytes
                .is_some_and(|bytes| !BRAIN_IMAGE_DOWNLOAD_BYTES_RANGE.contains(&bytes))
            || self
                .ai
                .brain_image_max_edge
                .is_some_and(|edge| !BRAIN_IMAGE_EDGE_RANGE.contains(&edge))
            || self
                .ai
                .brain_image_jpeg_quality
                .is_some_and(|quality| !BRAIN_IMAGE_JPEG_QUALITY_RANGE.contains(&quality))
            || self
                .bridges
                .matcher_max_ai_per_scan
                .is_some_and(|count| !MATCHER_MAX_AI_PER_SCAN_RANGE.contains(&count))
            || self
                .bridges
                .matcher_scan_interval_hours
                .is_some_and(|hours| !MATCHER_SCAN_INTERVAL_HOURS_RANGE.contains(&hours))
        {
            return Err(invalid());
        }
        if self
            .ai
            .brain_api_timeout_ms
            .is_some_and(|ms| !HTTP_TIMEOUT_MS_RANGE.contains(&ms))
            || [
                self.bridges.http_connect_timeout_ms,
                self.bridges.http_request_timeout_ms,
            ]
            .into_iter()
            .flatten()
            .any(|ms| !HTTP_TIMEOUT_MS_RANGE.contains(&ms))
            || self
                .bridges
                .http_connect_timeout_ms
                .unwrap_or(HTTP_DEFAULT_CONNECT_TIMEOUT_MS)
                > self
                    .bridges
                    .http_request_timeout_ms
                    .unwrap_or(HTTP_DEFAULT_REQUEST_TIMEOUT_MS)
            || [
                self.bridges.steam_rank_timeout_ms,
                self.bridges.steam_rankcheck_timeout_ms,
                self.bridges.steam_sync_timeout_ms,
            ]
            .into_iter()
            .flatten()
            .any(|ms| !STEAM_FORWARD_TIMEOUT_MS_RANGE.contains(&ms))
            || self
                .bridges
                .steam_invite_timeout_ms
                .is_some_and(|ms| !STEAM_INVITE_TIMEOUT_MS_RANGE.contains(&ms))
            || self
                .community
                .supporter_sync_interval_seconds
                .is_some_and(|seconds| !SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.contains(&seconds))
            || self
                .community
                .supporter_role_sync_max_revokes
                .is_some_and(|count| !SUPPORTER_MAX_REVOKES_RANGE.contains(&count))
            || self
                .community
                .supporter_funding_goal_cents
                .is_some_and(|cents| !SUPPORTER_FUNDING_GOAL_CENTS_RANGE.contains(&cents))
            || self
                .community
                .supporter_tiers_eur
                .as_deref()
                .is_some_and(|tiers| !supporter_tiers_valid(tiers))
            || (self.ai.brain_command_enabled == Some(true)
                && self
                    .ai
                    .brain_api_endpoint
                    .as_deref()
                    .is_none_or(str::is_empty))
        {
            return Err(invalid());
        }
        for text in [
            &self.ai.openai_model,
            &self.ai.turnier_model,
            &self.ai.moderation_text_model,
            &self.ai.moderation_image_model,
            &self.ai.moderation_verify_model,
            &self.ai.concierge_model,
            &self.community.recording_archive_base,
        ]
        .into_iter()
        .flatten()
        {
            if text.trim().is_empty() || text.chars().any(char::is_control) {
                return Err(invalid());
            }
        }
        if self
            .start
            .log_filter
            .as_deref()
            .is_some_and(|filter| tracing_subscriber::EnvFilter::try_new(filter).is_err())
        {
            return Err(invalid());
        }
        if self
            .dashboard
            .discord_client_id
            .as_deref()
            .is_some_and(|value| {
                value
                    .parse::<u64>()
                    .map_or(true, |id| !DISCORD_ID_RANGE.contains(&id))
            })
            || self
                .start
                .command_prefix
                .as_deref()
                .is_some_and(|value| value.trim().is_empty() || value.chars().any(char::is_control))
            || self.start.presence_status.as_deref().is_some_and(|value| {
                value.trim().is_empty()
                    || value.chars().count() > PRESENCE_STATUS_MAX_CHARS
                    || value.chars().any(char::is_control)
            })
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
            if !seconds.is_finite() || !BROKER_SECONDS_RANGE.contains(&seconds) {
                return Err(invalid());
            }
        }
        if self
            .start
            .broker_idempotency_max_entries
            .is_some_and(|value| !BROKER_IDEMPOTENCY_ENTRIES_RANGE.contains(&value))
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
            if !PORT_RANGE.contains(&port) {
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
        if self.bridges.twitch_api_url.is_none()
            && !self.bridges.twitch_allow_non_loopback.unwrap_or(false)
            && self
                .bridges
                .twitch_host
                .is_some_and(|host| !host.is_loopback())
        {
            return Err(invalid());
        }
        for threshold in [
            self.moderation.analyze_flag_threshold,
            self.moderation.auto_verify_threshold,
            self.moderation.propose_verify_threshold,
        ]
        .into_iter()
        .flatten()
        {
            if !threshold.is_finite() || !MODERATION_THRESHOLD_RANGE.contains(&threshold) {
                return Err(invalid());
            }
        }
        for ttl in [
            self.community.survey_interval_days,
            self.community.concierge_active_minutes,
            self.moderation.timeout_minutes,
            self.moderation.behavior_proposal_timeout_minutes,
            self.dashboard.session_ttl_seconds,
            self.dashboard.oauth_state_ttl_seconds,
        ]
        .into_iter()
        .flatten()
        {
            if !RUNTIME_TTL_RANGE.contains(&ttl) {
                return Err(invalid());
            }
        }
        if self.bridges.twitch_timeout_seconds.is_some_and(|value| {
            !value.is_finite() || !TWITCH_TIMEOUT_SECONDS_RANGE.contains(&value)
        }) || self
            .bridges
            .matcher_fuzzy_floor
            .is_some_and(|value| !value.is_finite() || !MATCHER_FUZZY_FLOOR_RANGE.contains(&value))
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
            if !MATCHER_THRESHOLD_RANGE.contains(&threshold) {
                return Err(invalid());
            }
        }
        for id in [
            self.start.owner_id,
            self.start.command_guild_id,
            self.start.mcp_guild_id,
            self.start.mcp_verified_role_id,
            self.bridges.matcher_guild_id,
            self.bridges.matcher_notify_channel_id,
            self.bridges.matcher_role_id,
            self.dashboard.owner_user_id,
            self.dashboard.moderator_role_id,
            self.dashboard.audit_bot_user_id,
            self.dashboard.insights_guild_id,
            self.community.lfg_panel_channel_id,
            self.community.lfg_forum_channel_id,
            self.community.lfg_freetext_channel_id,
            self.community.brain_help_channel_id,
            self.community.scrim_category_id,
            self.community.concierge_fallback_category_id,
            self.community.concierge_pate_category_id,
            self.community.concierge_pate_channel_id,
            self.community.concierge_pate_panel_channel_id,
            self.community.concierge_pate_help_channel_id,
            self.community.concierge_mod_role_id,
            self.moderation.channel_id,
            self.ai.transparency_channel_id,
        ]
        .into_iter()
        .flatten()
        {
            if !DISCORD_ID_RANGE.contains(&id) {
                return Err(invalid());
            }
        }
        for list in [
            &self.start.broker_channels,
            &self.start.broker_guilds,
            &self.start.broker_roles,
            &self.dashboard.auth_guild_ids,
            &self.community.concierge_test_users,
            &self.moderation.scan_channel_ids,
            &self.ai.brain_channels,
        ]
        .into_iter()
        .flatten()
        {
            if list.iter().any(|id| !DISCORD_ID_RANGE.contains(id)) {
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
            &self.community.recording_state_dir,
            &self.community.recording_rclone_path,
            &self.ai.brain_bin,
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
            (&self.community.supporter_twitch_api_base, true),
            (&self.web.callback_url, false),
            (&self.ai.openai_base_url, false),
            (&self.ai.fireworks_base_url, false),
        ] {
            if let Some(address) = address {
                validate_url(address, local)?;
            }
        }
        if let Some(base) = &self.community.supporter_twitch_api_base {
            if url::Url::parse(base).map_err(|_| invalid())?.path() != "/" {
                return Err(invalid());
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
        || url
            .port_or_known_default()
            .is_some_and(|port| !PORT_RANGE.contains(&port))
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
        // Explizite Fälle entsprechen der bestehenden dl-ai-Fabrik. Keine neue
        // Providerentscheidung und keine Katalog-/Latest-Auswahl beim Start.
        use crate::bot_config::{Provider, UseCase};
        let llm_case = key
            .strip_prefix("DL_LLM_PROVIDER_")
            .or_else(|| key.strip_prefix("DL_LLM_MODEL_"))
            .or_else(|| key.strip_prefix("DL_LLM_MAX_OUTPUT_TOKENS_"))
            .or_else(|| key.strip_prefix("DL_LLM_TEMPERATURE_"))
            .or_else(|| key.strip_prefix("DL_LLM_REASONING_EFFORT_"))
            .or_else(|| key.strip_prefix("DL_LLM_REQUEST_TIMEOUT_SECONDS_"))
            .and_then(|suffix| match suffix {
                "BOT_PATE" => Some(UseCase::BotPate),
                "FAQ" => Some(UseCase::Faq),
                "LFG_FREITEXT" => Some(UseCase::LfgFreitext),
                "SCRIM_LAGEBILD" => Some(UseCase::ScrimLagebild),
                "VERBINDER_MATCH" => Some(UseCase::VerbinderMatch),
                "VERBINDER_KRITIK" => Some(UseCase::VerbinderKritik),
                "AI_ONBOARDING" => Some(UseCase::AiOnboarding),
                "BRAIN_ANTWORT" => Some(UseCase::BrainAntwort),
                "COACHING_ANFRAGE" => Some(UseCase::CoachingAnfrage),
                "MODERATION_TEXT" => Some(UseCase::ModerationText),
                "MODERATION_VERIFY" => Some(UseCase::ModerationVerify),
                "STREAMER_MATCHER" => Some(UseCase::StreamerMatcher),
                "TURNIER_VORSCHLAG" => Some(UseCase::TurnierVorschlag),
                "VOICE_HINT" => Some(UseCase::VoiceHint),
                _ => None,
            });
        if let Some(case) = llm_case {
            return self.llm.use_cases.get(&case).and_then(|config| {
                if key.starts_with("DL_LLM_PROVIDER_") {
                    config.provider.map(|provider| {
                        match provider {
                            Provider::Fireworks => "fireworks",
                            Provider::Openai => "openai",
                        }
                        .into()
                    })
                } else if key.starts_with("DL_LLM_MODEL_") {
                    config.model.clone()
                } else if key.starts_with("DL_LLM_MAX_OUTPUT_TOKENS_") {
                    config.max_output_tokens.map(|value| value.to_string())
                } else if key.starts_with("DL_LLM_TEMPERATURE_") {
                    config.temperature.map(|value| value.to_string())
                } else if key.starts_with("DL_LLM_REASONING_EFFORT_") {
                    config.reasoning_effort.clone()
                } else {
                    config
                        .request_timeout_seconds
                        .map(|value| value.to_string())
                }
            });
        }
        match key {
            "DL_LLM_PROVIDER_DEFAULT" => self.llm.default_provider.map(|provider| match provider {
                Provider::Fireworks => "fireworks".into(),
                Provider::Openai => "openai".into(),
            }),
            "FIREWORK_MODEL" | "FIREWORKS_MODEL" => self.llm.fireworks.model.clone(),
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
        "DISCORD_BRAIN_CLIENT_TOKEN" | "DISCORD_PUBLIC_FACTS_TOKEN" => {
            crate::token_snapshot::value(key)
                .and_then(crate::internal_tokens::token_value)
                .map(str::to_owned)
        }
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
        | "MINIMAX_TOKEN_PLAN_KEY" => {
            // Nach dem privaten Start-Snapshot niemals auf ENV zurückfallen,
            // auch wenn ein erlaubtes Secret darin fehlt.
            let value = if crate::token_snapshot::installed() {
                crate::token_snapshot::value(key).map(str::to_owned)
            } else {
                // Unbetroffene Legacy-Binaries behalten ihren Startvertrag.
                std::env::var(key).ok()
            };
            value.and_then(|value| crate::internal_tokens::token_value(&value).map(str::to_owned))
        }
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

#[cfg(test)]
mod discord_brain_daily_limit_tests {
    use crate::bot_config::BotConfig;

    #[test]
    fn tagesgrenze_kommt_aus_dem_ai_abschnitt_der_toml_konfiguration() {
        for limit in [1, 50, 75] {
            let config = BotConfig::parse(&format!(
                "schema_version=1\n[runtime.ai]\nbrain_daily_user_limit={limit}\n"
            ))
            .expect("Synthetische Betriebskonfiguration");
            assert_eq!(config.runtime.ai.brain_daily_user_limit, Some(limit));
            assert_eq!(
                config.runtime_value("BRAIN_DAILY_USER_LIMIT"),
                Some(limit.to_string())
            );
        }
        let config = BotConfig::parse("schema_version=1\n").expect("Standardkonfiguration");
        assert_eq!(config.runtime.ai.brain_daily_user_limit, None);
        assert!(
            BotConfig::parse("schema_version=1\n[runtime.ai]\nbrain_daily_user_limit=0\n").is_err()
        );
    }
}

#[cfg(test)]
mod discord_brain_secret_tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn discord_brain_betriebsfelder_bleiben_geschuetzt_und_ohne_werteexport() {
        let config = crate::bot_config::BotConfig::parse(
            "schema_version=1\n[runtime.ai]\nbrain_api_endpoint='http://127.0.0.1:12345'\nbrain_api_timeout_ms=65000\nbrain_retrieval_url='http://127.0.0.1:12345'\n[runtime.community]\nbrain_help_channel_id=456\n[runtime.start]\nmcp_verified_role_id=123\n",
        ).expect("Normale Testkonfiguration");
        let catalog = crate::admin_settings::catalog(&config);
        for path in [
            "runtime.ai.brain_api_endpoint",
            "runtime.ai.brain_api_timeout_ms",
            "runtime.ai.brain_retrieval_url",
            "runtime.community.brain_help_channel_id",
            "runtime.start.mcp_verified_role_id",
        ] {
            let field = catalog
                .fields
                .iter()
                .find(|field| field.path == path)
                .expect("Vollständiger Katalog");
            assert!(!field.writable, "{path}");
            assert!(!catalog.values.contains_key(path), "{path}");
        }
    }

    #[test]
    fn beide_discord_zugaenge_stammen_getrennt_aus_dem_privaten_snapshot() {
        let directory = tempfile::tempdir().expect("Privater Testordner");
        std::fs::write(directory.path().join("infisical.json"),
            br#"{"secret_values_fd":3,"project_id":"fixture","environment":"fixture","secret_path":"/","socket_path":"/nonexistent","database_secret":"DEADLOCK_CENTRAL_DSN"}"#
        ).expect("Private Testkonfiguration");
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exec 3<&0; exec \"$@\"", "private-fifo-test"])
            .arg(std::env::current_exe().expect("Testbinary"))
            .args(["--ignored", "--exact", "runtime_config::discord_brain_secret_tests::privater_discord_snapshot_im_kindprozess"])
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .spawn().expect("Isolierter Testprozess");
        child.stdin.take().expect("Testpipe").write_all(
            br#"{"DEADLOCK_CENTRAL_DSN":"synthetic-dsn","DISCORD_BRAIN_CLIENT_TOKEN":"fixture-consumer","DISCORD_PUBLIC_FACTS_TOKEN":"fixture-public-facts","TWITCH_INTERNAL_API_TOKEN":"fixture-general"}"#
        ).expect("Privater Testsnapshot");
        assert!(child.wait().expect("Testprozess").success());
    }

    #[test]
    #[ignore = "isolierter Kindprozess mit privater Testpipe"]
    fn privater_discord_snapshot_im_kindprozess() {
        for key in ["DISCORD_BRAIN_CLIENT_TOKEN", "DISCORD_PUBLIC_FACTS_TOKEN"] {
            assert!(super::secret_value(key).is_none());
        }
        let path = std::env::current_dir()
            .expect("Testordner")
            .join("bot.toml");
        crate::token_snapshot::load(&path).expect("Privater Testsnapshot");
        assert_eq!(
            super::secret_value("DISCORD_BRAIN_CLIENT_TOKEN").as_deref(),
            Some("fixture-consumer")
        );
        assert_eq!(
            super::secret_value("DISCORD_PUBLIC_FACTS_TOKEN").as_deref(),
            Some("fixture-public-facts")
        );
        assert!(super::secret_value("RUST_LOG").is_none());
    }
}

#[cfg(test)]
mod supporter_bound_tests {
    use crate::bot_config::BotConfig;

    fn parse(line: &str) -> bool {
        BotConfig::parse(&format!("schema_version=1\n[runtime.community]\n{line}\n")).is_ok()
    }

    fn parse_in(section: &str, line: &str) -> bool {
        BotConfig::parse(&format!("schema_version=1\n[runtime.{section}]\n{line}\n")).is_ok()
    }

    #[test]
    fn unterstuetzer_werte_haben_ober_und_untergrenzen() {
        use super::*;
        let accepted = [
            format!(
                "supporter_sync_interval_seconds={}",
                SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.start()
            ),
            format!(
                "supporter_sync_interval_seconds={}",
                SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.end()
            ),
            format!(
                "supporter_role_sync_max_revokes={}",
                SUPPORTER_MAX_REVOKES_RANGE.start()
            ),
            format!(
                "supporter_role_sync_max_revokes={}",
                SUPPORTER_MAX_REVOKES_RANGE.end()
            ),
            format!(
                "supporter_funding_goal_cents={}",
                SUPPORTER_FUNDING_GOAL_CENTS_RANGE.start()
            ),
            format!(
                "supporter_funding_goal_cents={}",
                SUPPORTER_FUNDING_GOAL_CENTS_RANGE.end()
            ),
            "supporter_twitch_api_base='http://127.0.0.1:8765'".to_owned(),
            format!("supporter_tiers_eur=[{SUPPORTER_TIER_MIN_EUR}]"),
            format!("supporter_tiers_eur=[{SUPPORTER_TIER_MIN_EUR}, {SUPPORTER_TIER_MAX_EUR}]"),
            "supporter_tiers_eur=[5, 10, 20, 50, 100]".to_owned(),
        ];
        for line in &accepted {
            assert!(parse(line), "{line}");
        }
        let rejected = [
            format!(
                "supporter_sync_interval_seconds={}",
                SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.start() - 1
            ),
            format!(
                "supporter_sync_interval_seconds={}",
                SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.end() + 1
            ),
            format!(
                "supporter_role_sync_max_revokes={}",
                SUPPORTER_MAX_REVOKES_RANGE.end() + 1
            ),
            format!(
                "supporter_funding_goal_cents={}",
                SUPPORTER_FUNDING_GOAL_CENTS_RANGE.start() - 1
            ),
            format!(
                "supporter_funding_goal_cents={}",
                SUPPORTER_FUNDING_GOAL_CENTS_RANGE.end() + 1
            ),
            "supporter_request_timeout_ms=8000".to_owned(),
            "supporter_plus_monthly_cents=300".to_owned(),
            "supporter_steam_api_base='http://127.0.0.1:8783'".to_owned(),
            "supporter_twitch_api_base='http://127.0.0.1:8769/x'".to_owned(),
            "supporter_twitch_api_base=''".to_owned(),
            "supporter_tiers_eur=[]".to_owned(),
            format!("supporter_tiers_eur=[{}]", SUPPORTER_TIER_MIN_EUR - 1),
            format!(
                "supporter_tiers_eur=[{SUPPORTER_TIER_MIN_EUR}, {}]",
                SUPPORTER_TIER_MAX_EUR + 1
            ),
            "supporter_tiers_eur=[10, 5]".to_owned(),
            "supporter_tiers_eur=[5, 5]".to_owned(),
            "supporter_tiers_eur=[5, 10, 20, 50, 100, 200]".to_owned(),
            "supporter_tiers_eur=[5, 4294967296]".to_owned(),
        ];
        for line in &rejected {
            assert!(!parse(line), "{line}");
        }
    }

    #[test]
    fn http_zeitlimits_haben_grenzen_und_verbindung_bleibt_unter_anfrage() {
        use super::*;
        let (min, max) = (*HTTP_TIMEOUT_MS_RANGE.start(), *HTTP_TIMEOUT_MS_RANGE.end());
        for line in [
            format!("http_connect_timeout_ms={min}"),
            format!("http_request_timeout_ms={max}"),
            format!("http_connect_timeout_ms={min}\nhttp_request_timeout_ms={min}"),
        ] {
            assert!(parse_in("bridges", &line), "{line}");
        }
        for line in [
            format!("http_connect_timeout_ms={}", min - 1),
            format!("http_request_timeout_ms={}", max + 1),
            format!(
                "http_connect_timeout_ms={max}\nhttp_request_timeout_ms={}",
                max - 1
            ),
            format!(
                "http_connect_timeout_ms={}",
                HTTP_DEFAULT_REQUEST_TIMEOUT_MS + 1
            ),
        ] {
            assert!(!parse_in("bridges", &line), "{line}");
        }
        assert!(HTTP_TIMEOUT_MS_RANGE.contains(&HTTP_DEFAULT_CONNECT_TIMEOUT_MS));
        assert!(HTTP_TIMEOUT_MS_RANGE.contains(&HTTP_DEFAULT_REQUEST_TIMEOUT_MS));
        assert!(!parse_in(
            "ai",
            &format!("brain_api_timeout_ms={}", max + 1)
        ));
        assert!(parse_in("ai", &format!("brain_api_timeout_ms={max}")));
    }

    #[test]
    fn brain_und_zuordnung_haben_grenzen_mit_beiden_kanten() {
        use super::*;
        fn edges<T: std::fmt::Display + Copy>(
            section: &str,
            key: &str,
            range: &std::ops::RangeInclusive<T>,
            below: Option<T>,
            above: T,
        ) {
            assert!(
                parse_in(section, &format!("{key}={}", range.start())),
                "{key}"
            );
            assert!(
                parse_in(section, &format!("{key}={}", range.end())),
                "{key}"
            );
            if let Some(below) = below {
                assert!(!parse_in(section, &format!("{key}={below}")), "{key}");
            }
            assert!(!parse_in(section, &format!("{key}={above}")), "{key}");
        }
        edges(
            "ai",
            "brain_max_question_len",
            &BRAIN_MAX_QUESTION_LEN_RANGE,
            Some(0),
            65_537,
        );
        edges(
            "ai",
            "brain_image_max_count",
            &BRAIN_IMAGE_COUNT_RANGE,
            Some(0),
            5,
        );
        edges(
            "ai",
            "brain_image_max_download_bytes",
            &BRAIN_IMAGE_DOWNLOAD_BYTES_RANGE,
            Some(0),
            BRAIN_IMAGE_DOWNLOAD_BYTES_RANGE.end() + 1,
        );
        edges(
            "ai",
            "brain_image_max_edge",
            &BRAIN_IMAGE_EDGE_RANGE,
            Some(383),
            12_001,
        );
        edges(
            "ai",
            "brain_image_jpeg_quality",
            &BRAIN_IMAGE_JPEG_QUALITY_RANGE,
            Some(39),
            96,
        );
        edges(
            "ai",
            "brain_image_followup_seconds",
            &DURATION_SECONDS_RANGE,
            None,
            YEAR_SECONDS + 1,
        );
        edges(
            "bridges",
            "matcher_max_ai_per_scan",
            &MATCHER_MAX_AI_PER_SCAN_RANGE,
            None,
            1_001,
        );
        edges(
            "bridges",
            "matcher_scan_interval_hours",
            &MATCHER_SCAN_INTERVAL_HOURS_RANGE,
            Some(0),
            8_761,
        );
    }

    fn catalog_help(key: &str) -> String {
        let prefix = if key.contains('.') {
            format!("runtime.{key}\t")
        } else {
            format!("runtime.community.{key}\t")
        };
        crate::admin_settings::SPEC
            .lines()
            .find(|line| line.starts_with(&prefix))
            .and_then(|line| line.split('\t').nth(5))
            .expect("Katalogzeile")
            .to_string()
    }

    #[test]
    fn katalogtexte_nennen_die_werte_aus_dem_code() {
        use super::*;
        assert!(catalog_help("supporter_role_sync_max_revokes")
            .starts_with(&format!("Standard ist {SUPPORTER_DEFAULT_MAX_REVOKES}.")));
        assert!(
            catalog_help("supporter_sync_interval_seconds").starts_with(&format!(
                "Standard ist {SUPPORTER_DEFAULT_SYNC_INTERVAL_SECONDS}."
            ))
        );
        assert!(
            catalog_help("supporter_funding_goal_cents").starts_with(&format!(
                "Standard ist {SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS}, also {} €.",
                SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS / 100
            ))
        );
        let tiers = SUPPORTER_DEFAULT_TIERS_EUR
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let tier_help = catalog_help("supporter_tiers_eur");
        assert!(tier_help.starts_with(&format!("Standard ist {tiers}.")));
        assert!(tier_help.contains(&format!("mindestens {SUPPORTER_TIER_MIN_EUR} €")));
        assert_eq!(SUPPORTER_MAX_TIERS, 5);
        assert!(tier_help.contains("Eine bis fünf Stufen"));
        assert!(supporter_tiers_valid(SUPPORTER_DEFAULT_TIERS_EUR));
        assert!(SUPPORTER_FUNDING_GOAL_CENTS_RANGE.contains(&SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS));
        assert!(SUPPORTER_MAX_REVOKES_RANGE.contains(&SUPPORTER_DEFAULT_MAX_REVOKES));
        assert!(SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE
            .contains(&SUPPORTER_DEFAULT_SYNC_INTERVAL_SECONDS));
        assert!(catalog_help("bridges.http_connect_timeout_ms")
            .starts_with(&format!("Standard ist {HTTP_DEFAULT_CONNECT_TIMEOUT_MS}.")));
        assert!(catalog_help("bridges.http_request_timeout_ms")
            .starts_with(&format!("Standard ist {HTTP_DEFAULT_REQUEST_TIMEOUT_MS}.")));
        for (key, default) in [
            ("steam_rank_timeout_ms", STEAM_RANK_DEFAULT_TIMEOUT_MS),
            (
                "steam_rankcheck_timeout_ms",
                STEAM_RANKCHECK_DEFAULT_TIMEOUT_MS,
            ),
            ("steam_sync_timeout_ms", STEAM_SYNC_DEFAULT_TIMEOUT_MS),
            ("steam_invite_timeout_ms", STEAM_INVITE_DEFAULT_TIMEOUT_MS),
        ] {
            assert!(
                catalog_help(&format!("bridges.{key}"))
                    .starts_with(&format!("Standard ist {default}.")),
                "{key}"
            );
        }
        assert!(catalog_help("bridges.steam_invite_timeout_ms")
            .contains(&format!("je bis zu {STEAM_INVITE_TASK_WAIT_MS}")));
    }

    #[test]
    fn steam_wartezeiten_haben_grenzen_und_invite_bleibt_ueber_dem_steam_worst_case() {
        use super::*;
        for default in [
            STEAM_RANK_DEFAULT_TIMEOUT_MS,
            STEAM_RANKCHECK_DEFAULT_TIMEOUT_MS,
            STEAM_SYNC_DEFAULT_TIMEOUT_MS,
        ] {
            assert!(STEAM_FORWARD_TIMEOUT_MS_RANGE.contains(&default));
        }
        assert!(STEAM_INVITE_TIMEOUT_MS_RANGE.contains(&STEAM_INVITE_DEFAULT_TIMEOUT_MS));
        assert!(*STEAM_INVITE_TIMEOUT_MS_RANGE.start() > STEAM_INVITE_WORST_CASE_MS);
        let (min, max) = (
            *STEAM_FORWARD_TIMEOUT_MS_RANGE.start(),
            *STEAM_FORWARD_TIMEOUT_MS_RANGE.end(),
        );
        let invite = STEAM_INVITE_TIMEOUT_MS_RANGE;
        for key in [
            "steam_rank_timeout_ms",
            "steam_rankcheck_timeout_ms",
            "steam_sync_timeout_ms",
        ] {
            assert!(parse_in("bridges", &format!("{key}={min}")), "{key}");
            assert!(parse_in("bridges", &format!("{key}={max}")), "{key}");
            assert!(!parse_in("bridges", &format!("{key}={}", min - 1)), "{key}");
            assert!(!parse_in("bridges", &format!("{key}={}", max + 1)), "{key}");
        }
        assert!(parse_in(
            "bridges",
            &format!("steam_invite_timeout_ms={}", invite.start())
        ));
        assert!(parse_in(
            "bridges",
            &format!("steam_invite_timeout_ms={}", invite.end())
        ));
        assert!(!parse_in(
            "bridges",
            &format!("steam_invite_timeout_ms={STEAM_INVITE_WORST_CASE_MS}")
        ));
        assert!(!parse_in(
            "bridges",
            &format!("steam_invite_timeout_ms={}", invite.end() + 1)
        ));
    }
}
