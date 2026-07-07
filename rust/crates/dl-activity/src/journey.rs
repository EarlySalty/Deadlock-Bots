//! Journey-State-Machine, Metadaten-Ingestion und Phase-1-Analytics.
//!
//! Grundsatz: Message-Inhalte werden nie persistiert. Der Message-Pfad speichert
//! nur wer/wo/wann/Laenge/Anhang/Reply. Rohereignisse werden nach fester Frist
//! in anonyme Tagesaggregate verdichtet und geloescht.

use std::collections::HashSet;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::db::{
    discord_id_to_i64, i64_to_i32, i64_to_u64, utc_from_unix_seconds, validate_json_text,
    ActivityDbResult,
};

/// Startwert aus dem Onboarding-Konzept §5.2: Roh-Metadaten werden maximal
/// 180 Tage nutzerbezogen gehalten und danach anonymen Tagesaggregaten
/// zugeschlagen.
pub const RAW_EVENT_RETENTION_DAYS: i64 = 180;
pub const RETENTION_JOB_INTERVAL: StdDuration = StdDuration::from_secs(24 * 3600);
pub const MAX_INTERACTION_ROUTE_LEN: usize = 96;
const MAX_INTERACTION_ROUTE_SQL_LEN: i32 = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    Human,
    Bot,
    System,
}

impl ActorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Bot => "bot",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JourneyEventType {
    Join,
    ScreeningCompleted,
    NativeOnboardingCompleted,
    WeicheChanged,
    SteamLink,
    InviteFriendRequestSent,
    InviteFriendRequestAccepted,
    InviteSent,
    InviteAccepted,
    FirstMessage,
    FirstVoice,
    FirstMatch,
    SquadJoin,
    OptOut,
    StreamerContactActivation,
    D7Activity,
    D14Activity,
    ConciergeT0Sent,
    ConciergeReply,
    ConciergeTourDone,
    SteckbriefPosted,
    NudgeSent,
    PateOffered,
    ConciergeOptedOut,
    CongratsSent,
}

