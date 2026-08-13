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

#[derive(Debug, Clone, Copy)]
enum TargetRefSet {
    Discord,
    Steam,
}

#[derive(Debug, Clone, Copy)]
struct TextUserRefColumnSpec {
    key_table: &'static str,
    key_col: &'static str,
    relation: &'static str,
    col: &'static str,
    kind_col: &'static str,
    kind_value: &'static str,
    target_ref_set: TargetRefSet,
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

impl TextUserRefColumnSpec {
    const fn new(
        key_table: &'static str,
        key_col: &'static str,
        relation: &'static str,
        col: &'static str,
        kind_col: &'static str,
        kind_value: &'static str,
        target_ref_set: TargetRefSet,
    ) -> Self {
        Self {
            key_table,
            key_col,
            relation,
            col,
            kind_col,
            kind_value,
            target_ref_set,
        }
    }

    fn count_key(self) -> String {
        format!("{}.{}", self.key_table, self.key_col)
    }
}

#[derive(Debug, Clone, Copy)]
struct RedactionSpec {
    key_table: &'static str,
    key_col: &'static str,
    relation: &'static str,
    col: &'static str,
    display_col: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
struct JsonUserColumnSpec {
    key_table: &'static str,
    key_col: &'static str,
    relation: &'static str,
    col: &'static str,
    export_mode: JsonUserColumnExport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonUserColumnExport {
    GenericProjected,
    HashOnly,
}

impl JsonUserColumnSpec {
    const fn new(
        key_table: &'static str,
        key_col: &'static str,
        relation: &'static str,
        col: &'static str,
    ) -> Self {
        Self {
            key_table,
            key_col,
            relation,
            col,
            export_mode: JsonUserColumnExport::GenericProjected,
        }
    }

    const fn hash_only(
        key_table: &'static str,
        key_col: &'static str,
        relation: &'static str,
        col: &'static str,
    ) -> Self {
        Self {
            key_table,
            key_col,
            relation,
            col,
            export_mode: JsonUserColumnExport::HashOnly,
        }
    }

    fn count_key(self) -> String {
        format!("{}.{}", self.key_table, self.key_col)
    }
}

impl RedactionSpec {
    const fn new(
        key_table: &'static str,
        key_col: &'static str,
        relation: &'static str,
        col: &'static str,
        display_col: Option<&'static str>,
    ) -> Self {
        Self {
            key_table,
            key_col,
            relation,
            col,
            display_col,
        }
    }

    fn count_key(self) -> String {
        format!("{}.{}.redacted", self.key_table, self.key_col)
    }

