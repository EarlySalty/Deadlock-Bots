//! Privacy/DSGVO-Kern - Opt-out-Gate und vollständige Datenlöschung.
//!
//! Der Löschpfad ist der Erasure-Vertrag: hartes DELETE über alle
//! nutzerbezogenen Tabellen, Steam-seitige Tabellen je verknüpfter `steam_id`,
//! KV-Einträge und anschließend der bleibende Opt-out-Grabstein in
//! `core.user_privacy`.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration as StdDuration;

use dl_central_db::kv;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::db::{utc_from_unix, CommunityDbError, CommunityDbResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnType {
    I64,
    Text,
}

#[derive(Debug, Clone, Copy)]
struct TableSpec {
    key_table: &'static str,
    key_col: &'static str,
    relation: &'static str,
    col: &'static str,
    col_type: ColumnType,
}

impl TableSpec {
    const fn new(
        key_table: &'static str,
        key_col: &'static str,
        relation: &'static str,
        col: &'static str,
        col_type: ColumnType,
    ) -> Self {
        Self {
            key_table,
            key_col,
            relation,
            col,
            col_type,
        }
    }

    fn count_key(self) -> String {
        format!("{}.{}", self.key_table, self.key_col)
    }
}

/// Nutzerbezogene Tabellen aus dem alten Python-Vertrag, auf zentrale Schemas
/// gemappt. Nicht vorhandene Alt-Tabellen werden via `to_regclass` übersprungen.
const USER_TABLES: &[TableSpec] = &[
    TableSpec::new(
        "core_users",
        "discord_id",
        "core.users",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "user_tags",
        "user_id",
        "core.user_tags",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "user_mod_tags",
        "user_id",
        "core.user_mod_tags",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "voice_stats",
        "user_id",
        "voice.voice_stats",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "voice_session_log",
        "user_id",
        "activity.voice_session_log",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "voice_feedback_requests",
        "user_id",
        "activity.voice_feedback_requests",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "voice_feedback_responses",
        "user_id",
        "activity.voice_feedback_responses",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "user_activity_patterns",
        "user_id",
        "activity.user_activity_patterns",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "message_activity",
        "user_id",
        "activity.message_activity",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "text_conversation_log",
        "user_id",
        "activity.text_conversation_log",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "text_stats",
        "user_id",
        "activity.text_stats",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "journey_events",
        "user_id",
        "activity.journey_events",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "journey_user_state",
        "user_id",
        "activity.journey_user_state",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "message_metadata_events",
        "user_id",
        "activity.message_metadata_events",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "voice_metadata_events",
        "user_id",
        "activity.voice_metadata_events",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "voice_open_sessions",
        "user_id",
        "activity.voice_open_sessions",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "interaction_events",
        "user_id",
        "activity.interaction_events",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "presence_daily_seen",
        "user_id",
        "activity.presence_daily_seen",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "guild_member_directory",
        "user_id",
        "activity.guild_member_directory",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "member_events",
        "user_id",
        "activity.member_events",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "member_leave_surveys",
        "user_id",
        "activity.member_leave_surveys",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "lfg_watches",
        "user_id",
        "activity.lfg_watches",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_links",
        "user_id",
        "core.steam_links",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "discord_role_connection_tokens",
        "user_id",
        "core.discord_role_connection_tokens",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "discord_role_connection_sync_state",
        "user_id",
        "core.discord_role_connection_sync_state",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_cleanup_poll_state",
        "user_id",
        "steam.steam_cleanup_poll_state",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_friendship_miss_tracker",
        "user_id",
        "steam.steam_friendship_miss_tracker",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_role_cleanup_pending",
        "user_id",
        "steam.steam_role_cleanup_pending",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_beta_invites",
        "discord_id",
        "steam.steam_beta_invites",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_intent",
        "discord_id",
        "steam.beta_invite_intent",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_audit",
        "discord_id",
        "steam.beta_invite_audit",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_auto_failure_alerts",
        "discord_id",
        "steam.beta_invite_auto_failure_alerts",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_friendship_auto_poll",
        "discord_id",
        "steam.beta_invite_friendship_auto_poll",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_panel_clicks",
        "discord_id",
        "steam.beta_invite_panel_clicks",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "bot_event_log",
        "discord_id",
        "steam.bot_event_log",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_pending_payments",
        "discord_id",
        "steam.beta_invite_pending_payments",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_supporter_role_grants",
        "discord_id",
        "steam.beta_invite_supporter_role_grants",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_tickets",
        "discord_id",
        "steam.beta_invite_tickets",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_launch_tokens",
        "user_id",
        "steam.steam_launch_tokens",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_links_archive",
        "user_id",
        "steam.steam_links_archive",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_links_leave_archive",
        "user_id",
        "steam.steam_links_leave_archive",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_rank_assignments",
        "user_id",
        "steam.steam_rank_assignments",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_rank_history",
        "user_id",
        "steam.steam_rank_history",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "rank_history_visibility",
        "user_id",
        "steam.rank_history_visibility",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "customgames_tournament_signups",
        "user_id",
        "bot.customgames_tournament_signups",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "claimed_threads",
        "assigned_user_id",
        "bot.claimed_threads",
        "assigned_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "faq_chat_sessions",
        "user_id",
        "bot.faq_chat_sessions",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "concierge_profiles",
        "user_id",
        "bot.concierge_profiles",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "concierge_conversations",
        "user_id",
        "bot.concierge_conversations",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "concierge_patenschaften_user",
        "user_id",
        "bot.concierge_patenschaften",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "concierge_patenschaften_pate",
        "pate_id",
        "bot.concierge_patenschaften",
        "pate_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "onboarding_pending_verify",
        "user_id",
        "bot.onboarding_pending_verify",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "reaction_role_dm_log",
        "user_id",
        "bot.reaction_role_dm_log",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "server_faq_logs",
        "user_id",
        "bot.server_faq_logs",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "persistent_views",
        "user_id",
        "bot.persistent_views",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "streamer_link_intents",
        "discord_id",
        "bot.streamer_link_intents",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_auth_tokens",
        "user_id",
        "bot.turnier_auth_tokens",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "user_retention_tracking",
        "user_id",
        "activity.user_retention_tracking",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "user_retention_messages",
        "user_id",
        "activity.user_retention_messages",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "voice_channel_anchors",
        "user_id",
        "voice.voice_channel_anchors",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "router_user_prefs",
        "user_id",
        "voice.router_user_prefs",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "router_intro_dm",
        "user_id",
        "voice.router_intro_dm",
        "user_id",
        ColumnType::I64,
    ),
    // Offener `/invite`-Request, adressiert über die Discord-ID des
    // Eingeladenen. MUSS hier stehen und nicht nur in STEAM_SIDE_TABLES:
    // eingeladen wird jemand, der noch KEINEN core.steam_links-Eintrag hat,
    // über den steam_ids_for_user ihn finden könnte.
    TableSpec::new(
        "invite_requests_target",
        "target_discord_id",
        "steam.invite_requests",
        "target_discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tempvoice_presets",
        "user_id",
        "voice.tempvoice_presets",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tempvoice_rank_pref",
        "user_id",
        "voice.tempvoice_rank_pref",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "coaching_sessions",
        "user_id",
        "coaching.sessions",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "coaching_sessions_legacy",
        "user_id",
        "coaching.sessions_legacy",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "coaching_requests",
        "discord_user_id",
        "coaching.requests",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "coaching_coaches",
        "discord_user_id",
        "coaching.coaches",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "coaching_coach_applications",
        "discord_user_id",
        "coaching.coach_applications",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "coaching_bans",
        "discord_user_id",
        "coaching.bans",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_nudge_state",
        "user_id",
        "steam.steam_nudge_state",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "twitch_streamers",
        "discord_user_id",
        "bot.twitch_streamers",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "twitch_link_clicks",
        "discord_user_id",
        "bot.twitch_link_clicks",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "user_data",
        "user_id",
        "core.user_data",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "notification_log",
        "user_id",
        "bot.notification_log",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "notification_queue",
        "user_id",
        "bot.notification_queue",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "action_outbox",
        "user_id",
        "bot.action_outbox",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "survey_responses",
        "user_id",
        "bot.survey_responses",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "dm_response_tracking",
        "user_id",
        "bot.dm_response_tracking",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tempvoice_owner_prefs",
        "owner_id",
        "voice.tempvoice_owner_prefs",
        "owner_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tempvoice_lanes",
        "owner_id",
        "voice.tempvoice_lanes",
        "owner_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tempvoice_lurkers",
        "user_id",
        "voice.tempvoice_lurkers",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tempvoice_bans",
        "owner_id",
        "voice.tempvoice_bans",
        "owner_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tempvoice_bans",
        "banned_id",
        "voice.tempvoice_bans",
        "banned_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "lfg_posts",
        "owner_id",
        "voice.lfg_posts",
        "owner_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_quick_invites",
        "reserved_by",
        "steam.steam_quick_invites",
        "reserved_by",
        ColumnType::I64,
    ),
    TableSpec::new(
        "issue_reports",
        "user_id",
        "bot.issue_reports",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "ai_moderation_cases",
        "user_id",
        "moderation.ai_moderation_cases",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "ai_moderation_ragebait_hits",
        "user_id",
        "moderation.ai_moderation_ragebait_hits",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "security_guard_incidents",
        "user_id",
        "moderation.security_guard_incidents",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "scrim_participants",
        "discord_id",
        "scrim.participants",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "tierlist_meta_tier_lists",
        "owner_id",
        "tierlist.meta_tier_lists",
        "owner_id",
        ColumnType::Text,
    ),
    TableSpec::new(
        "tierlist_meta_votes",
        "user_id",
        "tierlist.meta_votes",
        "user_id",
        ColumnType::Text,
    ),
    TableSpec::new(
        "turnier_audit_log",
        "user_id",
        "turnier.audit_log",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_checkins",
        "discord_id",
        "turnier.checkins",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_match_casters",
        "discord_id",
        "turnier.match_casters",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_player_points",
        "discord_id",
        "turnier.player_points",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_rank_cache",
        "discord_id",
        "turnier.rank_cache",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_sessions",
        "discord_id",
        "turnier.sessions",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_team_applications",
        "discord_id",
        "turnier.team_applications",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_team_invitations",
        "discord_id",
        "turnier.team_invitations",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_team_members",
        "discord_id",
        "turnier.team_members",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_tournament_casters",
        "discord_id",
        "turnier.tournament_casters",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_tournament_checkins",
        "discord_id",
        "turnier.tournament_checkins",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_tournament_dm_optout",
        "discord_id",
        "turnier.tournament_dm_optout",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_tournament_signups",
        "discord_id",
        "turnier.tournament_signups",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_user_consents",
        "discord_id",
        "turnier.user_consents",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "turnier_user_profiles",
        "discord_id",
        "turnier.user_profiles",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "clip_submissions",
        "user_id",
        "clips.clip_submissions",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "clip_window_submissions",
        "user_id",
        "clips.clip_window_submissions",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "clip_contest_submissions",
        "user_id",
        "clips.clip_contest_submissions",
        "user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "coaching_coachees",
        "discord_user_id",
        "coaching.coachees",
        "discord_user_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "ai_decision_ledger",
        "subject_user_id",
        "bot.ai_decision_ledger",
        "subject_user_id",
        ColumnType::I64,
    ),
];

const NULLABLE_USER_COLUMNS: &[TableSpec] = &[TableSpec::new(
    "clip_contests",
    "winner_user_id",
    "clips.clip_contests",
    "winner_user_id",
    ColumnType::I64,
)];

const STEAM_SIDE_TABLES: &[TableSpec] = &[
    TableSpec::new(
        "live_player_state",
        "steam_id",
        "activity.live_player_state",
        "steam_id",
        ColumnType::Text,
    ),
    TableSpec::new(
        "deadlock_voice_watch",
        "steam_id",
        "voice.deadlock_voice_watch",
        "steam_id",
        ColumnType::Text,
    ),
    TableSpec::new(
        "steam_rich_presence",
        "steam_id",
        "steam.steam_rich_presence",
        "steam_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_presence_watchlist",
        "steam_id",
        "steam.steam_presence_watchlist",
        "steam_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_friend_requests",
        "steam_id",
        "steam.steam_friend_requests",
        "steam_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_friendship_miss_tracker",
        "steam_id",
        "steam.steam_friendship_miss_tracker",
        "steam_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "steam_beta_invites",
        "steam_id64",
        "steam.steam_beta_invites",
        "steam_id64",
        ColumnType::I64,
    ),
    TableSpec::new(
        "beta_invite_audit",
        "steam_id64",
        "steam.beta_invite_audit",
        "steam_id64",
        ColumnType::I64,
    ),
    // Offene Admin-Invites (`/invite`). Kurzlebig (24-h-TTL), enthält neben der
    // Steam-ID auch die Discord-ID des Eingeladenen — gehoert damit in die
    // Erasure-Kette, nicht nur in den Poller.
    TableSpec::new(
        "invite_requests",
        "steam_id64",
        "steam.invite_requests",
        "steam_id64",
        ColumnType::I64,
    ),
    TableSpec::new(
        "bot_event_log",
        "steam_id",
        "steam.bot_event_log",
        "steam_id",
        ColumnType::I64,
    ),
];

const USER_CO_PLAYERS_REL: &str = "activity.user_co_players";
const KV_REL: &str = "bot.kv_store";
const KV_NATIVE_ONBOARDING_COMPLETED_NS: &str = "native_onboarding:completed";
const KV_CONCIERGE_T0_NS: &str = "concierge:t0";
const KV_CONCIERGE_FALLBACK_NS: &str = "concierge:fallback_channel";
const KV_CONCIERGE_PATE_CLAIM_NS: &str = "concierge:pate_claim";
const KV_CONCIERGE_STECKBRIEF_REVOKED_NS: &str = "concierge:steckbrief_revoked";
const USER_PRIVACY_REL: &str = "core.user_privacy";
const SERVER_SYNC_ROLLBACK_EXPORTS_REL: &str = "server_config.rollback_exports";
const PRIVACY_RETENTION_JOB_INTERVAL: StdDuration = StdDuration::from_secs(24 * 3600);

#[derive(Debug, Default, Clone)]
pub struct DeleteSummary {
    pub counts: BTreeMap<String, i64>,
    pub steam_ids: Vec<String>,
}

impl DeleteSummary {
    pub fn sum(&self, keys: &[&str]) -> i64 {
        keys.iter().filter_map(|k| self.counts.get(*k)).sum()
    }
}

#[derive(Debug, Clone)]
struct SteamId {
    text: String,
    numeric: Option<i64>,
}

enum LookupValue<'a> {
    I64(i64),
    Text(&'a str),
}

fn coerce_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok())),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn value_mentions_user(value: &Value, user_id: i64) -> bool {
    if coerce_i64(value) == Some(user_id) {
        return true;
    }
    match value {
        Value::Array(values) => values
            .iter()
            .any(|value| value_mentions_user(value, user_id)),
        Value::Object(values) => values.iter().any(|(key, value)| {
            key.trim().parse::<i64>().ok() == Some(user_id) || value_mentions_user(value, user_id)
        }),
        _ => false,
    }
}