impl JourneyEventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::ScreeningCompleted => "screening_completed",
            Self::NativeOnboardingCompleted => "native_onboarding_completed",
            Self::WeicheChanged => "weiche_changed",
            Self::SteamLink => "steam_link",
            Self::InviteFriendRequestSent => "invite_friend_request_sent",
            Self::InviteFriendRequestAccepted => "invite_friend_request_accepted",
            Self::InviteSent => "invite_sent",
            Self::InviteAccepted => "invite_accepted",
            Self::FirstMessage => "first_message",
            Self::FirstVoice => "first_voice",
            Self::FirstMatch => "first_match",
            Self::SquadJoin => "squad_join",
            Self::OptOut => "opt_out",
            Self::StreamerContactActivation => "streamer_contact_activation",
            Self::D7Activity => "d7_activity",
            Self::D14Activity => "d14_activity",
            Self::ConciergeT0Sent => "concierge_t0_sent",
            Self::ConciergeReply => "concierge_reply",
            Self::ConciergeTourDone => "concierge_tour_done",
            Self::SteckbriefPosted => "steckbrief_posted",
            Self::NudgeSent => "nudge_sent",
            Self::PateOffered => "pate_offered",
            Self::ConciergeOptedOut => "opted_out",
            Self::CongratsSent => "congrats_sent",
        }
    }

    pub const fn all() -> &'static [Self] {
        &[
            Self::Join,
            Self::ScreeningCompleted,
            Self::NativeOnboardingCompleted,
            Self::WeicheChanged,
            Self::SteamLink,
            Self::InviteFriendRequestSent,
            Self::InviteFriendRequestAccepted,
            Self::InviteSent,
            Self::InviteAccepted,
            Self::FirstMessage,
            Self::FirstVoice,
            Self::FirstMatch,
            Self::SquadJoin,
            Self::OptOut,
            Self::StreamerContactActivation,
            Self::D7Activity,
            Self::D14Activity,
            Self::ConciergeT0Sent,
            Self::ConciergeReply,
            Self::ConciergeTourDone,
            Self::SteckbriefPosted,
            Self::NudgeSent,
            Self::PateOffered,
            Self::ConciergeOptedOut,
            Self::CongratsSent,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct JourneyEventInput {
    pub user_id: u64,
    pub guild_id: u64,
    pub event_type: JourneyEventType,
    pub event_source: &'static str,
    pub actor_kind: Option<ActorKind>,
    pub occurred_at: DateTime<Utc>,
    pub channel_id: Option<u64>,
    pub message_id: Option<u64>,
    pub metadata: Value,
}

impl JourneyEventInput {
    pub fn new(
        user_id: u64,
        guild_id: u64,
        event_type: JourneyEventType,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        Self {
            user_id,
            guild_id,
            event_type,
            event_source: "manual",
            actor_kind: None,
            occurred_at,
            channel_id: None,
            message_id: None,
            metadata: Value::Object(Default::default()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivationSummary {
    pub joined: i64,
    pub activated: i64,
    pub activation_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionSummary {
    pub joined: i64,
    pub d7_retained: i64,
    pub d14_retained: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunnelSummary {
    pub weiche_completed: i64,
    pub steam_linked: i64,
    pub invite_started: i64,
    pub invite_completed: i64,
    pub first_message: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NullActivityUser {
    pub user_id: u64,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawRetentionRun {
    pub message_rows_deleted: i64,
    pub voice_rows_deleted: i64,
    pub interaction_rows_deleted: i64,
    pub presence_rows_deleted: i64,
    pub journey_rows_deleted: i64,
    pub stale_open_voice_sessions_deleted: i64,
}

fn optional_discord_id(value: Option<u64>, field: &'static str) -> ActivityDbResult<Option<i64>> {
    value
        .map(|value| discord_id_to_i64(value, field))
        .transpose()
}

fn metadata_text(value: &Value) -> ActivityDbResult<String> {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    validate_json_text(raw, "metadata", "{}")
}

fn metadata_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn route_segment_looks_like_id(segment: &str) -> bool {
    !segment.is_empty()
        && (segment.bytes().all(|byte| byte.is_ascii_digit()) || segment.chars().count() > 15)
}

/// Sanitisiert Discord-Interaction-custom_ids vor jeder Persistierung.
///
/// Gespeichert wird nur ein stabiles Routenmuster: `:`-Segmente, die wie IDs
/// aussehen (rein numerisch oder laenger als 15 Zeichen), werden zu `*`.
/// Danach wird hart gekappt, damit weder Roh-IDs noch ueberlange custom_ids in
/// Raw-Events oder Tagesaggregaten landen.
pub fn sanitize_interaction_route(route: &str) -> String {
    let sanitized = route
        .split(':')
        .map(|segment| {
            if route_segment_looks_like_id(segment) {
                "*"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join(":");
    truncate_chars(&sanitized, MAX_INTERACTION_ROUTE_LEN)
}

fn sanitized_join_metadata(metadata: Value) -> Value {
    let mut out = serde_json::Map::new();
    let Value::Object(input) = metadata else {
        return Value::Object(out);
    };
    for key in [
        "join_source_bucket",
        "join_source_kind",
        "join_source_label",
        "join_source_confidence",
        "join_source_reason",
        "invite_code",
        "inviter_bot",
        "invite_channel_id",
        "twitch_streamer_login",
    ] {
        if let Some(value) = input.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(out)
}

async fn is_opted_out_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT 1 FROM core.user_privacy WHERE user_id = $1 AND opted_out = TRUE LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

#[allow(clippy::too_many_arguments)]
async fn insert_journey_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
    event_type: JourneyEventType,
    event_source: &str,
    actor_kind: Option<&str>,
    occurred_at: DateTime<Utc>,
    channel_id: Option<i64>,
    message_id: Option<i64>,
    metadata_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.journey_events(
            user_id, guild_id, event_type, event_source, actor_kind,
            occurred_at, channel_id, message_id, metadata
        )
        VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9::text::jsonb)
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(event_type.as_str())
    .bind(event_source)
    .bind(actor_kind)
    .bind(occurred_at)
    .bind(channel_id)
    .bind(message_id)
    .bind(metadata_json)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_first_journey_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
    event_type: JourneyEventType,
    occurred_at: DateTime<Utc>,
    channel_id: Option<i64>,
    message_id: Option<i64>,
    metadata_json: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.journey_events(
            user_id, guild_id, event_type, event_source, actor_kind,
            occurred_at, channel_id, message_id, metadata
        )
        VALUES($1, $2, $3, 'gateway', NULL, $4, $5, $6, $7::text::jsonb)
        ON CONFLICT (guild_id, user_id, event_type)
            WHERE event_type IN ('first_message', 'first_voice')
        DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(event_type.as_str())
    .bind(occurred_at)
    .bind(channel_id)
    .bind(message_id)
    .bind(metadata_json)
    .execute(&mut **tx)
    .await
    .map(|result| result.rows_affected() > 0)
}

#[allow(clippy::too_many_arguments)]
async fn upsert_journey_state_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
    event_type: JourneyEventType,
    occurred_at: DateTime<Utc>,
    actor_kind: Option<&str>,
    weiche_choice: Option<&str>,
    metadata_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.journey_user_state(
            user_id, guild_id, joined_at, screening_completed_at,
            native_onboarding_completed_at, weiche_choice, steam_linked_at,
            invite_friend_request_sent_at, invite_friend_request_accepted_at,
            invite_sent_at, invite_accepted_at, invite_actor_kind, first_message_at,
            first_voice_at, first_match_at, squad_joined_at,
            streamer_contact_activated_at, opt_out_at, last_event_at,
            last_event_type, metadata, updated_at
        )
        VALUES(
            $1,
            $2,
            CASE WHEN $3 = 'join' THEN $4 END,
            CASE WHEN $3 = 'screening_completed' THEN $4 END,
            CASE WHEN $3 = 'native_onboarding_completed' THEN $4 END,
            CASE WHEN $3 IN ('native_onboarding_completed', 'weiche_changed') THEN $6 END,
            CASE WHEN $3 = 'steam_link' THEN $4 END,
            CASE WHEN $3 = 'invite_friend_request_sent' THEN $4 END,
            CASE WHEN $3 = 'invite_friend_request_accepted' THEN $4 END,
            CASE WHEN $3 = 'invite_sent' THEN $4 END,
            CASE WHEN $3 = 'invite_accepted' THEN $4 END,
            CASE WHEN $3 IN (
                'invite_friend_request_sent',
                'invite_friend_request_accepted',
                'invite_sent',
                'invite_accepted'
            ) THEN $5 END,
            CASE WHEN $3 = 'first_message' THEN $4 END,
            CASE WHEN $3 = 'first_voice' THEN $4 END,
            CASE WHEN $3 = 'first_match' THEN $4 END,
            CASE WHEN $3 = 'squad_join' THEN $4 END,
            CASE WHEN $3 = 'streamer_contact_activation' THEN $4 END,
            CASE WHEN $3 = 'opt_out' THEN $4 END,
            $4,
            $3,
            $7::text::jsonb,
            now()
        )
        ON CONFLICT(user_id, guild_id) DO UPDATE SET
            joined_at = CASE
                WHEN $3 = 'join' THEN COALESCE(activity.journey_user_state.joined_at, EXCLUDED.joined_at)
                ELSE activity.journey_user_state.joined_at
            END,
            screening_completed_at = CASE
                WHEN $3 = 'screening_completed' THEN COALESCE(activity.journey_user_state.screening_completed_at, EXCLUDED.screening_completed_at)
                ELSE activity.journey_user_state.screening_completed_at
            END,
            native_onboarding_completed_at = CASE
                WHEN $3 = 'native_onboarding_completed' THEN COALESCE(activity.journey_user_state.native_onboarding_completed_at, EXCLUDED.native_onboarding_completed_at)
                ELSE activity.journey_user_state.native_onboarding_completed_at
            END,
            weiche_choice = CASE
                WHEN $3 IN ('native_onboarding_completed', 'weiche_changed') THEN COALESCE($6, activity.journey_user_state.weiche_choice)
                ELSE activity.journey_user_state.weiche_choice
            END,
            steam_linked_at = CASE
                WHEN $3 = 'steam_link' THEN COALESCE(activity.journey_user_state.steam_linked_at, EXCLUDED.steam_linked_at)
                ELSE activity.journey_user_state.steam_linked_at
            END,
            invite_friend_request_sent_at = CASE
                WHEN $3 = 'invite_friend_request_sent' THEN COALESCE(activity.journey_user_state.invite_friend_request_sent_at, EXCLUDED.invite_friend_request_sent_at)
                ELSE activity.journey_user_state.invite_friend_request_sent_at
            END,
            invite_friend_request_accepted_at = CASE
                WHEN $3 = 'invite_friend_request_accepted' THEN COALESCE(activity.journey_user_state.invite_friend_request_accepted_at, EXCLUDED.invite_friend_request_accepted_at)
                ELSE activity.journey_user_state.invite_friend_request_accepted_at
            END,
            invite_sent_at = CASE
                WHEN $3 = 'invite_sent' THEN COALESCE(activity.journey_user_state.invite_sent_at, EXCLUDED.invite_sent_at)
                ELSE activity.journey_user_state.invite_sent_at
            END,
            invite_accepted_at = CASE
                WHEN $3 = 'invite_accepted' THEN COALESCE(activity.journey_user_state.invite_accepted_at, EXCLUDED.invite_accepted_at)
                ELSE activity.journey_user_state.invite_accepted_at
            END,
            invite_actor_kind = CASE
                WHEN $3 IN (
                    'invite_friend_request_sent',
                    'invite_friend_request_accepted',
                    'invite_sent',
                    'invite_accepted'
                ) THEN COALESCE($5, activity.journey_user_state.invite_actor_kind)
                ELSE activity.journey_user_state.invite_actor_kind
            END,
            first_message_at = CASE
                WHEN $3 = 'first_message' THEN COALESCE(activity.journey_user_state.first_message_at, EXCLUDED.first_message_at)
                ELSE activity.journey_user_state.first_message_at
            END,
            first_voice_at = CASE
                WHEN $3 = 'first_voice' THEN COALESCE(activity.journey_user_state.first_voice_at, EXCLUDED.first_voice_at)
                ELSE activity.journey_user_state.first_voice_at
            END,
            first_match_at = CASE
                WHEN $3 = 'first_match' THEN COALESCE(activity.journey_user_state.first_match_at, EXCLUDED.first_match_at)
                ELSE activity.journey_user_state.first_match_at
            END,
            squad_joined_at = CASE
                WHEN $3 = 'squad_join' THEN COALESCE(activity.journey_user_state.squad_joined_at, EXCLUDED.squad_joined_at)
                ELSE activity.journey_user_state.squad_joined_at
            END,
            streamer_contact_activated_at = CASE
                WHEN $3 = 'streamer_contact_activation' THEN COALESCE(activity.journey_user_state.streamer_contact_activated_at, EXCLUDED.streamer_contact_activated_at)
                ELSE activity.journey_user_state.streamer_contact_activated_at
            END,
            opt_out_at = CASE
                WHEN $3 = 'opt_out' THEN COALESCE(activity.journey_user_state.opt_out_at, EXCLUDED.opt_out_at)
                ELSE activity.journey_user_state.opt_out_at
            END,
            last_event_at = CASE
                WHEN activity.journey_user_state.last_event_at IS NULL
                  OR EXCLUDED.last_event_at >= activity.journey_user_state.last_event_at
                THEN EXCLUDED.last_event_at
                ELSE activity.journey_user_state.last_event_at
            END,
            last_event_type = CASE
                WHEN activity.journey_user_state.last_event_at IS NULL
                  OR EXCLUDED.last_event_at >= activity.journey_user_state.last_event_at
                THEN EXCLUDED.last_event_type
                ELSE activity.journey_user_state.last_event_type
            END,
            metadata = activity.journey_user_state.metadata || EXCLUDED.metadata,
            updated_at = now()
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(event_type.as_str())
    .bind(occurred_at)
    .bind(actor_kind)
    .bind(weiche_choice)
    .bind(metadata_json)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn record_first_journey_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
    event_type: JourneyEventType,
    occurred_at: DateTime<Utc>,
    channel_id: Option<i64>,
    message_id: Option<i64>,
    metadata_json: &str,
) -> Result<bool, sqlx::Error> {
    if !matches!(
        event_type,
        JourneyEventType::FirstMessage | JourneyEventType::FirstVoice
    ) {
        return Ok(false);
    }
    let inserted = insert_first_journey_event_tx(
        tx,
        user_id,
        guild_id,
        event_type,
        occurred_at,
        channel_id,
        message_id,
        metadata_json,
    )
    .await?;
    if !inserted {
        return Ok(false);
    }
    upsert_journey_state_tx(
        tx,
        user_id,
        guild_id,
        event_type,
        occurred_at,
        None,
        None,
        metadata_json,
    )
    .await?;
    Ok(true)
}

async fn mark_first_interaction_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
    occurred_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.journey_user_state(
            user_id, guild_id, first_interaction_at, last_event_at,
            last_event_type, updated_at
        )
        VALUES($1, $2, $3, $3, 'interaction', now())
        ON CONFLICT(user_id, guild_id) DO UPDATE SET
            first_interaction_at = COALESCE(
                activity.journey_user_state.first_interaction_at,
                EXCLUDED.first_interaction_at
            ),
            last_event_at = CASE
                WHEN activity.journey_user_state.last_event_at IS NULL
                  OR EXCLUDED.last_event_at >= activity.journey_user_state.last_event_at
                THEN EXCLUDED.last_event_at
                ELSE activity.journey_user_state.last_event_at
            END,
            last_event_type = CASE
                WHEN activity.journey_user_state.last_event_at IS NULL
                  OR EXCLUDED.last_event_at >= activity.journey_user_state.last_event_at
                THEN EXCLUDED.last_event_type
                ELSE activity.journey_user_state.last_event_type
            END,
            updated_at = now()
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(occurred_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn record_journey_event(
    pool: &PgPool,
    input: JourneyEventInput,
) -> ActivityDbResult<bool> {
    let mut tx = pool.begin().await?;
    let recorded = record_journey_event_tx(&mut tx, input).await?;
    tx.commit().await?;
    Ok(recorded)
}

pub async fn record_journey_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: JourneyEventInput,
) -> ActivityDbResult<bool> {
    let user_id = discord_id_to_i64(input.user_id, "journey_events.user_id")?;
    let guild_id = discord_id_to_i64(input.guild_id, "journey_events.guild_id")?;
    let channel_id = optional_discord_id(input.channel_id, "journey_events.channel_id")?;
    let message_id = optional_discord_id(input.message_id, "journey_events.message_id")?;
    let actor_kind = input.actor_kind.map(ActorKind::as_str);
    let metadata_json = metadata_text(&input.metadata)?;
    let weiche_choice = metadata_string(&input.metadata, &["weiche_choice", "choice"]);

    if is_opted_out_tx(tx, user_id).await? {
        return Ok(false);
    }
    insert_journey_event_tx(
        tx,
        user_id,
        guild_id,
        input.event_type,
        input.event_source,
        actor_kind,
        input.occurred_at,
        channel_id,
        message_id,
        &metadata_json,
    )
    .await?;
    upsert_journey_state_tx(
        tx,
        user_id,
        guild_id,
        input.event_type,
        input.occurred_at,
        actor_kind,
        weiche_choice.as_deref(),
        &metadata_json,
    )
    .await?;
    Ok(true)
}

/// Oeffentliche Journey-Record-API fuer Produzenten ausserhalb des Gateway-
/// Ingestors. Sie respektiert den Privacy-Opt-out und persistiert nur
/// Metadaten, die der Aufrufer explizit uebergibt.
///
/// Externe Produzenten sollen keine eigenen Inserts in `activity.journey_*`
/// schreiben, sondern diesen Einstieg oder die spezialisierten Wrapper unten
/// nutzen.
pub async fn record_external_journey_event(
    pool: &PgPool,
    input: JourneyEventInput,
) -> ActivityDbResult<bool> {
    record_journey_event(pool, input).await
}

/// Hook fuer spaeteres Cross-Repo-Wiring: im Steam-Bot-Repo direkt nach einer
/// erfolgreich verifizierten Steam-Verknuepfung aufrufen.
pub async fn record_steam_link_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input =
        JourneyEventInput::new(user_id, guild_id, JourneyEventType::SteamLink, occurred_at);
    input.event_source = "steam_bot";
    record_external_journey_event(pool, input).await
}

/// Hook fuer Phase 3: im Invite-Funnel aufrufen, wenn die Steam-
/// Freundschaftsanfrage abgeschickt wurde.
pub async fn record_invite_friend_request_sent_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    actor_kind: ActorKind,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input = JourneyEventInput::new(
        user_id,
        guild_id,
        JourneyEventType::InviteFriendRequestSent,
        occurred_at,
    );
    input.event_source = "invite_funnel";
    input.actor_kind = Some(actor_kind);
    record_external_journey_event(pool, input).await
}

/// Hook fuer Phase 3: im Invite-Funnel aufrufen, wenn die Steam-
/// Freundschaftsanfrage angenommen wurde.
pub async fn record_invite_friend_request_accepted_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    actor_kind: ActorKind,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input = JourneyEventInput::new(
        user_id,
        guild_id,
        JourneyEventType::InviteFriendRequestAccepted,
        occurred_at,
    );
    input.event_source = "invite_funnel";
    input.actor_kind = Some(actor_kind);
    record_external_journey_event(pool, input).await
}

/// Hook fuer Phase 3: im Invite-Funnel aufrufen, wenn ein Playtest-Invite
/// verschickt wurde.
pub async fn record_invite_sent_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    actor_kind: ActorKind,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input =
        JourneyEventInput::new(user_id, guild_id, JourneyEventType::InviteSent, occurred_at);
    input.event_source = "invite_funnel";
    input.actor_kind = Some(actor_kind);
    record_external_journey_event(pool, input).await
}

/// Hook fuer Phase 3: im Invite-Funnel aufrufen, wenn der verschickte Invite
/// angenommen wurde.
pub async fn record_invite_accepted_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    actor_kind: ActorKind,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input = JourneyEventInput::new(
        user_id,
        guild_id,
        JourneyEventType::InviteAccepted,
        occurred_at,
    );
    input.event_source = "invite_funnel";
    input.actor_kind = Some(actor_kind);
    record_external_journey_event(pool, input).await
}

/// Hook fuer Phase 5: im Match-/Presence-Ingestor aufrufen, sobald der erste
/// echte Deadlock-Match fuer den Discord-User nachweisbar ist.
pub async fn record_first_match_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input =
        JourneyEventInput::new(user_id, guild_id, JourneyEventType::FirstMatch, occurred_at);
    input.event_source = "match_ingest";
    record_external_journey_event(pool, input).await
}

/// Hook fuer Phase 5: im Squad-/LFG-Modul aufrufen, wenn ein User erstmals
/// einem Squad beitritt.
pub async fn record_squad_join_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input =
        JourneyEventInput::new(user_id, guild_id, JourneyEventType::SquadJoin, occurred_at);
    input.event_source = "squad_module";
    record_external_journey_event(pool, input).await
}

/// Hook fuer das spaetere Streamer-Kontakt-Wiring: im Steam-/Twitch-Bot-Fluss
/// aufrufen, sobald ein Streamer-Kontakt aktiviert wurde.
pub async fn record_streamer_contact_activation_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input = JourneyEventInput::new(
        user_id,
        guild_id,
        JourneyEventType::StreamerContactActivation,
        occurred_at,
    );
    input.event_source = "streamer_contact";
    record_external_journey_event(pool, input).await
}

/// Native Discord-/Bot-Onboarding abgeschlossen. Wird im aktuellen Repo ueber
/// Rollen- bzw. Complete-Role-Deltas verdrahtet.
pub async fn record_native_onboarding_completed_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    source: &'static str,
    occurred_at: DateTime<Utc>,
    metadata: Value,
) -> ActivityDbResult<bool> {
    let mut input = JourneyEventInput::new(
        user_id,
        guild_id,
        JourneyEventType::NativeOnboardingCompleted,
        occurred_at,
    );
    input.event_source = source;
    input.metadata = metadata;
    record_external_journey_event(pool, input).await
}

/// Weiche-/Onboarding-Auswahl geaendert. Wird im aktuellen Repo aus
/// Discord-Rollendeltas und TagService-Events gefeuert.
pub async fn record_weiche_changed_event(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    choice: &str,
    change: &str,
    source: &'static str,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let mut input = JourneyEventInput::new(
        user_id,
        guild_id,
        JourneyEventType::WeicheChanged,
        occurred_at,
    );
    input.event_source = source;
    input.metadata = serde_json::json!({
        "weiche_choice": truncate_chars(choice, 80),
        "change": truncate_chars(change, 32),
    });
    record_external_journey_event(pool, input).await
}