    fn table_spec(self) -> TableSpec {
        TableSpec::new(
            self.key_table,
            self.key_col,
            self.relation,
            self.col,
            ColumnType::Text,
        )
    }
}

fn uses_subject_projected_export(spec: RedactionSpec) -> bool {
    matches!(
        spec.relation,
        "scrim.announcement_drafts"
            | "scrim.announcement_approvals"
            | "scrim.status_publication_approvals"
            | "scrim.matches"
            | "scrim.match_requests"
            | "scrim.replacement_needs"
            | "scrim.replacement_requests"
            | "scrim.match_lineup_snapshots"
            | "scrim.match_result_refs"
            | "scrim.match_result_clarifications"
    )
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
    // Existieren nur, solange ein Rollback von Migration 2026081301 nicht
    // aufgeraeumt ist (rollbacks/2026081301_..._rollback.sql legt sie an). Sie
    // tragen verschluesselte OAuth-Tokens, also muss ein Loeschantrag sie
    // treffen; `relation_exists` ueberspringt sie, wenn es sie nicht gibt.
    TableSpec::new(
        "discord_role_connection_tokens_rollback_backup",
        "user_id",
        "core.discord_role_connection_tokens_rollback_backup",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "discord_role_connection_sync_state_rollback_backup",
        "user_id",
        "core.discord_role_connection_sync_state_rollback_backup",
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
    // Plus-Abo: Abozustand und Prio-Slots haengen beide am Discord-Konto und
    // muessen deshalb in Auskunft und Loeschung auftauchen. Die Stripe-Seite
    // (Kunde, Zahlungen) liegt bei Stripe und wird von dort geloescht, nicht
    // von hier.
    TableSpec::new(
        "plus_subscriptions",
        "discord_id",
        "steam.plus_subscriptions",
        "discord_id",
        ColumnType::I64,
    ),
    TableSpec::new(
        "plus_priority_slots",
        "discord_id",
        "steam.plus_priority_slots",
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
        "voice_pair_guard_locks",
        "blocked_user_id",
        "voice.voice_pair_guard_locks",
        "blocked_user_id",
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
        "scrim_match_request_responses",
        "discord_user_id",
        "scrim.match_request_responses",
        "discord_user_id",
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

const REDACTED_TEXT_USER_COLUMNS: &[RedactionSpec] = &[
    RedactionSpec::new(
        "scrim_slot_presets",
        "created_by_user_id",
        "scrim.slot_presets",
        "created_by_user_id",
        None,
    ),
    RedactionSpec::new(
        "scrim_match_request_batches",
        "created_by_user_id",
        "scrim.match_request_batches",
        "created_by_user_id",
        Some("created_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_matches",
        "lobby_code_source_user_id",
        "scrim.matches",
        "lobby_code_source_user_id",
        Some("lobby_code_source_display_name"),
    ),
    RedactionSpec::new(
        "scrim_match_requests",
        "released_by_user_id",
        "scrim.match_requests",
        "released_by_user_id",
        Some("released_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_match_request_reminders",
        "approved_by_user_id",
        "scrim.match_request_reminders",
        "approved_by_user_id",
        Some("approved_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_lagebild_corrections",
        "author_user_id",
        "scrim.lagebild_corrections",
        "author_user_id",
        Some("author_display_name"),
    ),
    RedactionSpec::new(
        "scrim_announcement_drafts",
        "created_by_user_id",
        "scrim.announcement_drafts",
        "created_by_user_id",
        Some("created_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_announcement_drafts",
        "approved_by_user_id",
        "scrim.announcement_drafts",
        "approved_by_user_id",
        Some("approved_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_announcement_approvals",
        "decided_by_user_id",
        "scrim.announcement_approvals",
        "decided_by_user_id",
        Some("decided_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_status_publication_approvals",
        "decided_by_user_id",
        "scrim.status_publication_approvals",
        "decided_by_user_id",
        Some("decided_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_replacement_needs",
        "created_by_user_id",
        "scrim.replacement_needs",
        "created_by_user_id",
        Some("created_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_replacement_requests",
        "requested_by_user_id",
        "scrim.replacement_requests",
        "requested_by_user_id",
        Some("requested_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_match_lineup_snapshots",
        "created_by_user_id",
        "scrim.match_lineup_snapshots",
        "created_by_user_id",
        Some("created_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_match_result_refs",
        "source_user_id",
        "scrim.match_result_refs",
        "source_user_id",
        Some("source_display_name"),
    ),
    RedactionSpec::new(
        "scrim_match_result_refs",
        "selected_by_user_id",
        "scrim.match_result_refs",
        "selected_by_user_id",
        None,
    ),
    RedactionSpec::new(
        "scrim_match_result_selections",
        "selected_by_user_id",
        "scrim.match_result_selections",
        "selected_by_user_id",
        Some("selected_by_display_name"),
    ),
    RedactionSpec::new(
        "scrim_match_result_clarifications",
        "requested_by_user_id",
        "scrim.match_result_clarifications",
        "requested_by_user_id",
        Some("requested_by_display_name"),
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
const SCRIM_AUDIT_ACTOR_PSEUDONYMS_REL: &str = "scrim.audit_actor_pseudonyms";

const JSON_USER_COLUMNS: &[JsonUserColumnSpec] = &[
    JsonUserColumnSpec::new(
        "scrim_command_receipts",
        "payload",
        "scrim.command_receipts",
        "payload",
    ),
    JsonUserColumnSpec::new(
        "scrim_command_receipts",
        "result_payload",
        "scrim.command_receipts",
        "result_payload",
    ),
    JsonUserColumnSpec::new(
        "scrim_inbox_events",
        "payload",
        "scrim.inbox_events",
        "payload",
    ),
    JsonUserColumnSpec::new(
        "scrim_outbox_effects",
        "payload",
        "scrim.outbox_effects",
        "payload",
    ),
    JsonUserColumnSpec::new(
        "scrim_effect_receipts",
        "receipt_payload",
        "scrim.effect_receipts",
        "receipt_payload",
    ),
    JsonUserColumnSpec::hash_only(
        "scrim_replacement_candidates",
        "candidate_data",
        "scrim.replacement_candidates",
        "candidate_data",
    ),
    JsonUserColumnSpec::hash_only(
        "scrim_replacement_candidates",
        "score_data",
        "scrim.replacement_candidates",
        "score_data",
    ),
    JsonUserColumnSpec::hash_only(
        "scrim_replacement_requests",
        "request_payload",
        "scrim.replacement_requests",
        "request_payload",
    ),
    JsonUserColumnSpec::new(
        "scrim_matches",
        "result_json",
        "scrim.matches",
        "result_json",
    ),
    JsonUserColumnSpec::hash_only(
        "scrim_matches",
        "lobby_code_corrections",
        "scrim.matches",
        "lobby_code_corrections",
    ),
    JsonUserColumnSpec::new(
        "scrim_match_lineup_snapshots",
        "lineup_payload",
        "scrim.match_lineup_snapshots",
        "lineup_payload",
    ),
    JsonUserColumnSpec::hash_only(
        "scrim_match_result_refs",
        "clarification_payload",
        "scrim.match_result_refs",
        "clarification_payload",
    ),
    JsonUserColumnSpec::new(
        "scrim_match_result_refs",
        "raw_result_json",
        "scrim.match_result_refs",
        "raw_result_json",
    ),
    JsonUserColumnSpec::new(
        "scrim_match_result_refs",
        "normalized_result_json",
        "scrim.match_result_refs",
        "normalized_result_json",
    ),
    JsonUserColumnSpec::hash_only(
        "scrim_match_result_clarifications",
        "request_payload",
        "scrim.match_result_clarifications",
        "request_payload",
    ),
    JsonUserColumnSpec::hash_only(
        "scrim_match_result_clarifications",
        "response_payload",
        "scrim.match_result_clarifications",
        "response_payload",
    ),
    JsonUserColumnSpec::new(
        "steam_v1_workflows",
        "payload",
        "steam.v1_workflows",
        "payload",
    ),
    JsonUserColumnSpec::new(
        "steam_v1_operations",
        "payload",
        "steam.v1_operations",
        "payload",
    ),
    JsonUserColumnSpec::new(
        "steam_v1_operations",
        "result_payload",
        "steam.v1_operations",
        "result_payload",
    ),
    JsonUserColumnSpec::new(
        "steam_v1_operation_results",
        "result_payload",
        "steam.v1_operation_results",
        "result_payload",
    ),
    JsonUserColumnSpec::new(
        "steam_v1_operation_events",
        "payload",
        "steam.v1_operation_events",
        "payload",
    ),
    JsonUserColumnSpec::new(
        "steam_v1_deliveries",
        "payload",
        "steam.v1_deliveries",
        "payload",
    ),
];

const TEXT_USER_REF_COLUMNS: &[TextUserRefColumnSpec] = &[
    TextUserRefColumnSpec::new(
        "scrim_ai_runs",
        "subject_id",
        "scrim.ai_runs",
        "subject_id",
        "subject_kind",
        "discord_user",
        TargetRefSet::Discord,
    ),
    TextUserRefColumnSpec::new(
        "scrim_ai_runs",
        "subject_id",
        "scrim.ai_runs",
        "subject_id",
        "subject_kind",
        "steam_user",
        TargetRefSet::Steam,
    ),
    TextUserRefColumnSpec::new(
        "steam_v1_workflows",
        "aggregate_id",
        "steam.v1_workflows",
        "aggregate_id",
        "aggregate_kind",
        "player",
        TargetRefSet::Steam,
    ),
    TextUserRefColumnSpec::new(
        "steam_v1_workflows",
        "subject_id",
        "steam.v1_workflows",
        "subject_id",
        "subject_kind",
        "discord_user",
        TargetRefSet::Discord,
    ),
    TextUserRefColumnSpec::new(
        "steam_v1_workflows",
        "subject_id",
        "steam.v1_workflows",
        "subject_id",
        "subject_kind",
        "steam_user",
        TargetRefSet::Steam,
    ),
    TextUserRefColumnSpec::new(
        "steam_v1_operations",
        "aggregate_id",
        "steam.v1_operations",
        "aggregate_id",
        "aggregate_kind",
        "player",
        TargetRefSet::Steam,
    ),
    TextUserRefColumnSpec::new(
        "steam_v1_operations",
        "subject_id",
        "steam.v1_operations",
        "subject_id",
        "subject_kind",
        "discord_user",
        TargetRefSet::Discord,
    ),
    TextUserRefColumnSpec::new(
        "steam_v1_operations",
        "subject_id",
        "steam.v1_operations",
        "subject_id",
        "subject_kind",
        "steam_user",
        TargetRefSet::Steam,
    ),
];
const SERVER_SYNC_ROLLBACK_EXPORTS_REL: &str = "server_config.rollback_exports";

/// Kopien, die der Rueckweg von Migration 2026081301 anlegt
/// (dl-central-db/rollbacks/). Sie tragen verschluesselte OAuth-Tokens und
/// existieren nur nach einem Rollback; `rollback_expires_at` steht dort auf 180
/// Tagen wie `expires_at` bei den Server-Sync-Exporten. Der eigene Name ist
/// Absicht: die Tokens-Kopie hat schon ein `expires_at`, und das ist der
/// OAuth-Ablauf des Tokens.
const ROLE_CONNECTION_ROLLBACK_BACKUP_RELS: [&str; 2] = [
    "core.discord_role_connection_tokens_rollback_backup",
    "core.discord_role_connection_sync_state_rollback_backup",
];
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
    account_id: Option<u32>,
}

const STEAM_ID64_ACCOUNT_ID_OFFSET: i64 = 76_561_197_960_265_728;

fn steam_account_id_from_steam64(steam_id64: i64) -> Option<u32> {
    let account_id = steam_id64.checked_sub(STEAM_ID64_ACCOUNT_ID_OFFSET)?;
    u32::try_from(account_id).ok()
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
        .chain(REDACTED_TEXT_USER_COLUMNS.iter().map(|spec| spec.relation))
        .chain(TEXT_USER_REF_COLUMNS.iter().map(|spec| spec.relation))
        .chain(JSON_USER_COLUMNS.iter().map(|spec| spec.relation))
        .chain([
            "scrim.replacement_needs",
            "scrim.replacement_candidates",
            "scrim.replacement_requests",
        ])
        .chain([
            USER_CO_PLAYERS_REL,
            KV_REL,
            USER_PRIVACY_REL,
            SCRIM_AUDIT_ACTOR_PSEUDONYMS_REL,
            "scrim.match_result_selection_events",
        ])
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
        .map(|row| {
            let text = row.steam_id.trim().to_string();
            let numeric = row.steam_id64.or_else(|| text.parse().ok());
            SteamId {
                text,
                numeric,
                account_id: numeric.and_then(steam_account_id_from_steam64),
            }
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
            let text = text.trim().to_string();
            let numeric = numeric.or_else(|| text.parse().ok());
            Some(SteamId {
                text,
                numeric,
                account_id: numeric.and_then(steam_account_id_from_steam64),
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

fn privacy_target_refs(user_key: &str, steam_ids: &[SteamId]) -> Vec<String> {
    let mut refs = vec![user_key.to_string()];
    for sid in steam_ids {
        push_unique_ref(&mut refs, sid.text.clone());
        if let Some(numeric) = sid.numeric {
            push_unique_ref(&mut refs, numeric.to_string());
        }
        if let Some(account_id) = sid.account_id {
            push_unique_ref(&mut refs, account_id.to_string());
        }
    }
    refs
}

fn steam_target_refs(steam_ids: &[SteamId]) -> Vec<String> {
    let mut refs = Vec::new();
    for sid in steam_ids {
        push_unique_ref(&mut refs, sid.text.clone());
        if let Some(numeric) = sid.numeric {
            push_unique_ref(&mut refs, numeric.to_string());
        }
        if let Some(account_id) = sid.account_id {
            push_unique_ref(&mut refs, account_id.to_string());
        }
    }
    refs
}

fn push_unique_ref(refs: &mut Vec<String>, value: String) {
    if !value.is_empty() && !refs.iter().any(|existing| existing == &value) {
        refs.push(value);
    }
}

fn text_ref_targets(
    spec: TextUserRefColumnSpec,
    user_key: &str,
    steam_ids: &[SteamId],
) -> Vec<String> {
    match spec.target_ref_set {
        TargetRefSet::Discord => vec![user_key.to_string()],
        TargetRefSet::Steam => steam_target_refs(steam_ids),
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

async fn redact_text_user_column(
    tx: &mut Transaction<'_, Postgres>,
    spec: RedactionSpec,
    user_key: &str,
) -> Result<i64, sqlx::Error> {
    let sql = match spec.display_col {
        Some(display_col) => format!(
            "UPDATE {} SET {} = 'redacted', {} = 'redacted' WHERE {} = $1",
            spec.relation, spec.col, display_col, spec.col
        ),
        None => format!(
            "UPDATE {} SET {} = 'redacted' WHERE {} = $1",
            spec.relation, spec.col, spec.col
        ),
    };
    let result = sqlx::query(&sql).bind(user_key).execute(&mut **tx).await?;
    Ok(rows_to_i64(result.rows_affected()))
}

async fn delete_scrim_replacement_rows_for_user(
    tx: &mut Transaction<'_, Postgres>,
    relations: &HashSet<&'static str>,
    user_id: i64,
) -> Result<BTreeMap<String, i64>, sqlx::Error> {
    let mut counts = BTreeMap::new();
    if !(relations.contains("scrim.replacement_requests")
        && relations.contains("scrim.replacement_candidates")
        && relations.contains("scrim.replacement_needs")
        && relations.contains("scrim.participants"))
    {
        return Ok(counts);
    }

    let requests = sqlx::query(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        ),
        target_needs AS (
            SELECT id
              FROM scrim.replacement_needs
             WHERE participant_id IN (SELECT id FROM target_participants)
        ),
        target_candidates AS (
            SELECT id
              FROM scrim.replacement_candidates
             WHERE discord_user_id = $1
                OR participant_id IN (SELECT id FROM target_participants)
                OR need_id IN (SELECT id FROM target_needs)
        )
        DELETE FROM scrim.replacement_requests
         WHERE discord_user_id = $1
            OR participant_id IN (SELECT id FROM target_participants)
            OR candidate_id IN (SELECT id FROM target_candidates)
            OR need_id IN (SELECT id FROM target_needs)
        "#,
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    counts.insert(
        "scrim_replacement_requests.user_identity".to_string(),
        rows_to_i64(requests.rows_affected()),
    );

    let candidates = sqlx::query(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        ),
        target_needs AS (
            SELECT id
              FROM scrim.replacement_needs
             WHERE participant_id IN (SELECT id FROM target_participants)
        )
        DELETE FROM scrim.replacement_candidates
         WHERE discord_user_id = $1
            OR participant_id IN (SELECT id FROM target_participants)
            OR need_id IN (SELECT id FROM target_needs)
        "#,
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    counts.insert(
        "scrim_replacement_candidates.user_identity".to_string(),
        rows_to_i64(candidates.rows_affected()),
    );

    let needs = sqlx::query(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        )
        DELETE FROM scrim.replacement_needs
         WHERE participant_id IN (SELECT id FROM target_participants)
        "#,
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    counts.insert(
        "scrim_replacement_needs.participant_id".to_string(),
        rows_to_i64(needs.rows_affected()),
    );

    Ok(counts)
}

async fn delete_scrim_audit_actor_mapping_for_user(
    tx: &mut Transaction<'_, Postgres>,
    user_key: &str,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM scrim.audit_actor_pseudonyms WHERE actor_type = 'user' AND actor_ref = $1",
    )
    .bind(user_key)
    .execute(&mut **tx)
    .await?;
    Ok(rows_to_i64(result.rows_affected()))
}

async fn select_scrim_audit_actor_mapping_for_user(
    pool: &PgPool,
    user_key: &str,
) -> CommunityDbResult<Value> {
    let rows = select_rows_lookup(
        pool,
        TableSpec::new(
            "scrim_audit_actor_pseudonyms",
            "actor_ref",
            SCRIM_AUDIT_ACTOR_PSEUDONYMS_REL,
            "actor_ref",
            ColumnType::Text,
        ),
        LookupValue::Text(user_key),
    )
    .await?;
    Ok(Value::Array(rows))
}

async fn set_privacy_erasure_context(
    tx: &mut Transaction<'_, Postgres>,
    user_key: &str,
    target_ref: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', $1, true),
             set_config('scrim.privacy_erasure_target_ref', $2, true)",
    )
    .bind(user_key)
    .bind(target_ref)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn redact_json_user_columns(
    tx: &mut Transaction<'_, Postgres>,
    relations: &HashSet<&'static str>,
    user_key: &str,
    target_refs: &[String],
) -> Result<BTreeMap<String, i64>, sqlx::Error> {
    let mut counts = BTreeMap::new();
    for &spec in JSON_USER_COLUMNS {
        if !relations.contains(spec.relation) {
            continue;
        }
        let sql = format!(
            "UPDATE {} SET {} = scrim.jsonb_redact_user_ref({}, $1) WHERE scrim.jsonb_contains_user_ref({}, $1)",
            spec.relation, spec.col, spec.col, spec.col
        );
        for target_ref in target_refs {
            set_privacy_erasure_context(tx, user_key, target_ref).await?;
            let result = sqlx::query(&sql)
                .bind(target_ref)
                .execute(&mut **tx)
                .await?;
            *counts.entry(spec.count_key()).or_insert(0) += rows_to_i64(result.rows_affected());
        }
    }
    Ok(counts)
}

async fn redact_text_user_ref_columns(
    tx: &mut Transaction<'_, Postgres>,
    relations: &HashSet<&'static str>,
    user_key: &str,
    steam_ids: &[SteamId],
) -> Result<BTreeMap<String, i64>, sqlx::Error> {
    let mut counts = BTreeMap::new();
    for &spec in TEXT_USER_REF_COLUMNS {
        if !relations.contains(spec.relation) {
            continue;
        }
        let sql = format!(
            "UPDATE {} SET {} = 'redacted' WHERE {} = $1 AND {} = $2",
            spec.relation, spec.col, spec.col, spec.kind_col
        );
        let target_refs = text_ref_targets(spec, user_key, steam_ids);
        for target_ref in target_refs {
            set_privacy_erasure_context(tx, user_key, target_ref.as_str()).await?;
            let result = sqlx::query(&sql)
                .bind(target_ref)
                .bind(spec.kind_value)
                .execute(&mut **tx)
                .await?;
            *counts.entry(spec.count_key()).or_insert(0) += rows_to_i64(result.rows_affected());
        }
    }
    Ok(counts)
}

fn is_ref_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn has_ref_boundary(text: &str, start: usize, end: usize) -> bool {
    let before_ok = text[..start]
        .chars()
        .next_back()
        .is_none_or(|ch| !is_ref_token_char(ch));
    let after_ok = text[end..]
        .chars()
        .next()
        .is_none_or(|ch| !is_ref_token_char(ch));
    before_ok && after_ok
}

fn scrub_exact_target_ref(text: &str, target_ref: &str) -> String {
    if target_ref.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(relative_start) = text[cursor..].find(target_ref) {
        let start = cursor + relative_start;
        let end = start + target_ref.len();
        if has_ref_boundary(text, start, end) {
            out.push_str(&text[cursor..start]);
            out.push_str("redacted");
            cursor = end;
        } else {
            out.push_str(&text[cursor..end]);
            cursor = end;
        }
    }
    out.push_str(&text[cursor..]);
    out
}

fn scrub_text_target_refs(text: &str, target_refs: &[String]) -> String {
    let mut scrubbed = text.to_string();
    let mut refs = target_refs
        .iter()
        .filter(|target| !target.is_empty())
        .collect::<Vec<_>>();
    refs.sort_by_key(|target| std::cmp::Reverse(target.len()));
    refs.dedup();
    for target_ref in refs {
        scrubbed = scrub_exact_target_ref(&scrubbed, target_ref);
    }
    scrubbed
}

async fn redact_match_result_ref_last_errors(
    tx: &mut Transaction<'_, Postgres>,
    relations: &HashSet<&'static str>,
    user_key: &str,
    target_refs: &[String],
) -> Result<i64, sqlx::Error> {
    if !relations.contains("scrim.match_result_refs") || target_refs.is_empty() {
        return Ok(0);
    }

    let rows = sqlx::query(
        "SELECT id, last_error
           FROM scrim.match_result_refs
          WHERE last_error IS NOT NULL
            AND EXISTS (
                SELECT 1
                 FROM unnest($1::TEXT[]) AS target(target_ref)
                WHERE target.target_ref <> ''
                  AND strpos(last_error, target.target_ref) > 0
            )
          FOR UPDATE",
    )
    .bind(target_refs)
    .fetch_all(&mut **tx)
    .await?;

    let mut updated = 0i64;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let last_error: String = row.try_get("last_error")?;
        let scrubbed = scrub_text_target_refs(&last_error, target_refs);
        if scrubbed == last_error {
            continue;
        }
        set_privacy_erasure_context(tx, user_key, user_key).await?;
        let result =
            sqlx::query("UPDATE scrim.match_result_refs SET last_error = $1 WHERE id = $2")
                .bind(scrubbed)
                .bind(id)
                .execute(&mut **tx)
                .await?;
        updated += rows_to_i64(result.rows_affected());
    }
    Ok(updated)
}

async fn redact_match_request_free_text_refs(
    tx: &mut Transaction<'_, Postgres>,
    relations: &HashSet<&'static str>,
    user_key: &str,
    target_refs: &[String],
) -> Result<BTreeMap<String, i64>, sqlx::Error> {
    let mut counts = BTreeMap::new();
    if !relations.contains("scrim.match_requests") || target_refs.is_empty() {
        return Ok(counts);
    }

    for column in ["override_reason", "status_message_last_error"] {
        let select_sql = format!(
            "SELECT id, {column} AS value
               FROM scrim.match_requests
              WHERE {column} IS NOT NULL
                AND EXISTS (
                    SELECT 1
                      FROM unnest($1::TEXT[]) AS target(target_ref)
                     WHERE target.target_ref <> ''
                       AND strpos({column}, target.target_ref) > 0
                )
              FOR UPDATE"
        );
        let rows = sqlx::query(&select_sql)
            .bind(target_refs)
            .fetch_all(&mut **tx)
            .await?;

        let mut updated = 0i64;
        for row in rows {
            let id: i32 = row.try_get("id")?;
            let value: String = row.try_get("value")?;
            let scrubbed = scrub_text_target_refs(&value, target_refs);
            if scrubbed == value {
                continue;
            }
            set_privacy_erasure_context(tx, user_key, user_key).await?;
            let update_sql = format!("UPDATE scrim.match_requests SET {column} = $1 WHERE id = $2");
            let result = sqlx::query(&update_sql)
                .bind(scrubbed)
                .bind(id)
                .execute(&mut **tx)
                .await?;
            updated += rows_to_i64(result.rows_affected());
        }
        counts.insert(format!("scrim_match_requests.{column}"), updated);
    }

    Ok(counts)
}

async fn select_scrim_replacement_rows_for_user(
    pool: &PgPool,
    relations: &HashSet<&'static str>,
    user_id: i64,
    user_key: &str,
) -> CommunityDbResult<BTreeMap<String, Value>> {
    let mut rows = BTreeMap::new();
    if !(relations.contains("scrim.replacement_requests")
        && relations.contains("scrim.replacement_candidates")
        && relations.contains("scrim.replacement_needs")
        && relations.contains("scrim.participants"))
    {
        return Ok(rows);
    }

    let needs: Value = sqlx::query_scalar(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        ),
        target_needs AS (
            SELECT *
              FROM scrim.replacement_needs
             WHERE participant_id IN (SELECT id FROM target_participants)
        )
        SELECT COALESCE(
            jsonb_agg(
                jsonb_set(
                    jsonb_set(
                        to_jsonb(n),
                        '{created_by_user_id}',
                        CASE WHEN n.created_by_user_id = $2 THEN to_jsonb(n.created_by_user_id) ELSE '"redacted"'::jsonb END
                    ),
                    '{created_by_display_name}',
                    CASE WHEN n.created_by_user_id = $2 THEN to_jsonb(n.created_by_display_name) ELSE '"redacted"'::jsonb END
                )
                ORDER BY n.id
            ),
            '[]'::jsonb
        )
        FROM target_needs AS n
        "#,
    )
    .bind(user_id)
    .bind(user_key)
    .fetch_one(pool)
    .await?;
    rows.insert("scrim_replacement_needs.participant_id".to_string(), needs);

    let candidates: Value = sqlx::query_scalar(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        ),
        target_needs AS (
            SELECT id
              FROM scrim.replacement_needs
             WHERE participant_id IN (SELECT id FROM target_participants)
        ),
        target_candidates AS (
            SELECT *
              FROM scrim.replacement_candidates
             WHERE discord_user_id = $1
                OR participant_id IN (SELECT id FROM target_participants)
                OR need_id IN (SELECT id FROM target_needs)
        )
        SELECT COALESCE(
            jsonb_agg(
                to_jsonb(c) || jsonb_build_object(
                    'discord_user_id',
                    CASE
                        WHEN c.discord_user_id IS NULL THEN 'null'::jsonb
                        WHEN c.discord_user_id = $1 THEN to_jsonb(c.discord_user_id)
                        ELSE '"redacted"'::jsonb
                    END,
                    'participant_id',
                    CASE
                        WHEN c.participant_id IS NULL THEN 'null'::jsonb
                        WHEN c.participant_id IN (SELECT id FROM target_participants) THEN to_jsonb(c.participant_id)
                        ELSE '"redacted"'::jsonb
                    END,
                    'candidate_data', jsonb_build_object('md5', md5(COALESCE(c.candidate_data, 'null'::jsonb)::text)),
                    'score_data', jsonb_build_object('md5', md5(COALESCE(c.score_data, 'null'::jsonb)::text))
                )
                ORDER BY c.id
            ),
            '[]'::jsonb
        )
        FROM target_candidates AS c
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    rows.insert(
        "scrim_replacement_candidates.user_identity".to_string(),
        candidates,
    );

    let requests: Value = sqlx::query_scalar(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        ),
        target_needs AS (
            SELECT id
              FROM scrim.replacement_needs
             WHERE participant_id IN (SELECT id FROM target_participants)
        ),
        target_candidates AS (
            SELECT id
              FROM scrim.replacement_candidates
             WHERE discord_user_id = $1
                OR participant_id IN (SELECT id FROM target_participants)
                OR need_id IN (SELECT id FROM target_needs)
        ),
        subject_candidates AS (
            SELECT id
              FROM scrim.replacement_candidates
             WHERE discord_user_id = $1
                OR participant_id IN (SELECT id FROM target_participants)
        ),
        target_requests AS (
            SELECT *
              FROM scrim.replacement_requests
             WHERE discord_user_id = $1
                OR participant_id IN (SELECT id FROM target_participants)
                OR candidate_id IN (SELECT id FROM target_candidates)
                OR need_id IN (SELECT id FROM target_needs)
        )
        SELECT COALESCE(
            jsonb_agg(
                to_jsonb(r) || jsonb_build_object(
                    'discord_user_id',
                    CASE
                        WHEN r.discord_user_id IS NULL THEN 'null'::jsonb
                        WHEN r.discord_user_id = $1 THEN to_jsonb(r.discord_user_id)
                        ELSE '"redacted"'::jsonb
                    END,
                    'participant_id',
                    CASE
                        WHEN r.participant_id IS NULL THEN 'null'::jsonb
                        WHEN r.participant_id IN (SELECT id FROM target_participants) THEN to_jsonb(r.participant_id)
                        ELSE '"redacted"'::jsonb
                    END,
                    'requested_by_user_id',
                    CASE WHEN r.requested_by_user_id = $2 THEN to_jsonb(r.requested_by_user_id) ELSE '"redacted"'::jsonb END,
                    'requested_by_display_name',
                    CASE WHEN r.requested_by_user_id = $2 THEN to_jsonb(r.requested_by_display_name) ELSE '"redacted"'::jsonb END,
                    'request_payload', jsonb_build_object('md5', md5(COALESCE(r.request_payload, 'null'::jsonb)::text))
                )
                ORDER BY r.id
            ),
            '[]'::jsonb
        )
        FROM target_requests AS r
        "#,
    )
    .bind(user_id)
    .bind(user_key)
    .fetch_one(pool)
    .await?;
    rows.insert(
        "scrim_replacement_requests.user_identity".to_string(),
        requests,
    );

    let created_needs: Value = sqlx::query_scalar(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        ),
        actor_needs AS (
            SELECT *
              FROM scrim.replacement_needs
             WHERE created_by_user_id = $2
        )
        SELECT COALESCE(
            jsonb_agg(
                to_jsonb(n) || jsonb_build_object(
                    'participant_id',
                    CASE
                        WHEN n.participant_id IS NULL THEN 'null'::jsonb
                        WHEN n.participant_id IN (SELECT id FROM target_participants) THEN to_jsonb(n.participant_id)
                        ELSE '"redacted"'::jsonb
                    END,
                    'created_by_user_id', to_jsonb(n.created_by_user_id),
                    'created_by_display_name', to_jsonb(n.created_by_display_name)
                )
                ORDER BY n.id
            ),
            '[]'::jsonb
        )
        FROM actor_needs AS n
        "#,
    )
    .bind(user_id)
    .bind(user_key)
    .fetch_one(pool)
    .await?;
    rows.insert(
        "scrim_replacement_needs.created_by_user_id.redacted".to_string(),
        created_needs,
    );

    let requested_requests: Value = sqlx::query_scalar(
        r#"
        WITH target_participants AS (
            SELECT id FROM scrim.participants WHERE discord_id = $1
        ),
        actor_requests AS (
            SELECT *
              FROM scrim.replacement_requests
             WHERE requested_by_user_id = $2
        )
        SELECT COALESCE(
            jsonb_agg(
                to_jsonb(r) || jsonb_build_object(
                    'discord_user_id',
                    CASE
                        WHEN r.discord_user_id IS NULL THEN 'null'::jsonb
                        WHEN r.discord_user_id = $1 THEN to_jsonb(r.discord_user_id)
                        ELSE '"redacted"'::jsonb
                    END,
                    'participant_id',
                    CASE
                        WHEN r.participant_id IS NULL THEN 'null'::jsonb
                        WHEN r.participant_id IN (SELECT id FROM target_participants) THEN to_jsonb(r.participant_id)
                        ELSE '"redacted"'::jsonb
                    END,
                    'requested_by_user_id', to_jsonb(r.requested_by_user_id),
                    'requested_by_display_name', to_jsonb(r.requested_by_display_name),
                    'request_payload', jsonb_build_object('md5', md5(COALESCE(r.request_payload, 'null'::jsonb)::text))
                )
                ORDER BY r.id
            ),
            '[]'::jsonb
        )
        FROM actor_requests AS r
        "#,
    )
    .bind(user_id)
    .bind(user_key)
    .fetch_one(pool)
    .await?;
    rows.insert(
        "scrim_replacement_requests.requested_by_user_id.redacted".to_string(),
        requested_requests,
    );

    Ok(rows)
}

async fn select_projected_rows_by_text_user(
    pool: &PgPool,
    sql: &str,
    user_key: &str,
) -> CommunityDbResult<Value> {
    Ok(sqlx::query_scalar(sql)
        .bind(user_key)
        .fetch_one(pool)
        .await?)
}

fn is_user_id_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "user_id"
        || key == "id"
        || key == "steam_id"
        || key == "steam_id64"
        || key.ends_with("_user_id")
        || key == "discord_id"
        || key == "discord_user_id"
        || key == "target_id"
        || key == "subject_id"
        || key == "aggregate_id"
        || key.ends_with("_discord_id")
        || key.ends_with("_id")
        || key.ends_with("_ids")
}

fn is_label_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("display")
        || key.ends_with("_name")
        || key == "name"
        || key.contains("nickname")
        || key.contains("tag")
}

fn is_raw_error_text_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("last_error")
}

fn is_foreign_context_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("foreign") || key.starts_with("other_") || key.ends_with("_other")
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn scalar_matches_any_target(value: &Value, target_refs: &[String]) -> bool {
    scalar_text(value).is_some_and(|value| target_refs.iter().any(|target| target == &value))
}

fn key_matches_any_target(key: &str, target_refs: &[String]) -> bool {
    let key = key.trim();
    target_refs.iter().any(|target| target == key)
}

fn object_key_is_ref_like(key: &str) -> bool {
    let key = key.trim();
    let lower = key.to_ascii_lowercase();
    let has_digit = key.chars().any(|ch| ch.is_ascii_digit());
    !key.is_empty()
        && (key.chars().all(|ch| ch.is_ascii_digit())
            || (has_digit
                && (key.contains('_')
                    || key.contains('-')
                    || key.contains(':')
                    || lower.contains("user")
                    || lower.contains("steam")
                    || lower.contains("discord"))))
}

fn scalar_is_ref_like(value: &Value) -> bool {
    match value {
        Value::Number(_) => true,
        Value::String(value) => {
            let value = value.trim();
            value.len() >= 2 && value.chars().any(|ch| ch.is_ascii_digit())
        }
        _ => false,
    }
}

fn user_ref_state(value: &Value, target_refs: &[String], key: Option<&str>) -> (bool, bool) {
    let key_has_target = key.is_some_and(|key| key_matches_any_target(key, target_refs));
    let key_has_foreign = key.is_some_and(|key| !key_has_target && object_key_is_ref_like(key));
    match value {
        Value::Object(map) => map.iter().fold(
            (key_has_target, key_has_foreign),
            |state, (child_key, child)| {
                let child_state = user_ref_state(child, target_refs, Some(child_key));
                (state.0 || child_state.0, state.1 || child_state.1)
            },
        ),
        Value::Array(items) => {
            items
                .iter()
                .fold((key_has_target, key_has_foreign), |state, child| {
                    let child_state = user_ref_state(child, target_refs, key);
                    (state.0 || child_state.0, state.1 || child_state.1)
                })
        }
        _ => {
            if key_has_target || scalar_matches_any_target(value, target_refs) {
                (true, key_has_foreign)
            } else if matches!(value, Value::String(_) | Value::Number(_))
                && (key.is_some_and(|key| is_user_id_key(key) || is_foreign_context_key(key))
                    || scalar_is_ref_like(value))
            {
                (false, true)
            } else {
                (false, key_has_foreign)
            }
        }
    }
}

fn projected_object_key(
    key: &str,
    target_refs: &[String],
    fallback_index: usize,
    used_keys: &mut HashSet<String>,
) -> String {
    let base = if key_matches_any_target(key, target_refs) || !object_key_is_ref_like(key) {
        key.to_string()
    } else {
        format!("redacted_key_{fallback_index}")
    };
    let mut candidate = base.clone();
    let mut suffix = 1usize;
    while used_keys.contains(&candidate) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    used_keys.insert(candidate.clone());
    candidate
}

fn redact_non_target_subtree(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = serde_json::Map::new();
            let mut used_keys = HashSet::new();
            for (index, (key, value)) in map.iter().enumerate() {
                let key = projected_object_key(key, &[], index, &mut used_keys);
                redacted.insert(key, redact_non_target_subtree(value));
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_non_target_subtree).collect()),
        Value::String(_) | Value::Number(_) | Value::Bool(_) => {
            Value::String("redacted".to_string())
        }
        _ => value.clone(),
    }
}

fn redact_foreign_user_ids(value: &Value, target_refs: &[String]) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_foreign_user_ids(item, target_refs))
                .collect(),
        ),
        _ if scalar_matches_any_target(value, target_refs) => value.clone(),
        Value::String(_) | Value::Number(_) => Value::String("redacted".to_string()),
        _ => value.clone(),
    }
}

fn project_json_for_targets(value: &Value, target_refs: &[String], key: Option<&str>) -> Value {
    match value {
        Value::Object(map) => {
            let (has_target, has_foreign) = user_ref_state(value, target_refs, key);
            if !has_target {
                return redact_non_target_subtree(value);
            }
            let mut projected = serde_json::Map::new();
            let mut used_keys = HashSet::new();
            for (index, (child_key, child)) in map.iter().enumerate() {
                let child_key_has_target = key_matches_any_target(child_key, target_refs);
                let child_key_has_foreign =
                    !child_key_has_target && object_key_is_ref_like(child_key);
                let (child_value_has_target, child_value_has_foreign) =
                    user_ref_state(child, target_refs, Some(child_key));
                let child_has_target = child_key_has_target || child_value_has_target;
                let child_has_foreign = child_key_has_foreign || child_value_has_foreign;
                let child_value = if child_has_target {
                    project_json_for_targets(child, target_refs, Some(child_key))
                } else if child_has_foreign
                    || (has_foreign
                        && (is_user_id_key(child_key)
                            || is_label_key(child_key)
                            || is_foreign_context_key(child_key)))
                {
                    redact_non_target_subtree(child)
                } else {
                    child.clone()
                };
                let projected_key =
                    projected_object_key(child_key, target_refs, index, &mut used_keys);
                projected.insert(projected_key, child_value);
            }
            Value::Object(projected)
        }
        Value::Array(items) => {
            let array_has_target = items
                .iter()
                .any(|item| user_ref_state(item, target_refs, key).0);
            Value::Array(
                items
                    .iter()
                    .map(|item| {
                        if array_has_target && !user_ref_state(item, target_refs, key).0 {
                            redact_non_target_subtree(item)
                        } else {
                            project_json_for_targets(item, target_refs, key)
                        }
                    })
                    .collect(),
            )
        }
        _ if key.is_some_and(is_user_id_key) => redact_foreign_user_ids(value, target_refs),
        _ => value.clone(),
    }
}

fn json_columns_for_relation(relation: &str) -> Vec<&'static str> {
    JSON_USER_COLUMNS
        .iter()
        .filter_map(|spec| (spec.relation == relation).then_some(spec.col))
        .collect()
}

fn project_privacy_row_for_targets(
    mut row: Value,
    target_refs: &[String],
    json_columns: &[&str],
) -> Value {
    let Value::Object(ref mut obj) = row else {
        return row;
    };
    let (_row_has_target, row_has_foreign) =
        user_ref_state(&Value::Object(obj.clone()), target_refs, None);
    for (key, value) in obj.iter_mut() {
        if key == "id" {
            continue;
        } else if json_columns.contains(&key.as_str()) {
            *value = project_json_for_targets(value, target_refs, Some(key));
        } else if is_raw_error_text_key(key) && !value.is_null() {
            *value = Value::String("redacted".to_string());
        } else if is_user_id_key(key) {
            *value = redact_foreign_user_ids(value, target_refs);
        } else if row_has_foreign && (is_label_key(key) || is_foreign_context_key(key)) {
            *value = redact_non_target_subtree(value);
        }
    }
    row
}

async fn select_json_user_rows_for_user(
    pool: &PgPool,
    relations: &HashSet<&'static str>,
    target_refs: &[String],
) -> CommunityDbResult<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    if target_refs.is_empty() {
        return Ok(out);
    }
    for &spec in JSON_USER_COLUMNS {
        if !relations.contains(spec.relation) {
            continue;
        }
        let rows = match spec.export_mode {
            JsonUserColumnExport::GenericProjected => {
                let sql = format!(
                    "SELECT row_to_json(t)::text AS row_json FROM (SELECT id, {} FROM {} WHERE EXISTS (SELECT 1 FROM unnest($1::TEXT[]) AS target(target_ref) WHERE scrim.jsonb_contains_user_ref({}, target.target_ref)) ORDER BY id) t",
                    spec.col,
                    spec.relation, spec.col
                );
                sqlx::query(&sql)
                    .bind(target_refs)
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| {
                        let raw: String = row.try_get("row_json")?;
                        let value = serde_json::from_str::<Value>(&raw)?;
                        Ok(project_privacy_row_for_targets(
                            value,
                            target_refs,
                            &[spec.col],
                        ))
                    })
                    .collect::<CommunityDbResult<Vec<_>>>()?
            }
            JsonUserColumnExport::HashOnly => {
                let sql = format!(
                    "SELECT row_to_json(t)::text AS row_json FROM (SELECT id, jsonb_build_object('md5', md5(COALESCE({}, 'null'::jsonb)::text)) AS {} FROM {} WHERE EXISTS (SELECT 1 FROM unnest($1::TEXT[]) AS target(target_ref) WHERE scrim.jsonb_contains_user_ref({}, target.target_ref)) ORDER BY id) t",
                    spec.col,
                    spec.col,
                    spec.relation,
                    spec.col
                );
                sqlx::query(&sql)
                    .bind(target_refs)
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| {
                        let raw: String = row.try_get("row_json")?;
                        serde_json::from_str::<Value>(&raw).map_err(CommunityDbError::from)
                    })
                    .collect::<CommunityDbResult<Vec<_>>>()?
            }
        };
        out.insert(spec.count_key(), Value::Array(rows));
    }
    Ok(out)
}

async fn select_text_user_ref_rows_for_user(
    pool: &PgPool,
    relations: &HashSet<&'static str>,
    user_key: &str,
    steam_ids: &[SteamId],
) -> CommunityDbResult<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for &spec in TEXT_USER_REF_COLUMNS {
        if !relations.contains(spec.relation) {
            continue;
        }
        let target_refs = text_ref_targets(spec, user_key, steam_ids);
        if target_refs.is_empty() {
            continue;
        }
        let sql = format!(
            "SELECT row_to_json(t)::text AS row_json FROM (SELECT * FROM {} WHERE {} = ANY($1::TEXT[]) AND {} = $2 ORDER BY id) t",
            spec.relation, spec.col, spec.kind_col
        );
        let json_columns = json_columns_for_relation(spec.relation);
        let rows = sqlx::query(&sql)
            .bind(target_refs.clone())
            .bind(spec.kind_value)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| {
                let raw: String = row.try_get("row_json")?;
                let value = serde_json::from_str::<Value>(&raw)?;
                Ok(project_privacy_row_for_targets(
                    value,
                    &target_refs,
                    &json_columns,
                ))
            })
            .collect::<CommunityDbResult<Vec<_>>>()?;
        let entry = out
            .entry(spec.count_key())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(existing) = entry {
            existing.extend(rows);
        }
    }
    Ok(out)
}

async fn select_scrim_projected_actor_rows_for_user(
    pool: &PgPool,
    relations: &HashSet<&'static str>,
    user_key: &str,
) -> CommunityDbResult<BTreeMap<String, Value>> {
    let mut rows = BTreeMap::new();

    if relations.contains("scrim.announcement_drafts") {
        let created: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT d.id,
                       to_jsonb(d) || jsonb_build_object(
                           'created_by_user_id', to_jsonb(d.created_by_user_id),
                           'created_by_display_name', to_jsonb(d.created_by_display_name),
                           'approved_by_user_id',
                           CASE
                               WHEN d.approved_by_user_id IS NULL THEN 'null'::jsonb
                               WHEN d.approved_by_user_id = $1 THEN to_jsonb(d.approved_by_user_id)
                               ELSE '"redacted"'::jsonb
                           END,
                           'approved_by_display_name',
                           CASE
                               WHEN d.approved_by_user_id IS NULL THEN 'null'::jsonb
                               WHEN d.approved_by_user_id = $1 THEN COALESCE(to_jsonb(d.approved_by_display_name), 'null'::jsonb)
                               ELSE '"redacted"'::jsonb
                           END,
                           'title', '"redacted"'::jsonb,
                           'body', '"redacted"'::jsonb,
                           'payload', jsonb_build_object('md5', md5(COALESCE(d.payload, 'null'::jsonb)::text))
                       ) AS row_json
                  FROM scrim.announcement_drafts AS d
                 WHERE d.created_by_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_announcement_drafts.created_by_user_id.redacted".to_string(),
            created,
        );

        let approved: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT d.id,
                       to_jsonb(d) || jsonb_build_object(
                           'created_by_user_id',
                           CASE
                               WHEN d.created_by_user_id = $1 THEN to_jsonb(d.created_by_user_id)
                               ELSE '"redacted"'::jsonb
                           END,
                           'created_by_display_name',
                           CASE
                               WHEN d.created_by_user_id = $1 THEN to_jsonb(d.created_by_display_name)
                               ELSE '"redacted"'::jsonb
                           END,
                           'approved_by_user_id', to_jsonb(d.approved_by_user_id),
                           'approved_by_display_name', COALESCE(to_jsonb(d.approved_by_display_name), 'null'::jsonb),
                           'title', '"redacted"'::jsonb,
                           'body', '"redacted"'::jsonb,
                           'payload', jsonb_build_object('md5', md5(COALESCE(d.payload, 'null'::jsonb)::text))
                       ) AS row_json
                  FROM scrim.announcement_drafts AS d
                 WHERE d.approved_by_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_announcement_drafts.approved_by_user_id.redacted".to_string(),
            approved,
        );
    }