fn raw_value_mentions_user(raw: &str, user_id: i64) -> bool {
    raw.trim().parse::<i64>().ok() == Some(user_id)
        || serde_json::from_str::<Value>(raw)
            .ok()
            .is_some_and(|value| value_mentions_user(&value, user_id))
}

fn key_mentions_user(key: &str, user_id: i64) -> bool {
    let user_id = user_id.to_string();
    key == user_id
        || key
            .rsplit_once(':')
            .is_some_and(|(_, tail)| tail == user_id)
}

fn concierge_claim_belongs_to_user(ns: &str, key: &str, raw: &str, user_id: i64) -> bool {
    match ns {
        KV_CONCIERGE_T0_NS | KV_CONCIERGE_FALLBACK_NS | KV_CONCIERGE_STECKBRIEF_REVOKED_NS => {
            key_mentions_user(key, user_id)
        }
        KV_CONCIERGE_PATE_CLAIM_NS => {
            key_mentions_user(key, user_id) || raw_value_mentions_user(raw, user_id)
        }
        _ => false,
    }
}

fn concierge_claim_export(ns: &str, key: String, raw: String, user_id: i64) -> Value {
    let value = serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw));
    if ns != KV_CONCIERGE_PATE_CLAIM_NS {
        return serde_json::json!({ "namespace": ns, "key": key, "value": value });
    }
    let key = if key_mentions_user(&key, user_id) {
        Value::String(key)
    } else {
        Value::String("redacted".to_string())
    };
    let value = if coerce_i64(&value) == Some(user_id) {
        Value::from(user_id)
    } else {
        Value::String("redacted".to_string())
    };
    serde_json::json!({ "namespace": ns, "key": key, "value": value })
}

pub(crate) async fn delete_concierge_claims(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> CommunityDbResult<i64> {
    let claims = sqlx::query(
        "SELECT ns, k, v
           FROM bot.kv_store
          WHERE ns IN ($1, $2, $3, $4)",
    )
    .bind(KV_CONCIERGE_T0_NS)
    .bind(KV_CONCIERGE_FALLBACK_NS)
    .bind(KV_CONCIERGE_PATE_CLAIM_NS)
    .bind(KV_CONCIERGE_STECKBRIEF_REVOKED_NS)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .filter_map(|row| {
        let ns: String = row.try_get("ns").ok()?;
        let key: String = row.try_get("k").ok()?;
        let raw: String = row.try_get("v").ok()?;
        concierge_claim_belongs_to_user(&ns, &key, &raw, user_id).then_some((ns, key))
    })
    .collect::<Vec<_>>();
    let mut deleted = 0i64;
    for (ns, key) in claims {
        let result = sqlx::query("DELETE FROM bot.kv_store WHERE ns = $1 AND k = $2")
            .bind(ns)
            .bind(key)
            .execute(&mut **tx)
            .await?;
        deleted += rows_to_i64(result.rows_affected());
    }
    Ok(deleted)
}

fn rows_to_i64(rows: u64) -> i64 {
    i64::try_from(rows).unwrap_or(i64::MAX)
}

async fn relation_exists(pool: &PgPool, relation: &str) -> Result<bool, sqlx::Error> {
    let row = sqlx::query("SELECT to_regclass($1) IS NOT NULL AS exists")
        .bind(relation)
        .fetch_one(pool)
        .await?;
    row.try_get("exists")
}

async fn existing_relations(pool: &PgPool) -> Result<HashSet<&'static str>, sqlx::Error> {
    let mut set = HashSet::new();
    for relation in USER_TABLES
        .iter()
        .chain(NULLABLE_USER_COLUMNS.iter())
        .chain(STEAM_SIDE_TABLES.iter())
        .map(|spec| spec.relation)
        .chain([USER_CO_PLAYERS_REL, KV_REL, USER_PRIVACY_REL])
    {
        if relation_exists(pool, relation).await? {
            set.insert(relation);
        }
    }
    Ok(set)
}

async fn steam_ids_for_user(pool: &PgPool, user_id: i64) -> Result<Vec<SteamId>, sqlx::Error> {
    if !relation_exists(pool, "core.steam_links").await? {
        return Ok(Vec::new());
    }

    let rows = sqlx::query!(
        r#"
        SELECT steam_id, steam_id64
          FROM core.steam_links
         WHERE discord_id = $1
         ORDER BY steam_id
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| SteamId {
            numeric: row.steam_id64.or_else(|| row.steam_id.trim().parse().ok()),
            text: row.steam_id.trim().to_string(),
        })
        .filter(|sid| !sid.text.is_empty())
        .collect())
}

async fn steam_ids_for_user_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> Result<Vec<SteamId>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT steam_id, steam_id64 FROM core.steam_links WHERE discord_id = $1 ORDER BY steam_id",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let text: String = row.try_get("steam_id").ok()?;
            let numeric: Option<i64> = row.try_get("steam_id64").ok()?;
            Some(SteamId {
                numeric: numeric.or_else(|| text.trim().parse().ok()),
                text: text.trim().to_string(),
            })
        })
        .filter(|sid| !sid.text.is_empty())
        .collect())
}

fn steam_lookup<'a>(sid: &'a SteamId, col_type: ColumnType) -> Option<LookupValue<'a>> {
    match col_type {
        ColumnType::I64 => sid.numeric.map(LookupValue::I64),
        ColumnType::Text => Some(LookupValue::Text(&sid.text)),
    }
}

async fn delete_rows_lookup(
    tx: &mut Transaction<'_, Postgres>,
    spec: TableSpec,
    value: LookupValue<'_>,
) -> Result<i64, sqlx::Error> {
    // Dynamic SQL is unavoidable for the privacy contract; `spec` is a static
    // whitelist entry and all user data is bound as typed parameters.
    let sql = format!("DELETE FROM {} WHERE {} = $1", spec.relation, spec.col);
    let query = sqlx::query(&sql);
    let result = match value {
        LookupValue::I64(value) => query.bind(value).execute(&mut **tx).await?,
        LookupValue::Text(value) => query.bind(value).execute(&mut **tx).await?,
    };
    Ok(rows_to_i64(result.rows_affected()))
}

async fn delete_rows_user(
    tx: &mut Transaction<'_, Postgres>,
    spec: TableSpec,
    user_id: i64,
    user_key: &str,
) -> Result<i64, sqlx::Error> {
    match spec.col_type {
        ColumnType::I64 => delete_rows_lookup(tx, spec, LookupValue::I64(user_id)).await,
        ColumnType::Text => delete_rows_lookup(tx, spec, LookupValue::Text(user_key)).await,
    }
}

async fn null_user_column_lookup(
    tx: &mut Transaction<'_, Postgres>,
    spec: TableSpec,
    value: LookupValue<'_>,
) -> Result<i64, sqlx::Error> {
    // Dynamic SQL is unavoidable for the privacy contract; `spec` is a static
    // whitelist entry and all user data is bound as typed parameters.
    let sql = format!(
        "UPDATE {} SET {} = NULL WHERE {} = $1",
        spec.relation, spec.col, spec.col
    );
    let query = sqlx::query(&sql);
    let result = match value {
        LookupValue::I64(value) => query.bind(value).execute(&mut **tx).await?,
        LookupValue::Text(value) => query.bind(value).execute(&mut **tx).await?,
    };
    Ok(rows_to_i64(result.rows_affected()))
}

async fn null_user_column(
    tx: &mut Transaction<'_, Postgres>,
    spec: TableSpec,
    user_id: i64,
    user_key: &str,
) -> Result<i64, sqlx::Error> {
    match spec.col_type {
        ColumnType::I64 => null_user_column_lookup(tx, spec, LookupValue::I64(user_id)).await,
        ColumnType::Text => null_user_column_lookup(tx, spec, LookupValue::Text(user_key)).await,
    }
}

async fn select_rows_i64(
    pool: &PgPool,
    spec: TableSpec,
    value: i64,
) -> Result<Vec<Value>, CommunityDbError> {
    select_rows_lookup(pool, spec, LookupValue::I64(value)).await
}

async fn select_rows_lookup(
    pool: &PgPool,
    spec: TableSpec,
    value: LookupValue<'_>,
) -> Result<Vec<Value>, CommunityDbError> {
    let sql = format!(
        "SELECT row_to_json(t)::text AS row_json FROM (SELECT * FROM {} WHERE {} = $1) t",
        spec.relation, spec.col
    );
    let query = sqlx::query(&sql);
    let rows = match value {
        LookupValue::I64(value) => query.bind(value).fetch_all(pool).await?,
        LookupValue::Text(value) => query.bind(value).fetch_all(pool).await?,
    };

    rows.into_iter()
        .map(|row| {
            let raw: String = row.try_get("row_json")?;
            serde_json::from_str(&raw).map_err(CommunityDbError::from)
        })
        .collect()
}

async fn select_rows_user(
    pool: &PgPool,
    spec: TableSpec,
    user_id: i64,
    user_key: &str,
) -> Result<Vec<Value>, CommunityDbError> {
    match spec.col_type {
        ColumnType::I64 => select_rows_lookup(pool, spec, LookupValue::I64(user_id)).await,
        ColumnType::Text => select_rows_lookup(pool, spec, LookupValue::Text(user_key)).await,
    }
}

async fn kv_value(pool: &PgPool, ns: &str, key: &str) -> CommunityDbResult<Option<Value>> {
    let raw = kv::get(pool, ns, key).await?;
    Ok(raw.map(|s| serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s))))
}

fn field_or(row: &serde_json::Map<String, Value>, key: &str, uid: i64) -> i64 {
    row.get(key)
        .and_then(coerce_i64)
        .filter(|v| *v != 0)
        .unwrap_or(uid)
}

fn redact_co_players(rows: Vec<Value>, uid: i64) -> Vec<Value> {
    let mut out = Vec::new();
    for row in rows {
        let Value::Object(mut obj) = row else {
            continue;
        };
        let uval = field_or(&obj, "user_id", uid);
        let coval = field_or(&obj, "co_player_id", uid);
        if uval != uid && coval != uid {
            continue;
        }
        if obj.contains_key("co_player_id") {
            obj.insert("co_player_id".into(), Value::from("redacted"));
        }
        if uval != uid {
            obj.insert("user_id".into(), Value::from(uid));
        }
        if obj.contains_key("user_display_name") && uval != uid {
            obj.insert("user_display_name".into(), Value::from("redacted"));
        }
        if obj.contains_key("co_player_display_name") && coval != uid {
            obj.insert("co_player_display_name".into(), Value::from("redacted"));
        }
        out.push(Value::Object(obj));
    }
    out
}