/// Privacy-Opt-out-Signal fuer den DSGVO-Loeschpfad.
///
/// Der eigentliche Erasure-Pfad darf nach dem Delete keine neue user_id-Zeile
/// in `journey_events`/`journey_user_state` erzeugen. Deshalb wird hier nur das
/// anonyme Tagesaggregat erhoeht.
pub async fn record_privacy_opt_out_aggregate(
    pool: &PgPool,
    guild_id: u64,
    occurred_at: DateTime<Utc>,
) -> ActivityDbResult<bool> {
    let guild_id = discord_id_to_i64(guild_id, "journey_daily_aggregates.guild_id")?;
    sqlx::query(
        r#"
        INSERT INTO activity.journey_daily_aggregates(
            day, guild_id, event_type, actor_kind, event_count, distinct_user_count
        )
        VALUES($1::date, $2, 'opt_out', 'human', 1, 1)
        ON CONFLICT(day, guild_id, event_type, actor_kind) DO UPDATE SET
            event_count = activity.journey_daily_aggregates.event_count + 1,
            distinct_user_count = GREATEST(
                activity.journey_daily_aggregates.distinct_user_count,
                EXCLUDED.distinct_user_count
            )
        "#,
    )
    .bind(occurred_at)
    .bind(guild_id)
    .execute(pool)
    .await?;
    Ok(true)
}

