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
];

const USER_CO_PLAYERS_REL: &str = "activity.user_co_players";
const KV_REL: &str = "bot.kv_store";
const KV_NATIVE_ONBOARDING_COMPLETED_NS: &str = "native_onboarding:completed";
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

pub async fn is_opted_out(pool: &PgPool, user_id: i64) -> bool {
    sqlx::query!(
        r#"
        SELECT opted_out
          FROM core.user_privacy
         WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|row| row.opted_out)
    .unwrap_or(false)
}

pub async fn set_opt_in(pool: &PgPool, user_id: i64, now: i64) -> CommunityDbResult<()> {
    let now = utc_from_unix(now)?;
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
    .execute(pool)
    .await?;
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
    let steam_ids = steam_ids_for_user(pool, user_id).await?;
    let user_key = user_id.to_string();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    counts.insert(
        "server_config.rollback_exports.expired".to_string(),
        expired_rollback_exports,
    );

    let mut tx = pool.begin().await?;

    for &spec in USER_TABLES {
        if !relations.contains(spec.relation) {
            continue;
        }
        let n = delete_rows_user(&mut tx, spec, user_id, &user_key).await?;
        counts.insert(spec.count_key(), n);
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
        // `server_config.*`: Audit-Referenzen auf Mod-/Admin-AKTIONEN am
        // Server-Soll-Modell (wer hat Diff erstellt / Apply angefordert /
        // Drift adoptiert). Kein Community-Verhaltensdatum; Aufbewahrung zur
        // Nachvollziehbarkeit administrativer Server-Aenderungen
        // (berechtigtes Interesse). Ein Opt-out darf die Aenderungs-
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
        // Rollback-Artefakte koennen verschachtelte Member-IDs im JSON
        // enthalten; sie sind deshalb ueber `expires_at` auf 180 Tage
        // begrenzt und werden durch den Privacy-Retention-Purge geloescht.
        // Die Admin-ID bleibt nur als Ersteller-Auditreferenz allowlisted.
        out.insert((
            "server_config.rollback_exports".to_string(),
            "created_by_user_id".to_string(),
        ));
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
            for chunk in raw.split("CREATE TABLE IF NOT EXISTS ").skip(1) {
                let Some((relation, rest)) = chunk.split_once(" (") else {
                    continue;
                };
                let relation = normalize_sql_ident(relation);
                let Some((body, _)) = rest.split_once("\n);") else {
                    continue;
                };
                for line in body.lines() {
                    let trimmed = line.trim().trim_end_matches(',');
                    let Some(column) = trimmed.split_whitespace().next() else {
                        continue;
                    };
                    let column = normalize_sql_ident(column);
                    if is_user_id_like_column(&column) {
                        out.insert((relation.clone(), column));
                    }
                }
            }
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
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use chrono::Utc;
    use dl_central_db::testing::{test_pool, TestDb};

    async fn mk_db() -> TestDb {
        test_pool().await.expect("test_pool")
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
    async fn is_opted_out_fail_open_und_wertbasiert() {
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