fn redact_other_id(rows: &mut [Value], uid: i64, redact_field: &str) {
    for row in rows.iter_mut() {
        if let Value::Object(obj) = row {
            if obj.contains_key(redact_field) && field_or(obj, redact_field, 0) != uid {
                obj.insert(redact_field.into(), Value::from("redacted"));
            }
        }
    }
}

pub async fn purge_expired_server_sync_rollback_exports(
    pool: &PgPool,
    now: chrono::DateTime<chrono::Utc>,
) -> CommunityDbResult<i64> {
    if !relation_exists(pool, SERVER_SYNC_ROLLBACK_EXPORTS_REL).await? {
        return Ok(0);
    }
    let result = sqlx::query("DELETE FROM server_config.rollback_exports WHERE expires_at <= $1")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(rows_to_i64(result.rows_affected()))
}

const MODERATION_CONTENT_RETENTION_DAYS: i64 = 90;

pub async fn anonymize_expired_moderation_content(
    pool: &PgPool,
    now: chrono::DateTime<chrono::Utc>,
) -> CommunityDbResult<u64> {
    let cutoff = now - chrono::Duration::days(MODERATION_CONTENT_RETENTION_DAYS);
    // ponytail: NULL created_at wird bewusst nicht erfasst; diese Altlast ist akzeptiert.
    let cases = sqlx::query(
        r#"
        UPDATE moderation.ai_moderation_cases
        SET original_content = NULL,
            attachments = NULL,
            user_tag = NULL,
            ai_reason = NULL,
            mod_deny_reason = NULL
        WHERE created_at < $1
          AND original_content IS NOT NULL
        "#,
    )
    .bind(cutoff)
    .execute(pool)
    .await?
    .rows_affected();
    let ragebait_hits = sqlx::query(
        r#"
        UPDATE moderation.ai_moderation_ragebait_hits
        SET content_preview = NULL
        WHERE created_at < $1
          AND content_preview IS NOT NULL
        "#,
    )
    .bind(cutoff)
    .execute(pool)
    .await?
    .rows_affected();
    let security_incidents = sqlx::query(
        r#"
        UPDATE moderation.security_guard_incidents
        SET messages = NULL
        WHERE created_at < $1
          AND messages IS NOT NULL
        "#,
    )
    .bind(cutoff)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(cases + ragebait_hits + security_incidents)
}

pub fn spawn_server_sync_rollback_export_retention(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match purge_expired_server_sync_rollback_exports(&pool, chrono::Utc::now()).await {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!(
                            deleted,
                            "Server-Sync-Rollback-Export-Retention abgeschlossen"
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, "Server-Sync-Rollback-Export-Retention fehlgeschlagen");
                }
            }
            tokio::time::sleep(PRIVACY_RETENTION_JOB_INTERVAL).await;
        }
    })
}

pub fn spawn_moderation_content_retention(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match anonymize_expired_moderation_content(&pool, chrono::Utc::now()).await {
                Ok(anonymized) => {
                    if anonymized > 0 {
                        tracing::info!(anonymized, "Moderation-Content-Retention abgeschlossen");
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, "Moderation-Content-Retention fehlgeschlagen");
                }
            }
            tokio::time::sleep(PRIVACY_RETENTION_JOB_INTERVAL).await;
        }
    })
}

pub async fn is_opted_out(pool: &PgPool, user_id: i64) -> bool {
    match sqlx::query!(
        r#"
        SELECT opted_out
          FROM core.user_privacy
         WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await
    {
        Ok(row) => {
            let opted_out = row.is_some_and(|row| row.opted_out);
            tracing::debug!(user_id, opted_out, "Datenschutz-Opt-out-Entscheidung");
            opted_out
        }
        Err(err) => {
            tracing::error!(user_id, %err, opted_out = true, "Datenschutz-Opt-out-Status nicht lesbar; fail-closed");
            true
        }
    }
}

/// Serialisiert Privacy-Erasure und neue nutzerbezogene Writes fuer denselben User.
pub async fn lock_user_privacy(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> CommunityDbResult<()> {
    dl_central_db::lock_user_privacy(tx, user_id).await?;
    Ok(())
}

pub(crate) async fn erasure_completed_under_lock(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> CommunityDbResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM core.user_privacy
              WHERE user_id = $1 AND deleted_at IS NOT NULL
         )",
    )
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?)
}

pub(crate) async fn scrub_pate_journey_metadata(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> CommunityDbResult<(i64, i64)> {
    let user_id = user_id.to_string();
    let events = sqlx::query(
        "UPDATE activity.journey_events
            SET metadata = metadata - 'pate_id' - 'channel_id'
          WHERE metadata ->> 'pate_id' = $1",
    )
    .bind(&user_id)
    .execute(&mut **tx)
    .await?;
    let states = sqlx::query(
        "UPDATE activity.journey_user_state AS state
            SET metadata = CASE
                WHEN EXISTS (
                    SELECT 1
                      FROM activity.journey_events AS event
                     WHERE event.user_id = state.user_id
                       AND event.guild_id = state.guild_id
                       AND event.event_type = 'steckbrief_posted'
                       AND event.metadata ->> 'channel_id' = state.metadata ->> 'channel_id'
                ) THEN state.metadata - 'pate_id'
                ELSE state.metadata - 'pate_id' - 'channel_id'
            END
          WHERE state.metadata ->> 'pate_id' = $1",
    )
    .bind(&user_id)
    .execute(&mut **tx)
    .await?;
    Ok((
        rows_to_i64(events.rows_affected()),
        rows_to_i64(states.rows_affected()),
    ))
}

async fn scrub_action_outbox_requester(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> CommunityDbResult<(i64, i64)> {
    let deleted = sqlx::query(
        "DELETE FROM bot.action_outbox
          WHERE status <> 'sent'
            AND payload ->> 'requester_id' = $1",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    let redacted = sqlx::query(
        "UPDATE bot.action_outbox
            SET payload = '{}'::jsonb
          WHERE status = 'sent'
            AND payload ->> 'requester_id' = $1",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok((
        rows_to_i64(deleted.rows_affected()),
        rows_to_i64(redacted.rows_affected()),
    ))
}

pub async fn set_opt_in(pool: &PgPool, user_id: i64, now: i64) -> CommunityDbResult<()> {
    let now = utc_from_unix(now)?;
    let mut tx = pool.begin().await?;
    lock_user_privacy(&mut tx, user_id).await?;
    sqlx::query!(
        r#"
        INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
        VALUES ($1, FALSE, NULL, 'user_opt_in', $2)
        ON CONFLICT(user_id) DO UPDATE SET
          opted_out = FALSE,
          deleted_at = NULL,
          reason = excluded.reason,
          updated_at = excluded.updated_at
        "#,
        user_id,
        now,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_user_data(
    pool: &PgPool,
    user_id: i64,
    reason: String,
    now: i64,
) -> CommunityDbResult<DeleteSummary> {
    let now = utc_from_unix(now)?;
    let expired_rollback_exports = purge_expired_server_sync_rollback_exports(pool, now).await?;
    let relations = existing_relations(pool).await?;
    let user_key = user_id.to_string();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    counts.insert(
        "server_config.rollback_exports.expired".to_string(),
        expired_rollback_exports,
    );

    let mut tx = pool.begin().await?;
    dl_central_db::lock_raw_event_retention_erasure(&mut tx).await?;
    lock_user_privacy(&mut tx, user_id).await?;
    let steam_ids = if relations.contains("core.steam_links") {
        steam_ids_for_user_tx(&mut tx, user_id).await?
    } else {
        Vec::new()
    };

    for &spec in USER_TABLES {
        if spec.relation == "activity.message_metadata_events" {
            let (journey_events, journey_states) =
                scrub_pate_journey_metadata(&mut tx, user_id).await?;
            counts.insert(
                "journey_events.metadata.pate_id".to_string(),
                journey_events,
            );
            counts.insert(
                "journey_user_state.metadata.pate_id".to_string(),
                journey_states,
            );
        }
        if !relations.contains(spec.relation) {
            continue;
        }
        let n = delete_rows_user(&mut tx, spec, user_id, &user_key).await?;
        counts.insert(spec.count_key(), n);
    }

    if relations.contains("bot.action_outbox") {
        let (deleted, redacted) = scrub_action_outbox_requester(&mut tx, &user_key).await?;
        counts.insert(
            "action_outbox.payload.requester_id.deleted".to_string(),
            deleted,
        );
        counts.insert(
            "action_outbox.payload.requester_id.redacted".to_string(),
            redacted,
        );
    }

    for &spec in NULLABLE_USER_COLUMNS {
        if !relations.contains(spec.relation) {
            continue;
        }
        let n = null_user_column(&mut tx, spec, user_id, &user_key).await?;
        counts.insert(spec.count_key(), n);
    }

    if relations.contains(USER_CO_PLAYERS_REL) {
        let result = sqlx::query(
            "DELETE FROM activity.user_co_players WHERE user_id = $1 OR co_player_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        counts.insert(
            "user_co_players".to_string(),
            rows_to_i64(result.rows_affected()),
        );
    }

    for sid in &steam_ids {
        for &spec in STEAM_SIDE_TABLES {
            if !relations.contains(spec.relation) {
                continue;
            }
            let n = match steam_lookup(sid, spec.col_type) {
                Some(value) => delete_rows_lookup(&mut tx, spec, value).await?,
                None => 0,
            };
            counts.insert(format!("{}:{}", spec.key_table, sid.text), n);
        }
    }

    if relations.contains(KV_REL) {
        let uid_key = user_id.to_string();
        let sessions =
            sqlx::query("DELETE FROM bot.kv_store WHERE ns = 'ai_onboarding:sessions' AND k = $1")
                .bind(&uid_key)
                .execute(&mut *tx)
                .await?;
        counts.insert(
            "kv_ai_onboarding_sessions".to_string(),
            rows_to_i64(sessions.rows_affected()),
        );

        let pending = sqlx::query(
            "SELECT k, v FROM bot.kv_store WHERE ns = 'ai_onboarding:persistent_views'",
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .filter_map(|row| {
            let key: String = row.try_get("k").ok()?;
            let raw: String = row.try_get("v").ok()?;
            (serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|value| value.get("user_id").and_then(coerce_i64))
                == Some(user_id))
            .then_some(key)
        })
        .collect::<Vec<_>>();

        let mut views = 0i64;
        for key in pending {
            let result = sqlx::query(
                "DELETE FROM bot.kv_store WHERE ns = 'ai_onboarding:persistent_views' AND k = $1",
            )
            .bind(key)
            .execute(&mut *tx)
            .await?;
            views += rows_to_i64(result.rows_affected());
        }
        counts.insert("kv_ai_onboarding_views".to_string(), views);

        let mut nudge = 0i64;
        for ns in ["voice_nudge_first_seen", "voice_nudge_done"] {
            let result = sqlx::query("DELETE FROM bot.kv_store WHERE ns = $1 AND k = $2")
                .bind(ns)
                .bind(&uid_key)
                .execute(&mut *tx)
                .await?;
            nudge += rows_to_i64(result.rows_affected());
        }
        counts.insert("kv_voice_nudge".to_string(), nudge);

        let native_onboarding = sqlx::query(
            "DELETE FROM bot.kv_store
              WHERE ns = $1
                AND (k = $2 OR k LIKE $3)",
        )
        .bind(KV_NATIVE_ONBOARDING_COMPLETED_NS)
        .bind(&uid_key)
        .bind(format!("%:{uid_key}"))
        .execute(&mut *tx)
        .await?;
        counts.insert(
            "kv_native_onboarding_completed".to_string(),
            rows_to_i64(native_onboarding.rows_affected()),
        );

        let deleted_claims = delete_concierge_claims(&mut tx, user_id).await?;
        counts.insert("kv_concierge_claims".to_string(), deleted_claims);
    }

    if relations.contains(USER_PRIVACY_REL) {
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
            VALUES ($1, TRUE, $2, $3, $2)
            ON CONFLICT(user_id) DO UPDATE SET
              opted_out = TRUE,
              deleted_at = excluded.deleted_at,
              reason = excluded.reason,
              updated_at = excluded.updated_at
            "#,
            user_id,
            now,
            reason,
        )
        .execute(&mut *tx)
        .await?;
        counts.insert("user_privacy_updated".to_string(), 1);
    }

    tx.commit().await?;

    Ok(DeleteSummary {
        counts,
        steam_ids: steam_ids.into_iter().map(|sid| sid.text).collect(),
    })
}

pub async fn export_user_data(pool: &PgPool, user_id: i64, now: i64) -> CommunityDbResult<Value> {
    let relations = existing_relations(pool).await?;
    let user_key = user_id.to_string();
    let mut tbl = serde_json::Map::new();

    for &spec in USER_TABLES {
        if !relations.contains(spec.relation) {
            continue;
        }
        tbl.insert(
            spec.count_key(),
            Value::Array(select_rows_user(pool, spec, user_id, &user_key).await?),
        );
    }

    for &spec in NULLABLE_USER_COLUMNS {
        if !relations.contains(spec.relation) {
            continue;
        }
        tbl.insert(
            spec.count_key(),
            Value::Array(select_rows_user(pool, spec, user_id, &user_key).await?),
        );
    }

    if relations.contains("bot.action_outbox") {
        let rows: Value = sqlx::query_scalar(
            "SELECT COALESCE(jsonb_agg(to_jsonb(outbox) - 'user_id' ORDER BY outbox.id), '[]'::jsonb)
               FROM bot.action_outbox AS outbox
              WHERE outbox.user_id <> $1
                AND outbox.payload ->> 'requester_id' = $2",
        )
        .bind(user_id)
        .bind(&user_key)
        .fetch_one(pool)
        .await?;
        tbl.insert("action_outbox.payload.requester_id".into(), rows);
    }

    if relations.contains("bot.faq_chat_sessions")
        && relation_exists(pool, "bot.faq_chat_messages").await?
    {
        let rows: Value = sqlx::query_scalar(
            r#"
            SELECT COALESCE(jsonb_agg(to_jsonb(message) ORDER BY message.id), '[]'::jsonb)
              FROM bot.faq_chat_messages message
              JOIN bot.faq_chat_sessions session USING (session_id)
             WHERE session.user_id = $1
            "#,
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;
        tbl.insert("faq_chat_messages.session_id".into(), rows);
    }

    if relations.contains(USER_CO_PLAYERS_REL) {
        let rows = select_rows_i64(
            pool,
            TableSpec::new(
                "user_co_players",
                "user_id",
                USER_CO_PLAYERS_REL,
                "user_id",
                ColumnType::I64,
            ),
            user_id,
        )
        .await?;
        let mut other_side = select_rows_i64(
            pool,
            TableSpec::new(
                "user_co_players",
                "co_player_id",
                USER_CO_PLAYERS_REL,
                "co_player_id",
                ColumnType::I64,
            ),
            user_id,
        )
        .await?;
        let mut rows = rows;
        rows.append(&mut other_side);
        tbl.insert(
            "user_co_players".into(),
            Value::Array(redact_co_players(rows, user_id)),
        );
    }

    let steam_ids = steam_ids_for_user(pool, user_id).await?;
    for sid in &steam_ids {
        for &spec in STEAM_SIDE_TABLES {
            if !relations.contains(spec.relation) {
                continue;
            }
            let rows = match steam_lookup(sid, spec.col_type) {
                Some(value) => select_rows_lookup(pool, spec, value).await?,
                None => Vec::new(),
            };
            tbl.insert(
                format!("{}:{}", spec.key_table, sid.text),
                Value::Array(rows),
            );
        }
    }

    if let Some(Value::Array(rows)) = tbl.get_mut("voice_session_log.user_id") {
        for row in rows.iter_mut() {
            if let Value::Object(obj) = row {
                if obj.contains_key("co_player_ids") {
                    obj.insert("co_player_ids".into(), Value::Null);
                }
            }
        }
    }
    if let Some(Value::Array(rows)) = tbl.get_mut("tempvoice_bans.owner_id") {
        redact_other_id(rows, user_id, "banned_id");
    }
    if let Some(Value::Array(rows)) = tbl.get_mut("tempvoice_bans.banned_id") {
        redact_other_id(rows, user_id, "owner_id");
    }
    if let Some(Value::Array(rows)) = tbl.get_mut("concierge_patenschaften_user.user_id") {
        redact_other_id(rows, user_id, "pate_id");
    }
    if let Some(Value::Array(rows)) = tbl.get_mut("concierge_patenschaften_pate.pate_id") {
        redact_other_id(rows, user_id, "user_id");
    }

    let mut kv_out = serde_json::Map::new();
    if relations.contains(KV_REL) {
        let uid_key = user_id.to_string();
        kv_out.insert(
            "ai_onboarding_sessions".into(),
            kv_value(pool, "ai_onboarding:sessions", &uid_key)
                .await?
                .unwrap_or(Value::Null),
        );

        let rows = sqlx::query!(
            r#"
            SELECT v
              FROM bot.kv_store
             WHERE ns = 'ai_onboarding:persistent_views'
            "#
        )
        .fetch_all(pool)
        .await?;
        let views = rows
            .into_iter()
            .filter_map(|row| serde_json::from_str::<Value>(&row.v).ok())
            .filter(|value| value.get("user_id").and_then(coerce_i64) == Some(user_id))
            .collect::<Vec<_>>();
        kv_out.insert("ai_onboarding_views".into(), Value::Array(views));
        kv_out.insert(
            "voice_nudge".into(),
            serde_json::json!({
                "first_seen": kv_value(pool, "voice_nudge_first_seen", &uid_key).await?,
                "done": kv_value(pool, "voice_nudge_done", &uid_key).await?,
            }),
        );
        let rows = sqlx::query(
            "SELECT k, v
               FROM bot.kv_store
              WHERE ns = $1
                AND (k = $2 OR k LIKE $3)",
        )
        .bind(KV_NATIVE_ONBOARDING_COMPLETED_NS)
        .bind(&uid_key)
        .bind(format!("%:{uid_key}"))
        .fetch_all(pool)
        .await?;
        let values = rows
            .into_iter()
            .filter_map(|row| {
                let key: String = row.try_get("k").ok()?;
                let raw: String = row.try_get("v").ok()?;
                Some(serde_json::json!({
                    "key": key,
                    "value": serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw)),
                }))
            })
            .collect::<Vec<_>>();
        kv_out.insert("native_onboarding_completed".into(), Value::Array(values));

        let claims = sqlx::query(
            "SELECT ns, k, v
               FROM bot.kv_store
              WHERE ns IN ($1, $2, $3, $4)
              ORDER BY ns, k",
        )
        .bind(KV_CONCIERGE_T0_NS)
        .bind(KV_CONCIERGE_FALLBACK_NS)
        .bind(KV_CONCIERGE_PATE_CLAIM_NS)
        .bind(KV_CONCIERGE_STECKBRIEF_REVOKED_NS)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| {
            let ns: String = row.try_get("ns").ok()?;
            let key: String = row.try_get("k").ok()?;
            let raw: String = row.try_get("v").ok()?;
            concierge_claim_belongs_to_user(&ns, &key, &raw, user_id)
                .then(|| concierge_claim_export(&ns, key, raw, user_id))
        })
        .collect::<Vec<_>>();
        kv_out.insert("concierge_claims".into(), Value::Array(claims));
    }

    let user_privacy = if relations.contains(USER_PRIVACY_REL) {
        Value::Array(
            select_rows_i64(
                pool,
                TableSpec::new(
                    "user_privacy",
                    "user_id",
                    USER_PRIVACY_REL,
                    "user_id",
                    ColumnType::I64,
                ),
                user_id,
            )
            .await?,
        )
    } else {
        Value::Null
    };

    Ok(serde_json::json!({
        "user_id": user_id,
        "generated_at": now,
        "tables": Value::Object(tbl),
        "kv": Value::Object(kv_out),
        "steam_ids": steam_ids.into_iter().map(|sid| sid.text).collect::<Vec<_>>(),
        "user_privacy": user_privacy,
    }))
}