pub async fn record_message_metadata(
    pool: &PgPool,
    event: &dl_discord::MessageEvent,
) -> ActivityDbResult<bool> {
    let Some(guild_id) = event.guild_id else {
        return Ok(false);
    };
    let user_id = discord_id_to_i64(event.author_id, "message_metadata_events.user_id")?;
    let guild_id = discord_id_to_i64(guild_id, "message_metadata_events.guild_id")?;
    let channel_id = discord_id_to_i64(event.channel_id, "message_metadata_events.channel_id")?;
    let message_id = discord_id_to_i64(event.message_id, "message_metadata_events.message_id")?;
    let occurred_at = utc_from_unix_seconds(event.message_created_at)?;
    let message_length = i64_to_i32(
        i64::try_from(event.content.chars().count()).unwrap_or(i64::MAX),
        "message_length",
    )?;
    let attachment_count = i64_to_i32(i64::from(event.attachment_count), "attachment_count")?;
    let metadata_json = "{}";

    let mut tx = pool.begin().await?;
    if is_opted_out_tx(&mut tx, user_id).await? {
        tx.commit().await?;
        return Ok(false);
    }
    let inserted = sqlx::query(
        r#"
        INSERT INTO activity.message_metadata_events(
            user_id, guild_id, channel_id, message_id, occurred_at,
            message_length, has_attachment, attachment_count, is_reply
        )
        VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (guild_id, channel_id, message_id) DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(channel_id)
    .bind(message_id)
    .bind(occurred_at)
    .bind(message_length)
    .bind(event.attachment_count > 0)
    .bind(attachment_count)
    .bind(event.is_reply)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    if inserted {
        record_first_journey_event_tx(
            &mut tx,
            user_id,
            guild_id,
            JourneyEventType::FirstMessage,
            occurred_at,
            Some(channel_id),
            Some(message_id),
            metadata_json,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(inserted)
}

pub async fn record_interaction_metadata(
    pool: &PgPool,
    event: &dl_discord::InteractionEvent,
) -> ActivityDbResult<bool> {
    let Some(guild_id) = event.guild_id else {
        return Ok(false);
    };
    let user_id = discord_id_to_i64(event.user_id, "interaction_events.user_id")?;
    let guild_id = discord_id_to_i64(guild_id, "interaction_events.guild_id")?;
    let channel_id = optional_discord_id(event.channel_id, "interaction_events.channel_id")?;
    let message_id = optional_discord_id(event.message_id, "interaction_events.message_id")?;
    let interaction_id =
        discord_id_to_i64(event.interaction_id, "interaction_events.interaction_id")?;
    let occurred_at = utc_from_unix_seconds(event.occurred_at)?;
    let route = event.route.as_deref().map(sanitize_interaction_route);

    let mut tx = pool.begin().await?;
    if is_opted_out_tx(&mut tx, user_id).await? {
        tx.commit().await?;
        return Ok(false);
    }
    let inserted = sqlx::query(
        r#"
        INSERT INTO activity.interaction_events(
            user_id, guild_id, channel_id, message_id, interaction_id,
            interaction_kind, route, occurred_at
        )
        VALUES($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (interaction_id) DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(channel_id)
    .bind(message_id)
    .bind(interaction_id)
    .bind(event.interaction_kind)
    .bind(route.as_deref())
    .bind(occurred_at)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if inserted {
        mark_first_interaction_tx(&mut tx, user_id, guild_id, occurred_at).await?;
    }
    tx.commit().await?;
    Ok(inserted)
}

pub async fn record_presence_seen(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
    day: NaiveDate,
) -> ActivityDbResult<bool> {
    let guild_id = discord_id_to_i64(guild_id, "presence_daily_seen.guild_id")?;
    let user_id = discord_id_to_i64(user_id, "presence_daily_seen.user_id")?;

    let mut tx = pool.begin().await?;
    if is_opted_out_tx(&mut tx, user_id).await? {
        tx.commit().await?;
        return Ok(false);
    }
    let inserted = sqlx::query(
        r#"
        INSERT INTO activity.presence_daily_seen(guild_id, user_id, day)
        VALUES($1, $2, $3)
        ON CONFLICT(guild_id, user_id, day) DO NOTHING
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(day)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    tx.commit().await?;
    Ok(inserted)
}

async fn open_voice_join_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
    channel_id: i64,
    joined_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.voice_open_sessions(user_id, guild_id, channel_id, joined_at, updated_at)
        VALUES($1, $2, $3, $4, now())
        ON CONFLICT(user_id, guild_id) DO UPDATE SET
            channel_id = EXCLUDED.channel_id,
            joined_at = EXCLUDED.joined_at,
            updated_at = now()
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(channel_id)
    .bind(joined_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn close_voice_session_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
) -> Result<Option<(i64, DateTime<Utc>)>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        DELETE FROM activity.voice_open_sessions
        WHERE user_id = $1
          AND guild_id = $2
        RETURNING channel_id, joined_at
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| Ok((row.try_get("channel_id")?, row.try_get("joined_at")?)))
        .transpose()
}

fn duration_seconds(start: DateTime<Utc>, end: DateTime<Utc>) -> i64 {
    end.signed_duration_since(start).num_seconds().max(0)
}

#[allow(clippy::too_many_arguments)]
async fn insert_voice_metadata_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    guild_id: i64,
    channel_id: i64,
    event_type: &str,
    occurred_at: DateTime<Utc>,
    duration_seconds: Option<i64>,
    from_channel_id: Option<i64>,
    to_channel_id: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.voice_metadata_events(
            user_id, guild_id, channel_id, event_type, occurred_at,
            duration_seconds, from_channel_id, to_channel_id
        )
        VALUES($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(user_id)
    .bind(guild_id)
    .bind(channel_id)
    .bind(event_type)
    .bind(occurred_at)
    .bind(duration_seconds)
    .bind(from_channel_id)
    .bind(to_channel_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn record_voice_metadata(
    pool: &PgPool,
    event: &dl_discord::VoiceEvent,
) -> ActivityDbResult<bool> {
    let now = Utc::now();
    let (guild_id, user_id) = match *event {
        dl_discord::VoiceEvent::Join {
            guild_id, user_id, ..
        }
        | dl_discord::VoiceEvent::Leave {
            guild_id, user_id, ..
        }
        | dl_discord::VoiceEvent::Move {
            guild_id, user_id, ..
        }
        | dl_discord::VoiceEvent::Update {
            guild_id, user_id, ..
        } => (
            discord_id_to_i64(guild_id, "voice_metadata_events.guild_id")?,
            discord_id_to_i64(user_id, "voice_metadata_events.user_id")?,
        ),
    };

    let mut tx = pool.begin().await?;
    if is_opted_out_tx(&mut tx, user_id).await? {
        tx.commit().await?;
        return Ok(false);
    }

    match *event {
        dl_discord::VoiceEvent::Join { channel_id, .. } => {
            let channel_id = discord_id_to_i64(channel_id, "voice_metadata_events.channel_id")?;
            insert_voice_metadata_tx(
                &mut tx, user_id, guild_id, channel_id, "join", now, None, None, None,
            )
            .await?;
            open_voice_join_tx(&mut tx, user_id, guild_id, channel_id, now).await?;
            record_first_journey_event_tx(
                &mut tx,
                user_id,
                guild_id,
                JourneyEventType::FirstVoice,
                now,
                Some(channel_id),
                None,
                "{}",
            )
            .await?;
        }
        dl_discord::VoiceEvent::Leave { channel_id, .. } => {
            let channel_id = discord_id_to_i64(channel_id, "voice_metadata_events.channel_id")?;
            let closed = close_voice_session_tx(&mut tx, user_id, guild_id).await?;
            let duration = closed.map(|(_, joined_at)| duration_seconds(joined_at, now));
            insert_voice_metadata_tx(
                &mut tx, user_id, guild_id, channel_id, "leave", now, duration, None, None,
            )
            .await?;
        }
        dl_discord::VoiceEvent::Move {
            from_channel_id,
            to_channel_id,
            ..
        } => {
            let from_channel_id =
                discord_id_to_i64(from_channel_id, "voice_metadata_events.from_channel_id")?;
            let to_channel_id =
                discord_id_to_i64(to_channel_id, "voice_metadata_events.to_channel_id")?;
            let closed = close_voice_session_tx(&mut tx, user_id, guild_id).await?;
            let duration = closed.map(|(_, joined_at)| duration_seconds(joined_at, now));
            insert_voice_metadata_tx(
                &mut tx,
                user_id,
                guild_id,
                to_channel_id,
                "move",
                now,
                duration,
                Some(from_channel_id),
                Some(to_channel_id),
            )
            .await?;
            open_voice_join_tx(&mut tx, user_id, guild_id, to_channel_id, now).await?;
            record_first_journey_event_tx(
                &mut tx,
                user_id,
                guild_id,
                JourneyEventType::FirstVoice,
                now,
                Some(to_channel_id),
                None,
                "{}",
            )
            .await?;
        }
        dl_discord::VoiceEvent::Update { channel_id, .. } => {
            let channel_id = discord_id_to_i64(channel_id, "voice_metadata_events.channel_id")?;
            insert_voice_metadata_tx(
                &mut tx, user_id, guild_id, channel_id, "update", now, None, None, None,
            )
            .await?;
        }
    }

    tx.commit().await?;
    Ok(true)
}

pub async fn handle_member_journey(
    pool: &PgPool,
    event: dl_discord::MemberEvent,
) -> ActivityDbResult<bool> {
    match event {
        dl_discord::MemberEvent::Join {
            guild_id,
            user_id,
            is_bot,
            metadata,
            ..
        } => {
            if is_bot {
                return Ok(false);
            }
            let mut input =
                JourneyEventInput::new(user_id, guild_id, JourneyEventType::Join, Utc::now());
            input.event_source = "member_join";
            input.metadata = sanitized_join_metadata(metadata);
            record_journey_event(pool, input).await
        }
        dl_discord::MemberEvent::ScreeningCompleted { guild_id, user_id } => {
            let mut input = JourneyEventInput::new(
                user_id,
                guild_id,
                JourneyEventType::ScreeningCompleted,
                Utc::now(),
            );
            input.event_source = "member_update";
            record_journey_event(pool, input).await
        }
        dl_discord::MemberEvent::Remove { .. }
        | dl_discord::MemberEvent::Ban { .. }
        | dl_discord::MemberEvent::Unban { .. }
        | dl_discord::MemberEvent::NativeOnboardingCompleted { .. } => Ok(false),
    }
}

pub fn spawn_ingestion(
    pool: PgPool,
    dispatcher: &dl_discord::Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::with_capacity(5);

    let mut messages = dispatcher.subscribe_messages();
    let message_pool = pool.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    if let Err(err) = record_message_metadata(&message_pool, &event).await {
                        tracing::warn!(%err, "journey/message-metadata write fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "journey/message-metadata Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    let mut members = dispatcher.subscribe_members();
    let member_pool = pool.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match members.recv().await {
                Ok(event) => {
                    if let Err(err) = handle_member_journey(&member_pool, event).await {
                        tracing::warn!(%err, "journey/member write fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "journey/member Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    let mut voice = dispatcher.subscribe_voice();
    let voice_pool = pool.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match voice.recv().await {
                Ok(event) => {
                    if let Err(err) = record_voice_metadata(&voice_pool, &event).await {
                        tracing::warn!(%err, "journey/voice-metadata write fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "journey/voice Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    let mut interactions = dispatcher.subscribe_interactions();
    let interaction_pool = pool.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match interactions.recv().await {
                Ok(event) => {
                    if let Err(err) = record_interaction_metadata(&interaction_pool, &event).await {
                        tracing::warn!(%err, "journey/interaction-metadata write fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "journey/interaction Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    let mut presences = dispatcher.subscribe_presence();
    handles.push(tokio::spawn(async move {
        let mut day = Utc::now().date_naive();
        let mut seen: HashSet<(u64, u64)> = HashSet::new();
        loop {
            match presences.recv().await {
                Ok(event) => {
                    let today = Utc::now().date_naive();
                    if today != day {
                        day = today;
                        seen.clear();
                    }
                    if !seen.insert((event.guild_id, event.user_id)) {
                        continue;
                    }
                    if let Err(err) =
                        record_presence_seen(&pool, event.guild_id, event.user_id, day).await
                    {
                        tracing::warn!(%err, "journey/presence write fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "journey/presence Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    handles
}

async fn compact_messages_tx(
    tx: &mut Transaction<'_, Postgres>,
    cutoff: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    // Approximation: Nach einem frueheren Retention-Lauf sind die Raw-User-IDs
    // fuer denselben Aggregat-Key geloescht. Beim Nachverdichten kann daher
    // kein echter Distinct ueber alle Batches mehr berechnet werden; `GREATEST`
    // ist eine dokumentierte Untergrenze statt eines exakten Gesamt-Distincts.
    sqlx::query(
        r#"
        INSERT INTO activity.message_daily_aggregates(
            day, guild_id, channel_id, message_count, total_message_length,
            attachment_message_count, reply_message_count, distinct_user_count
        )
        SELECT occurred_at::date,
               guild_id,
               channel_id,
               COUNT(*),
               COALESCE(SUM(message_length), 0),
               COUNT(*) FILTER (WHERE has_attachment),
               COUNT(*) FILTER (WHERE is_reply),
               COUNT(DISTINCT user_id)
          FROM activity.message_metadata_events
         WHERE occurred_at < $1
         GROUP BY occurred_at::date, guild_id, channel_id
        ON CONFLICT(day, guild_id, channel_id) DO UPDATE SET
            message_count = activity.message_daily_aggregates.message_count + EXCLUDED.message_count,
            total_message_length = activity.message_daily_aggregates.total_message_length + EXCLUDED.total_message_length,
            attachment_message_count = activity.message_daily_aggregates.attachment_message_count + EXCLUDED.attachment_message_count,
            reply_message_count = activity.message_daily_aggregates.reply_message_count + EXCLUDED.reply_message_count,
            distinct_user_count = GREATEST(activity.message_daily_aggregates.distinct_user_count, EXCLUDED.distinct_user_count)
        "#,
    )
    .bind(cutoff)
    .execute(&mut **tx)
    .await?;
    let deleted =
        sqlx::query("DELETE FROM activity.message_metadata_events WHERE occurred_at < $1")
            .bind(cutoff)
            .execute(&mut **tx)
            .await?
            .rows_affected();
    Ok(i64::try_from(deleted).unwrap_or(i64::MAX))
}

async fn compact_voice_tx(
    tx: &mut Transaction<'_, Postgres>,
    cutoff: DateTime<Utc>,
) -> Result<(i64, i64), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.voice_daily_aggregates(
            day, guild_id, channel_id, join_count, leave_count, move_count,
            update_count, total_duration_seconds, distinct_user_count
        )
        SELECT occurred_at::date,
               guild_id,
               channel_id,
               COUNT(*) FILTER (WHERE event_type = 'join'),
               COUNT(*) FILTER (WHERE event_type = 'leave'),
               COUNT(*) FILTER (WHERE event_type = 'move'),
               COUNT(*) FILTER (WHERE event_type = 'update'),
               COALESCE(SUM(duration_seconds), 0),
               COUNT(DISTINCT user_id)
          FROM activity.voice_metadata_events
         WHERE occurred_at < $1
         GROUP BY occurred_at::date, guild_id, channel_id
        ON CONFLICT(day, guild_id, channel_id) DO UPDATE SET
            join_count = activity.voice_daily_aggregates.join_count + EXCLUDED.join_count,
            leave_count = activity.voice_daily_aggregates.leave_count + EXCLUDED.leave_count,
            move_count = activity.voice_daily_aggregates.move_count + EXCLUDED.move_count,
            update_count = activity.voice_daily_aggregates.update_count + EXCLUDED.update_count,
            total_duration_seconds = activity.voice_daily_aggregates.total_duration_seconds + EXCLUDED.total_duration_seconds,
            distinct_user_count = GREATEST(activity.voice_daily_aggregates.distinct_user_count, EXCLUDED.distinct_user_count)
        "#,
    )
    .bind(cutoff)
    .execute(&mut **tx)
    .await?;
    let deleted = sqlx::query("DELETE FROM activity.voice_metadata_events WHERE occurred_at < $1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    let stale_open = sqlx::query("DELETE FROM activity.voice_open_sessions WHERE joined_at < $1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok((
        i64::try_from(deleted).unwrap_or(i64::MAX),
        i64::try_from(stale_open).unwrap_or(i64::MAX),
    ))
}

async fn compact_interactions_tx(
    tx: &mut Transaction<'_, Postgres>,
    cutoff: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        r#"
        WITH sanitized AS (
            SELECT occurred_at,
                   guild_id,
                   interaction_kind,
                   user_id,
                   LEFT(
                       COALESCE((
                           SELECT string_agg(
                                      CASE
                                          WHEN segment ~ '^[0-9]+$'
                                            OR char_length(segment) > 15
                                          THEN '*'
                                          ELSE segment
                                      END,
                                      ':' ORDER BY ord
                                  )
                             FROM unnest(string_to_array(COALESCE(route, ''), ':'))
                                  WITH ORDINALITY AS parts(segment, ord)
                       ), ''),
                       $2
                   ) AS route
              FROM activity.interaction_events
             WHERE occurred_at < $1
        )
        INSERT INTO activity.interaction_daily_aggregates(
            day, guild_id, interaction_kind, route, interaction_count, distinct_user_count
        )
        SELECT occurred_at::date,
               guild_id,
               interaction_kind,
               route,
               COUNT(*),
               COUNT(DISTINCT user_id)
          FROM sanitized
         GROUP BY occurred_at::date, guild_id, interaction_kind, route
        ON CONFLICT(day, guild_id, interaction_kind, route) DO UPDATE SET
            interaction_count = activity.interaction_daily_aggregates.interaction_count + EXCLUDED.interaction_count,
            distinct_user_count = GREATEST(activity.interaction_daily_aggregates.distinct_user_count, EXCLUDED.distinct_user_count)
        "#,
    )
    .bind(cutoff)
    .bind(MAX_INTERACTION_ROUTE_SQL_LEN)
    .execute(&mut **tx)
    .await?;
    let deleted = sqlx::query("DELETE FROM activity.interaction_events WHERE occurred_at < $1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(i64::try_from(deleted).unwrap_or(i64::MAX))
}

async fn compact_presence_tx(
    tx: &mut Transaction<'_, Postgres>,
    cutoff: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.presence_daily_aggregates(
            day, guild_id, distinct_user_count
        )
        SELECT day,
               guild_id,
               COUNT(DISTINCT user_id)
          FROM activity.presence_daily_seen
         WHERE day < $1::date
         GROUP BY day, guild_id
        ON CONFLICT(day, guild_id) DO UPDATE SET
            distinct_user_count = GREATEST(
                activity.presence_daily_aggregates.distinct_user_count,
                EXCLUDED.distinct_user_count
            )
        "#,
    )
    .bind(cutoff)
    .execute(&mut **tx)
    .await?;
    let deleted = sqlx::query("DELETE FROM activity.presence_daily_seen WHERE day < $1::date")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(i64::try_from(deleted).unwrap_or(i64::MAX))
}

async fn compact_journey_tx(
    tx: &mut Transaction<'_, Postgres>,
    cutoff: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO activity.journey_daily_aggregates(
            day, guild_id, event_type, actor_kind, event_count, distinct_user_count
        )
        SELECT occurred_at::date,
               guild_id,
               event_type,
               COALESCE(actor_kind, ''),
               COUNT(*),
               COUNT(DISTINCT user_id)
          FROM activity.journey_events
         WHERE occurred_at < $1
         GROUP BY occurred_at::date, guild_id, event_type, COALESCE(actor_kind, '')
        ON CONFLICT(day, guild_id, event_type, actor_kind) DO UPDATE SET
            event_count = activity.journey_daily_aggregates.event_count + EXCLUDED.event_count,
            distinct_user_count = GREATEST(activity.journey_daily_aggregates.distinct_user_count, EXCLUDED.distinct_user_count)
        "#,
    )
    .bind(cutoff)
    .execute(&mut **tx)
    .await?;
    let deleted = sqlx::query("DELETE FROM activity.journey_events WHERE occurred_at < $1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(i64::try_from(deleted).unwrap_or(i64::MAX))
}

pub async fn compact_raw_events(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> ActivityDbResult<RawRetentionRun> {
    let cutoff = now - Duration::days(RAW_EVENT_RETENTION_DAYS);
    let mut tx = pool.begin().await?;
    let message_rows_deleted = compact_messages_tx(&mut tx, cutoff).await?;
    let (voice_rows_deleted, stale_open_voice_sessions_deleted) =
        compact_voice_tx(&mut tx, cutoff).await?;
    let interaction_rows_deleted = compact_interactions_tx(&mut tx, cutoff).await?;
    let presence_rows_deleted = compact_presence_tx(&mut tx, cutoff).await?;
    let journey_rows_deleted = compact_journey_tx(&mut tx, cutoff).await?;
    tx.commit().await?;
    Ok(RawRetentionRun {
        message_rows_deleted,
        voice_rows_deleted,
        interaction_rows_deleted,
        presence_rows_deleted,
        journey_rows_deleted,
        stale_open_voice_sessions_deleted,
    })
}

pub fn spawn_retention(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match compact_raw_events(&pool, Utc::now()).await {
                Ok(summary) => tracing::info!(
                    messages = summary.message_rows_deleted,
                    voice = summary.voice_rows_deleted,
                    interactions = summary.interaction_rows_deleted,
                    presence = summary.presence_rows_deleted,
                    journey = summary.journey_rows_deleted,
                    stale_voice = summary.stale_open_voice_sessions_deleted,
                    retention_days = RAW_EVENT_RETENTION_DAYS,
                    "Journey-Rohdaten-Retention abgeschlossen"
                ),
                Err(err) => tracing::warn!(%err, "Journey-Rohdaten-Retention fehlgeschlagen"),
            }
            tokio::time::sleep(RETENTION_JOB_INTERVAL).await;
        }
    })
}

/// Discord-Insights-Definition `pct_communicated`: Ein Neumitglied hat
/// innerhalb von 14 Tagen kommuniziert, wenn es entweder die erste
/// Textnachricht (`first_message_at`) oder die erste Voice-Teilnahme
/// (`first_voice_at`) innerhalb dieses Fensters hatte.
pub async fn activation_summary(
    pool: &PgPool,
    guild_id: u64,
    cohort_start: DateTime<Utc>,
    cohort_end: DateTime<Utc>,
) -> ActivityDbResult<ActivationSummary> {
    let guild_id = discord_id_to_i64(guild_id, "journey_user_state.guild_id")?;
    let row = sqlx::query(
        r#"
        SELECT COUNT(*)::int8 AS joined,
               COUNT(*) FILTER (
                   WHERE joined_at IS NOT NULL
                     AND (
                         (first_message_at IS NOT NULL
                          AND first_message_at <= joined_at + INTERVAL '14 days')
                      OR (first_voice_at IS NOT NULL
                          AND first_voice_at <= joined_at + INTERVAL '14 days')
                     )
               )::int8 AS activated
          FROM activity.journey_user_state
         WHERE guild_id = $1
           AND joined_at >= $2
           AND joined_at < $3
        "#,
    )
    .bind(guild_id)
    .bind(cohort_start)
    .bind(cohort_end)
    .fetch_one(pool)
    .await?;
    let joined: i64 = row.try_get("joined")?;
    let activated: i64 = row.try_get("activated")?;
    Ok(ActivationSummary {
        joined,
        activated,
        activation_rate: if joined == 0 {
            0.0
        } else {
            activated as f64 / joined as f64
        },
    })
}

pub async fn retention_summary(
    pool: &PgPool,
    guild_id: u64,
    cohort_start: DateTime<Utc>,
    cohort_end: DateTime<Utc>,
) -> ActivityDbResult<RetentionSummary> {
    let guild_id = discord_id_to_i64(guild_id, "journey_user_state.guild_id")?;
    let row = sqlx::query(
        r#"
        SELECT COUNT(*)::int8 AS joined,
               COUNT(*) FILTER (
                   WHERE joined_at IS NOT NULL
                     AND last_event_at >= joined_at + INTERVAL '7 days'
               )::int8 AS d7_retained,
               COUNT(*) FILTER (
                   WHERE joined_at IS NOT NULL
                     AND last_event_at >= joined_at + INTERVAL '14 days'
               )::int8 AS d14_retained
          FROM activity.journey_user_state
         WHERE guild_id = $1
           AND joined_at >= $2
           AND joined_at < $3
        "#,
    )
    .bind(guild_id)
    .bind(cohort_start)
    .bind(cohort_end)
    .fetch_one(pool)
    .await?;
    Ok(RetentionSummary {
        joined: row.try_get("joined")?,
        d7_retained: row.try_get("d7_retained")?,
        d14_retained: row.try_get("d14_retained")?,
    })
}

pub async fn funnel_summary(
    pool: &PgPool,
    guild_id: u64,
    cohort_start: DateTime<Utc>,
    cohort_end: DateTime<Utc>,
) -> ActivityDbResult<FunnelSummary> {
    let guild_id = discord_id_to_i64(guild_id, "journey_user_state.guild_id")?;
    let row = sqlx::query(
        r#"
        SELECT COUNT(*) FILTER (WHERE native_onboarding_completed_at IS NOT NULL)::int8 AS weiche_completed,
               COUNT(*) FILTER (WHERE steam_linked_at IS NOT NULL)::int8 AS steam_linked,
               COUNT(*) FILTER (
                   WHERE invite_friend_request_sent_at IS NOT NULL
                      OR invite_sent_at IS NOT NULL
               )::int8 AS invite_started,
               COUNT(*) FILTER (
                   WHERE invite_friend_request_accepted_at IS NOT NULL
                      OR invite_accepted_at IS NOT NULL
               )::int8 AS invite_completed,
               COUNT(*) FILTER (WHERE first_message_at IS NOT NULL)::int8 AS first_message
          FROM activity.journey_user_state
         WHERE guild_id = $1
           AND joined_at >= $2
           AND joined_at < $3
        "#,
    )
    .bind(guild_id)
    .bind(cohort_start)
    .bind(cohort_end)
    .fetch_one(pool)
    .await?;
    Ok(FunnelSummary {
        weiche_completed: row.try_get("weiche_completed")?,
        steam_linked: row.try_get("steam_linked")?,
        invite_started: row.try_get("invite_started")?,
        invite_completed: row.try_get("invite_completed")?,
        first_message: row.try_get("first_message")?,
    })
}

pub async fn null_activity_users(
    pool: &PgPool,
    guild_id: u64,
    now: DateTime<Utc>,
    min_age_days: i64,
) -> ActivityDbResult<Vec<NullActivityUser>> {
    let guild_id = discord_id_to_i64(guild_id, "journey_user_state.guild_id")?;
    let cutoff = now - Duration::days(min_age_days);
    let rows = sqlx::query(
        r#"
        SELECT user_id, joined_at
          FROM activity.journey_user_state
         WHERE guild_id = $1
           AND joined_at <= $2
           AND first_message_at IS NULL
           AND first_voice_at IS NULL
           AND first_interaction_at IS NULL
           AND opt_out_at IS NULL
         ORDER BY joined_at, user_id
        "#,
    )
    .bind(guild_id)
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let user_id: i64 = row.try_get("user_id")?;
            let joined_at: DateTime<Utc> = row.try_get("joined_at")?;
            Ok(NullActivityUser {
                user_id: i64_to_u64(user_id, "journey_user_state.user_id").unwrap_or_default(),
                joined_at,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "testing")]
    fn message_event(
        user_id: u64,
        guild_id: u64,
        message_id: u64,
        content: &str,
    ) -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id: Some(guild_id),
            channel_id: 10,
            message_id,
            author_id: user_id,
            author_display_name: "User".into(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.into(),
            message_created_at: 1_788_200_000,
            is_reply: true,
            reply_message_id: Some(1),
            reply_channel_id: Some(10),
            attachment_count: 1,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 1_700_000_000,
            author_joined_at: None,
        }
    }

    #[cfg(feature = "testing")]
    fn interaction_event(
        user_id: u64,
        guild_id: u64,
        interaction_id: u64,
        route: &str,
        occurred_at: DateTime<Utc>,
    ) -> dl_discord::InteractionEvent {
        dl_discord::InteractionEvent {
            guild_id: Some(guild_id),
            channel_id: Some(10),
            message_id: Some(20),
            interaction_id,
            user_id,
            interaction_kind: "component",
            route: Some(route.to_string()),
            occurred_at: occurred_at.timestamp(),
        }
    }

    #[test]
    fn interaction_route_sanitizer_redigiert_repo_custom_id_muster() {
        assert_eq!(
            sanitize_interaction_route("ob:tone:banter_ok:7:123456789012345678"),
            "ob:tone:banter_ok:*:*"
        );
        assert_eq!(
            sanitize_interaction_route("sg:ban:1289721245281292288:987654321098765432"),
            "sg:ban:*:*"
        );
        assert_eq!(
            sanitize_interaction_route("tv_owner_claim"),
            "tv_owner_claim"
        );

        let very_long = format!("ob:{}", "a".repeat(200));
        assert!(
            sanitize_interaction_route(&very_long).chars().count() <= MAX_INTERACTION_ROUTE_LEN
        );
    }

    #[test]
    fn event_catalog_deckt_phase1_funnel_ab() {
        let names = JourneyEventType::all()
            .iter()
            .map(|event| event.as_str())
            .collect::<Vec<_>>();
        for required in [
            "join",
            "screening_completed",
            "native_onboarding_completed",
            "steam_link",
            "invite_friend_request_sent",
            "invite_friend_request_accepted",
            "invite_sent",
            "invite_accepted",
            "first_message",
            "first_voice",
            "first_match",
            "squad_join",
            "opt_out",
            "streamer_contact_activation",
        ] {
            assert!(
                names.contains(&required),
                "{required} fehlt im Journey-Katalog"
            );
        }
    }

    #[cfg(feature = "testing")]
    async fn setup() -> Result<dl_central_db::TestDb, Box<dyn std::error::Error>> {
        Ok(dl_central_db::testing::test_pool().await?)
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn message_metadata_speichert_keinen_content_und_setzt_first_message(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = setup().await?;
        let pool = db.pool();
        record_journey_event(
            pool,
            JourneyEventInput {
                event_source: "test",
                ..JourneyEventInput::new(
                    42,
                    1,
                    JourneyEventType::Join,
                    DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")?.with_timezone(&Utc),
                )
            },
        )
        .await?;
        assert!(record_message_metadata(pool, &message_event(42, 1, 100, "hallo welt")).await?);
        assert!(!record_message_metadata(pool, &message_event(42, 1, 100, "duplikat")).await?);
        assert!(record_message_metadata(pool, &message_event(42, 1, 101, "zweite msg")).await?);

        let row = sqlx::query(
            r#"
            SELECT message_length, has_attachment, is_reply
              FROM activity.message_metadata_events
             WHERE user_id = 42
             ORDER BY message_id
             LIMIT 1
            "#,
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(row.try_get::<i32, _>("message_length")?, 10);
        assert!(row.try_get::<bool, _>("has_attachment")?);
        assert!(row.try_get::<bool, _>("is_reply")?);

        let columns: Vec<String> = sqlx::query_scalar(
            r#"
            SELECT column_name
              FROM information_schema.columns
             WHERE table_schema = 'activity'
               AND table_name = 'message_metadata_events'
             ORDER BY ordinal_position
            "#,
        )
        .fetch_all(pool)
        .await?;
        assert!(!columns.iter().any(|column| column == "content"));

        let first_message_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8 FROM activity.journey_events WHERE user_id = 42 AND event_type = 'first_message'",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(first_message_count, 1);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn first_journey_event_dedupe_ist_db_hart() -> Result<(), Box<dyn std::error::Error>> {
        let db = setup().await?;
        let pool = db.pool();
        let occurred_at = DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")?.with_timezone(&Utc);
        let mut tx = pool.begin().await?;

        assert!(
            record_first_journey_event_tx(
                &mut tx,
                42,
                1,
                JourneyEventType::FirstMessage,
                occurred_at,
                Some(10),
                Some(100),
                "{}",
            )
            .await?
        );
        assert!(
            !record_first_journey_event_tx(
                &mut tx,
                42,
                1,
                JourneyEventType::FirstMessage,
                occurred_at + Duration::minutes(1),
                Some(10),
                Some(101),
                "{}",
            )
            .await?
        );
        tx.commit().await?;

        let first_message_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8 FROM activity.journey_events WHERE user_id = 42 AND event_type = 'first_message'",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(first_message_count, 1);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn ingestion_respektiert_optout() -> Result<(), Box<dyn std::error::Error>> {
        let db = setup().await?;
        let pool = db.pool();
        sqlx::query("INSERT INTO core.user_privacy(user_id, opted_out) VALUES(77, TRUE)")
            .execute(pool)
            .await?;

        assert!(!record_message_metadata(pool, &message_event(77, 1, 200, "still")).await?);
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8 FROM activity.message_metadata_events WHERE user_id = 77",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn interaction_route_wird_vor_raw_write_und_aggregation_sanitisiert(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = setup().await?;
        let pool = db.pool();
        let old = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")?.with_timezone(&Utc);
        assert!(
            record_interaction_metadata(
                pool,
                &interaction_event(
                    42,
                    1,
                    4200,
                    "sg:ban:1289721245281292288:987654321098765432",
                    old,
                ),
            )
            .await?
        );

        let raw_route: String = sqlx::query_scalar(
            "SELECT route FROM activity.interaction_events WHERE interaction_id = 4200",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(raw_route, "sg:ban:*:*");

        compact_raw_events(pool, old + Duration::days(RAW_EVENT_RETENTION_DAYS + 1)).await?;
        let aggregate_route: String =
            sqlx::query_scalar("SELECT route FROM activity.interaction_daily_aggregates")
                .fetch_one(pool)
                .await?;
        assert_eq!(aggregate_route, "sg:ban:*:*");
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn analytics_liefert_activation_retention_funnel_und_null_activity(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = setup().await?;
        let pool = db.pool();
        let base = DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")?.with_timezone(&Utc);
        let rows = [
            (
                1,
                base,
                Some(base + Duration::days(1)),
                Some(base + Duration::days(8)),
                Some(base + Duration::days(2)),
                true,
                true,
                true,
            ),
            (
                2,
                base,
                Some(base + Duration::days(20)),
                Some(base + Duration::days(15)),
                Some(base + Duration::days(2)),
                true,
                false,
                false,
            ),
            (3, base, None, None, None, false, false, false),
        ];
        for (user_id, joined, first_message, last_event, first_voice, weiche, steam, invite) in rows
        {
            sqlx::query(
                r#"
                INSERT INTO activity.journey_user_state(
                    user_id, guild_id, joined_at, native_onboarding_completed_at,
                    steam_linked_at, invite_sent_at, invite_accepted_at,
                    first_message_at, first_voice_at, last_event_at
                )
                VALUES($1, 1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
            )
            .bind(user_id)
            .bind(joined)
            .bind(weiche.then_some(joined + Duration::hours(1)))
            .bind(steam.then_some(joined + Duration::hours(2)))
            .bind(invite.then_some(joined + Duration::hours(3)))
            .bind(invite.then_some(joined + Duration::hours(4)))
            .bind(first_message)
            .bind(first_voice)
            .bind(last_event)
            .execute(pool)
            .await?;
        }

        let activation = activation_summary(pool, 1, base, base + Duration::days(1)).await?;
        assert_eq!(activation.joined, 3);
        assert_eq!(activation.activated, 2);
        assert!((activation.activation_rate - (2.0 / 3.0)).abs() < f64::EPSILON);

        let retention = retention_summary(pool, 1, base, base + Duration::days(1)).await?;
        assert_eq!(retention.d7_retained, 2);
        assert_eq!(retention.d14_retained, 1);

        let funnel = funnel_summary(pool, 1, base, base + Duration::days(1)).await?;
        assert_eq!(funnel.weiche_completed, 2);
        assert_eq!(funnel.steam_linked, 1);
        assert_eq!(funnel.invite_started, 1);
        assert_eq!(funnel.invite_completed, 1);
        assert_eq!(funnel.first_message, 2);

        let nulls = null_activity_users(pool, 1, base + Duration::days(3), 2).await?;
        assert_eq!(nulls.iter().map(|u| u.user_id).collect::<Vec<_>>(), vec![3]);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn retention_verdichtet_rohdaten_anonym_und_loescht_user_rows(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = setup().await?;
        let pool = db.pool();
        let old = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")?.with_timezone(&Utc);
        let now = old + Duration::days(RAW_EVENT_RETENTION_DAYS + 1);
        sqlx::query(
            r#"
            INSERT INTO activity.message_metadata_events(
                user_id, guild_id, channel_id, message_id, occurred_at,
                message_length, has_attachment, attachment_count, is_reply
            )
            VALUES(10, 1, 20, 30, $1, 5, TRUE, 1, FALSE)
            "#,
        )
        .bind(old)
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO activity.voice_metadata_events(
                user_id, guild_id, channel_id, event_type, occurred_at, duration_seconds
            )
            VALUES(10, 1, 21, 'leave', $1, 600)
            "#,
        )
        .bind(old)
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO activity.interaction_events(
                user_id, guild_id, interaction_id, interaction_kind, route, occurred_at
            )
            VALUES(10, 1, 99, 'component', 'x:y', $1)
            "#,
        )
        .bind(old)
        .execute(pool)
        .await?;
        record_journey_event(
            pool,
            JourneyEventInput {
                event_source: "test",
                ..JourneyEventInput::new(10, 1, JourneyEventType::Join, old)
            },
        )
        .await?;

        let summary = compact_raw_events(pool, now).await?;
        assert_eq!(summary.message_rows_deleted, 1);
        assert_eq!(summary.voice_rows_deleted, 1);
        assert_eq!(summary.interaction_rows_deleted, 1);
        assert_eq!(summary.journey_rows_deleted, 1);

        let raw_messages: i64 =
            sqlx::query_scalar("SELECT COUNT(*)::int8 FROM activity.message_metadata_events")
                .fetch_one(pool)
                .await?;
        let aggregate_messages: i64 =
            sqlx::query_scalar("SELECT message_count FROM activity.message_daily_aggregates")
                .fetch_one(pool)
                .await?;
        assert_eq!(raw_messages, 0);
        assert_eq!(aggregate_messages, 1);

        let aggregate_columns: Vec<String> = sqlx::query_scalar(
            r#"
            SELECT column_name
              FROM information_schema.columns
             WHERE table_schema = 'activity'
               AND table_name = 'message_daily_aggregates'
            "#,
        )
        .fetch_all(pool)
        .await?;
        assert!(!aggregate_columns.iter().any(|column| column == "user_id"));
        Ok(())
    }
}