    if relations.contains("scrim.announcement_approvals") {
        let approvals: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT a.id,
                       to_jsonb(a) || jsonb_build_object(
                           'decided_by_user_id', to_jsonb(a.decided_by_user_id),
                           'decided_by_display_name', to_jsonb(a.decided_by_display_name),
                           'decision_data', jsonb_build_object('md5', md5(COALESCE(a.decision_data, 'null'::jsonb)::text))
                       ) AS row_json
                  FROM scrim.announcement_approvals AS a
                 WHERE a.decided_by_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_announcement_approvals.decided_by_user_id.redacted".to_string(),
            approvals,
        );
    }

    if relations.contains("scrim.status_publication_approvals") {
        let approvals: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT a.id,
                       to_jsonb(a) || jsonb_build_object(
                           'target_id',
                           CASE WHEN a.target_id = $1 THEN to_jsonb(a.target_id) ELSE '"redacted"'::jsonb END,
                           'decided_by_user_id', COALESCE(to_jsonb(a.decided_by_user_id), 'null'::jsonb),
                           'decided_by_display_name', COALESCE(to_jsonb(a.decided_by_display_name), 'null'::jsonb),
                           'payload', jsonb_build_object('md5', md5(COALESCE(a.payload, 'null'::jsonb)::text)),
                           'decision_data', jsonb_build_object('md5', md5(COALESCE(a.decision_data, 'null'::jsonb)::text))
                       ) AS row_json
                  FROM scrim.status_publication_approvals AS a
                 WHERE a.decided_by_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_status_publication_approvals.decided_by_user_id.redacted".to_string(),
            approvals,
        );
    }

    if relations.contains("scrim.matches") {
        let matches: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT m.id,
                       jsonb_build_object(
                           'id', m.id,
                           'team_a_id', m.team_a_id,
                           'team_b_id', COALESCE(to_jsonb(m.team_b_id), 'null'::jsonb),
                           'status', m.status,
                           'scheduled_at', COALESCE(to_jsonb(m.scheduled_at), 'null'::jsonb),
                           'created_at', m.created_at,
                           'updated_at', COALESCE(to_jsonb(m.updated_at), 'null'::jsonb),
                           'steam_match_id', COALESCE(to_jsonb(m.steam_match_id), 'null'::jsonb),
                           'winner_team_id', COALESCE(to_jsonb(m.winner_team_id), 'null'::jsonb),
                           'lobby_state', COALESCE(to_jsonb(m.lobby_state), 'null'::jsonb),
                           'lobby_code_source_user_id', to_jsonb(m.lobby_code_source_user_id),
                           'lobby_code_source_display_name', COALESCE(to_jsonb(m.lobby_code_source_display_name), 'null'::jsonb),
                           'lobby_code_updated_at', COALESCE(to_jsonb(m.lobby_code_updated_at), 'null'::jsonb),
                           'lobby_code_message_ids', jsonb_build_object('md5', md5(COALESCE(m.lobby_code_message_ids, '{}'::jsonb)::text)),
                           'lobby_code_corrections', jsonb_build_object('md5', md5(COALESCE(m.lobby_code_corrections, '[]'::jsonb)::text)),
                           'result_json',
                           CASE
                               WHEN m.result_json IS NULL THEN 'null'::jsonb
                               ELSE jsonb_build_object('md5', md5(m.result_json::text))
                           END,
                           'when_text',
                           CASE
                               WHEN m.when_text IS NULL THEN 'null'::jsonb
                               ELSE '"redacted"'::jsonb
                           END
                       ) AS row_json
                  FROM scrim.matches AS m
                 WHERE m.lobby_code_source_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_matches.lobby_code_source_user_id.redacted".to_string(),
            matches,
        );
    }

    if relations.contains("scrim.match_requests") {
        let requests: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT r.id,
                       jsonb_build_object(
                           'id', r.id,
                           'batch_id', r.batch_id,
                           'team_a_id', r.team_a_id,
                           'team_b_id', COALESCE(to_jsonb(r.team_b_id), 'null'::jsonb),
                           'status', r.status,
                           'created_at', r.created_at,
                           'updated_at', r.updated_at,
                           'posted_at', COALESCE(to_jsonb(r.posted_at), 'null'::jsonb),
                           'released_slot_index', COALESCE(to_jsonb(r.released_slot_index), 'null'::jsonb),
                           'released_at', COALESCE(to_jsonb(r.released_at), 'null'::jsonb),
                           'released_by_user_id', to_jsonb(r.released_by_user_id),
                           'released_by_display_name', COALESCE(to_jsonb(r.released_by_display_name), 'null'::jsonb),
                           'status_message_state', r.status_message_state,
                           'status_message_posted_at', COALESCE(to_jsonb(r.status_message_posted_at), 'null'::jsonb),
                           'status_message_updated_at', COALESCE(to_jsonb(r.status_message_updated_at), 'null'::jsonb),
                           'slot_options', jsonb_build_object('md5', md5(COALESCE(r.slot_options, 'null'::jsonb)::text)),
                           'released_slot',
                           CASE
                               WHEN r.released_slot IS NULL THEN 'null'::jsonb
                               ELSE jsonb_build_object('md5', md5(r.released_slot::text))
                           END,
                           'team_query_message_ids', jsonb_build_object('md5', md5(COALESCE(r.team_query_message_ids, '{}'::jsonb)::text)),
                           'team_status_message_ids', jsonb_build_object('md5', md5(COALESCE(r.team_status_message_ids, '{}'::jsonb)::text)),
                           'override_reason',
                           CASE
                               WHEN r.override_reason IS NULL THEN 'null'::jsonb
                               ELSE '"redacted"'::jsonb
                           END,
                           'status_message_last_error',
                           CASE
                               WHEN r.status_message_last_error IS NULL THEN 'null'::jsonb
                               ELSE '"redacted"'::jsonb
                           END
                       ) AS row_json
                  FROM scrim.match_requests AS r
                 WHERE r.released_by_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_match_requests.released_by_user_id.redacted".to_string(),
            requests,
        );
    }

    if relations.contains("scrim.match_lineup_snapshots") {
        let target_refs = vec![user_key.to_string()];
        let lineup_rows = sqlx::query(
            "SELECT row_to_json(t)::text AS row_json
               FROM (
                    SELECT *
                      FROM scrim.match_lineup_snapshots
                     WHERE created_by_user_id = $1
                     ORDER BY id
               ) t",
        )
        .bind(user_key)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| {
            let raw: String = row.try_get("row_json")?;
            let value = serde_json::from_str::<Value>(&raw)?;
            Ok(project_privacy_row_for_targets(
                value,
                &target_refs,
                &["lineup_payload"],
            ))
        })
        .collect::<CommunityDbResult<Vec<_>>>()?;
        rows.insert(
            "scrim_match_lineup_snapshots.created_by_user_id.redacted".to_string(),
            Value::Array(lineup_rows),
        );
    }

    if relations.contains("scrim.match_result_refs") {
        let source_refs: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT r.id,
                       to_jsonb(r) || jsonb_build_object(
                           'source_user_id', to_jsonb(r.source_user_id),
                           'source_display_name', to_jsonb(r.source_display_name),
                           'selected_by_user_id',
                           CASE
                               WHEN r.selected_by_user_id IS NULL THEN 'null'::jsonb
                               WHEN r.selected_by_user_id = $1 THEN to_jsonb(r.selected_by_user_id)
                               ELSE '"redacted"'::jsonb
                           END,
                            'clarification_payload', jsonb_build_object('md5', md5(COALESCE(r.clarification_payload, 'null'::jsonb)::text)),
                            'last_error',
                            CASE
                                WHEN r.last_error IS NULL THEN 'null'::jsonb
                                ELSE jsonb_build_object('md5', md5(r.last_error))
                            END,
                            'raw_result_json',
                            CASE
                                WHEN r.raw_result_json IS NULL THEN 'null'::jsonb
                               ELSE jsonb_build_object('md5', md5(r.raw_result_json::text))
                           END,
                           'normalized_result_json',
                           CASE
                               WHEN r.normalized_result_json IS NULL THEN 'null'::jsonb
                               ELSE jsonb_build_object('md5', md5(r.normalized_result_json::text))
                           END
                       ) AS row_json
                  FROM scrim.match_result_refs AS r
                 WHERE r.source_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_match_result_refs.source_user_id.redacted".to_string(),
            source_refs,
        );

        let selected_refs: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT r.id,
                       to_jsonb(r) || jsonb_build_object(
                           'source_user_id',
                           CASE WHEN r.source_user_id = $1 THEN to_jsonb(r.source_user_id) ELSE '"redacted"'::jsonb END,
                           'source_display_name',
                            CASE WHEN r.source_user_id = $1 THEN to_jsonb(r.source_display_name) ELSE '"redacted"'::jsonb END,
                            'selected_by_user_id', to_jsonb(r.selected_by_user_id),
                            'clarification_payload', jsonb_build_object('md5', md5(COALESCE(r.clarification_payload, 'null'::jsonb)::text)),
                            'last_error',
                            CASE
                                WHEN r.last_error IS NULL THEN 'null'::jsonb
                                ELSE jsonb_build_object('md5', md5(r.last_error))
                            END,
                            'raw_result_json',
                            CASE
                                WHEN r.raw_result_json IS NULL THEN 'null'::jsonb
                               ELSE jsonb_build_object('md5', md5(r.raw_result_json::text))
                           END,
                           'normalized_result_json',
                           CASE
                               WHEN r.normalized_result_json IS NULL THEN 'null'::jsonb
                               ELSE jsonb_build_object('md5', md5(r.normalized_result_json::text))
                           END
                       ) AS row_json
                  FROM scrim.match_result_refs AS r
                 WHERE r.selected_by_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_match_result_refs.selected_by_user_id.redacted".to_string(),
            selected_refs,
        );
    }

    if relations.contains("scrim.match_result_clarifications") {
        let clarifications: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT c.id,
                       to_jsonb(c) || jsonb_build_object(
                           'requested_by_user_id', to_jsonb(c.requested_by_user_id),
                           'requested_by_display_name', to_jsonb(c.requested_by_display_name),
                           'request_payload', jsonb_build_object('md5', md5(COALESCE(c.request_payload, 'null'::jsonb)::text)),
                           'response_payload',
                           CASE
                               WHEN c.response_payload IS NULL THEN 'null'::jsonb
                               ELSE jsonb_build_object('md5', md5(c.response_payload::text))
                           END
                       ) AS row_json
                  FROM scrim.match_result_clarifications AS c
                 WHERE c.requested_by_user_id = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_match_result_clarifications.requested_by_user_id.redacted".to_string(),
            clarifications,
        );
    }

    if relations.contains("scrim.match_result_selection_events")
        && relations.contains(SCRIM_AUDIT_ACTOR_PSEUDONYMS_REL)
    {
        let selection_events: Value = select_projected_rows_by_text_user(
            pool,
            r#"
            WITH projected AS (
                SELECT e.id,
                       to_jsonb(e) || jsonb_build_object(
                           'before_data', jsonb_build_object('md5', md5(COALESCE(e.before_data, 'null'::jsonb)::text)),
                           'after_data', jsonb_build_object('md5', md5(COALESCE(e.after_data, 'null'::jsonb)::text))
                       ) AS row_json
                  FROM scrim.match_result_selection_events AS e
                  JOIN scrim.audit_actor_pseudonyms AS p
                    ON p.actor_pseudonym = e.actor_pseudonym
                 WHERE p.actor_type = 'user'
                   AND p.actor_ref = $1
            )
            SELECT COALESCE(jsonb_agg(row_json ORDER BY id), '[]'::jsonb)
              FROM projected
            "#,
            user_key,
        )
        .await?;
        rows.insert(
            "scrim_match_result_selection_events.actor_pseudonym".to_string(),
            selection_events,
        );
    }

    Ok(rows)
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

/// Raeumt die Rollback-Kopien der Linked-Role-Tabellen nach Fristablauf.
/// Vergessenes Aufraeumen waere eine unbefristete Token-Halde neben der
/// Live-Tabelle; die Tabellen fehlen im Normalbetrieb, deshalb der Existenztest.
pub async fn purge_expired_role_connection_rollback_backups(
    pool: &PgPool,
    now: chrono::DateTime<chrono::Utc>,
) -> CommunityDbResult<Vec<(&'static str, i64)>> {
    let mut deleted = Vec::new();
    for relation in ROLE_CONNECTION_ROLLBACK_BACKUP_RELS {
        if !relation_exists(pool, relation).await? {
            continue;
        }
        // `rollback_expires_at`, nicht `expires_at`: letzteres traegt in der
        // Tokens-Kopie den OAuth-Ablauf des Tokens und wuerde die Sicherung
        // binnen Tagen loeschen.
        let result = sqlx::query(&format!(
            "DELETE FROM {relation} WHERE rollback_expires_at <= $1"
        ))
        .bind(now)
        .execute(pool)
        .await?;
        deleted.push((relation, rows_to_i64(result.rows_affected())));
    }
    Ok(deleted)
}