#[cfg(test)]
mod privacy_contract_tests {
    use super::*;
    use regex::Regex;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    fn privacy_user_table_columns() -> BTreeSet<(String, String)> {
        let mut out = USER_TABLES
            .iter()
            .chain(NULLABLE_USER_COLUMNS.iter())
            .map(|spec| (spec.relation.to_string(), spec.col.to_string()))
            .collect::<BTreeSet<_>>();
        out.insert((USER_CO_PLAYERS_REL.to_string(), "user_id".to_string()));
        out
    }

    fn privacy_contract_allowlist() -> BTreeSet<(String, String)> {
        let mut out = BTreeSet::new();
        // `core.user_privacy` ist der notwendige Opt-out-/Erasure-Grabstein:
        // diese eine User-ID bleibt bewusst erhalten, damit zukuenftige Writes
        // geblockt werden und der Delete-Zeitpunkt auditierbar bleibt.
        out.insert((USER_PRIVACY_REL.to_string(), "user_id".to_string()));
        // Das Discord-Guild-Audit-Log bleibt als unveraenderliche Sicherheits-
        // und Aenderungshistorie vollstaendig erhalten. `user_id` bezeichnet
        // den Actor, `target_id` kann je nach Aktion einen User bezeichnen und
        // `changes`, `options` sowie `reason` koennen weitere personenbezogene
        // Werte enthalten. Die restlichen Felder sichern Ereignis, Guild,
        // Aktion und Zeitbezug; normale Community-Aktivitaet wird hier nicht
        // protokolliert. Die vollstaendige Liste macht Schema-Drift testbar.
        for column in [
            "entry_id",
            "guild_id",
            "action_type",
            "user_id",
            "target_id",
            "changes",
            "options",
            "reason",
            "occurred_at",
            "ingested_at",
        ] {
            out.insert(("core.discord_audit_log".to_string(), column.to_string()));
        }
        // `server_config.*`: Audit-Referenzen auf Mod-/Admin-AKTIONEN am
        // Server-Soll-Modell (wer hat Diff erstellt / Apply angefordert /
        // Drift adoptiert). Kein Community-Verhaltensdatum; Aufbewahrung zur
        // Nachvollziehbarkeit administrativer Server-Änderungen
        // (berechtigtes Interesse). Ein Opt-out darf die Änderungs-
        // Historie des Servers nicht zerstoeren.
        out.insert((
            "server_config.diff_previews".to_string(),
            "created_by_user_id".to_string(),
        ));
        out.insert((
            "server_config.apply_runs".to_string(),
            "requested_by_user_id".to_string(),
        ));
        out.insert((
            "server_config.adoption_events".to_string(),
            "adopted_by_user_id".to_string(),
        ));
        // Rollback-Artefakte können verschachtelte Member-IDs im JSON
        // enthalten; sie sind deshalb über `expires_at` auf 180 Tage
        // begrenzt und werden durch den Privacy-Retention-Purge gelöscht.
        // Die Admin-ID bleibt nur als Ersteller-Auditreferenz allowlisted.
        out.insert((
            "server_config.rollback_exports".to_string(),
            "created_by_user_id".to_string(),
        ));
        // `core.discord_audit_log.user_id` ist der AUSFÜHRENDE einer Moderations-
        // aktion (Discord-Semantik: user_id handelt, target_id ist betroffen).
        // Gleiche Kategorie wie server_config.*: Audit einer Admin-Aktion,
        // Aufbewahrung im berechtigten Interesse. Ein Opt-out des Moderators darf
        // die Moderationshistorie des Servers nicht löschen.
        out.insert(("core.discord_audit_log".to_string(), "user_id".to_string()));
        // `steam.invite_requests.admin_id` ist der Admin, der `/invite` ausgeloest
        // hat — Audit-Referenz auf eine Admin-AKTION, gleiche Kategorie wie
        // server_config.*. Sein Opt-out darf den offenen Invite eines Dritten
        // nicht mitreissen; der Eingeladene selbst wird über `target_discord_id`
        // in USER_TABLES gelöscht, und die Zeile lebt ohnehin nur 24 h.
        out.insert(("steam.invite_requests".to_string(), "admin_id".to_string()));
        out
    }

    fn is_user_id_like_column(column: &str) -> bool {
        column == "user_id"
            || column.ends_with("_user_id")
            || column == "discord_id"
            || column == "member_id"
            || column == "owner_id"
            || (column.starts_with("actor") && column.ends_with("_id"))
            || column.starts_with("target_user")
    }

    fn normalize_sql_ident(identifier: &str) -> String {
        identifier.trim().trim_matches('"').replace('"', "")
    }

    fn create_table_columns_from_sql(raw: &str) -> BTreeSet<(String, String)> {
        let without_line_comments = raw
            .lines()
            .map(|line| line.split_once("--").map_or(line, |(sql, _)| sql))
            .collect::<Vec<_>>()
            .join("\n");
        let create = Regex::new(
            r#"(?is)\bCREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?((?:"[^"]+"|[a-z_][a-z0-9_$]*)\s*\.\s*(?:"[^"]+"|[a-z_][a-z0-9_$]*))\s*\((.*?)\)\s*;"#,
        )
        .expect("CREATE TABLE regex");
        let mut columns = BTreeSet::new();
        for captures in create.captures_iter(&without_line_comments) {
            let relation = normalize_sql_ident(&captures[1])
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>();
            for line in captures[2].lines() {
                let Some(column) = line.trim().trim_end_matches(',').split_whitespace().next()
                else {
                    continue;
                };
                let column = normalize_sql_ident(column.trim_end_matches(','));
                if ![
                    "CHECK",
                    "CONSTRAINT",
                    "EXCLUDE",
                    "FOREIGN",
                    "PRIMARY",
                    "UNIQUE",
                ]
                .iter()
                .any(|keyword| column.eq_ignore_ascii_case(keyword))
                {
                    columns.insert((relation.clone(), column));
                }
            }
        }
        columns
    }

    fn table_columns_from_sql(raw: &str, relation: &str) -> BTreeSet<String> {
        let mut columns = create_table_columns_from_sql(raw)
            .into_iter()
            .filter_map(|(table, column)| table.eq_ignore_ascii_case(relation).then_some(column))
            .collect::<BTreeSet<_>>();

        let without_line_comments = raw
            .lines()
            .map(|line| line.split_once("--").map_or(line, |(sql, _)| sql))
            .collect::<Vec<_>>()
            .join("\n");
        // ponytail: Repo-Migrationen bleiben direktes DDL; bei dynamischem SQL auf Schematest wechseln.
        let alter = Regex::new(
            r#"(?is)\bALTER\s+TABLE\s+(?:IF\s+EXISTS\s+)?(?:ONLY\s+)?((?:"[^"]+"|[a-z_][a-z0-9_$]*)\s*\.\s*(?:"[^"]+"|[a-z_][a-z0-9_$]*))\s+([^;]+)"#,
        )
        .expect("ALTER TABLE regex");
        let add_column = Regex::new(
            r#"(?im)(?:^|,)\s*ADD\s+(?:COLUMN\s+)?(?:IF\s+NOT\s+EXISTS\s+)?("[^"]+"|[a-z_][a-z0-9_$]*)"#,
        )
        .expect("ADD COLUMN regex");
        for captures in alter.captures_iter(&without_line_comments) {
            let table = normalize_sql_ident(&captures[1])
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>();
            if table.eq_ignore_ascii_case(relation) {
                columns.extend(add_column.captures_iter(&captures[2]).filter_map(|added| {
                    let column = normalize_sql_ident(&added[1]);
                    (![
                        "CHECK",
                        "CONSTRAINT",
                        "EXCLUDE",
                        "FOREIGN",
                        "PRIMARY",
                        "UNIQUE",
                    ]
                    .iter()
                    .any(|keyword| column.eq_ignore_ascii_case(keyword)))
                    .then_some(column)
                }));
            }
        }

        columns
    }