async fn purge_expired_server_sync_rollback_exports_tx(
    tx: &mut Transaction<'_, Postgres>,
    now: chrono::DateTime<chrono::Utc>,
) -> CommunityDbResult<i64> {
    let result = sqlx::query("DELETE FROM server_config.rollback_exports WHERE expires_at <= $1")
        .bind(now)
        .execute(&mut **tx)
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
            // Im selben Takt, weil es dieselbe Sorte Artefakt ist: eine Kopie mit
            // Tokens, die nach einem Rollback liegen bleibt.
            match purge_expired_role_connection_rollback_backups(&pool, chrono::Utc::now()).await {
                Ok(deleted) => {
                    // Je Tabelle getrennt: eine Summe wuerde verstecken, dass die
                    // Token-Kopien verschwunden sind und die Sync-Kopien nicht.
                    for (relation, count) in deleted.iter().filter(|(_, count)| *count > 0) {
                        tracing::info!(
                            relation,
                            deleted = count,
                            "Linked-Role-Rollback-Kopien nach Fristablauf geraeumt"
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, "Linked-Role-Rollback-Retention fehlgeschlagen");
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
    let relations = existing_relations(pool).await?;
    let user_key = user_id.to_string();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();

    let mut tx = pool.begin().await?;
    dl_central_db::lock_raw_event_retention_erasure(&mut tx).await?;
    lock_user_privacy(&mut tx, user_id).await?;
    let expired_rollback_exports = if relations.contains(SERVER_SYNC_ROLLBACK_EXPORTS_REL) {
        purge_expired_server_sync_rollback_exports_tx(&mut tx, now).await?
    } else {
        0
    };
    counts.insert(
        "server_config.rollback_exports.expired".to_string(),
        expired_rollback_exports,
    );
    let steam_ids = if relations.contains("core.steam_links") {
        steam_ids_for_user_tx(&mut tx, user_id).await?
    } else {
        Vec::new()
    };
    let target_refs = privacy_target_refs(&user_key, &steam_ids);

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

    for (key, value) in
        redact_json_user_columns(&mut tx, &relations, &user_key, &target_refs).await?
    {
        counts.insert(key, value);
    }

    let redacted_result_errors =
        redact_match_result_ref_last_errors(&mut tx, &relations, &user_key, &target_refs).await?;
    counts.insert(
        "scrim_match_result_refs.last_error".to_string(),
        redacted_result_errors,
    );
    for (key, value) in
        redact_match_request_free_text_refs(&mut tx, &relations, &user_key, &target_refs).await?
    {
        counts.insert(key, value);
    }

    for (key, value) in
        redact_text_user_ref_columns(&mut tx, &relations, &user_key, &steam_ids).await?
    {
        counts.insert(key, value);
    }

    set_privacy_erasure_context(&mut tx, &user_key, &user_key).await?;
    for &spec in REDACTED_TEXT_USER_COLUMNS {
        if !relations.contains(spec.relation) {
            continue;
        }
        let n = redact_text_user_column(&mut tx, spec, &user_key).await?;
        counts.insert(spec.count_key(), n);
    }

    for (key, value) in delete_scrim_replacement_rows_for_user(&mut tx, &relations, user_id).await?
    {
        counts.insert(key, value);
    }

    if relations.contains(SCRIM_AUDIT_ACTOR_PSEUDONYMS_REL) {
        let n = delete_scrim_audit_actor_mapping_for_user(&mut tx, &user_key).await?;
        counts.insert("scrim_audit_actor_pseudonyms.actor_ref".to_string(), n);
    }

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

        let mut onboarding_dm = 0i64;
        for ns in ["onboarding_bridge_dm", "onboarding_tour"] {
            let result = sqlx::query("DELETE FROM bot.kv_store WHERE ns = $1 AND k = $2")
                .bind(ns)
                .bind(&uid_key)
                .execute(&mut *tx)
                .await?;
            onboarding_dm += rows_to_i64(result.rows_affected());
        }
        counts.insert("kv_onboarding_dm".to_string(), onboarding_dm);

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

    tx.commit().await?;

    Ok(DeleteSummary {
        counts,
        steam_ids: steam_ids.into_iter().map(|sid| sid.text).collect(),
    })
}

pub async fn export_user_data(pool: &PgPool, user_id: i64, now: i64) -> CommunityDbResult<Value> {
    let relations = existing_relations(pool).await?;
    let user_key = user_id.to_string();
    let steam_ids = steam_ids_for_user(pool, user_id).await?;
    let target_refs = privacy_target_refs(&user_key, &steam_ids);
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

    for &spec in REDACTED_TEXT_USER_COLUMNS {
        if !relations.contains(spec.relation) || uses_subject_projected_export(spec) {
            continue;
        }
        tbl.insert(
            spec.count_key(),
            Value::Array(
                select_rows_lookup(pool, spec.table_spec(), LookupValue::Text(&user_key)).await?,
            ),
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

    for (key, value) in
        select_scrim_replacement_rows_for_user(pool, &relations, user_id, &user_key).await?
    {
        tbl.insert(key, value);
    }
    if relations.contains(SCRIM_AUDIT_ACTOR_PSEUDONYMS_REL) {
        tbl.insert(
            "scrim_audit_actor_pseudonyms.actor_ref".to_string(),
            select_scrim_audit_actor_mapping_for_user(pool, &user_key).await?,
        );
    }
    for (key, value) in select_json_user_rows_for_user(pool, &relations, &target_refs).await? {
        tbl.insert(key, value);
    }
    for (key, value) in
        select_text_user_ref_rows_for_user(pool, &relations, &user_key, &steam_ids).await?
    {
        tbl.insert(key, value);
    }
    for (key, value) in
        select_scrim_projected_actor_rows_for_user(pool, &relations, &user_key).await?
    {
        tbl.insert(key, value);
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
        kv_out.insert(
            "onboarding_dm".into(),
            serde_json::json!({
                "bridge": kv_value(pool, "onboarding_bridge_dm", &uid_key).await?,
                "tour": kv_value(pool, "onboarding_tour", &uid_key).await?,
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
        out.extend(
            REDACTED_TEXT_USER_COLUMNS
                .iter()
                .map(|spec| (spec.relation.to_string(), spec.col.to_string())),
        );
        out.extend(
            TEXT_USER_REF_COLUMNS
                .iter()
                .map(|spec| (spec.relation.to_string(), spec.col.to_string())),
        );
        out.extend(
            JSON_USER_COLUMNS
                .iter()
                .map(|spec| (spec.relation.to_string(), spec.col.to_string())),
        );
        out.insert((
            "scrim.replacement_needs".to_string(),
            "participant_id".to_string(),
        ));
        out.insert((
            "scrim.replacement_candidates".to_string(),
            "participant_id".to_string(),
        ));
        out.insert((
            "scrim.replacement_candidates".to_string(),
            "discord_user_id".to_string(),
        ));
        out.insert((
            "scrim.replacement_requests".to_string(),
            "participant_id".to_string(),
        ));
        out.insert((
            "scrim.replacement_requests".to_string(),
            "discord_user_id".to_string(),
        ));
        out.insert((
            SCRIM_AUDIT_ACTOR_PSEUDONYMS_REL.to_string(),
            "actor_ref".to_string(),
        ));
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

    #[test]
    fn steam64_account_id_ist_zusaetzlicher_privacy_target_ref() {
        let steam_ids = vec![SteamId {
            text: "76561198000000420".to_string(),
            numeric: Some(76_561_198_000_000_420),
            account_id: steam_account_id_from_steam64(76_561_198_000_000_420),
        }];

        assert_eq!(
            privacy_target_refs("42", &steam_ids),
            vec![
                "42".to_string(),
                "76561198000000420".to_string(),
                "39734692".to_string(),
            ]
        );
        assert!(steam_account_id_from_steam64(42).is_none());
        assert!(steam_account_id_from_steam64(80_856_166_756_561_024).is_none());
    }

    #[test]
    fn result_last_error_scrubbt_nur_exakte_target_refs() {
        let refs = vec![
            "42".to_string(),
            "76561198000000420".to_string(),
            "39734692".to_string(),
        ];

        assert_eq!(
            scrub_text_target_refs(
                "discord 42 steam 76561198000000420 account 39734692 foreign 99 keep 4242 user42",
                &refs,
            ),
            "discord redacted steam redacted account redacted foreign 99 keep 4242 user42"
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

#[cfg(test)]
mod runtime_gate_privacy_tests {
    use super::*;
    use dl_central_db::testing::{set_scrim_runtime_turniere, test_pool};

    #[tokio::test]
    async fn privacy_loeschung_funktioniert_im_turniere_runtime(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = test_pool().await?;
        sqlx::query(
            "INSERT INTO scrim.participants(
                 id, discord_id, display_name, rank_source, status, source,
                 created_at, updated_at
             )
             VALUES(1, 42, 'Privacy', 'manual', 'new', 'test', now(), now())",
        )
        .execute(db.pool())
        .await?;
        set_scrim_runtime_turniere(db.pool()).await?;

        delete_user_data(db.pool(), 42, "test".to_string(), 1_000).await?;

        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM scrim.participants WHERE discord_id = 42")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(remaining, 0);
        Ok(())
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use dl_activity::journey::compact_raw_events;
    use dl_central_db::testing::{test_pool, TestDb};
    use std::collections::BTreeSet;

    type SteamAccountRefRow = (String, Option<String>, Option<String>, Value, Option<Value>);

    async fn mk_db() -> TestDb {
        test_pool().await.expect("test_pool")
    }

    fn scrim_json_contains(value: &Value, target_ref: &str) -> bool {
        match value {
            Value::Object(map) => map
                .iter()
                .any(|(key, child)| key == target_ref || scrim_json_contains(child, target_ref)),
            Value::Array(items) => items
                .iter()
                .any(|child| scrim_json_contains(child, target_ref)),
            Value::String(value) => value == target_ref,
            Value::Number(value) => value.to_string() == target_ref,
            _ => false,
        }
    }

    fn database_error_code<T>(result: &Result<T, sqlx::Error>) -> Option<String> {
        result
            .as_ref()
            .err()
            .and_then(sqlx::Error::as_database_error)
            .and_then(|err| err.code())
            .map(|code| code.to_string())
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
    async fn scrim_foundation_privacy_redigiert_actoren_und_loescht_replacement_ketten() {
        let db = mk_db().await;
        let pool = db.pool();
        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42), (99)")
            .execute(pool)
            .await
            .expect("core user");
        sqlx::query(
            "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
             VALUES
                (42, '7656119800000042', 7656119800000042),
                (99, '7656119800000099', 7656119800000099)",
        )
        .execute(pool)
        .await
        .expect("steam links");
        sqlx::query(
            "INSERT INTO scrim.participants(
                 id, discord_id, display_name, rank_source, status, source, created_at, updated_at
              ) VALUES
                (424200, 42, 'DeleteMe', 'manual', 'active', 'test', now(), now()),
                (424299, 99, 'OtherUser', 'manual', 'active', 'test', now(), now())",
        )
        .execute(pool)
        .await
        .expect("scrim participant");
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES (424201, 'A', now()), (424202, 'B', now())")
            .execute(pool)
            .await
            .expect("scrim teams");
        sqlx::query(
            "INSERT INTO scrim.matches(id, team_a_id, team_b_id, status, created_at)
             VALUES (424203, 424201, 424202, 'scheduled', now())",
        )
        .execute(pool)
        .await
        .expect("scrim match");

        let audit_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.audit_events(event_type, entity_type, actor_type, actor_pseudonym, actor_source)
             VALUES ('privacy_test', 'match', 'user', scrim.audit_actor_pseudonym('user', '42'), 'user')
              RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("audit event");
        sqlx::query(
            "SELECT applied, current_epoch
               FROM scrim.transition_runtime_control(
                    0, 'draining', 'turniere', '42', 'DeleteMe', 'privacy:test', 'privacy:test', '{}'::jsonb
               )",
        )
        .execute(pool)
        .await
        .expect("runtime actor");
        let announcement_draft_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.announcement_drafts(
                 scope, title, body, payload, payload_hash, idempotency_key,
                 created_by_user_id, created_by_display_name,
                 approved_by_user_id, approved_by_display_name
             ) VALUES (
                 'global',
                 'T OtherUser',
                 'B 99',
                 '{\"other_user\":\"99\",\"display\":\"OtherUser\"}'::jsonb,
                 scrim.announcement_effect_hash(
                     'global',
                     NULL,
                     'T OtherUser',
                     'B 99',
                     '{\"other_user\":\"99\",\"display\":\"OtherUser\"}'::jsonb
                 ),
                  'privacy:announcement',
                 '42',
                 'DeleteMe',
                 '99',
                 'OtherUser'
             )
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("announcement draft");
        let approved_announcement_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.announcement_drafts(
                 scope, title, body, payload, payload_hash, idempotency_key,
                 created_by_user_id, created_by_display_name,
                 approved_by_user_id, approved_by_display_name
             ) VALUES (
                 'global',
                 'Other created',
                 'Approved by 42',
                 '{}'::jsonb,
                 scrim.announcement_effect_hash('global', NULL, 'Other created', 'Approved by 42', '{}'::jsonb),
                  'privacy:announcement_approved_by',
                 '99',
                 'OtherUser',
                 '42',
                 'DeleteMe'
             )
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("approved announcement draft");
        sqlx::query(
            "INSERT INTO scrim.announcement_approvals(
                 draft_id, decision, decided_by_user_id, decided_by_display_name, decision_data
             ) VALUES ($1, 'approved', '42', 'DeleteMe', '{\"other_user\":\"99\"}'::jsonb)",
        )
        .bind(announcement_draft_id)
        .execute(pool)
        .await
        .expect("announcement approval");
        sqlx::query(
            "INSERT INTO scrim.status_publication_approvals(
                 target_kind, target_id, status_kind, payload, payload_hash,
                 decision, decided_by_user_id, decided_by_display_name, decision_data, decided_at
             ) VALUES (
                 'match',
                 '99',
                 'status',
                 '{\"other_user\":\"99\",\"display\":\"OtherUser\"}'::jsonb,
                 scrim.status_publication_effect_hash(
                     'match',
                     '99',
                     'status',
                     '{\"other_user\":\"99\",\"display\":\"OtherUser\"}'::jsonb
                 ),
                 'approved',
                 '42',
                 'DeleteMe',
                 '{\"approved_for\":\"99\"}'::jsonb,
                 now()
             )",
        )
        .execute(pool)
        .await
        .expect("status publication approval");
        let need_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_needs(
                 participant_id, reason, created_by_user_id, created_by_display_name
             ) VALUES (424200, 'privacy', '42', 'DeleteMe')
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("replacement need");
        let candidate_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_candidates(need_id, participant_id)
             VALUES ($1, 424200)
             RETURNING id",
        )
        .bind(need_id)
        .fetch_one(pool)
        .await
        .expect("replacement candidate");
        let foreign_candidate_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_candidates(need_id, participant_id, candidate_data, score_data)
             VALUES (
                 $1,
                 424299,
                  '{\"foreign_marker\":\"FOREIGN_CANDIDATE_MARKER\",\"note\":\"RAW_REPLACEMENT_NOTE\"}'::jsonb,
                  '{\"foreign_marker\":\"FOREIGN_SCORE_MARKER\",\"note\":\"RAW_REPLACEMENT_NOTE\"}'::jsonb
             )
             RETURNING id",
        )
        .bind(need_id)
        .fetch_one(pool)
        .await
        .expect("foreign replacement candidate");
        let _direct_discord_candidate_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_candidates(need_id, discord_user_id)
             VALUES ($1, 42)
             RETURNING id",
        )
        .bind(need_id)
        .fetch_one(pool)
        .await
        .expect("direct discord replacement candidate");
        sqlx::query(
            "INSERT INTO scrim.replacement_requests(
                  need_id, candidate_id, requested_by_user_id, requested_by_display_name
              ) VALUES ($1, $2, '42', 'DeleteMe')",
        )
        .bind(need_id)
        .bind(candidate_id)
        .execute(pool)
        .await
        .expect("replacement request");
        sqlx::query(
            "INSERT INTO scrim.replacement_requests(
                  need_id, candidate_id, requested_by_user_id, requested_by_display_name, request_payload
              ) VALUES (
                  $1,
                  $2,
                  '99',
                  'OtherUser',
                  '{\"foreign_marker\":\"FOREIGN_REQUEST_MARKER\",\"note\":\"RAW_REPLACEMENT_NOTE\"}'::jsonb
              )",
        )
        .bind(need_id)
        .bind(foreign_candidate_id)
        .execute(pool)
        .await
        .expect("foreign replacement request");
        sqlx::query(
            "INSERT INTO scrim.replacement_requests(
                  need_id, discord_user_id, requested_by_user_id, requested_by_display_name
              ) VALUES ($1, 42, '42', 'DeleteMe')",
        )
        .bind(need_id)
        .execute(pool)
        .await
        .expect("direct discord replacement request");
        let result_ref_id: i64 = sqlx::query_scalar(
             "INSERT INTO scrim.match_result_refs(
                  match_id, steam_match_id, source_user_id, source_display_name,
                  selected_by_user_id, clarification_payload, last_error, raw_result_json, normalized_result_json,
                  fetch_status, winner_team_id, validation_status
                ) VALUES (
                  424203,
                 424204,
                  '42',
                  'DeleteMe',
                  '99',
                  '{\"target_user_id\":\"42\",\"other_user\":\"99\"}'::jsonb,
                  'source actor 42 should not leak; foreign 99 may remain',
                  '{\"display\":\"OtherUser\"}'::jsonb,
                  '{\"winner\":\"99\"}'::jsonb,
                 'fetched',
                 424201,
                 'valid'
               )
               RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("result ref");
        let selected_result_ref_id: i64 = sqlx::query_scalar(
             "INSERT INTO scrim.match_result_refs(
                  match_id, steam_match_id, source_user_id, source_display_name,
                  selected_by_user_id, clarification_payload,
                  last_error, fetch_status, winner_team_id, normalized_result_json, validation_status
                ) VALUES (
                  424203,
                  424205,
                 '99',
                  'OtherUser',
                  '42',
                  '{\"target_user_id\":\"42\",\"other_user\":\"99\"}'::jsonb,
                  'selected actor 42 should not leak; foreign 99 may remain',
                  'fetched',
                  424202,
                 '{\"winner\":\"42\"}'::jsonb,
                 'valid'
               )
               RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("selected result ref");
        sqlx::query(
            "INSERT INTO scrim.match_result_selections(
                 match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
             ) VALUES (424203, $1, '42', 'DeleteMe', 'privacy_initial_selection')",
        )
        .bind(result_ref_id)
        .execute(pool)
        .await
        .expect("canonical result selection");
        sqlx::query(
            "UPDATE scrim.match_result_selections
                SET result_ref_id = $1,
                    selected_by_user_id = '42',
                    selected_by_display_name = 'DeleteMe',
                    selection_reason = 'privacy_reselection',
                    selected_at = selected_at + interval '1 second'
              WHERE match_id = 424203",
        )
        .bind(selected_result_ref_id)
        .execute(pool)
        .await
        .expect("canonical result reselection");
        sqlx::query(
            "INSERT INTO scrim.match_result_clarifications(
                 result_ref_id, clarification_kind, requested_by_user_id, requested_by_display_name,
                 request_payload, response_payload
              ) VALUES (
                 $1,
                 'manual_review',
                 '42',
                 'DeleteMe',
                  '{\"target_user_id\":\"42\",\"other_user\":\"99\"}'::jsonb,
                  '{\"steam_id\":\"7656119800000042\",\"display\":\"OtherUser\"}'::jsonb
              )",
        )
        .bind(result_ref_id)
        .execute(pool)
        .await
        .expect("result clarification");

        let json_hash = vec![6_u8; 32];
        sqlx::query(
            r#"INSERT INTO scrim.command_receipts(
                 command_scope, idempotency_key, payload_hash, payload, result_payload,
                 state, completed_at
             ) VALUES (
                  'privacy_json',
                  'command:json',
                 $1,
                 '{"target_user_id":"42","members":{"42":{"display_name":"TargetKey","note":"TARGET_KEY_MARKER"},"990099":{"display_name":"ForeignKey","note":"FOREIGN_KEY_MARKER"}},"participants":[{"discord_id":"42","display_name":"DeleteMe","note":"TARGET_QUEUE_MARKER"},{"discord_id":"990099","display_name":"OtherUser","note":"FOREIGN_QUEUE_MARKER"}]}'::jsonb,
                 '{"reviewer_user_id":"42","other_user_id":"990099","foreign_marker":"FOREIGN_RESULT_MARKER"}'::jsonb,
                 'completed',
                 now()
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("command receipt json payload");
        sqlx::query(
            r#"INSERT INTO scrim.inbox_events(event_source, source_event_id, idempotency_key, payload_hash, payload)
             VALUES (
                 'privacy',
                  'inbox:json',
                  'inbox:json',
                 $1,
                 '{"event_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_INBOX_MARKER"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("inbox json payload");
        sqlx::query(
            r#"INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload)
             VALUES (
                  'privacy_json',
                  'outbox:json',
                 $1,
                 '{"recipient_user_id":"42","cc_user_id":"990099","foreign_marker":"FOREIGN_OUTBOX_MARKER"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("outbox json payload");
        sqlx::query(
            r#"INSERT INTO scrim.effect_receipts(
                 remote_system, remote_message_id, payload_hash, receipt_payload, status
             ) VALUES (
                 'privacy',
                  'receipt:json',
                 $1,
                 '{"observer_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_RECEIPT_MARKER"}'::jsonb,
                 'observed'
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("effect receipt json payload");
        sqlx::query(
            r#"INSERT INTO scrim.command_receipts(
                 command_scope, idempotency_key, payload_hash, payload, state, completed_at
             ) VALUES (
                  'privacy_steam',
                  'command:steam',
                 $1,
                 '{"players":[{"id":"7656119800000042","display_name":"TargetSteam","role":"carry"},{"id":"7656119800000099","display_name":"ForeignSteam","shadowHandle":"unknown-99"}],"by_steam_id":{"7656119800000042":{"display_name":"TargetSteamKey","note":"TARGET_STEAM_KEY_MARKER"},"7656119800000099":{"display_name":"ForeignSteamKey","note":"FOREIGN_STEAM_KEY_MARKER"}},"metadata":{"unknown_member_ref":"7656119800000099","label":"ForeignSteam"}}'::jsonb,
                 'completed',
                 now()
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("command receipt steam-only payload");
        sqlx::query(
            r#"INSERT INTO scrim.inbox_events(event_source, source_event_id, idempotency_key, payload_hash, payload)
             VALUES (
                 'privacy',
                  'inbox:steam',
                  'inbox:steam',
                 $1,
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("inbox steam-only payload");
        sqlx::query(
            r#"INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload)
             VALUES (
                  'privacy_steam',
                  'outbox:steam',
                 $1,
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("outbox steam-only payload");
        sqlx::query(
            r#"INSERT INTO scrim.effect_receipts(
                 remote_system, remote_message_id, payload_hash, receipt_payload, status
             ) VALUES (
                 'privacy',
                  'receipt:steam',
                 $1,
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb,
                 'observed'
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("effect receipt steam-only payload");
        sqlx::query(
            r#"INSERT INTO scrim.match_lineup_snapshots(
                 match_id, snapshot_kind, lineup_payload, source, created_by_user_id, created_by_display_name
             ) VALUES (
                 424203,
                 'planned',
                 '{"players":[{"id":"42","display_name":"Target","role":"carry"},{"id":"43","display_name":"Foreign","mysteryProfile":"opaque-43","role":"FOREIGN_LINEUP_MARKER"}]}'::jsonb,
                 'turniere',
                 '99',
                 'OtherUser'
             )"#,
        )
        .execute(pool)
        .await
        .expect("lineup json payload");
        sqlx::query(
            r#"INSERT INTO steam.v1_workflows(
                  workflow_key, workflow_type, aggregate_kind, aggregate_id, subject_kind, subject_id, payload
              ) VALUES (
                  'privacy:workflow',
                  'friend_request',
                  'match',
                  '424203',
                  'discord_user',
                  '42',
                  '{"target_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_WORKFLOW_MARKER"}'::jsonb
              )"#,
        )
        .execute(pool)
        .await
        .expect("steam workflow privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_operations(
                 operation_id, operation_type, aggregate_kind, aggregate_id, subject_kind, subject_id,
                 idempotency_key, payload_hash, payload, result_payload
             ) VALUES (
                  'op:privacy_json',
                  'friend_request',
                  'match',
                  '424203',
                  'discord_user',
                  '42',
                 'operation:json',
                 $1,
                 '{"target_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_OPERATION_MARKER"}'::jsonb,
                 '{"target_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_OPERATION_RESULT_MARKER"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("steam operation privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_operation_results(operation_id, operation_generation, result_payload, status)
             VALUES (
                 'op:privacy_json',
                 0,
                 '{"target_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_STEAM_RESULT_MARKER"}'::jsonb,
                 'succeeded'
             )"#,
        )
        .execute(pool)
        .await
        .expect("steam operation result privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_operation_events(operation_id, operation_generation, event_type, payload)
             VALUES (
                 'op:privacy_json',
                 0,
                 'privacy_event',
                 '{"target_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_STEAM_EVENT_MARKER"}'::jsonb
             )"#,
        )
        .execute(pool)
        .await
        .expect("steam operation event privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_deliveries(
                 operation_id, operation_generation, delivery_type, idempotency_key,
                 remote_system, payload_hash, payload
             ) VALUES (
                 'op:privacy_json',
                 0,
                 'privacy_delivery',
                 'delivery:json',
                 'steam',
                 $1,
                 '{"target_user_id":"42","foreign_user_id":"990099","foreign_marker":"FOREIGN_STEAM_DELIVERY_MARKER"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("steam delivery privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_workflows(
                  workflow_key, workflow_type, aggregate_kind, aggregate_id, subject_kind, subject_id, payload
              ) VALUES (
                  'privacy:workflow_steam',
                  'friend_request',
                  'player',
                  '7656119800000042',
                  'steam_user',
                 '7656119800000099',
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb
             )"#,
        )
        .execute(pool)
        .await
        .expect("steam workflow steam-only privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_operations(
                 operation_id, operation_type, aggregate_kind, aggregate_id, subject_kind, subject_id,
                 idempotency_key, payload_hash, payload, result_payload
             ) VALUES (
                  'op:privacy_steam',
                  'friend_request',
                  'player',
                  '7656119800000042',
                 'steam_user',
                 '7656119800000042',
                 'operation:steam',
                 $1,
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb,
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("steam operation steam-only privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_operation_results(operation_id, operation_generation, result_payload, status)
             VALUES (
                 'op:privacy_steam',
                 0,
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb,
                 'succeeded'
             )"#,
        )
        .execute(pool)
        .await
        .expect("steam operation result steam-only privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_operation_events(operation_id, operation_generation, event_type, payload)
             VALUES (
                 'op:privacy_steam',
                 0,
                 'privacy_event',
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb
             )"#,
        )
        .execute(pool)
        .await
        .expect("steam operation event steam-only privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_deliveries(
                 operation_id, operation_generation, delivery_type, idempotency_key,
                 remote_system, payload_hash, payload
             ) VALUES (
                 'op:privacy_steam',
                 0,
                 'privacy_delivery',
                 'delivery:steam',
                 'steam',
                 $1,
                 '{"steam_id":"7656119800000042","foreign_steam_id":"7656119800000099","foreign_label":"ForeignSteam"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("steam delivery steam-only privacy row");
        sqlx::query(
            r#"INSERT INTO steam.v1_workflows(
                 workflow_key, workflow_type, aggregate_kind, aggregate_id, payload
             ) VALUES (
                 'privacy:workflow_kind_collision',
                 'friend_request',
                 'match',
                 '42',
                 '{"domain":"match-only"}'::jsonb
             )"#,
        )
        .execute(pool)
        .await
        .expect("steam workflow aggregate kind collision row");
        sqlx::query(
            r#"INSERT INTO steam.v1_operations(
                 operation_id, operation_type, aggregate_kind, aggregate_id,
                 idempotency_key, payload_hash, payload
             ) VALUES (
                 'op:privacy_kind_collision',
                 'friend_request',
                 'match',
                 '42',
                 'operation:kind_collision',
                 $1,
                 '{"domain":"match-only"}'::jsonb
             )"#,
        )
        .bind(&json_hash)
        .execute(pool)
        .await
        .expect("steam operation aggregate kind collision row");
        let ai_discord_run_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
             VALUES ('lagebild', 'discord_user', '42', 'privacy:ai_discord_target', $1)
             RETURNING id"#,
        )
        .bind(&json_hash)
        .fetch_one(pool)
        .await
        .expect("ai run discord target row");
        let ai_steam_run_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
             VALUES ('lagebild', 'steam_user', '7656119800000042', 'privacy:ai_steam_target', $1)
             RETURNING id"#,
        )
        .bind(&json_hash)
        .fetch_one(pool)
        .await
        .expect("ai run steam target row");
        let ai_team_collision_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
             VALUES ('lagebild', 'team', '42', 'privacy:ai_team_collision', $1)
             RETURNING id"#,
        )
        .bind(&json_hash)
        .fetch_one(pool)
        .await
        .expect("ai run team kind collision row");
        let ai_match_collision_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
             VALUES ('lagebild', 'match', '42', 'privacy:ai_match_collision', $1)
             RETURNING id"#,
        )
        .bind(&json_hash)
        .fetch_one(pool)
        .await
        .expect("ai run match kind collision row");
        let ai_foreign_run_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
             VALUES ('lagebild', 'discord_user', '99', 'privacy:ai_foreign_user', $1)
             RETURNING id"#,
        )
        .bind(&json_hash)
        .fetch_one(pool)
        .await
        .expect("ai run foreign user row");

        let export = export_user_data(pool, 42, 1_000)
            .await
            .expect("privacy export");
        let exported_needs = export["tables"]["scrim_replacement_needs.participant_id"]
            .as_array()
            .expect("replacement needs export");
        let exported_candidates = export["tables"]["scrim_replacement_candidates.user_identity"]
            .as_array()
            .expect("replacement candidates export");
        let exported_requests = export["tables"]["scrim_replacement_requests.user_identity"]
            .as_array()
            .expect("replacement requests export");
        let exported_tables = export["tables"].as_object().expect("export tables object");
        for (hash_only_payload_key, column) in [
            (
                "scrim_replacement_candidates.candidate_data",
                "candidate_data",
            ),
            ("scrim_replacement_candidates.score_data", "score_data"),
            (
                "scrim_replacement_requests.request_payload",
                "request_payload",
            ),
            (
                "scrim_match_result_refs.clarification_payload",
                "clarification_payload",
            ),
            (
                "scrim_match_result_clarifications.request_payload",
                "request_payload",
            ),
            (
                "scrim_match_result_clarifications.response_payload",
                "response_payload",
            ),
        ] {
            let rows = exported_tables[hash_only_payload_key]
                .as_array()
                .unwrap_or_else(|| {
                    panic!("hash-only payload key missing: {hash_only_payload_key}")
                });
            for row in rows {
                let row = row.as_object().expect("hash-only row object");
                assert_eq!(
                    row.len(),
                    2,
                    "hash-only payload export exposed extra fields: {hash_only_payload_key}: {row:?}"
                );
                assert!(row["id"].is_number());
                assert!(row[column]["md5"].is_string());
            }
        }
        let full_export = serde_json::to_string(&export).expect("serialize full export");
        assert!(
            !full_export.contains("RAW_REPLACEMENT_NOTE"),
            "raw replacement payload note leaked in DSAR export: {full_export}"
        );
        assert_eq!(exported_needs.len(), 1);
        assert_eq!(exported_candidates.len(), 3);
        assert_eq!(exported_requests.len(), 3);

        let audit_mapping_export = export["tables"]["scrim_audit_actor_pseudonyms.actor_ref"]
            .as_array()
            .expect("audit actor mapping export");
        assert_eq!(audit_mapping_export.len(), 1);
        assert_eq!(
            audit_mapping_export[0]["actor_ref"],
            serde_json::json!("42")
        );
        let command_payload_export = export["tables"]["scrim_command_receipts.payload"]
            .as_array()
            .expect("command payload export");
        let command_result_export = export["tables"]["scrim_command_receipts.result_payload"]
            .as_array()
            .expect("command result payload export");
        let inbox_payload_export = export["tables"]["scrim_inbox_events.payload"]
            .as_array()
            .expect("inbox payload export");
        let outbox_payload_export = export["tables"]["scrim_outbox_effects.payload"]
            .as_array()
            .expect("outbox payload export");
        let effect_receipt_export = export["tables"]["scrim_effect_receipts.receipt_payload"]
            .as_array()
            .expect("effect receipt payload export");
        let lineup_payload_export = export["tables"]["scrim_match_lineup_snapshots.lineup_payload"]
            .as_array()
            .expect("lineup payload export");
        let ai_run_subject_export = export["tables"]["scrim_ai_runs.subject_id"]
            .as_array()
            .expect("ai run subject export");
        let workflow_ref_export = export["tables"]["steam_v1_workflows.aggregate_id"]
            .as_array()
            .expect("steam workflow aggregate export");
        let workflow_subject_export = export["tables"]["steam_v1_workflows.subject_id"]
            .as_array()
            .expect("steam workflow subject export");
        let operation_subject_export = export["tables"]["steam_v1_operations.subject_id"]
            .as_array()
            .expect("steam operation subject export");
        assert!(!command_payload_export.is_empty());
        assert!(!command_result_export.is_empty());
        assert!(!inbox_payload_export.is_empty());
        assert!(!outbox_payload_export.is_empty());
        assert!(!effect_receipt_export.is_empty());
        assert!(!lineup_payload_export.is_empty());
        assert_eq!(ai_run_subject_export.len(), 2);
        assert!(ai_run_subject_export.iter().any(|row| {
            row["idempotency_key"] == serde_json::json!("privacy:ai_discord_target")
                && row["subject_kind"] == serde_json::json!("discord_user")
                && row["subject_id"] == serde_json::json!("42")
        }));
        assert!(ai_run_subject_export.iter().any(|row| {
            row["idempotency_key"] == serde_json::json!("privacy:ai_steam_target")
                && row["subject_kind"] == serde_json::json!("steam_user")
                && row["subject_id"] == serde_json::json!("7656119800000042")
        }));
        assert!(!ai_run_subject_export.iter().any(|row| {
            matches!(
                row["idempotency_key"].as_str(),
                Some(
                    "privacy:ai_team_collision"
                        | "privacy:ai_match_collision"
                        | "privacy:ai_foreign_user"
                )
            )
        }));
        assert!(!workflow_ref_export.is_empty());
        assert!(!workflow_subject_export.is_empty());
        assert!(operation_subject_export.len() >= 2);
        assert!(!workflow_ref_export.iter().any(|row| {
            row["workflow_key"] == serde_json::json!("privacy:workflow_kind_collision")
        }));
        let projected_payloads = serde_json::to_string(&serde_json::json!({
            "command": command_payload_export,
            "command_result": command_result_export,
            "inbox": inbox_payload_export,
            "outbox": outbox_payload_export,
            "receipt": effect_receipt_export,
            "lineup": lineup_payload_export,
            "ai_runs": ai_run_subject_export,
            "workflow": workflow_ref_export,
            "workflow_subject": workflow_subject_export,
            "operation": operation_subject_export,
        }))
        .expect("serialize projected json exports");
        assert!(projected_payloads.contains("42"));
        assert!(projected_payloads.contains("Target"));
        assert!(projected_payloads.contains("TARGET_KEY_MARKER"));
        assert!(projected_payloads.contains("7656119800000042"));
        assert!(projected_payloads.contains("TARGET_STEAM_KEY_MARKER"));
        for marker in [
            "990099",
            "OtherUser",
            "ForeignKey",
            "ForeignSteam",
            "ForeignSteamKey",
            "7656119800000099",
            "unknown-99",
            "FOREIGN_KEY_MARKER",
            "FOREIGN_STEAM_KEY_MARKER",
            "FOREIGN_QUEUE_MARKER",
            "FOREIGN_RESULT_MARKER",
            "FOREIGN_LINEUP_MARKER",
            "FOREIGN_WORKFLOW_MARKER",
            "FOREIGN_OPERATION_MARKER",
        ] {
            assert!(
                !projected_payloads.contains(marker),
                "foreign JSON/Steam marker leaked: {marker}: {projected_payloads}"
            );
        }
        assert!(
            !projected_payloads.contains("\\\"43\\\""),
            "foreign nested id leaked: {projected_payloads}"
        );
        assert!(exported_candidates
            .iter()
            .any(|row| row["discord_user_id"] == serde_json::json!(42)));
        assert!(exported_candidates
            .iter()
            .any(|row| row["participant_id"] == serde_json::json!("redacted")));
        assert!(exported_requests
            .iter()
            .any(|row| row["discord_user_id"] == serde_json::json!(42)));
        assert!(exported_requests
            .iter()
            .any(|row| row["requested_by_user_id"] == serde_json::json!("redacted")));

        let foreign_candidate_export = exported_candidates
            .iter()
            .find(|row| row["participant_id"] == serde_json::json!("redacted"))
            .expect("foreign replacement candidate export");
        assert!(foreign_candidate_export["candidate_data"]["md5"].is_string());
        assert!(foreign_candidate_export["score_data"]["md5"].is_string());
        let foreign_request_export = exported_requests
            .iter()
            .find(|row| row["requested_by_user_id"] == serde_json::json!("redacted"))
            .expect("foreign replacement request export");
        assert!(foreign_request_export["request_payload"]["md5"].is_string());
        let replacement_export = serde_json::to_string(&serde_json::json!({
            "candidates": exported_candidates,
            "requests": exported_requests,
        }))
        .expect("serialize replacement export rows");
        for marker in [
            "FOREIGN_CANDIDATE_MARKER",
            "FOREIGN_SCORE_MARKER",
            "FOREIGN_REQUEST_MARKER",
        ] {
            assert!(
                !replacement_export.contains(marker),
                "foreign replacement marker leaked: {marker}: {replacement_export}"
            );
        }

        let created_announcements = export["tables"]
            ["scrim_announcement_drafts.created_by_user_id.redacted"]
            .as_array()
            .expect("created announcement export");
        let created_announcement = created_announcements
            .iter()
            .find(|row| row["id"] == serde_json::json!(announcement_draft_id))
            .expect("created announcement row");
        assert_eq!(
            created_announcement["created_by_user_id"],
            serde_json::json!("42")
        );
        assert_eq!(
            created_announcement["approved_by_user_id"],
            serde_json::json!("redacted")
        );
        assert_eq!(created_announcement["title"], serde_json::json!("redacted"));
        assert!(created_announcement["payload"]["md5"].is_string());

        let approved_announcements = export["tables"]
            ["scrim_announcement_drafts.approved_by_user_id.redacted"]
            .as_array()
            .expect("approved announcement export");
        let approved_announcement = approved_announcements
            .iter()
            .find(|row| row["id"] == serde_json::json!(approved_announcement_id))
            .expect("approved announcement row");
        assert_eq!(
            approved_announcement["created_by_user_id"],
            serde_json::json!("redacted")
        );
        assert_eq!(
            approved_announcement["approved_by_user_id"],
            serde_json::json!("42")
        );

        let announcement_approvals = export["tables"]
            ["scrim_announcement_approvals.decided_by_user_id.redacted"]
            .as_array()
            .expect("announcement approval export");
        assert_eq!(announcement_approvals.len(), 1);
        assert_eq!(
            announcement_approvals[0]["decided_by_user_id"],
            serde_json::json!("42")
        );
        assert!(announcement_approvals[0]["decision_data"]["md5"].is_string());

        let status_approvals = export["tables"]
            ["scrim_status_publication_approvals.decided_by_user_id.redacted"]
            .as_array()
            .expect("status publication export");
        assert_eq!(status_approvals.len(), 1);
        assert_eq!(
            status_approvals[0]["target_id"],
            serde_json::json!("redacted")
        );
        assert!(status_approvals[0]["payload"]["md5"].is_string());
        assert!(status_approvals[0]["decision_data"]["md5"].is_string());

        let source_refs = export["tables"]["scrim_match_result_refs.source_user_id.redacted"]
            .as_array()
            .expect("source result refs export");
        let source_ref = source_refs
            .iter()
            .find(|row| row["id"] == serde_json::json!(result_ref_id))
            .expect("source result ref row");
        assert_eq!(source_ref["source_user_id"], serde_json::json!("42"));
        assert_eq!(
            source_ref["selected_by_user_id"],
            serde_json::json!("redacted")
        );
        assert!(source_ref["clarification_payload"]["md5"].is_string());
        assert!(source_ref["last_error"]["md5"].is_string());
        assert!(source_ref["raw_result_json"]["md5"].is_string());
        assert!(source_ref["normalized_result_json"]["md5"].is_string());

        let selected_refs = export["tables"]
            ["scrim_match_result_refs.selected_by_user_id.redacted"]
            .as_array()
            .expect("selected result refs export");
        let selected_ref = selected_refs
            .iter()
            .find(|row| row["id"] == serde_json::json!(selected_result_ref_id))
            .expect("selected result ref row");
        assert_eq!(selected_ref["selected_by_user_id"], serde_json::json!("42"));
        assert_eq!(
            selected_ref["source_user_id"],
            serde_json::json!("redacted")
        );
        assert_eq!(
            selected_ref["source_display_name"],
            serde_json::json!("redacted")
        );
        assert!(selected_ref["last_error"]["md5"].is_string());

        let canonical_selections = export["tables"]
            ["scrim_match_result_selections.selected_by_user_id.redacted"]
            .as_array()
            .expect("canonical selection export");
        assert_eq!(canonical_selections.len(), 1);
        assert_eq!(
            canonical_selections[0]["selected_by_user_id"],
            serde_json::json!("42")
        );
        assert_eq!(
            canonical_selections[0]["result_ref_id"],
            serde_json::json!(selected_result_ref_id)
        );
        assert_eq!(
            canonical_selections[0]["selection_reason"],
            serde_json::json!("privacy_reselection")
        );

        let selection_events = export["tables"]
            ["scrim_match_result_selection_events.actor_pseudonym"]
            .as_array()
            .expect("selection history export");
        assert_eq!(selection_events.len(), 2);
        assert_eq!(
            selection_events[0]["event_type"],
            serde_json::json!("selected")
        );
        assert_eq!(
            selection_events[0]["old_result_ref_id"],
            serde_json::Value::Null
        );
        assert_eq!(
            selection_events[0]["new_result_ref_id"],
            serde_json::json!(result_ref_id)
        );
        assert_eq!(
            selection_events[1]["event_type"],
            serde_json::json!("reselected")
        );
        assert_eq!(
            selection_events[1]["old_result_ref_id"],
            serde_json::json!(result_ref_id)
        );
        assert_eq!(
            selection_events[1]["new_result_ref_id"],
            serde_json::json!(selected_result_ref_id)
        );
        assert!(selection_events[1]["before_data"]["md5"].is_string());
        assert!(selection_events[1]["after_data"]["md5"].is_string());
        let selection_events_serialized =
            serde_json::to_string(selection_events).expect("serialize selection history export");
        assert!(!selection_events_serialized.contains("DeleteMe"));

        let clarifications = export["tables"]
            ["scrim_match_result_clarifications.requested_by_user_id.redacted"]
            .as_array()
            .expect("result clarification export");
        assert_eq!(clarifications.len(), 1);
        assert_eq!(
            clarifications[0]["requested_by_user_id"],
            serde_json::json!("42")
        );
        assert!(clarifications[0]["request_payload"]["md5"].is_string());
        assert!(clarifications[0]["response_payload"]["md5"].is_string());

        for rows in [
            created_announcements,
            approved_announcements,
            announcement_approvals,
            status_approvals,
            source_refs,
            selected_refs,
            canonical_selections,
            selection_events,
            clarifications,
        ] {
            let serialized = serde_json::to_string(rows).expect("serialize projected export rows");
            assert!(
                !serialized.contains("OtherUser"),
                "foreign display name leaked: {serialized}"
            );
        }

        let audit_pseudonym_before_delete: String =
            sqlx::query_scalar("SELECT actor_pseudonym FROM scrim.audit_events WHERE id = $1")
                .bind(audit_id)
                .fetch_one(pool)
                .await
                .expect("audit pseudonym before delete");
        let mapping_before_delete: String = sqlx::query_scalar(
            "SELECT actor_pseudonym
               FROM scrim.audit_actor_pseudonyms
              WHERE actor_type = 'user'
                AND actor_ref = '42'",
        )
        .fetch_one(pool)
        .await
        .expect("audit mapping before delete");
        assert_eq!(mapping_before_delete, audit_pseudonym_before_delete);

        let summary = delete_user_data(pool, 42, "privacy-test".into(), 2_000)
            .await
            .expect("delete user data");

        assert_eq!(
            summary.counts.get("scrim_audit_actor_pseudonyms.actor_ref"),
            Some(&1)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_replacement_requests.user_identity"),
            Some(&3)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_replacement_candidates.user_identity"),
            Some(&3)
        );
        assert_eq!(
            summary.counts.get("scrim_replacement_needs.participant_id"),
            Some(&1)
        );
        assert!(summary.counts.get("scrim_command_receipts.payload") >= Some(&2));
        assert_eq!(
            summary.counts.get("scrim_command_receipts.result_payload"),
            Some(&1)
        );
        assert!(summary.counts.get("scrim_inbox_events.payload") >= Some(&2));
        assert!(summary.counts.get("scrim_outbox_effects.payload") >= Some(&2));
        assert!(summary.counts.get("scrim_effect_receipts.receipt_payload") >= Some(&2));
        assert_eq!(
            summary
                .counts
                .get("scrim_match_lineup_snapshots.lineup_payload"),
            Some(&1)
        );
        assert!(
            summary
                .counts
                .get("scrim_match_result_refs.clarification_payload")
                >= Some(&1)
        );
        assert!(
            summary
                .counts
                .get("scrim_match_result_clarifications.request_payload")
                >= Some(&1)
        );
        assert!(
            summary
                .counts
                .get("scrim_match_result_clarifications.response_payload")
                >= Some(&1)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_match_result_selections.selected_by_user_id.redacted"),
            Some(&1)
        );
        assert_eq!(summary.counts.get("scrim_ai_runs.subject_id"), Some(&2));
        assert!(summary.counts.get("steam_v1_workflows.aggregate_id") >= Some(&1));
        assert!(summary.counts.get("steam_v1_workflows.subject_id") >= Some(&1));
        assert!(summary.counts.get("steam_v1_operations.aggregate_id") >= Some(&1));
        assert!(summary.counts.get("steam_v1_workflows.payload") >= Some(&2));
        assert!(summary.counts.get("steam_v1_operations.subject_id") >= Some(&2));
        assert!(summary.counts.get("steam_v1_operations.payload") >= Some(&2));
        assert!(
            summary
                .counts
                .get("steam_v1_operation_results.result_payload")
                >= Some(&2)
        );
        assert!(summary.counts.get("steam_v1_operation_events.payload") >= Some(&2));
        assert!(summary.counts.get("steam_v1_deliveries.payload") >= Some(&2));
        let mapping_after_delete: Option<String> = sqlx::query_scalar(
            "SELECT actor_pseudonym
               FROM scrim.audit_actor_pseudonyms
              WHERE actor_type = 'user'
                AND actor_ref = '42'",
        )
        .fetch_optional(pool)
        .await
        .expect("audit mapping after delete");
        assert_eq!(mapping_after_delete, None);
        let evidence_rows: i64 = sqlx::query_scalar(
            "SELECT
                 (SELECT count(*) FROM scrim.command_receipts WHERE command_scope LIKE 'privacy_%')
               + (SELECT count(*) FROM scrim.inbox_events WHERE event_source = 'privacy')
               + (SELECT count(*) FROM scrim.outbox_effects WHERE effect_type LIKE 'privacy_%')
               + (SELECT count(*) FROM scrim.effect_receipts WHERE remote_system = 'privacy')
               + (SELECT count(*) FROM scrim.match_lineup_snapshots WHERE match_id = 424203)
               + (SELECT count(*) FROM scrim.ai_runs WHERE idempotency_key LIKE 'privacy:ai_%')
               + (SELECT count(*) FROM steam.v1_workflows WHERE workflow_key LIKE 'privacy:workflow%')
               + (SELECT count(*) FROM steam.v1_operations WHERE operation_id LIKE 'op:privacy_%')
               + (SELECT count(*) FROM steam.v1_operation_results WHERE operation_id LIKE 'op:privacy_%')
               + (SELECT count(*) FROM steam.v1_operation_events WHERE operation_id LIKE 'op:privacy_%')
               + (SELECT count(*) FROM steam.v1_deliveries WHERE operation_id LIKE 'op:privacy_%')",
        )
        .fetch_one(pool)
        .await
        .expect("privacy evidence rows remain");
        assert_eq!(evidence_rows, 26);
        let target_json_refs_remaining: bool = sqlx::query_scalar(
            "SELECT
                 EXISTS (SELECT 1 FROM scrim.command_receipts WHERE scrim.jsonb_contains_user_ref(payload, '42') OR scrim.jsonb_contains_user_ref(payload, '7656119800000042') OR scrim.jsonb_contains_user_ref(COALESCE(result_payload, '{}'::jsonb), '42') OR scrim.jsonb_contains_user_ref(COALESCE(result_payload, '{}'::jsonb), '7656119800000042'))
              OR EXISTS (SELECT 1 FROM scrim.inbox_events WHERE scrim.jsonb_contains_user_ref(payload, '42') OR scrim.jsonb_contains_user_ref(payload, '7656119800000042'))
              OR EXISTS (SELECT 1 FROM scrim.outbox_effects WHERE scrim.jsonb_contains_user_ref(payload, '42') OR scrim.jsonb_contains_user_ref(payload, '7656119800000042'))
              OR EXISTS (SELECT 1 FROM scrim.effect_receipts WHERE scrim.jsonb_contains_user_ref(receipt_payload, '42') OR scrim.jsonb_contains_user_ref(receipt_payload, '7656119800000042'))
              OR EXISTS (SELECT 1 FROM scrim.match_lineup_snapshots WHERE scrim.jsonb_contains_user_ref(lineup_payload, '42') OR scrim.jsonb_contains_user_ref(lineup_payload, '7656119800000042'))
              OR EXISTS (SELECT 1 FROM scrim.ai_runs WHERE (subject_kind = 'discord_user' AND subject_id = '42') OR (subject_kind = 'steam_user' AND subject_id = '7656119800000042'))
              OR EXISTS (SELECT 1 FROM steam.v1_workflows WHERE (aggregate_kind = 'player' AND aggregate_id = '7656119800000042') OR (subject_kind = 'discord_user' AND subject_id = '42') OR (subject_kind = 'steam_user' AND subject_id = '7656119800000042') OR scrim.jsonb_contains_user_ref(payload, '42') OR scrim.jsonb_contains_user_ref(payload, '7656119800000042'))
              OR EXISTS (SELECT 1 FROM steam.v1_operations WHERE (aggregate_kind = 'player' AND aggregate_id = '7656119800000042') OR (subject_kind = 'discord_user' AND subject_id = '42') OR (subject_kind = 'steam_user' AND subject_id = '7656119800000042') OR scrim.jsonb_contains_user_ref(payload, '42') OR scrim.jsonb_contains_user_ref(payload, '7656119800000042') OR scrim.jsonb_contains_user_ref(COALESCE(result_payload, '{}'::jsonb), '42') OR scrim.jsonb_contains_user_ref(COALESCE(result_payload, '{}'::jsonb), '7656119800000042'))
              OR EXISTS (SELECT 1 FROM steam.v1_operation_results WHERE scrim.jsonb_contains_user_ref(result_payload, '42') OR scrim.jsonb_contains_user_ref(result_payload, '7656119800000042'))
              OR EXISTS (SELECT 1 FROM steam.v1_operation_events WHERE scrim.jsonb_contains_user_ref(payload, '42') OR scrim.jsonb_contains_user_ref(payload, '7656119800000042'))
              OR EXISTS (SELECT 1 FROM steam.v1_deliveries WHERE scrim.jsonb_contains_user_ref(payload, '42') OR scrim.jsonb_contains_user_ref(payload, '7656119800000042'))",
        )
        .fetch_one(pool)
        .await
        .expect("target json refs remaining");
        assert!(!target_json_refs_remaining);
        let ai_subject_rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT idempotency_key, subject_kind, subject_id
               FROM scrim.ai_runs
              WHERE id = ANY($1)
              ORDER BY idempotency_key",
        )
        .bind([
            ai_discord_run_id,
            ai_steam_run_id,
            ai_team_collision_id,
            ai_match_collision_id,
            ai_foreign_run_id,
        ])
        .fetch_all(pool)
        .await
        .expect("ai run subjects after delete");
        assert_eq!(
            ai_subject_rows,
            vec![
                (
                    "privacy:ai_discord_target".to_string(),
                    "discord_user".to_string(),
                    "redacted".to_string(),
                ),
                (
                    "privacy:ai_foreign_user".to_string(),
                    "discord_user".to_string(),
                    "99".to_string(),
                ),
                (
                    "privacy:ai_match_collision".to_string(),
                    "match".to_string(),
                    "42".to_string(),
                ),
                (
                    "privacy:ai_steam_target".to_string(),
                    "steam_user".to_string(),
                    "redacted".to_string(),
                ),
                (
                    "privacy:ai_team_collision".to_string(),
                    "team".to_string(),
                    "42".to_string(),
                ),
            ]
        );
        let kind_collision_refs: (String, String) = sqlx::query_as(
            "SELECT
                 (SELECT aggregate_id FROM steam.v1_workflows WHERE workflow_key = 'privacy:workflow_kind_collision'),
                 (SELECT aggregate_id FROM steam.v1_operations WHERE operation_id = 'op:privacy_kind_collision')",
        )
        .fetch_one(pool)
        .await
        .expect("kind collision aggregate refs after delete");
        assert_eq!(kind_collision_refs, ("42".to_string(), "42".to_string()));
        let steam_links_remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core.steam_links WHERE discord_id = 42")
                .fetch_one(pool)
                .await
                .expect("steam links after delete");
        assert_eq!(steam_links_remaining, 0);
        let audit_actor: (String, String) = sqlx::query_as(
            "SELECT actor_pseudonym, actor_source FROM scrim.audit_events WHERE id = $1",
        )
        .bind(audit_id)
        .fetch_one(pool)
        .await
        .expect("audit actor pseudonymous");
        assert_eq!(
            audit_actor,
            (audit_pseudonym_before_delete.clone(), "user".to_string())
        );
        assert_ne!(audit_actor.0, "42");
        let regenerated_pseudonym: String =
            sqlx::query_scalar("SELECT scrim.audit_actor_pseudonym('user', '42')")
                .fetch_one(pool)
                .await
                .expect("regenerated audit pseudonym");
        assert_ne!(regenerated_pseudonym, audit_pseudonym_before_delete);
        let mapping_after_regeneration: Option<String> = sqlx::query_scalar(
            "SELECT actor_pseudonym
               FROM scrim.audit_actor_pseudonyms
              WHERE actor_type = 'user'
                AND actor_ref = '42'",
        )
        .fetch_optional(pool)
        .await
        .expect("audit mapping after one-off pseudonym");
        assert_eq!(mapping_after_regeneration, None);

        let runtime_actor: (String, String) =
            sqlx::query_as("SELECT actor_pseudonym, actor_source FROM scrim.runtime_control")
                .fetch_one(pool)
                .await
                .expect("runtime actor pseudonymous");
        assert!(runtime_actor.0.starts_with("act_"));
        assert_eq!(runtime_actor.1, "user");
        let announcement_actor: (String, String) = sqlx::query_as(
            "SELECT created_by_user_id, created_by_display_name
               FROM scrim.announcement_drafts
              WHERE id = $1",
        )
        .bind(announcement_draft_id)
        .fetch_one(pool)
        .await
        .expect("announcement actor redacted");
        assert_eq!(
            announcement_actor,
            ("redacted".to_string(), "redacted".to_string())
        );
        let approved_announcement_actor: (String, String, String, String) = sqlx::query_as(
            "SELECT created_by_user_id,
                    created_by_display_name,
                    approved_by_user_id,
                    approved_by_display_name
               FROM scrim.announcement_drafts
              WHERE id = $1",
        )
        .bind(approved_announcement_id)
        .fetch_one(pool)
        .await
        .expect("approved announcement actor redacted");
        assert_eq!(
            approved_announcement_actor,
            (
                "99".to_string(),
                "OtherUser".to_string(),
                "redacted".to_string(),
                "redacted".to_string(),
            )
        );
        let replacement_rows: i64 = sqlx::query_scalar(
            "SELECT
                (SELECT count(*) FROM scrim.replacement_requests)
              + (SELECT count(*) FROM scrim.replacement_candidates)
              + (SELECT count(*) FROM scrim.replacement_needs)",
        )
        .fetch_one(pool)
        .await
        .expect("replacement row count");
        assert_eq!(replacement_rows, 0);
        let participant_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM scrim.participants WHERE discord_id = 42")
                .fetch_one(pool)
                .await
                .expect("participant row count");
        assert_eq!(participant_rows, 0);
        let result_ref_actor: (String, String) = sqlx::query_as(
            "SELECT source_user_id, source_display_name FROM scrim.match_result_refs WHERE id = $1",
        )
        .bind(result_ref_id)
        .fetch_one(pool)
        .await
        .expect("result ref actor redacted");
        assert_eq!(
            result_ref_actor,
            ("redacted".to_string(), "redacted".to_string())
        );
        let selected_ref_actor: (String, String, String) = sqlx::query_as(
            "SELECT source_user_id, source_display_name, selected_by_user_id
               FROM scrim.match_result_refs
              WHERE id = $1",
        )
        .bind(selected_result_ref_id)
        .fetch_one(pool)
        .await
        .expect("selected result ref actor redacted");
        assert_eq!(
            selected_ref_actor,
            (
                "99".to_string(),
                "OtherUser".to_string(),
                "redacted".to_string(),
            )
        );
        let clarification_actor: (String, String) = sqlx::query_as(
            "SELECT requested_by_user_id, requested_by_display_name FROM scrim.match_result_clarifications",
        )
        .fetch_one(pool)
        .await
        .expect("clarification actor redacted");
        assert_eq!(
            clarification_actor,
            ("redacted".to_string(), "redacted".to_string())
        );
    }

    #[tokio::test]
    async fn scrim_replacement_requester_erasure_schrubbt_payload_ohne_fremde_zeile_zu_loeschen() {
        let db = mk_db().await;
        let pool = db.pool();
        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42), (99)")
            .execute(pool)
            .await
            .expect("core users");
        sqlx::query(
            "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
             VALUES (42, '7656119800000042', 7656119800000042)",
        )
        .execute(pool)
        .await
        .expect("linked steam target");
        sqlx::query(
            "INSERT INTO scrim.participants(
                 id, discord_id, display_name, rank_source, status, source, created_at, updated_at
              ) VALUES (925099, 99, 'OtherUser', 'manual', 'active', 'test', now(), now())",
        )
        .execute(pool)
        .await
        .expect("foreign participant");
        sqlx::query(
            "INSERT INTO scrim.teams(id, name, created_at) VALUES (925001, 'Replacement', now())",
        )
        .execute(pool)
        .await
        .expect("replacement team");
        let creator_need_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_needs(
                 team_id, reason, created_by_user_id, created_by_display_name
              ) VALUES (925001, 'creator_only', '42', 'DeleteMe')
              RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("creator-only replacement need");
        let need_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_needs(
                 team_id, reason, created_by_user_id, created_by_display_name
              ) VALUES (925001, 'requester_only', '99', 'OtherUser')
              RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("foreign replacement need");
        let candidate_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_candidates(
                 need_id, participant_id, candidate_data, score_data
              ) VALUES (
                 $1,
                 925099,
                 '{\"candidate\":\"99\"}'::jsonb,
                 '{\"score\":\"safe\"}'::jsonb
              ) RETURNING id",
        )
        .bind(need_id)
        .fetch_one(pool)
        .await
        .expect("foreign replacement candidate");
        let request_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_requests(
                 need_id, candidate_id, requested_by_user_id, requested_by_display_name,
                 request_payload, remote_system, remote_message_id
              ) VALUES (
                 $1,
                 $2,
                 '42',
                 'DeleteMe',
                 '{\"requester_id\":\"42\",\"steam_id\":\"7656119800000042\",\"foreign_id\":\"99\"}'::jsonb,
                 'discord',
                 'msg:replacement_requester_only'
              ) RETURNING id",
        )
        .bind(need_id)
        .bind(candidate_id)
        .fetch_one(pool)
        .await
        .expect("requester-only replacement request");

        let summary = delete_user_data(pool, 42, "test".to_string(), 1_000)
            .await
            .expect("privacy delete");

        assert_eq!(
            summary
                .counts
                .get("scrim_replacement_needs.created_by_user_id.redacted"),
            Some(&1)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_replacement_requests.request_payload"),
            Some(&2)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_replacement_requests.requested_by_user_id.redacted"),
            Some(&1)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_replacement_requests.user_identity"),
            Some(&0)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_replacement_candidates.user_identity"),
            Some(&0)
        );
        let row: (String, String, Value, bool, bool, bool) = sqlx::query_as(
            "SELECT requested_by_user_id,
                    requested_by_display_name,
                    request_payload,
                    scrim.jsonb_contains_user_ref(request_payload, '42'),
                    scrim.jsonb_contains_user_ref(request_payload, '7656119800000042'),
                    scrim.jsonb_contains_user_ref(request_payload, '99')
               FROM scrim.replacement_requests
              WHERE id = $1",
        )
        .bind(request_id)
        .fetch_one(pool)
        .await
        .expect("requester-only row survived");
        assert_eq!(row.0, "redacted");
        assert_eq!(row.1, "redacted");
        assert!(!row.3);
        assert!(!row.4);
        assert!(row.5);
        assert_eq!(row.2["foreign_id"], serde_json::json!("99"));
        let creator_row: (String, String) = sqlx::query_as(
            "SELECT created_by_user_id, created_by_display_name
               FROM scrim.replacement_needs
              WHERE id = $1",
        )
        .bind(creator_need_id)
        .fetch_one(pool)
        .await
        .expect("creator-only row survived");
        assert_eq!(
            creator_row,
            ("redacted".to_string(), "redacted".to_string())
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM scrim.replacement_candidates")
                .fetch_one(pool)
                .await
                .expect("candidate count"),
            1
        );
    }

    #[tokio::test]
    async fn scrim_hashonly_dsar_deckt_payload_only_treffer_ohne_rohdaten() {
        let db = mk_db().await;
        let pool = db.pool();
        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42), (77), (99)")
            .execute(pool)
            .await
            .expect("core users");
        sqlx::query(
            "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
             VALUES
                (42, '76561198000000420', 76561198000000420),
                (77, '76561197960265805', 76561197960265805)",
        )
        .execute(pool)
        .await
        .expect("target steam link");
        sqlx::query(
            "INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
             VALUES
                (42, true, now(), 'namespace-test', now()),
                (77, true, now(), 'namespace-collision-test', now())",
        )
        .execute(pool)
        .await
        .expect("privacy namespace tombstones");
        let linked_account_namespace: (bool, Option<String>) = sqlx::query_as(
            "WITH privacy AS (
                 SELECT set_config('scrim.privacy_erasure_user_id', '42', true),
                        set_config('scrim.privacy_erasure_target_ref', '39734692', true)
             )
             SELECT scrim.privacy_erasure_authorized('39734692'),
                    scrim.privacy_erasure_target_namespace('39734692')
               FROM privacy",
        )
        .fetch_one(pool)
        .await
        .expect("linked account namespace");
        assert_eq!(
            linked_account_namespace,
            (true, Some("steam_user".to_string()))
        );
        let wrong_account_namespace: (bool, Option<String>) = sqlx::query_as(
            "WITH privacy AS (
                 SELECT set_config('scrim.privacy_erasure_user_id', '42', true),
                        set_config('scrim.privacy_erasure_target_ref', '39734693', true)
             )
             SELECT scrim.privacy_erasure_authorized('39734693'),
                    scrim.privacy_erasure_target_namespace('39734693')
               FROM privacy",
        )
        .fetch_one(pool)
        .await
        .expect("wrong account namespace");
        assert_eq!(wrong_account_namespace, (false, None));
        let collision_namespace: (bool, Option<String>) = sqlx::query_as(
            "WITH privacy AS (
                 SELECT set_config('scrim.privacy_erasure_user_id', '77', true),
                        set_config('scrim.privacy_erasure_target_ref', '77', true)
             )
             SELECT scrim.privacy_erasure_authorized('77'),
                    scrim.privacy_erasure_target_namespace('77')
               FROM privacy",
        )
        .fetch_one(pool)
        .await
        .expect("colliding account namespace");
        assert_eq!(
            collision_namespace,
            (true, Some("discord_user".to_string()))
        );
        sqlx::query(
            "INSERT INTO scrim.participants(
                 id, discord_id, display_name, rank_source, status, source, created_at, updated_at
              ) VALUES (945099, 99, 'OtherUser', 'manual', 'active', 'test', now(), now())",
        )
        .execute(pool)
        .await
        .expect("foreign participant");
        sqlx::query(
            "INSERT INTO scrim.teams(id, name, created_at)
             VALUES (945001, 'Hash A', now()), (945002, 'Hash B', now())",
        )
        .execute(pool)
        .await
        .expect("hash teams");
        sqlx::query(
            "INSERT INTO scrim.matches(id, team_a_id, team_b_id, status, created_at)
             VALUES (945003, 945001, 945002, 'scheduled', now())",
        )
        .execute(pool)
        .await
        .expect("hash match");
        let need_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.replacement_needs(
                 team_id, reason, created_by_user_id, created_by_display_name
              ) VALUES (945001, 'privacy_hashonly', '99', 'OtherUser')
              RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("foreign replacement need");
        let candidate_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.replacement_candidates(
                 need_id, participant_id, candidate_data, score_data
              ) VALUES (
                 $1,
                 945099,
                 '{"target_user_id":"42","note":"RAW_HASHONLY_CANDIDATE","foreign":"FOREIGN_HASHONLY_CANDIDATE"}'::jsonb,
                 '{"target_account_id":"39734692","note":"RAW_HASHONLY_SCORE","foreign":"FOREIGN_HASHONLY_SCORE"}'::jsonb
              ) RETURNING id"#,
        )
        .bind(need_id)
        .fetch_one(pool)
        .await
        .expect("payload-only replacement candidate");
        sqlx::query(
            r#"INSERT INTO scrim.replacement_candidates(
                 need_id, discord_user_id, candidate_data, score_data
              ) VALUES (
                 $1,
                 99,
                 '{"target_user_id":"99","note":"UNRELATED_CANDIDATE"}'::jsonb,
                 '{"target_account_id":"39734693","note":"UNRELATED_SCORE"}'::jsonb
              )"#,
        )
        .bind(need_id)
        .execute(pool)
        .await
        .expect("unrelated replacement candidate");
        let request_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.replacement_requests(
                 need_id, candidate_id, requested_by_user_id, requested_by_display_name,
                 request_payload
              ) VALUES (
                 $1,
                 $2,
                 '99',
                 'OtherUser',
                 '{"steam_id":"76561198000000420","note":"RAW_HASHONLY_REQUEST","foreign":"FOREIGN_HASHONLY_REQUEST"}'::jsonb
              ) RETURNING id"#,
        )
        .bind(need_id)
        .bind(candidate_id)
        .fetch_one(pool)
        .await
        .expect("payload-only replacement request");
        sqlx::query(
            r#"INSERT INTO scrim.replacement_requests(
                 need_id, discord_user_id, requested_by_user_id, requested_by_display_name,
                 request_payload
              ) VALUES (
                 $1,
                 99,
                 '99',
                 'OtherUser',
                 '{"target_user_id":"99","note":"UNRELATED_REQUEST"}'::jsonb
              )"#,
        )
        .bind(need_id)
        .execute(pool)
        .await
        .expect("unrelated replacement request");
        let result_ref_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.match_result_refs(
                 match_id, steam_match_id, source_user_id, source_display_name,
                 fetch_status, winner_team_id, clarification_payload, validation_status
              ) VALUES (
                 945003,
                 945301,
                 '99',
                 'OtherUser',
                 'fetched',
                 945001,
                 '{"target_user_id":"42","note":"RAW_HASHONLY_RESULT_REF","foreign":"FOREIGN_HASHONLY_RESULT_REF"}'::jsonb,
                 'valid'
              ) RETURNING id"#,
        )
        .fetch_one(pool)
        .await
        .expect("payload-only result ref");
        sqlx::query(
            r#"INSERT INTO scrim.match_result_refs(
                 match_id, steam_match_id, source_user_id, source_display_name,
                 fetch_status, winner_team_id, clarification_payload, validation_status
              ) VALUES (
                 945003,
                 945302,
                 '99',
                 'OtherUser',
                 'fetched',
                 945002,
                 '{"target_user_id":"99","note":"UNRELATED_RESULT_REF"}'::jsonb,
                 'valid'
              )"#,
        )
        .execute(pool)
        .await
        .expect("unrelated result ref");
        let clarification_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.match_result_clarifications(
                 result_ref_id, clarification_kind, requested_by_user_id, requested_by_display_name,
                 request_payload, response_payload
              ) VALUES (
                 $1,
                 'manual_review',
                 '99',
                 'OtherUser',
                 '{"target_user_id":"42","note":"RAW_HASHONLY_CLARIFICATION_REQUEST","foreign":"FOREIGN_HASHONLY_CLARIFICATION_REQUEST"}'::jsonb,
                 '{"steam_id":"76561198000000420","note":"RAW_HASHONLY_CLARIFICATION_RESPONSE","foreign":"FOREIGN_HASHONLY_CLARIFICATION_RESPONSE"}'::jsonb
              ) RETURNING id"#,
        )
        .bind(result_ref_id)
        .fetch_one(pool)
        .await
        .expect("payload-only clarification");

        let export = export_user_data(pool, 42, 1_000)
            .await
            .expect("hash-only privacy export");
        let tables = export["tables"].as_object().expect("tables object");
        for (key, id, column) in [
            (
                "scrim_replacement_candidates.candidate_data",
                candidate_id,
                "candidate_data",
            ),
            (
                "scrim_replacement_candidates.score_data",
                candidate_id,
                "score_data",
            ),
            (
                "scrim_replacement_requests.request_payload",
                request_id,
                "request_payload",
            ),
            (
                "scrim_match_result_refs.clarification_payload",
                result_ref_id,
                "clarification_payload",
            ),
            (
                "scrim_match_result_clarifications.request_payload",
                clarification_id,
                "request_payload",
            ),
            (
                "scrim_match_result_clarifications.response_payload",
                clarification_id,
                "response_payload",
            ),
        ] {
            let rows = tables[key]
                .as_array()
                .unwrap_or_else(|| panic!("hash-only rows missing for {key}"));
            assert_eq!(rows.len(), 1, "unrelated rows leaked for {key}: {rows:?}");
            let row = rows[0].as_object().expect("hash-only row object");
            assert_eq!(
                row.len(),
                2,
                "hash-only DSAR exposed extra columns: {row:?}"
            );
            assert_eq!(row["id"], serde_json::json!(id));
            assert!(
                row[column]["md5"].is_string(),
                "missing payload hash for {key}: {row:?}"
            );
        }
        let serialized = serde_json::to_string(&export).expect("serialize hash-only export");
        for marker in [
            "RAW_HASHONLY_CANDIDATE",
            "RAW_HASHONLY_SCORE",
            "RAW_HASHONLY_REQUEST",
            "RAW_HASHONLY_RESULT_REF",
            "RAW_HASHONLY_CLARIFICATION_REQUEST",
            "RAW_HASHONLY_CLARIFICATION_RESPONSE",
            "FOREIGN_HASHONLY_CANDIDATE",
            "FOREIGN_HASHONLY_SCORE",
            "FOREIGN_HASHONLY_REQUEST",
            "FOREIGN_HASHONLY_RESULT_REF",
            "FOREIGN_HASHONLY_CLARIFICATION_REQUEST",
            "FOREIGN_HASHONLY_CLARIFICATION_RESPONSE",
            "UNRELATED_CANDIDATE",
            "UNRELATED_SCORE",
            "UNRELATED_REQUEST",
            "UNRELATED_RESULT_REF",
        ] {
            assert!(
                !serialized.contains(marker),
                "hash-only DSAR leaked raw marker {marker}: {serialized}"
            );
        }

        let summary = delete_user_data(pool, 42, "hash-only-test".to_string(), 2_000)
            .await
            .expect("hash-only privacy delete");
        for key in [
            "scrim_replacement_candidates.candidate_data",
            "scrim_replacement_candidates.score_data",
            "scrim_replacement_requests.request_payload",
            "scrim_match_result_refs.clarification_payload",
            "scrim_match_result_clarifications.request_payload",
            "scrim_match_result_clarifications.response_payload",
        ] {
            assert!(
                summary.counts.get(key) >= Some(&1),
                "{key} was not scrubbed"
            );
        }
        let target_refs_remaining: bool = sqlx::query_scalar(
            "SELECT
                 EXISTS (SELECT 1 FROM scrim.replacement_candidates WHERE id = $1 AND (scrim.jsonb_contains_user_ref(candidate_data, '42') OR scrim.jsonb_contains_user_ref(score_data, '39734692')))
              OR EXISTS (SELECT 1 FROM scrim.replacement_requests WHERE id = $2 AND scrim.jsonb_contains_user_ref(request_payload, '76561198000000420'))
              OR EXISTS (SELECT 1 FROM scrim.match_result_refs WHERE id = $3 AND scrim.jsonb_contains_user_ref(clarification_payload, '42'))
              OR EXISTS (SELECT 1 FROM scrim.match_result_clarifications WHERE id = $4 AND (scrim.jsonb_contains_user_ref(request_payload, '42') OR scrim.jsonb_contains_user_ref(COALESCE(response_payload, '{}'::jsonb), '76561198000000420')))"
        )
        .bind(candidate_id)
        .bind(request_id)
        .bind(result_ref_id)
        .bind(clarification_id)
        .fetch_one(pool)
        .await
        .expect("target refs after hash-only delete");
        assert!(!target_refs_remaining);
    }

    #[tokio::test]
    async fn scrim_account_id_erasure_autorisiert_nur_verlinkte_steam_account_targets() {
        let db = mk_db().await;
        let pool = db.pool();
        let target_account_id = "39734692";
        let wrong_account_id = "39734693";
        let hash = vec![9_u8; 32];

        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42), (99)")
            .execute(pool)
            .await
            .expect("core users");
        sqlx::query(
            "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
             VALUES (42, '76561198000000420', 76561198000000420)",
        )
        .execute(pool)
        .await
        .expect("target steam link");
        sqlx::query(
            r#"INSERT INTO scrim.command_receipts(
                 command_scope, idempotency_key, payload_hash, payload, state, completed_at
              ) VALUES
                ('privacy_account', 'account:target_queue', $1, '{"account_id":"39734692","note":"TARGET_ACCOUNT_QUEUE"}'::jsonb, 'completed', now()),
                ('privacy_account', 'account:wrong_queue', $1, '{"account_id":"39734693","note":"WRONG_ACCOUNT_QUEUE"}'::jsonb, 'completed', now())"#,
        )
        .bind(&hash)
        .execute(pool)
        .await
        .expect("protected queue account payloads");
        let delete_queue = sqlx::query(
            "DELETE FROM scrim.command_receipts WHERE idempotency_key = 'account:target_queue'",
        )
        .execute(pool)
        .await;
        assert_eq!(database_error_code(&delete_queue).as_deref(), Some("55000"));
        sqlx::query(
            r#"INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
             VALUES
                ('lagebild', 'steam_user', '39734692', 'privacy:ai_account_target', $1),
                ('lagebild', 'steam_user', '39734693', 'privacy:ai_account_wrong', $1),
                ('lagebild', 'team', '39734692', 'privacy:ai_account_kind_collision', $1)"#,
        )
        .bind(&hash)
        .execute(pool)
        .await
        .expect("ai account targets");
        sqlx::query(
            r#"INSERT INTO steam.v1_workflows(
                 workflow_key, workflow_type, aggregate_kind, aggregate_id, subject_kind, subject_id, payload
              ) VALUES
                ('privacy:workflow_account_target', 'friend_request', 'player', '39734692', 'steam_user', '39734692', '{"account_id":"39734692","note":"TARGET_ACCOUNT_WORKFLOW"}'::jsonb),
                ('privacy:workflow_account_wrong', 'friend_request', 'player', '39734693', 'steam_user', '39734693', '{"account_id":"39734693","note":"WRONG_ACCOUNT_WORKFLOW"}'::jsonb),
                ('privacy:workflow_account_kind_collision', 'friend_request', 'match', '39734692', NULL, NULL, '{"domain":"match"}'::jsonb)"#,
        )
        .execute(pool)
        .await
        .expect("steam workflow account targets");
        sqlx::query(
            r#"INSERT INTO steam.v1_operations(
                 operation_id, operation_type, aggregate_kind, aggregate_id, subject_kind, subject_id,
                 idempotency_key, payload_hash, payload, result_payload
              ) VALUES
                ('op:privacy_account_target', 'friend_request', 'player', '39734692', 'steam_user', '39734692', 'operation:account_target', $1, '{"account_id":"39734692","note":"TARGET_ACCOUNT_OPERATION"}'::jsonb, '{"account_id":"39734692","note":"TARGET_ACCOUNT_OPERATION_RESULT"}'::jsonb),
                ('op:privacy_account_wrong', 'friend_request', 'player', '39734693', 'steam_user', '39734693', 'operation:account_wrong', $1, '{"account_id":"39734693","note":"WRONG_ACCOUNT_OPERATION"}'::jsonb, '{"account_id":"39734693","note":"WRONG_ACCOUNT_OPERATION_RESULT"}'::jsonb),
                ('op:privacy_account_kind_collision', 'friend_request', 'match', '39734692', NULL, NULL, 'operation:account_kind_collision', $1, '{"domain":"match"}'::jsonb, NULL)"#,
        )
        .bind(&hash)
        .execute(pool)
        .await
        .expect("steam operation account targets");

        let summary = delete_user_data(pool, 42, "account-id-test".to_string(), 2_000)
            .await
            .expect("account id privacy delete");
        assert_eq!(
            summary.counts.get("scrim_command_receipts.payload"),
            Some(&1)
        );
        assert_eq!(summary.counts.get("scrim_ai_runs.subject_id"), Some(&1));
        assert_eq!(
            summary.counts.get("steam_v1_workflows.aggregate_id"),
            Some(&1)
        );
        assert_eq!(
            summary.counts.get("steam_v1_workflows.subject_id"),
            Some(&1)
        );
        assert_eq!(summary.counts.get("steam_v1_workflows.payload"), Some(&1));
        assert_eq!(
            summary.counts.get("steam_v1_operations.aggregate_id"),
            Some(&1)
        );
        assert_eq!(
            summary.counts.get("steam_v1_operations.subject_id"),
            Some(&1)
        );
        assert_eq!(summary.counts.get("steam_v1_operations.payload"), Some(&1));
        assert_eq!(
            summary.counts.get("steam_v1_operations.result_payload"),
            Some(&1)
        );

        let queue_payloads: Vec<(String, Value)> = sqlx::query_as(
            "SELECT idempotency_key, payload
               FROM scrim.command_receipts
              WHERE command_scope = 'privacy_account'
              ORDER BY idempotency_key",
        )
        .fetch_all(pool)
        .await
        .expect("queue payloads after account delete");
        assert_eq!(queue_payloads.len(), 2);
        assert!(!scrim_json_contains(
            &queue_payloads[0].1,
            target_account_id
        ));
        assert!(scrim_json_contains(&queue_payloads[1].1, wrong_account_id));

        let ai_subjects: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT idempotency_key, subject_kind, subject_id
               FROM scrim.ai_runs
              WHERE idempotency_key LIKE 'privacy:ai_account%'
              ORDER BY idempotency_key",
        )
        .fetch_all(pool)
        .await
        .expect("ai account subjects after delete");
        assert_eq!(
            ai_subjects,
            vec![
                (
                    "privacy:ai_account_kind_collision".to_string(),
                    "team".to_string(),
                    target_account_id.to_string(),
                ),
                (
                    "privacy:ai_account_target".to_string(),
                    "steam_user".to_string(),
                    "redacted".to_string(),
                ),
                (
                    "privacy:ai_account_wrong".to_string(),
                    "steam_user".to_string(),
                    wrong_account_id.to_string(),
                ),
            ]
        );
        let steam_refs: Vec<SteamAccountRefRow> = sqlx::query_as(
            "SELECT workflow_key, aggregate_id, subject_id, payload, NULL::jsonb AS result_payload
                   FROM steam.v1_workflows
                  WHERE workflow_key LIKE 'privacy:workflow_account%'
                 UNION ALL
                 SELECT operation_id, aggregate_id, subject_id, payload, result_payload
                   FROM steam.v1_operations
                  WHERE operation_id LIKE 'op:privacy_account%'
                  ORDER BY 1",
        )
        .fetch_all(pool)
        .await
        .expect("steam account refs after delete");
        for (key, aggregate_id, subject_id, payload, result_payload) in steam_refs {
            match key.as_str() {
                "op:privacy_account_target" | "privacy:workflow_account_target" => {
                    assert_eq!(aggregate_id.as_deref(), Some("redacted"));
                    assert_eq!(subject_id.as_deref(), Some("redacted"));
                    assert!(!scrim_json_contains(&payload, target_account_id));
                    if let Some(result_payload) = result_payload {
                        assert!(!scrim_json_contains(&result_payload, target_account_id));
                    }
                }
                "op:privacy_account_wrong" | "privacy:workflow_account_wrong" => {
                    assert_eq!(aggregate_id.as_deref(), Some(wrong_account_id));
                    assert_eq!(subject_id.as_deref(), Some(wrong_account_id));
                    assert!(scrim_json_contains(&payload, wrong_account_id));
                }
                "op:privacy_account_kind_collision" | "privacy:workflow_account_kind_collision" => {
                    assert_eq!(aggregate_id.as_deref(), Some(target_account_id));
                    assert!(subject_id.is_none());
                }
                other => panic!("unexpected steam account row: {other}"),
            }
        }
    }

    #[tokio::test]
    async fn scrim_result_json_privacy_export_und_erasure_deckt_account_id_und_last_error() {
        let db = mk_db().await;
        let pool = db.pool();
        let target_steam_id64 = "76561198000000420";
        let target_account_id = "39734692";

        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42), (99)")
            .execute(pool)
            .await
            .expect("core users");
        sqlx::query(
            "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
             VALUES (42, $1, 76561198000000420)",
        )
        .bind(target_steam_id64)
        .execute(pool)
        .await
        .expect("target steam link");
        sqlx::query(
            "INSERT INTO scrim.teams(id, name, created_at)
             VALUES (935001, 'Result A', now()), (935002, 'Result B', now())",
        )
        .execute(pool)
        .await
        .expect("result teams");
        sqlx::query(
            r#"INSERT INTO scrim.matches(
                 id, team_a_id, team_b_id, status, join_code, steam_match_id, winner_team_id,
                 result_json, lobby_code_corrections, created_at, updated_at
             ) VALUES (
                 935003,
                 935001,
                 935002,
                 'completed',
                 'SECRET_LOBBY_CODE_ADJACENT',
                 935301,
                 935001,
                 '{
                    "legacy": true,
                    "players": [
                      {"account_id":39734692,"steam_id":"76561198000000420","display_name":"Target Legacy","note":"TARGET_LEGACY_RESULT"},
                      {"account_id":123456,"steam_id":"76561198000000999","display_name":"Foreign Legacy","note":"FOREIGN_LEGACY_RESULT"}
                     ]
                   }'::jsonb,
                 '[{"actor_user_id":"42","actor_display_name":"ADJACENT_LOBBY_CORRECTION_PII"}]'::jsonb,
                 now(),
                 now()
             )"#,
        )
        .execute(pool)
        .await
        .expect("legacy result match");
        sqlx::query(
            "INSERT INTO scrim.matches(id, team_a_id, team_b_id, status, created_at)
             VALUES (935006, 935001, 935002, 'scheduled', now())",
        )
        .execute(pool)
        .await
        .expect("result-ref match");
        let result_ref_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO scrim.match_result_refs(
                 match_id, steam_match_id, source_user_id, source_display_name,
                 fetch_status, winner_team_id, clarification_payload, raw_result_json,
                 normalized_result_json, validation_status, last_error
             ) VALUES (
                 935006,
                 935302,
                 '99',
                 'Foreign Actor',
                 'fetched',
                 935001,
                 '{"manual_review":"ADJACENT_CLARIFICATION_PII_42","target_user_id":"42"}'::jsonb,
                 '{
                    "raw_players": [
                      {"account_id":"39734692","steam_id":"76561198000000420","name":"Target Raw","score":7},
                      {"account_id":"123456","steam_id":"76561198000000999","name":"Foreign Raw","note":"FOREIGN_RAW_RESULT"}
                    ]
                  }'::jsonb,
                 '{
                    "players_by_account": {
                      "39734692": {"display_name":"Target Selected","kills":7,"note":"TARGET_SELECTED_RESULT"},
                      "123456": {"display_name":"Foreign Selected","kills":3,"note":"FOREIGN_SELECTED_RESULT"}
                    },
                    "players": [
                      {"account_id":39734692,"steam_id":"76561198000000420","display_name":"Target Selected Array","kills":7},
                      {"account_id":123456,"steam_id":"76561198000000999","display_name":"Foreign Selected Array","note":"FOREIGN_SELECTED_ARRAY"}
                    ]
                  }'::jsonb,
                 'valid',
                 'failed discord 42 steam 76561198000000420 account 39734692 foreign 99 keep 4242 user42'
             )
             RETURNING id"#,
        )
        .fetch_one(pool)
        .await
        .expect("result ref with result json");
        sqlx::query(
            "INSERT INTO scrim.match_result_selections(
                 match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
             ) VALUES (935006, $1, '99', 'Foreign Actor', 'privacy_result_json')",
        )
        .bind(result_ref_id)
        .execute(pool)
        .await
        .expect("selected result ref");

        let export = export_user_data(pool, 42, 1_000)
            .await
            .expect("result privacy export");
        let legacy_rows = export["tables"]["scrim_matches.result_json"]
            .as_array()
            .expect("legacy result_json export");
        let raw_rows = export["tables"]["scrim_match_result_refs.raw_result_json"]
            .as_array()
            .expect("raw result_json export");
        let normalized_rows = export["tables"]["scrim_match_result_refs.normalized_result_json"]
            .as_array()
            .expect("normalized result_json export");
        assert_eq!(legacy_rows.len(), 1);
        assert_eq!(raw_rows.len(), 1);
        assert_eq!(normalized_rows.len(), 1);
        for (row, column) in [
            (&legacy_rows[0], "result_json"),
            (&raw_rows[0], "raw_result_json"),
            (&normalized_rows[0], "normalized_result_json"),
        ] {
            let object = row.as_object().expect("generic JSON DSAR row object");
            assert_eq!(
                object.len(),
                2,
                "generic JSON DSAR exposes adjacent fields: {object:?}"
            );
            assert!(object.contains_key("id"));
            assert!(object.contains_key(column));
        }
        let result_export = serde_json::to_string(&serde_json::json!({
            "legacy": legacy_rows,
            "raw": raw_rows,
            "normalized": normalized_rows,
        }))
        .expect("serialize result export");
        assert!(result_export.contains(target_account_id));
        assert!(result_export.contains(target_steam_id64));
        assert!(result_export.contains("TARGET_LEGACY_RESULT"));
        assert!(result_export.contains("TARGET_SELECTED_RESULT"));
        for marker in [
            "Foreign Legacy",
            "Foreign Raw",
            "Foreign Selected",
            "FOREIGN_LEGACY_RESULT",
            "FOREIGN_RAW_RESULT",
            "FOREIGN_SELECTED_RESULT",
            "FOREIGN_SELECTED_ARRAY",
            "76561198000000999",
            "123456",
            "SECRET_LOBBY_CODE_ADJACENT",
            "ADJACENT_LOBBY_CORRECTION_PII",
            "ADJACENT_CLARIFICATION_PII_42",
            "failed discord 42",
        ] {
            assert!(
                !result_export.contains(marker),
                "foreign result data leaked in DSAR export: {marker}: {result_export}"
            );
        }

        let summary = delete_user_data(pool, 42, "result-json-test".to_string(), 2_000)
            .await
            .expect("result privacy delete");
        assert!(summary.counts.get("scrim_matches.result_json") >= Some(&1));
        assert!(
            summary
                .counts
                .get("scrim_match_result_refs.raw_result_json")
                >= Some(&1)
        );
        assert!(
            summary
                .counts
                .get("scrim_match_result_refs.normalized_result_json")
                >= Some(&1)
        );
        assert!(
            summary
                .counts
                .get("scrim_match_result_refs.clarification_payload")
                >= Some(&1)
        );
        assert_eq!(
            summary.counts.get("scrim_match_result_refs.last_error"),
            Some(&1)
        );

        let target_result_refs_remaining: bool = sqlx::query_scalar(
            "SELECT
                 EXISTS (
                     SELECT 1 FROM scrim.matches
                      WHERE id = 935003
                        AND (
                            scrim.jsonb_contains_user_ref(result_json, '42')
                            OR scrim.jsonb_contains_user_ref(result_json, '76561198000000420')
                            OR scrim.jsonb_contains_user_ref(result_json, '39734692')
                        )
                 )
              OR EXISTS (
                     SELECT 1 FROM scrim.match_result_refs
                      WHERE id = $1
                        AND (
                            scrim.jsonb_contains_user_ref(COALESCE(raw_result_json, '{}'::jsonb), '42')
                            OR scrim.jsonb_contains_user_ref(COALESCE(raw_result_json, '{}'::jsonb), '76561198000000420')
                            OR scrim.jsonb_contains_user_ref(COALESCE(raw_result_json, '{}'::jsonb), '39734692')
                            OR scrim.jsonb_contains_user_ref(clarification_payload, '42')
                            OR scrim.jsonb_contains_user_ref(normalized_result_json, '42')
                            OR scrim.jsonb_contains_user_ref(normalized_result_json, '76561198000000420')
                            OR scrim.jsonb_contains_user_ref(normalized_result_json, '39734692')
                        )
                 )",
        )
        .bind(result_ref_id)
        .fetch_one(pool)
        .await
        .expect("target result refs remaining");
        assert!(!target_result_refs_remaining);

        let stored_last_error: String =
            sqlx::query_scalar("SELECT last_error FROM scrim.match_result_refs WHERE id = $1")
                .bind(result_ref_id)
                .fetch_one(pool)
                .await
                .expect("stored last_error after delete");
        for target in [" 42 ", target_steam_id64, target_account_id] {
            assert!(
                !stored_last_error.contains(target),
                "target ref survived in last_error: {stored_last_error}"
            );
        }
        assert!(stored_last_error.contains("foreign 99"));
        assert!(stored_last_error.contains("4242"));
        assert!(stored_last_error.contains("user42"));

        let selected: (Option<i64>, Option<Value>) = sqlx::query_as(
            "SELECT result_ref_id, result_json
               FROM scrim.selected_match_results
              WHERE match_id = 935006",
        )
        .fetch_one(pool)
        .await
        .expect("selected result after erasure");
        assert_eq!(selected.0, Some(result_ref_id));
        let selected_json = selected.1.expect("selected result json");
        let selected_serialized = serde_json::to_string(&selected_json).expect("selected json");
        assert!(!selected_serialized.contains(target_steam_id64));
        assert!(!selected_serialized.contains(target_account_id));
        assert!(!selected_serialized.contains("Target Selected"));
        assert!(selected_serialized.contains("Foreign Selected"));
        assert!(selected_serialized.contains("FOREIGN_SELECTED_RESULT"));
    }

    #[tokio::test]
    async fn scrim_match_und_request_actor_dsar_ist_sicher_und_lobby_corrections_werden_redigiert()
    {
        let db = mk_db().await;
        let pool = db.pool();
        let target_steam_id64 = "76561198000000420";
        let target_account_id = "39734692";

        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42), (99)")
            .execute(pool)
            .await
            .expect("core users");
        sqlx::query(
            "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
             VALUES (42, $1, 76561198000000420)",
        )
        .bind(target_steam_id64)
        .execute(pool)
        .await
        .expect("target steam link");
        sqlx::query(
            "INSERT INTO scrim.teams(id, name, created_at)
             VALUES (936001, 'Actor A', now()), (936002, 'Actor B', now())",
        )
        .execute(pool)
        .await
        .expect("actor teams");
        sqlx::query(
            r#"INSERT INTO scrim.matches(
                 id, team_a_id, team_b_id, when_text, scheduled_at, status,
                 party_id, join_code, result_json, lobby_state,
                 lobby_code_source_user_id, lobby_code_source_display_name,
                 lobby_code_updated_at, lobby_code_message_ids, lobby_code_corrections,
                 created_at, updated_at
             ) VALUES (
                 936003,
                 936001,
                 936002,
                 'private schedule SECRET_WHEN_TEXT',
                 now(),
                 'scheduled',
                 'SECRET_PARTY_ID',
                 'SECRET_LOBBY_CODE',
                 '{"target_user_id":"42","secret":"RAW_MATCH_RESULT_SECRET"}'::jsonb,
                 'waiting',
                 '42',
                 'DeleteMe',
                  now(),
                  '{"team_a":"QUERY_MESSAGE_ID_LEAK"}'::jsonb,
                  '[
                    {"actor_user_id":"42","actor_display_name":"DeleteMe","from":"DL-8Q4K","to":"DL-7N2P","note":"TARGET_CORRECTION_NOTE"},
                    {"actor_user_id":"99","actor_display_name":"OtherUser","from":"DL-5J1L","to":"DL-2X9R","note":"FOREIGN_CORRECTION_NOTE"}
                  ]'::jsonb,
                  now(),
                  now()
             )"#,
        )
        .execute(pool)
        .await
        .expect("actor match");
        sqlx::query(
            "INSERT INTO scrim.match_request_batches(
                 id, template, deadline_at, status, created_by_user_id, created_by_display_name
             ) VALUES (936009, 'regular_scrim', now() + interval '1 day', 'open', '99', 'OtherUser')",
        )
        .execute(pool)
        .await
        .expect("actor batch");
        sqlx::query(
            r#"INSERT INTO scrim.match_requests(
                 id, batch_id, team_a_id, team_b_id, status, slot_options,
                 team_query_message_ids, posted_at, released_slot_index, released_slot,
                 released_at, released_by_user_id, released_by_display_name, override_reason,
                 team_status_message_ids, status_message_state, status_message_last_error,
                 status_message_posted_at, status_message_updated_at
             ) VALUES (
                 936010,
                 936009,
                 936001,
                 936002,
                 'open',
                 '{"private":"SLOT_OPTION_SECRET","candidate_user":"42"}'::jsonb,
                 '{"team_a":"QUERY_MESSAGE_ID_LEAK"}'::jsonb,
                 now(),
                 0,
                 '{"steam_id":"76561198000000420","secret":"RELEASED_SLOT_SECRET"}'::jsonb,
                 now(),
                 '42',
                 'DeleteMe',
                 'override 42 steam 76561198000000420 account 39734692 foreign 99 keep 4242 user42',
                 '{"team_a":"STATUS_MESSAGE_ID_LEAK"}'::jsonb,
                 'posted',
                 'status error 42 steam 76561198000000420 account 39734692 foreign 99 keep 4242 user42',
                 now(),
                 now()
             )"#,
        )
        .execute(pool)
        .await
        .expect("actor match request");

        let export = export_user_data(pool, 42, 1_000)
            .await
            .expect("actor privacy export");
        let match_actor_rows = export["tables"]["scrim_matches.lobby_code_source_user_id.redacted"]
            .as_array()
            .expect("match actor export");
        assert_eq!(match_actor_rows.len(), 1);
        let match_actor = &match_actor_rows[0];
        assert_eq!(
            match_actor["lobby_code_source_user_id"],
            serde_json::json!("42")
        );
        assert_eq!(
            match_actor["lobby_code_source_display_name"],
            serde_json::json!("DeleteMe")
        );
        assert!(match_actor["result_json"]["md5"].is_string());
        assert!(match_actor["lobby_code_corrections"]["md5"].is_string());
        assert_eq!(
            match_actor["lobby_code_corrections"]
                .as_object()
                .expect("actor correction hash object")
                .len(),
            1,
            "actor export must keep lobby corrections hash-only"
        );
        assert!(match_actor["lobby_code_message_ids"]["md5"].is_string());

        let request_actor_rows = export["tables"]
            ["scrim_match_requests.released_by_user_id.redacted"]
            .as_array()
            .expect("match request actor export");
        assert_eq!(request_actor_rows.len(), 1);
        let request_actor = &request_actor_rows[0];
        assert_eq!(
            request_actor["released_by_user_id"],
            serde_json::json!("42")
        );
        assert_eq!(
            request_actor["released_by_display_name"],
            serde_json::json!("DeleteMe")
        );
        assert!(request_actor["slot_options"]["md5"].is_string());
        assert!(request_actor["released_slot"]["md5"].is_string());
        assert!(request_actor["team_query_message_ids"]["md5"].is_string());
        assert!(request_actor["team_status_message_ids"]["md5"].is_string());
        assert_eq!(
            request_actor["override_reason"],
            serde_json::json!("redacted")
        );
        assert_eq!(
            request_actor["status_message_last_error"],
            serde_json::json!("redacted")
        );

        let correction_rows = export["tables"]["scrim_matches.lobby_code_corrections"]
            .as_array()
            .expect("lobby code corrections subject export");
        assert_eq!(correction_rows.len(), 1);
        let correction_row = correction_rows[0]
            .as_object()
            .expect("lobby corrections hash-only row");
        assert_eq!(
            correction_row.len(),
            2,
            "per-field lobby corrections DSAR must expose only id plus hash"
        );
        assert_eq!(correction_row["id"], serde_json::json!(936003));
        assert!(correction_row["lobby_code_corrections"]["md5"].is_string());
        let correction_export = serde_json::to_string(correction_rows).expect("correction export");
        for marker in [
            "DL-8Q4K",
            "DL-7N2P",
            "DL-5J1L",
            "DL-2X9R",
            "TARGET_CORRECTION_NOTE",
            "FOREIGN_CORRECTION_NOTE",
            "DeleteMe",
            "OtherUser",
        ] {
            assert!(
                !correction_export.contains(marker),
                "hash-only lobby corrections DSAR leaked {marker}: {correction_export}"
            );
        }

        let actor_export = serde_json::to_string(&serde_json::json!({
            "match": match_actor_rows,
            "request": request_actor_rows,
        }))
        .expect("actor export json");
        for marker in [
            "SECRET_WHEN_TEXT",
            "SECRET_PARTY_ID",
            "SECRET_LOBBY_CODE",
            "RAW_MATCH_RESULT_SECRET",
            "QUERY_MESSAGE_ID_LEAK",
            "STATUS_MESSAGE_ID_LEAK",
            "SLOT_OPTION_SECRET",
            "RELEASED_SLOT_SECRET",
            "DL-8Q4K",
            "DL-7N2P",
            "DL-5J1L",
            "DL-2X9R",
            "TARGET_CORRECTION_NOTE",
            "FOREIGN_CORRECTION_NOTE",
            "override 42",
            "status error 42",
        ] {
            assert!(
                !actor_export.contains(marker),
                "unsafe actor DSAR field leaked: {marker}: {actor_export}"
            );
        }

        let summary = delete_user_data(pool, 42, "actor-match-test".to_string(), 2_000)
            .await
            .expect("actor privacy delete");
        assert_eq!(
            summary
                .counts
                .get("scrim_matches.lobby_code_source_user_id.redacted"),
            Some(&1)
        );
        assert!(summary.counts.get("scrim_matches.lobby_code_corrections") >= Some(&1));
        assert_eq!(
            summary
                .counts
                .get("scrim_match_requests.released_by_user_id.redacted"),
            Some(&1)
        );
        assert_eq!(
            summary.counts.get("scrim_match_requests.override_reason"),
            Some(&1)
        );
        assert_eq!(
            summary
                .counts
                .get("scrim_match_requests.status_message_last_error"),
            Some(&1)
        );

        let stored_corrections: Value = sqlx::query_scalar(
            "SELECT lobby_code_corrections FROM scrim.matches WHERE id = 936003",
        )
        .fetch_one(pool)
        .await
        .expect("stored corrections after delete");
        let stored_corrections =
            serde_json::to_string(&stored_corrections).expect("corrections json");
        assert!(!stored_corrections.contains("\"42\""));
        assert!(!stored_corrections.contains("DeleteMe"));
        assert!(stored_corrections.contains("DL-8Q4K"));
        assert!(stored_corrections.contains("DL-7N2P"));
        assert!(stored_corrections.contains("TARGET_CORRECTION_NOTE"));
        assert!(stored_corrections.contains("OtherUser"));
        assert!(stored_corrections.contains("DL-5J1L"));
        assert!(stored_corrections.contains("DL-2X9R"));
        assert!(stored_corrections.contains("FOREIGN_CORRECTION_NOTE"));

        let request_after: (String, String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT released_by_user_id,
                    released_by_display_name,
                    override_reason,
                    status_message_last_error
               FROM scrim.match_requests
              WHERE id = 936010",
        )
        .fetch_one(pool)
        .await
        .expect("request after delete");
        assert_eq!(request_after.0, "redacted");
        assert_eq!(request_after.1, "redacted");
        for text in [
            request_after.2.as_deref().expect("override reason"),
            request_after
                .3
                .as_deref()
                .expect("status message last error"),
        ] {
            assert!(!text.contains(" 42 "), "Discord target survived: {text}");
            assert!(
                !text.contains(target_steam_id64),
                "Steam target survived: {text}"
            );
            assert!(
                !text.contains(target_account_id),
                "account target survived: {text}"
            );
            assert!(text.contains("foreign 99"));
            assert!(text.contains("4242"));
            assert!(text.contains("user42"));
        }
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

    /// Legt die Kopien so an, wie der Rueckweg von Migration 2026081301 sie
    /// hinterlaesst: `LIKE` auf die Live-Tabelle plus die Frist-Spalte. `LIKE`
    /// ohne `INCLUDING DEFAULTS` uebernimmt keine Defaults — deshalb nennen die
    /// Inserts unten jede NOT-NULL-Spalte. Die echte Anlage steht in
    /// dl-central-db/rollbacks/ und wird von fresh_migrations_schema.rs gefahren.
    async fn lege_rollback_kopien_an(pool: &PgPool) {
        for tabelle in [
            "discord_role_connection_tokens",
            "discord_role_connection_sync_state",
        ] {
            sqlx::query(&format!(
                "CREATE TABLE core.{tabelle}_rollback_backup (LIKE core.{tabelle})"
            ))
            .execute(pool)
            .await
            .expect("kopie anlegen");
            sqlx::query(&format!(
                "ALTER TABLE core.{tabelle}_rollback_backup
                 ADD COLUMN rollback_expires_at TIMESTAMPTZ NOT NULL
                 DEFAULT (now() + INTERVAL '180 days')"
            ))
            .execute(pool)
            .await
            .expect("frist-spalte");
        }
    }

    #[tokio::test]
    async fn rollback_kopien_retention_raeumt_nur_die_abgelaufene_zeile_je_tabelle() {
        let db = mk_db().await;
        let now = Utc::now();

        // Fehlen die Kopien — der Normalfall ohne Rollback —, darf der Job nicht
        // an einer fehlenden Relation scheitern und nichts melden.
        let ohne_kopien = purge_expired_role_connection_rollback_backups(db.pool(), now)
            .await
            .expect("purge ohne kopien");
        assert!(ohne_kopien.is_empty(), "{ohne_kopien:?}");

        lege_rollback_kopien_an(db.pool()).await;

        // Die zweite Zeile ist die Falle: ihr OAuth-`expires_at` liegt Jahre in
        // der Vergangenheit (invalidiertes Token), die Aufbewahrungsfrist aber
        // nicht. Haengt die Retention am falschen Spaltennamen, loescht sie hier
        // die einzige Kopie der Creator-Tokens.
        sqlx::query(
            r#"
            INSERT INTO core.discord_role_connection_tokens_rollback_backup(
                discord_id, access_token, refresh_token, token_type, scope, expires_at,
                token_version, active, created_at, updated_at, provider, rollback_expires_at
            )
            VALUES
              (9970001, '\x01'::bytea, '\x02'::bytea, 'Bearer', 'identify', $1, 1, TRUE, $1, $1,
               'creator', $2),
              (9970002, '\x03'::bytea, '\x04'::bytea, 'Bearer', 'identify', $3, 1, TRUE, $1, $1,
               'creator', $4)
            "#,
        )
        .bind(now)
        .bind(now - Duration::seconds(1))
        .bind(now - Duration::days(900))
        .bind(now + Duration::days(180))
        .execute(db.pool())
        .await
        .expect("token-kopien");

        sqlx::query(
            r#"
            INSERT INTO core.discord_role_connection_sync_state_rollback_backup(
                discord_id, pending, reason, attempts, next_attempt_at, created_at, updated_at,
                provider, rollback_expires_at
            )
            VALUES
              (9970001, TRUE, 'creator_reconcile', 0, $1, $1, $1, 'creator', $2),
              (9970002, TRUE, 'creator_reconcile', 0, $1, $1, $1, 'creator', $3)
            "#,
        )
        .bind(now)
        .bind(now - Duration::seconds(1))
        .bind(now + Duration::days(180))
        .execute(db.pool())
        .await
        .expect("sync-kopien");

        let deleted = purge_expired_role_connection_rollback_backups(db.pool(), now)
            .await
            .expect("purge");
        assert_eq!(
            deleted,
            vec![
                ("core.discord_role_connection_tokens_rollback_backup", 1),
                ("core.discord_role_connection_sync_state_rollback_backup", 1),
            ],
            "Zaehlung je Tabelle getrennt, in der Reihenfolge der Konstante"
        );

        for relation in ROLE_CONNECTION_ROLLBACK_BACKUP_RELS {
            let uebrig: Vec<i64> = sqlx::query_scalar(&format!(
                "SELECT discord_id FROM {relation} ORDER BY discord_id"
            ))
            .fetch_all(db.pool())
            .await
            .expect("uebrige zeilen");
            assert_eq!(uebrig, vec![9970002], "{relation}");
        }

        // Zweiter Lauf ohne neue Frist: nichts mehr faellig, aber die Meldung
        // bleibt je Tabelle bestehen (0), damit der Job nicht stumm aussetzt.
        let zweiter = purge_expired_role_connection_rollback_backups(db.pool(), now)
            .await
            .expect("zweiter purge");
        assert_eq!(
            zweiter,
            vec![
                ("core.discord_role_connection_tokens_rollback_backup", 0),
                ("core.discord_role_connection_sync_state_rollback_backup", 0),
            ]
        );
    }

    #[tokio::test]
    async fn delete_rollbackt_purge_grabstein_und_redactions_bei_spaetem_fehler() {
        let db = mk_db().await;
        let now = Utc::now();
        sqlx::query("INSERT INTO core.users(discord_id) VALUES (42)")
            .execute(db.pool())
            .await
            .expect("core user");
        sqlx::query(
            "CREATE TABLE privacy_force_late_failure(
                 user_id BIGINT NOT NULL REFERENCES core.users(discord_id) ON DELETE RESTRICT
             )",
        )
        .execute(db.pool())
        .await
        .expect("force late failure table");
        sqlx::query("INSERT INTO privacy_force_late_failure(user_id) VALUES (42)")
            .execute(db.pool())
            .await
            .expect("force late failure row");
        sqlx::query(
            r#"
            INSERT INTO server_config.rollback_exports(
                guild_id, created_by_user_id, artifact_hash, artifact_json, metadata, expires_at
            )
            VALUES (1, 42, 'atomic-old', $1::text::jsonb, '{}'::jsonb, $2)
            "#,
        )
        .bind(r#"{"member_role_assignments":[{"member_id":42,"role_ids":[1]}]}"#)
        .bind(now - Duration::seconds(1))
        .execute(db.pool())
        .await
        .expect("insert expired rollback export");
        sqlx::query(
            "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload)
             VALUES ('privacy_atomic', 'atomic:redaction', $1, '{\"id\":\"42\"}'::jsonb)",
        )
        .bind(vec![7_u8; 32])
        .execute(db.pool())
        .await
        .expect("insert redaction row");
        sqlx::query(
            "INSERT INTO scrim.teams(id, name, created_at)
             VALUES (606001, 'Atomic A', now()), (606002, 'Atomic B', now())",
        )
        .execute(db.pool())
        .await
        .expect("insert atomic teams");
        sqlx::query(
            "INSERT INTO scrim.matches(id, team_a_id, team_b_id, status, created_at)
             VALUES (606003, 606001, 606002, 'scheduled', now())",
        )
        .execute(db.pool())
        .await
        .expect("insert atomic match");
        sqlx::query(
            "UPDATE scrim.matches
                SET result_json = '{\"account_id\":\"42\"}'::jsonb
              WHERE id = 606003",
        )
        .execute(db.pool())
        .await
        .expect("insert atomic legacy result json");
        let result_ref_id: i64 = sqlx::query_scalar(
            "INSERT INTO scrim.match_result_refs(
                 match_id, steam_match_id, source_user_id, source_display_name,
                 fetch_status, winner_team_id, raw_result_json, normalized_result_json,
                 validation_status, last_error
             ) VALUES (
                 606003, 606311, '99', 'Foreign', 'fetched', 606001,
                 '{\"account_id\":\"42\"}'::jsonb,
                 '{\"account_id\":\"42\"}'::jsonb,
                 'valid',
                 'target 42 must roll back'
             ) RETURNING id",
        )
        .fetch_one(db.pool())
        .await
        .expect("insert atomic result ref");

        let result =
            delete_user_data(db.pool(), 42, "atomic-test".to_string(), now.timestamp()).await;
        assert!(result.is_err(), "forced late FK failure must abort erasure");

        let rollback_export_still_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM server_config.rollback_exports WHERE artifact_hash = 'atomic-old')",
        )
        .fetch_one(db.pool())
        .await
        .expect("rollback export still exists");
        let tombstone_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core.user_privacy WHERE user_id = 42)")
                .fetch_one(db.pool())
                .await
                .expect("privacy tombstone rollback");
        let payload_still_contains_target: bool = sqlx::query_scalar(
            "SELECT scrim.jsonb_contains_user_ref(payload, '42')
               FROM scrim.command_receipts
              WHERE command_scope = 'privacy_atomic'",
        )
        .fetch_one(db.pool())
        .await
        .expect("payload rollback");
        assert!(rollback_export_still_exists);
        assert!(!tombstone_exists);
        assert!(payload_still_contains_target);
        let result_json_still_contains_target: bool = sqlx::query_scalar(
            "SELECT
                 (SELECT scrim.jsonb_contains_user_ref(result_json, '42')
                    FROM scrim.matches
                   WHERE id = 606003)
                 AND
                 (SELECT scrim.jsonb_contains_user_ref(raw_result_json, '42')
                    FROM scrim.match_result_refs
                   WHERE id = $1)
                 AND
                 (SELECT scrim.jsonb_contains_user_ref(normalized_result_json, '42')
                    FROM scrim.match_result_refs
                   WHERE id = $1)",
        )
        .bind(result_ref_id)
        .fetch_one(db.pool())
        .await
        .expect("result json rollback");
        let last_error_still_contains_target: bool = sqlx::query_scalar(
            "SELECT last_error LIKE '%42%'
               FROM scrim.match_result_refs
              WHERE id = $1",
        )
        .bind(result_ref_id)
        .fetch_one(db.pool())
        .await
        .expect("last error rollback");
        assert!(result_json_still_contains_target);
        assert!(last_error_still_contains_target);
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
        sqlx::query(
            r#"
            INSERT INTO bot.kv_store(ns,k,v) VALUES
              ('ai_onboarding:sessions','42','{}'),
              ('ai_onboarding:persistent_views','viewA','{"user_id":42}'),
              ('ai_onboarding:persistent_views','viewB','{"user_id":99}'),
              ('voice_nudge_done','42','1'),
              ('native_onboarding:completed','1:42','{"guild_id":1,"user_id":42}'),
              ('onboarding_bridge_dm','42','dm_done'),
              ('onboarding_tour','42','{"schema":1,"status":"active"}')
            "#,
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
        assert_eq!(s.counts.get("kv_onboarding_dm").copied(), Some(2));
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