    fn migration_columns_for_relation(relation: &str) -> BTreeSet<String> {
        let migration_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("dl-community under crates")
            .join("dl-central-db/migrations");
        let entries = fs::read_dir(&migration_dir)
            .unwrap_or_else(|err| panic!("read {}: {err}", migration_dir.display()));
        let mut columns = BTreeSet::new();
        for entry in entries {
            let path = entry
                .unwrap_or_else(|err| panic!("read_dir entry {}: {err}", migration_dir.display()))
                .path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
                continue;
            }
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
            columns.extend(table_columns_from_sql(&raw, relation));
        }
        columns
    }

    fn migration_user_id_columns() -> BTreeSet<(String, String)> {
        let migration_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("dl-community under crates")
            .join("dl-central-db/migrations");
        let mut out = BTreeSet::new();
        let entries = fs::read_dir(&migration_dir)
            .unwrap_or_else(|err| panic!("read {}: {err}", migration_dir.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|err| panic!("read_dir entry {}: {err}", migration_dir.display()))
                .path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
                continue;
            }
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
            out.extend(
                create_table_columns_from_sql(&raw)
                    .into_iter()
                    .filter(|(_, column)| is_user_id_like_column(column)),
            );
        }
        out
    }

    #[test]
    fn rollback_exports_json_member_ids_sind_retention_gebunden() {
        let migration = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("dl-community under crates")
            .join("dl-central-db/migrations/2026070240_server_sync_rollback_exports.sql");
        let raw = fs::read_to_string(&migration)
            .unwrap_or_else(|err| panic!("read {}: {err}", migration.display()));
        assert!(raw.contains("expires_at TIMESTAMPTZ NOT NULL DEFAULT"));
        assert!(raw.contains("INTERVAL '180 days'"));
        assert!(raw.contains("rollback_exports_expires_at_idx"));
    }

    #[test]
    fn discord_audit_log_ist_vollstaendig_im_permanenten_privacy_vertrag() {
        let schema_columns = migration_columns_for_relation("core.discord_audit_log");
        let retained_columns = privacy_contract_allowlist()
            .into_iter()
            .filter_map(|(relation, column)| {
                (relation == "core.discord_audit_log").then_some(column)
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            retained_columns, schema_columns,
            "jede Spalte der unveraenderlichen Guild-Audit-Historie muss explizit eingeordnet sein"
        );
    }

    #[test]
    fn migrationsscanner_erkennt_spaetere_audit_add_columns() {
        let sql = r#"
            -- spaetere Migration, absichtlich in mehreren ueblichen Schreibweisen
            ALTER TABLE core.discord_audit_log
                ADD COLUMN target_display_name TEXT,
                ADD COLUMN IF NOT EXISTS moderator_note TEXT;
            alter table if exists "core"."discord_audit_log"
                add column "target_profile" jsonb;
            ALTER TABLE ONLY core.discord_audit_log ADD member_note TEXT;
            ALTER TABLE CORE.DISCORD_AUDIT_LOG ADD COLUMN uppercase_note TEXT;
            ALTER TABLE core.discord_audit_log ADD CHECK (action_type >= 0);
            ALTER TABLE core.other_table ADD COLUMN ignored TEXT;
        "#;

        assert_eq!(
            table_columns_from_sql(sql, "core.discord_audit_log"),
            BTreeSet::from([
                "member_note".to_string(),
                "moderator_note".to_string(),
                "target_display_name".to_string(),
                "target_profile".to_string(),
                "uppercase_note".to_string(),
            ])
        );
    }

    #[test]
    fn migrationsscanner_erkennt_create_table_mit_und_ohne_if_not_exists() {
        let sql = r#"
            CREATE TABLE core.plain_events (
                id BIGSERIAL PRIMARY KEY,
                discord_id BIGINT NULL
            );
            CREATE TABLE IF NOT EXISTS core.guarded_events (
                id BIGSERIAL PRIMARY KEY,
                owner_id BIGINT NULL
            );
        "#;

        assert_eq!(
            table_columns_from_sql(sql, "core.plain_events"),
            BTreeSet::from(["discord_id".to_string(), "id".to_string()])
        );
        assert_eq!(
            table_columns_from_sql(sql, "core.guarded_events"),
            BTreeSet::from(["id".to_string(), "owner_id".to_string()])
        );
    }

    #[test]
    fn steam_bot_event_log_steam_id_index_liegt_in_additiver_migration() {
        let migration_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("dl-community under crates")
            .join("dl-central-db/migrations");
        let published = migration_dir.join("2026071111_steam_bot_event_log.sql");
        let published_raw = fs::read_to_string(&published)
            .unwrap_or_else(|err| panic!("read {}: {err}", published.display()));
        assert!(!published_raw.contains("bot_event_log_steam_id_idx"));

        let migration = migration_dir.join("2026071124_steam_bot_event_log_steam_id_idx.sql");
        assert!(
            migration.exists(),
            "Steam-ID-Index muss als neue additive Migration vorliegen"
        );
        let raw = fs::read_to_string(&migration)
            .unwrap_or_else(|err| panic!("read {}: {err}", migration.display()));
        assert_eq!(
            raw,
            "CREATE INDEX IF NOT EXISTS bot_event_log_steam_id_idx\n    ON steam.bot_event_log (steam_id);\n"
        );
    }

    #[test]
    fn steam_bot_event_log_ist_ueber_beide_identitaeten_im_privacy_vertrag() {
        assert!(USER_TABLES
            .iter()
            .any(|spec| { spec.relation == "steam.bot_event_log" && spec.col == "discord_id" }));
        assert!(STEAM_SIDE_TABLES
            .iter()
            .any(|spec| { spec.relation == "steam.bot_event_log" && spec.col == "steam_id" }));

        let allowlist = privacy_contract_allowlist();
        assert!(!allowlist.contains(&("steam.bot_event_log".to_string(), "discord_id".to_string())));
        assert!(!allowlist.contains(&("steam.bot_event_log".to_string(), "steam_id".to_string())));
    }

    #[test]
    fn alle_migration_user_id_spalten_sind_im_privacy_vertrag() {
        let schema_tables = migration_user_id_columns();
        let privacy_tables = privacy_user_table_columns();
        let allowlist = privacy_contract_allowlist();
        let missing = schema_tables
            .difference(&privacy_tables)
            .filter(|entry| !allowlist.contains(*entry))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "User-ID-Spalten fehlen in privacy.rs USER_TABLES oder Allowlist: {missing:?}"
        );
    }

    #[tokio::test]
    async fn is_opted_out_sperrt_bei_nicht_erreichbarem_privacy_status() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://postgres@localhost/privacy_test")
            .expect("lazy pool");
        pool.close().await;

        assert!(is_opted_out(&pool, 7).await);
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use dl_activity::journey::compact_raw_events;
    use dl_central_db::testing::{test_pool, TestDb};
    use std::collections::BTreeSet;

    async fn mk_db() -> TestDb {
        test_pool().await.expect("test_pool")
    }

    async fn wait_for_db_lock(pool: &PgPool, query_fragment: &str, wait_event: Option<&str>) {
        let query_pattern = format!("%{query_fragment}%");
        tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                let waiting = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(
                        SELECT 1
                          FROM pg_stat_activity
                         WHERE datname = current_database()
                           AND pid <> pg_backend_pid()
                           AND state = 'active'
                           AND wait_event_type = 'Lock'
                           AND query LIKE $1
                           AND ($2::TEXT IS NULL OR wait_event = $2)
                    )",
                )
                .bind(&query_pattern)
                .bind(wait_event)
                .fetch_one(pool)
                .await
                .expect("pg_stat_activity");
                if waiting {
                    return;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("DB-Lock-Wait fuer {query_fragment} nicht sichtbar"));
    }

    async fn wait_for_db_lock_count(pool: &PgPool, minimum: i64) {
        tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                let waiting = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*)
                       FROM pg_stat_activity
                      WHERE datname = current_database()
                        AND pid <> pg_backend_pid()
                        AND state = 'active'
                        AND wait_event_type = 'Lock'",
                )
                .fetch_one(pool)
                .await
                .expect("pg_stat_activity");
                if waiting >= minimum {
                    return;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("nur weniger als {minimum} DB-Lock-Waits sichtbar"));
    }

    async fn set_opt_out_for_test(pool: &PgPool, uid: i64) {
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
            VALUES ($1, TRUE, now())
            ON CONFLICT(user_id) DO UPDATE SET opted_out = TRUE
            "#,
            uid,
        )
        .execute(pool)
        .await
        .expect("set opt-out");
    }

    #[tokio::test]
    async fn is_opted_out_ist_wertbasiert() {
        let db = mk_db().await;
        assert!(!is_opted_out(db.pool(), 7).await);

        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
            VALUES (7, FALSE, now())
            "#
        )
        .execute(db.pool())
        .await
        .expect("insert privacy");
        assert!(!is_opted_out(db.pool(), 7).await);

        set_opt_out_for_test(db.pool(), 7).await;
        assert!(is_opted_out(db.pool(), 7).await);
    }

    #[tokio::test]
    async fn set_opt_in_hebt_opt_out_auf() {
        let db = mk_db().await;
        delete_user_data(db.pool(), 5, "test".into(), 1000)
            .await
            .expect("delete");
        assert!(is_opted_out(db.pool(), 5).await);
        set_opt_in(db.pool(), 5, 2000).await.expect("opt in");
        assert!(!is_opted_out(db.pool(), 5).await);
    }

    /// Legt einen offenen `/invite`-Request an: eingeladen wurde `target`,
    /// ausgeloest hat ihn `admin`. Bewusst OHNE `core.steam_links`-Eintrag —
    /// genau so sieht der Normalfall aus, denn eingeladen wird jemand, der
    /// noch nicht verknuepft ist.
    async fn seed_invite_request(pool: &PgPool, admin: i64, target: i64) {
        sqlx::query(
            r#"
            INSERT INTO steam.invite_requests
                (steam_id64, account_id, admin_id, target_discord_id, created_at)
            VALUES (76561199813018551, 1852752823, $1, $2, 1000)
            "#,
        )
        .bind(admin)
        .bind(target)
        .execute(pool)
        .await
        .expect("seed invite request");
    }

    async fn invite_requests_count(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*)::int8 FROM steam.invite_requests")
            .fetch_one(pool)
            .await
            .expect("count invite requests")
    }

    #[tokio::test]
    async fn erasure_loescht_offenen_invite_auch_ohne_steam_link() {
        let db = mk_db().await;
        seed_invite_request(db.pool(), 999, 4242).await;

        delete_user_data(db.pool(), 4242, "test".into(), 2000)
            .await
            .expect("delete");

        // Über STEAM_SIDE_TABLES allein wäre die Zeile unerreichbar:
        // steam_ids_for_user liest nur core.steam_links, und der Eingeladene
        // hat dort (noch) nichts stehen.
        assert_eq!(
            invite_requests_count(db.pool()).await,
            0,
            "offener Invite überlebt die Löschanfrage des Eingeladenen"
        );
    }

    #[tokio::test]
    async fn erasure_des_admins_loescht_fremde_invites_nicht() {
        let db = mk_db().await;
        seed_invite_request(db.pool(), 999, 4242).await;

        delete_user_data(db.pool(), 999, "test".into(), 2000)
            .await
            .expect("delete");

        // `admin_id` ist eine Audit-Referenz auf eine Admin-AKTION (allowlisted).
        // Ein Opt-out des Admins darf den offenen Invite eines Dritten nicht
        // mitreissen.
        assert_eq!(
            invite_requests_count(db.pool()).await,
            1,
            "Löschanfrage des Admins hat den Invite eines Dritten gelöscht"
        );
    }

    #[tokio::test]
    async fn steam_bot_event_log_wird_exportiert_und_einmal_vollstaendig_geloescht() {
        let db = mk_db().await;
        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42)")
            .execute(db.pool())
            .await
            .expect("core user");
        sqlx::query(
            "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
             VALUES (42, '4242', 4242)",
        )
        .execute(db.pool())
        .await
        .expect("steam link");
        sqlx::query(
            "INSERT INTO steam.bot_event_log(event_type, decision, discord_id, steam_id) VALUES
             ('discord-only', 'yes', 42, NULL),
             ('steam-only', 'yes', NULL, 4242),
             ('both', 'yes', 42, 4242),
             ('foreign', 'yes', 99, 9999)",
        )
        .execute(db.pool())
        .await
        .expect("event log");

        let export = export_user_data(db.pool(), 42, 1_000)
            .await
            .expect("privacy export");
        let discord_rows = export["tables"]["bot_event_log.discord_id"]
            .as_array()
            .expect("Discord Event-Log im Export");
        let steam_rows = export["tables"]["bot_event_log:4242"]
            .as_array()
            .expect("Steam Event-Log im Export");
        assert_eq!(
            discord_rows
                .iter()
                .map(|row| row["event_type"].as_str().expect("event type"))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["both", "discord-only"])
        );
        assert_eq!(
            steam_rows
                .iter()
                .map(|row| row["event_type"].as_str().expect("event type"))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["both", "steam-only"])
        );

        let summary = delete_user_data(db.pool(), 42, "test".into(), 2_000)
            .await
            .expect("privacy delete");
        assert_eq!(summary.counts.get("bot_event_log.discord_id"), Some(&2));
        assert_eq!(summary.counts.get("bot_event_log:4242"), Some(&1));
        assert_eq!(
            summary.sum(&["bot_event_log.discord_id", "bot_event_log:4242"]),
            3,
            "eine Zeile mit beiden IDs darf im Lösch-Summary nicht doppelt zählen"
        );
        let remaining = sqlx::query_scalar::<_, String>(
            "SELECT event_type FROM steam.bot_event_log ORDER BY id",
        )
        .fetch_all(db.pool())
        .await
        .expect("remaining event log");
        assert_eq!(remaining, ["foreign"]);
    }

    #[tokio::test]
    async fn opt_in_wartet_hinter_laufender_loeschung_und_bestimmt_endzustand() {
        let db = mk_db().await;
        let pool = db.pool().clone();
        sqlx::query(
            "INSERT INTO bot.concierge_profiles(
                 user_id, guild_id, last_interaction_at, created_at, updated_at
             ) VALUES(42, 1, now(), now(), now())",
        )
        .execute(&pool)
        .await
        .expect("profile");
        let mut blocker = pool.begin().await.expect("blocker tx");
        sqlx::query("SELECT user_id FROM bot.concierge_profiles WHERE user_id = 42 FOR UPDATE")
            .fetch_one(&mut *blocker)
            .await
            .expect("profile row lock");
        let erase_pool = pool.clone();
        let erase = tokio::spawn(async move {
            delete_user_data(&erase_pool, 42, "test".to_string(), 1_000).await
        });
        wait_for_db_lock(
            &pool,
            "DELETE FROM bot.concierge_profiles",
            Some("transactionid"),
        )
        .await;
        let optin_pool = pool.clone();
        let optin = tokio::spawn(async move { set_opt_in(&optin_pool, 42, 2_000).await });
        wait_for_db_lock(&pool, "pg_advisory_xact_lock", Some("advisory")).await;
        blocker.commit().await.expect("release profile row");

        erase.await.expect("erase task").expect("erase");
        optin.await.expect("optin task").expect("optin");
        let state = sqlx::query_as::<_, (bool, Option<chrono::DateTime<Utc>>, String)>(
            "SELECT opted_out, deleted_at, reason FROM core.user_privacy WHERE user_id = 42",
        )
        .fetch_one(&pool)
        .await
        .expect("privacy state");
        assert_eq!(state, (false, None, "user_opt_in".to_string()));
    }

    #[tokio::test]
    async fn loeschung_sieht_write_first_steam_link_erst_nach_privacy_lock() {
        let db = mk_db().await;
        let pool = db.pool().clone();
        sqlx::query("INSERT INTO core.users(discord_id) VALUES(42)")
            .execute(&pool)
            .await
            .expect("core user");
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let writer_pool = pool.clone();
        let writer_started = started.clone();
        let writer_release = release.clone();
        let writer = tokio::spawn(async move {
            let mut tx = writer_pool.begin().await.expect("writer tx");
            lock_user_privacy(&mut tx, 42).await.expect("privacy lock");
            sqlx::query(
                "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
                 VALUES(42, 'STEAM_RACE_42', 4242)",
            )
            .execute(&mut *tx)
            .await
            .expect("steam link");
            sqlx::query(
                "INSERT INTO activity.live_player_state(steam_id, last_gameid)
                 VALUES('STEAM_RACE_42', 'race')",
            )
            .execute(&mut *tx)
            .await
            .expect("steam side row");
            writer_started.notify_one();
            writer_release.notified().await;
            tx.commit().await.expect("writer commit");
        });
        tokio::time::timeout(StdDuration::from_secs(5), started.notified())
            .await
            .expect("writer start");
        let erase_pool = pool.clone();
        let erase = tokio::spawn(async move {
            delete_user_data(&erase_pool, 42, "test".to_string(), 1_000).await
        });
        wait_for_db_lock(&pool, "pg_advisory_xact_lock", Some("advisory")).await;
        release.notify_one();

        writer.await.expect("writer task");
        erase.await.expect("erase task").expect("erase");
        let links = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM core.steam_links WHERE discord_id = 42",
        )
        .fetch_one(&pool)
        .await
        .expect("links");
        let side_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM activity.live_player_state WHERE steam_id = 'STEAM_RACE_42'",
        )
        .fetch_one(&pool)
        .await
        .expect("side rows");
        assert_eq!((links, side_rows), (0, 0));
    }

    #[tokio::test]
    async fn rollback_export_retention_purged_abgelaufene_artefakte() {
        let db = mk_db().await;
        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO server_config.rollback_exports(
                guild_id, created_by_user_id, artifact_hash, artifact_json, metadata, expires_at
            )
            VALUES
              (1, 42, 'old', $1::text::jsonb, '{}'::jsonb, $2),
              (1, 42, 'new', $1::text::jsonb, '{}'::jsonb, $3)
            "#,
        )
        .bind(r#"{"member_role_assignments":[{"member_id":42,"role_ids":[1]}]}"#)
        .bind(now - chrono::Duration::seconds(1))
        .bind(now + chrono::Duration::days(180))
        .execute(db.pool())
        .await
        .expect("insert rollback exports");

        let deleted = purge_expired_server_sync_rollback_exports(db.pool(), now)
            .await
            .expect("purge");
        assert_eq!(deleted, 1);

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*)::int8 FROM server_config.rollback_exports")
                .fetch_one(db.pool())
                .await
                .expect("remaining");
        let newest_hash: String =
            sqlx::query_scalar("SELECT artifact_hash FROM server_config.rollback_exports")
                .fetch_one(db.pool())
                .await
                .expect("hash");
        assert_eq!(remaining, 1);
        assert_eq!(newest_hash, "new");
    }

    #[tokio::test]
    async fn moderation_content_retention_anonymisiert_nur_alte_inhalte() {
        let db = mk_db().await;
        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO moderation.ai_moderation_cases(
                case_id, guild_id, channel_id, message_id, user_id,
                original_content, ai_category, created_at
            )
            VALUES
              ('retention-old', 1, 2, 3, 4, 'geheim', 'spam', $1),
              ('retention-new', 1, 2, 4, 5, 'frisch', NULL, $2)
            "#,
        )
        .bind(now - chrono::Duration::days(91))
        .bind(now - chrono::Duration::days(1))
        .execute(db.pool())
        .await
        .expect("insert moderation cases");

        let anonymized = anonymize_expired_moderation_content(db.pool(), now)
            .await
            .expect("anonymize");
        assert_eq!(anonymized, 1);

        let old: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT original_content, ai_category FROM moderation.ai_moderation_cases WHERE case_id = 'retention-old'",
        )
        .fetch_one(db.pool())
        .await
        .expect("old case");
        let fresh: Option<String> = sqlx::query_scalar(
            "SELECT original_content FROM moderation.ai_moderation_cases WHERE case_id = 'retention-new'",
        )
        .fetch_one(db.pool())
        .await
        .expect("fresh case");

        assert!(old.0.is_none());
        assert_eq!(old.1.as_deref(), Some("spam"));
        assert_eq!(fresh.as_deref(), Some("frisch"));
    }

    #[tokio::test]
    async fn delete_loescht_alle_seiten_und_setzt_grabstein() {
        let db = mk_db().await;
        sqlx::query!("INSERT INTO core.users(discord_id) VALUES (42), (99), (7), (1), (2)")
            .execute(db.pool())
            .await
            .expect("users");
        sqlx::query!(
            "INSERT INTO voice.voice_stats(user_id,total_seconds,total_points) VALUES (42,1,1),(99,1,1)"
        )
        .execute(db.pool())
        .await
        .expect("voice_stats");
        sqlx::query!(
            "INSERT INTO activity.message_activity(user_id,guild_id,message_count) VALUES (42,1,10)"
        )
        .execute(db.pool())
        .await
        .expect("message_activity");
        sqlx::query(
            r#"
            INSERT INTO activity.journey_events(user_id, guild_id, event_type, event_source)
            VALUES (42, 1, 'join', 'privacy_test'), (99, 1, 'join', 'privacy_test')
            "#,
        )
        .execute(db.pool())
        .await
        .expect("journey_events");
        sqlx::query(
            r#"
            INSERT INTO activity.journey_user_state(user_id, guild_id, joined_at)
            VALUES (42, 1, now()), (99, 1, now())
            "#,
        )
        .execute(db.pool())
        .await
        .expect("journey_user_state");
        sqlx::query(
            r#"
            INSERT INTO activity.message_metadata_events(
                user_id, guild_id, channel_id, message_id, occurred_at,
                message_length, has_attachment, attachment_count, is_reply
            )
            VALUES (42, 1, 10, 420, now(), 5, false, 0, false),
                   (99, 1, 10, 990, now(), 7, false, 0, false)
            "#,
        )
        .execute(db.pool())
        .await
        .expect("message_metadata_events");
        sqlx::query(
            r#"
            INSERT INTO activity.voice_metadata_events(user_id, guild_id, channel_id, event_type)
            VALUES (42, 1, 20, 'join'), (99, 1, 20, 'join')
            "#,
        )
        .execute(db.pool())
        .await
        .expect("voice_metadata_events");
        sqlx::query(
            r#"
            INSERT INTO activity.voice_open_sessions(user_id, guild_id, channel_id, joined_at)
            VALUES (42, 1, 20, now()), (99, 1, 20, now())
            "#,
        )
        .execute(db.pool())
        .await
        .expect("voice_open_sessions");
        sqlx::query(
            r#"
            INSERT INTO activity.interaction_events(
                user_id, guild_id, interaction_id, interaction_kind, route
            )
            VALUES (42, 1, 4200, 'component', 'privacy:test'),
                   (99, 1, 9900, 'component', 'privacy:test')
            "#,
        )
        .execute(db.pool())
        .await
        .expect("interaction_events");
        sqlx::query(
            r#"
            INSERT INTO activity.presence_daily_seen(guild_id, user_id, day)
            VALUES (1, 42, CURRENT_DATE), (1, 99, CURRENT_DATE)
            "#,
        )
        .execute(db.pool())
        .await
        .expect("presence_daily_seen");
        sqlx::query(
            r#"
            INSERT INTO activity.guild_member_directory(
                guild_id, user_id, joined_at, account_created_at, is_bot, present, synced_at
            )
            VALUES
              (1, 42, now(), now(), false, true, now()),
              (1, 99, now(), now(), false, true, now())
            "#,
        )
        .execute(db.pool())
        .await
        .expect("guild_member_directory");
        sqlx::query!(
            r#"
            INSERT INTO coaching.requests(
                request_uid, website_request_id, discord_user_id, discord_username,
                rank, subrank, current_problems, status, message_id, channel_id,
                created_at, updated_at
            )
            VALUES
              ('privacy-delete-42', 'privacy-delete-web-42', 42, 'DeleteMe',
               'Phantom', 'III', 'private problem', 'pending', 4242, 4343, now(), now()),
              ('privacy-delete-99', 'privacy-delete-web-99', 99, 'KeepMe',
               'Phantom', 'III', 'other problem', 'pending', 9999, 9998, now(), now())
            "#
        )
        .execute(db.pool())
        .await
        .expect("coaching requests");
        sqlx::query!(
            "INSERT INTO core.steam_links(discord_id,steam_id,steam_id64) VALUES (42,'STEAM_42',42)"
        )
        .execute(db.pool())
        .await
        .expect("steam link");
        sqlx::query!(
            "INSERT INTO activity.live_player_state(steam_id,last_gameid) VALUES ('STEAM_42','a')"
        )
        .execute(db.pool())
        .await
        .expect("live state");
        sqlx::query!(
            "INSERT INTO activity.user_co_players(user_id,co_player_id) VALUES (42,99),(99,42),(1,2)"
        )
        .execute(db.pool())
        .await
        .expect("co players");
        sqlx::query!("INSERT INTO voice.tempvoice_bans(owner_id,banned_id) VALUES (42,7),(7,42)")
            .execute(db.pool())
            .await
            .expect("bans");
        sqlx::query!(
            r#"
            INSERT INTO bot.kv_store(ns,k,v) VALUES
              ('ai_onboarding:sessions','42','{}'),
              ('ai_onboarding:persistent_views','viewA','{"user_id":42}'),
              ('ai_onboarding:persistent_views','viewB','{"user_id":99}'),
              ('voice_nudge_done','42','1'),
              ('native_onboarding:completed','1:42','{"guild_id":1,"user_id":42}')
            "#
        )
        .execute(db.pool())
        .await
        .expect("kv");

        let s = delete_user_data(db.pool(), 42, "slash_datenschutz".into(), 1234)
            .await
            .expect("delete");

        assert_eq!(s.steam_ids, vec!["STEAM_42".to_string()]);
        assert_eq!(
            s.counts.get("coaching_requests.discord_user_id").copied(),
            Some(1)
        );
        assert_eq!(s.counts.get("live_player_state:STEAM_42").copied(), Some(1));
        assert_eq!(s.counts.get("journey_events.user_id").copied(), Some(1));
        assert_eq!(s.counts.get("journey_user_state.user_id").copied(), Some(1));
        assert_eq!(
            s.counts.get("message_metadata_events.user_id").copied(),
            Some(1)
        );
        assert_eq!(
            s.counts.get("voice_metadata_events.user_id").copied(),
            Some(1)
        );
        assert_eq!(
            s.counts.get("voice_open_sessions.user_id").copied(),
            Some(1)
        );
        assert_eq!(s.counts.get("interaction_events.user_id").copied(), Some(1));
        assert_eq!(
            s.counts.get("presence_daily_seen.user_id").copied(),
            Some(1)
        );
        assert_eq!(
            s.counts.get("guild_member_directory.user_id").copied(),
            Some(1)
        );
        assert_eq!(s.counts.get("user_co_players").copied(), Some(2));
        assert_eq!(s.counts.get("tempvoice_bans.owner_id").copied(), Some(1));
        assert_eq!(s.counts.get("tempvoice_bans.banned_id").copied(), Some(1));
        assert_eq!(s.counts.get("kv_ai_onboarding_sessions").copied(), Some(1));
        assert_eq!(s.counts.get("kv_ai_onboarding_views").copied(), Some(1));
        assert_eq!(s.counts.get("kv_voice_nudge").copied(), Some(1));
        assert_eq!(
            s.counts.get("kv_native_onboarding_completed").copied(),
            Some(1)
        );
        assert_eq!(s.counts.get("user_privacy_updated").copied(), Some(1));

        let vs_self = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM voice.voice_stats WHERE user_id = 42"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        let vs_other = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM voice.voice_stats WHERE user_id = 99"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        let cop_foreign = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM activity.user_co_players WHERE user_id = 1"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        let view_other = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM bot.kv_store WHERE k = 'viewB'"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        let coaching_self = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM coaching.requests WHERE discord_user_id = 42"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        let coaching_other = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM coaching.requests WHERE discord_user_id = 99"
        )
        .fetch_one(db.pool())
        .await
        .expect("count");
        let journey_self: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8 FROM activity.journey_events WHERE user_id = 42",
        )
        .fetch_one(db.pool())
        .await
        .expect("journey count");
        let journey_other: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8 FROM activity.journey_events WHERE user_id = 99",
        )
        .fetch_one(db.pool())
        .await
        .expect("journey other count");
        let message_meta_self: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8 FROM activity.message_metadata_events WHERE user_id = 42",
        )
        .fetch_one(db.pool())
        .await
        .expect("message metadata count");
        let interaction_self: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8 FROM activity.interaction_events WHERE user_id = 42",
        )
        .fetch_one(db.pool())
        .await
        .expect("interaction count");
        assert_eq!(vs_self, 0);
        assert_eq!(vs_other, 1);
        assert_eq!(cop_foreign, 1);
        assert_eq!(view_other, 1);
        assert_eq!(coaching_self, 0);
        assert_eq!(coaching_other, 1);
        assert_eq!(journey_self, 0);
        assert_eq!(journey_other, 1);
        assert_eq!(message_meta_self, 0);
        assert_eq!(interaction_self, 0);
        assert!(is_opted_out(db.pool(), 42).await);
    }

    #[tokio::test]
    async fn delete_ist_idempotent() {
        let db = mk_db().await;
        sqlx::query!(
            "INSERT INTO voice.voice_stats(user_id,total_seconds,total_points) VALUES (8,1,1)"
        )
        .execute(db.pool())
        .await
        .expect("insert");
        let first = delete_user_data(db.pool(), 8, "r".into(), 1)
            .await
            .expect("delete");
        assert_eq!(first.counts.get("voice_stats.user_id").copied(), Some(1));
        let second = delete_user_data(db.pool(), 8, "r".into(), 2)
            .await
            .expect("delete");
        assert_eq!(second.counts.get("voice_stats.user_id").copied(), Some(0));
    }

    #[tokio::test]
    async fn delete_entfernt_umfragen_puls_antworten() {
        let db = mk_db().await;
        let wave_id: i64 = sqlx::query_scalar(
            "INSERT INTO bot.survey_waves(config_snapshot) VALUES ('{}') RETURNING id",
        )
        .fetch_one(db.pool())
        .await
        .expect("survey wave");
        sqlx::query(
            "INSERT INTO bot.survey_responses(
                wave_id, user_id, satisfaction, events, freitext
             ) VALUES ($1, 42, 5, ARRAY['turnier'], 'privat')",
        )
        .bind(wave_id)
        .execute(db.pool())
        .await
        .expect("survey response");

        let summary = delete_user_data(db.pool(), 42, "test".into(), 1_000)
            .await
            .expect("delete");

        assert_eq!(
            summary.counts.get("survey_responses.user_id").copied(),
            Some(1)
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM bot.survey_responses WHERE user_id = 42",
            )
            .fetch_one(db.pool())
            .await
            .expect("survey response count"),
            0
        );
    }

    #[tokio::test]
    async fn delete_entfernt_requester_spuren_aus_fremden_outbox_zeilen() {
        let db = mk_db().await;
        sqlx::query(
            r#"
            INSERT INTO bot.action_outbox(
                action_type, user_id, guild_id, payload, anchor, idempotency_key, status
            ) VALUES
                ('future_action', 99, 1,
                 '{"requester_id":42,"source_message_id":100,"details":{"note":"privat"}}',
                 'pending', 'privacy-requester-pending', 'pending'),
                ('future_action', 99, 1,
                 '{"requester_id":"42","source_message_id":101,"details":{"note":"privat"}}',
                 'suppressed', 'privacy-requester-suppressed', 'suppressed'),
                ('future_action', 99, 1,
                 '{"requester_id":42,"source_message_id":102,"details":{"note":"privat"}}',
                 'sent', 'privacy-requester-sent', 'sent'),
                ('future_action', 99, 1,
                 '{"requester_id":7,"details":{"keep":"yes"}}',
                 'other', 'privacy-requester-other', 'sent')
            "#,
        )
        .execute(db.pool())
        .await
        .expect("outbox rows");

        delete_user_data(db.pool(), 42, "test".to_string(), 1_000)
            .await
            .expect("privacy delete");

        let pending_or_suppressed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM bot.action_outbox
              WHERE idempotency_key IN ('privacy-requester-pending', 'privacy-requester-suppressed')",
        )
        .fetch_one(db.pool())
        .await
        .expect("deleted requester rows");
        assert_eq!(pending_or_suppressed, 0);

        let sent_payload: Value = sqlx::query_scalar(
            "SELECT payload FROM bot.action_outbox
              WHERE idempotency_key = 'privacy-requester-sent'",
        )
        .fetch_one(db.pool())
        .await
        .expect("sent requester row");
        assert_eq!(sent_payload, serde_json::json!({}));

        let unrelated_payload: Value = sqlx::query_scalar(
            "SELECT payload FROM bot.action_outbox
              WHERE idempotency_key = 'privacy-requester-other'",
        )
        .fetch_one(db.pool())
        .await
        .expect("unrelated outbox row");
        assert_eq!(
            unrelated_payload,
            serde_json::json!({"requester_id": 7, "details": {"keep": "yes"}})
        );
    }

    #[tokio::test]
    async fn export_enthaelt_fremde_outbox_zeilen_des_requesters() {
        let db = mk_db().await;
        sqlx::query(
            r#"
            INSERT INTO bot.action_outbox(
                action_type, user_id, guild_id, payload, anchor, idempotency_key, status
            ) VALUES
                ('future_action', 99, 1, '{"requester_id":42,"details":{"note":"privat"}}',
                 'requester', 'privacy-export-requester', 'pending'),
                ('future_action', 99, 1, '{"requester_id":7,"details":{"keep":"yes"}}',
                 'other', 'privacy-export-other', 'pending')
            "#,
        )
        .execute(db.pool())
        .await
        .expect("outbox rows");

        let export = export_user_data(db.pool(), 42, 1_000)
            .await
            .expect("privacy export");
        let rows = export["tables"]["action_outbox.payload.requester_id"]
            .as_array()
            .expect("requester outbox rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["idempotency_key"], "privacy-export-requester");
        assert!(rows[0].get("user_id").is_none());
    }

    #[tokio::test]
    async fn delete_entfernt_paten_id_aus_fremden_journey_metadaten() {
        let db = mk_db().await;
        let metadata = serde_json::json!({
            "pate_id": "42",
            "channel_id": "900",
            "keep": "yes",
        });
        sqlx::query(
            "INSERT INTO activity.journey_events(
                user_id, guild_id, event_type, event_source, metadata
             ) VALUES (99, 1, 'join', 'privacy_test', $1)",
        )
        .bind(&metadata)
        .execute(db.pool())
        .await
        .expect("journey event");
        sqlx::query(
            "INSERT INTO activity.journey_user_state(
                 user_id, guild_id, joined_at, last_event_at, last_event_type, metadata
             ) VALUES
                 (99, 1, now(), now(), 'first_message', $1),
                 (100, 1, now(), now(), 'concierge_reply',
                  '{\"pate_id\":\"42\",\"channel_id\":\"901\",\"keep\":\"yes\"}'::jsonb)",
        )
        .bind(&metadata)
        .execute(db.pool())
        .await
        .expect("journey state");
        sqlx::query(
            "INSERT INTO activity.journey_events(
                 user_id, guild_id, event_type, event_source, metadata
             ) VALUES (
                 100, 1, 'steckbrief_posted', 'privacy_test',
                 '{\"channel_id\":\"901\"}'::jsonb
             )",
        )
        .execute(db.pool())
        .await
        .expect("proven later channel event");

        let summary = delete_user_data(db.pool(), 42, "test".to_string(), 1_000)
            .await
            .expect("privacy delete");

        assert_eq!(
            summary.counts.get("journey_events.metadata.pate_id"),
            Some(&1)
        );
        assert_eq!(
            summary.counts.get("journey_user_state.metadata.pate_id"),
            Some(&2)
        );
        let remaining_event: Value = sqlx::query_scalar(
            "SELECT metadata FROM activity.journey_events WHERE user_id = 99 AND guild_id = 1",
        )
        .fetch_one(db.pool())
        .await
        .expect("foreign journey event remains");
        assert_eq!(remaining_event, serde_json::json!({ "keep": "yes" }));

        let later_state: Value = sqlx::query_scalar(
            "SELECT metadata FROM activity.journey_user_state WHERE user_id = 99 AND guild_id = 1",
        )
        .fetch_one(db.pool())
        .await
        .expect("later foreign journey state remains");
        assert_eq!(later_state, serde_json::json!({ "keep": "yes" }));

        let pate_state: Value = sqlx::query_scalar(
            "SELECT metadata FROM activity.journey_user_state WHERE user_id = 100 AND guild_id = 1",
        )
        .fetch_one(db.pool())
        .await
        .expect("pate journey state remains");
        assert_eq!(
            pate_state,
            serde_json::json!({ "channel_id": "901", "keep": "yes" })
        );
    }

    #[tokio::test]
    async fn pate_scrub_und_raw_retention_deadlocken_nicht_cross_user(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = mk_db().await;
        let pool = db.pool().clone();
        let old = Utc::now() - Duration::days(181);
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO activity.journey_events(
                 user_id, guild_id, event_type, event_source, occurred_at, metadata
             ) VALUES(99, 1, 'pate_matched', 'privacy_lock_test', $1,
                      '{\"pate_id\":\"42\",\"channel_id\":\"900\"}'::jsonb)",
        )
        .bind(old)
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO activity.message_metadata_events(
                 user_id, guild_id, channel_id, message_id, occurred_at,
                 message_length, has_attachment, attachment_count, is_reply
             ) VALUES(42, 1, 20, 100, $1, 5, FALSE, 0, FALSE)",
        )
        .bind(old)
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO activity.journey_daily_aggregates(
                 day, guild_id, event_type, actor_kind, event_count, distinct_user_count
             ) VALUES($1, 1, 'pate_matched', '', 5, 0)",
        )
        .bind(old.date_naive())
        .execute(&pool)
        .await?;

        let mut aggregate_blocker = pool.begin().await?;
        sqlx::query(
            "SELECT 1
               FROM activity.journey_daily_aggregates
              WHERE day = $1 AND guild_id = 1
                AND event_type = 'pate_matched' AND actor_kind = ''
              FOR UPDATE",
        )
        .bind(old.date_naive())
        .fetch_one(&mut *aggregate_blocker)
        .await?;

        let retention_pool = pool.clone();
        let retention = tokio::spawn(async move { compact_raw_events(&retention_pool, now).await });
        wait_for_db_lock(&pool, "INSERT INTO activity.journey_daily_aggregates", None).await;

        let erase_pool = pool.clone();
        let erase = tokio::spawn(async move {
            delete_user_data(&erase_pool, 42, "test".to_string(), now.timestamp()).await
        });
        wait_for_db_lock_count(&pool, 2).await;
        aggregate_blocker.commit().await?;

        let (retention, erase) = tokio::time::timeout(StdDuration::from_secs(10), async {
            tokio::join!(retention, erase)
        })
        .await?;
        retention??;
        erase??;
        Ok(())
    }

    #[tokio::test]
    async fn raw_retention_serialisiert_am_gemeinsamen_erasure_lock(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = mk_db().await;
        let pool = db.pool().clone();
        sqlx::query("INSERT INTO core.users(discord_id) VALUES(42)")
            .execute(&pool)
            .await?;
        let mut row_blocker = pool.begin().await?;
        sqlx::query("SELECT 1 FROM core.users WHERE discord_id = 42 FOR UPDATE")
            .fetch_one(&mut *row_blocker)
            .await?;

        let erase_pool = pool.clone();
        let erase = tokio::spawn(async move {
            delete_user_data(&erase_pool, 42, "test".to_string(), Utc::now().timestamp()).await
        });
        wait_for_db_lock(&pool, "DELETE FROM core.users", Some("transactionid")).await;

        let retention_pool = pool.clone();
        let retention =
            tokio::spawn(async move { compact_raw_events(&retention_pool, Utc::now()).await });
        wait_for_db_lock(&pool, "pg_advisory_xact_lock", Some("advisory")).await;
        row_blocker.commit().await?;

        erase.await??;
        retention.await??;
        Ok(())
    }

    #[tokio::test]
    async fn concierge_claims_werden_exportiert_und_fuer_beide_patenrollen_geloescht() {
        let db = mk_db().await;
        sqlx::query(
            "INSERT INTO bot.kv_store(ns, k, v) VALUES
             ('concierge:t0', '1:42', 'claimed'),
             ('concierge:fallback_channel', '1:42', 'claimed'),
             ('concierge:steckbrief_revoked', '42', 'revoked'),
             ('concierge:pate_claim', '42', '77'),
             ('concierge:pate_claim', '99', '42'),
             ('concierge:pate_claim', '100', '77')",
        )
        .execute(db.pool())
        .await
        .expect("concierge claims");

        let export = export_user_data(db.pool(), 42, 1_000)
            .await
            .expect("privacy export");
        let claims = export["kv"]["concierge_claims"]
            .as_array()
            .expect("concierge claims array");
        assert_eq!(claims.len(), 5);
        let claims_json = serde_json::to_string(claims).expect("claims json");
        assert!(!claims_json.contains("77"));
        assert!(!claims_json.contains("99"));

        let summary = delete_user_data(db.pool(), 42, "test".to_string(), 1_000)
            .await
            .expect("privacy delete");
        assert_eq!(summary.counts.get("kv_concierge_claims"), Some(&5));
        let remaining = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM bot.kv_store WHERE ns LIKE 'concierge:%'",
        )
        .fetch_one(db.pool())
        .await
        .expect("remaining claims");
        assert_eq!(remaining, 1);
    }

    #[tokio::test]
    async fn export_enthaelt_nur_faq_nachrichten_eigener_sessions() {
        let db = mk_db().await;
        sqlx::query(
            r#"
            INSERT INTO bot.faq_chat_sessions(
                session_id, user_id, user_name, channel_id, guild_id, expires_at
            )
            VALUES
              ('privacy-faq-own-a', 42, 'me', 1, 1, now() + interval '1 hour'),
              ('privacy-faq-own-b', 42, 'me', 2, 1, now() + interval '1 hour'),
              ('privacy-faq-foreign', 99, 'DO_NOT_EXPORT', 3, 1, now() + interval '1 hour')
            "#,
        )
        .execute(db.pool())
        .await
        .expect("faq sessions");
        sqlx::query(
            r#"
            INSERT INTO bot.faq_chat_messages(session_id, role, content)
            VALUES
              ('privacy-faq-own-a', 'user', 'own question'),
              ('privacy-faq-own-b', 'assistant', 'own answer'),
              ('privacy-faq-foreign', 'user', 'DO_NOT_EXPORT')
            "#,
        )
        .execute(db.pool())
        .await
        .expect("faq messages");

        let snap = export_user_data(db.pool(), 42, 5000).await.expect("export");
        let messages = snap["tables"]["faq_chat_messages.session_id"]
            .as_array()
            .expect("faq messages in export");

        assert_eq!(
            messages
                .iter()
                .map(|row| row["session_id"].as_str().expect("session id"))
                .collect::<Vec<_>>(),
            ["privacy-faq-own-a", "privacy-faq-own-b"]
        );
        assert!(!snap.to_string().contains("DO_NOT_EXPORT"));
    }

    #[tokio::test]
    async fn export_redigiert_gegenparteien_in_patenschaften() {
        let db = mk_db().await;
        sqlx::query(
            r#"
            INSERT INTO bot.concierge_patenschaften(
                user_id, pate_id, guild_id, channel_id, created_at
            )
            VALUES (42, 99, 1, 420, now()), (77, 42, 1, 421, now())
            "#,
        )
        .execute(db.pool())
        .await
        .expect("patenschaften");

        let snap = export_user_data(db.pool(), 42, 5000).await.expect("export");
        let als_user = &snap["tables"]["concierge_patenschaften_user.user_id"][0];
        let als_pate = &snap["tables"]["concierge_patenschaften_pate.pate_id"][0];

        assert_eq!(als_user["user_id"], serde_json::json!(42));
        assert_eq!(als_user["pate_id"], serde_json::json!("redacted"));
        assert_eq!(als_user["channel_id"], serde_json::json!(420));
        assert_eq!(als_pate["pate_id"], serde_json::json!(42));
        assert_eq!(als_pate["user_id"], serde_json::json!("redacted"));
        assert_eq!(als_pate["channel_id"], serde_json::json!(421));
    }

    #[tokio::test]
    async fn export_redigiert_fremde_ids() {
        let db = mk_db().await;
        let now = Utc::now();
        sqlx::query!("INSERT INTO core.users(discord_id) VALUES (42)")
            .execute(db.pool())
            .await
            .expect("user");
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_session_log(
                id,user_id,started_at,ended_at,duration_seconds,points,co_player_ids
            )
            VALUES (1,42,$1,$1,60,1,'[99,100]'::jsonb)
            "#,
            now,
        )
        .execute(db.pool())
        .await
        .expect("voice log");
        sqlx::query!(
            r#"
            INSERT INTO activity.user_co_players(user_id,co_player_id,user_display_name,co_player_display_name)
            VALUES (42,99,'me','them'),(77,42,'them','me')
            "#
        )
        .execute(db.pool())
        .await
        .expect("co players");
        sqlx::query!("INSERT INTO voice.tempvoice_bans(owner_id,banned_id) VALUES (42,99),(88,42)")
            .execute(db.pool())
            .await
            .expect("bans");
        sqlx::query!(
            r#"
            INSERT INTO coaching.requests(
                request_uid, website_request_id, discord_user_id, discord_username,
                rank, subrank, current_problems, status, message_id, channel_id,
                created_at, updated_at
            )
            VALUES ('privacy-export-42', 'privacy-export-web-42', 42, 'ExportMe',
                    'Phantom', 'III', 'exported problem', 'pending', 4242, 4343,
                    now(), now())
            "#
        )
        .execute(db.pool())
        .await
        .expect("coaching request");

        let snap = export_user_data(db.pool(), 42, 5000).await.expect("export");

        assert!(snap["tables"]["voice_session_log.user_id"][0]["co_player_ids"].is_null());
        let cps = snap["tables"]["user_co_players"]
            .as_array()
            .expect("co players");
        assert_eq!(cps.len(), 2);
        for row in cps {
            assert_eq!(row["co_player_id"], serde_json::json!("redacted"));
            assert_eq!(row["user_id"], serde_json::json!(42));
        }
        assert_eq!(
            snap["tables"]["tempvoice_bans.owner_id"][0]["banned_id"],
            serde_json::json!("redacted")
        );
        assert_eq!(
            snap["tables"]["tempvoice_bans.banned_id"][0]["owner_id"],
            serde_json::json!("redacted")
        );
        assert_eq!(
            snap["tables"]["coaching_requests.discord_user_id"][0]["discord_username"],
            serde_json::json!("ExportMe")
        );
        assert_eq!(
            snap["tables"]["coaching_requests.discord_user_id"][0]["current_problems"],
            serde_json::json!("exported problem")
        );
        assert_eq!(snap["user_id"], serde_json::json!(42));
    }
}
