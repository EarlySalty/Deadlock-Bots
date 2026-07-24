use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use dl_squads::scrim_match::{
    fetch_scrim_match_result, fetch_scrim_match_result_by_steam_match_id, start_scrim_match,
    ScrimMatchResultOutcome, StartScrimMatchOutcome,
};
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Row};
use tokio::task::JoinHandle;
use tokio::time::{self, MissedTickBehavior};

const POLL_INTERVAL: Duration = Duration::from_secs(15);
const STATE_LOBBY_OPEN: &str = "lobby_open";
const STATE_LOBBY_POSTING: &str = "lobby_posting";
const STATE_LOBBY_POSTED: &str = "lobby_posted";
const STATE_LOBBY_POST_FAILED: &str = "lobby_post_failed";
const STATE_START_REQUESTED: &str = "start_requested";
const STATE_STARTING: &str = "starting";
const STATE_START_FAILED: &str = "start_failed";
const STATE_RESULT_REQUESTED: &str = "result_requested";
const STATE_RESULT_FETCHING: &str = "result_fetching";
const STATE_RESULT_FAILED: &str = "result_failed";
const STATE_FINISHED: &str = "finished";
const MATCH_RESULT_REF_STATUS_PENDING: &str = "pending";
const MATCH_RESULT_REF_STATUS_FETCHING: &str = "fetching";
const MATCH_RESULT_REF_STATUS_FETCHED: &str = "fetched";
const MATCH_RESULT_REF_STATUS_FAILED: &str = "failed";
const MATCH_REQUEST_STATUS_DRAFT: &str = "draft";
const MATCH_REQUEST_STATUS_POSTING: &str = "posting";
const MATCH_REQUEST_STATUS_OPEN: &str = "open";
const MATCH_REQUEST_STATUS_POST_FAILED: &str = "post_failed";
const MATCH_REQUEST_STATUS_CLOSED: &str = "closed";
const MATCH_REQUEST_REMINDER_STATUS_APPROVED: &str = "approved";
const MATCH_REQUEST_REMINDER_STATUS_POSTING: &str = "posting";
const MATCH_REQUEST_REMINDER_STATUS_POSTED: &str = "posted";
const MATCH_REQUEST_REMINDER_STATUS_FAILED: &str = "failed";
const MATCH_REQUEST_REMINDER_STATUS_CANCELLED: &str = "cancelled";
const MATCH_STATUS_MESSAGE_STATE_PENDING: &str = "pending";
const MATCH_STATUS_MESSAGE_STATE_POSTING: &str = "posting";
const MATCH_STATUS_MESSAGE_STATE_POSTED: &str = "posted";
const MATCH_STATUS_MESSAGE_STATE_POST_FAILED: &str = "post_failed";
const MATCH_REQUEST_RESPONSE_PREFIX: &str = "scrimreq:v1:";
const LOG_CHANNEL_ID: u64 = dl_moderation::LOG_CHANNEL_ID;
const MAIN_GUILD_ID: u64 = 1289721245281292288;
const SCRIM_VOICE_CHANNEL_NAME: &str = "Scrim Team";

#[derive(Debug, Clone, Copy)]
pub struct ScrimVoiceConfig {
    pub enabled: bool,
    pub category_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrimVoiceAction {
    Create,
    Keep,
    Cleanup,
    SkipDisabled,
    SkipMissingCategory,
    SkipNoChannel,
}

fn decide_scrim_voice_action(
    enabled: bool,
    category_id: Option<u64>,
    has_channel: bool,
    terminal: bool,
) -> ScrimVoiceAction {
    if terminal {
        return if has_channel {
            ScrimVoiceAction::Cleanup
        } else {
            ScrimVoiceAction::SkipNoChannel
        };
    }
    if has_channel {
        ScrimVoiceAction::Keep
    } else if !enabled {
        ScrimVoiceAction::SkipDisabled
    } else if category_id.is_none() {
        ScrimVoiceAction::SkipMissingCategory
    } else {
        ScrimVoiceAction::Create
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrimDriverAction {
    PostLobbyCode,
    Start,
    FetchResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResultQueueState {
    Pending,
    Fetching,
    Finished,
    Failed,
}

impl ScrimDriverAction {
    fn from_requested_state(state: &str) -> Option<Self> {
        match state {
            STATE_LOBBY_OPEN => Some(Self::PostLobbyCode),
            STATE_START_REQUESTED => Some(Self::Start),
            STATE_RESULT_REQUESTED | STATE_RESULT_FAILED => Some(Self::FetchResult),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaimedMatch {
    match_id: i64,
    action: ScrimDriverAction,
    team_channels: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchRequestTarget {
    team_id: i64,
    team_name: String,
    channel_id: u64,
    opponent_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ClaimedMatchRequest {
    request_id: i64,
    slot_options: Value,
    targets: Vec<MatchRequestTarget>,
    missing_targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ClaimedMatchRequestBatch {
    batch_id: i64,
    template: String,
    deadline_at: DateTime<Utc>,
    requests: Vec<ClaimedMatchRequest>,
}

struct MatchRequestPost {
    request_id: i64,
    team_id: i64,
    channel_id: u64,
    message_id: u64,
}

struct MatchRequestSendResult {
    posts: Vec<MatchRequestPost>,
    errors: Vec<String>,
}

struct LobbyCodeSyncResult {
    message_ids: BTreeMap<u64, u64>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaimedMatchResultRef {
    ref_id: i64,
    steam_match_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchStatusNeed {
    display_name: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchStatusTeam {
    team_id: i64,
    team_name: String,
    confirmed: Vec<String>,
    needs: Vec<MatchStatusNeed>,
    no_slot_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchStatusMember {
    participant_id: i64,
    display_name: String,
    is_bench: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchStatusResponse {
    participant_id: i64,
    slot_index: i32,
    response: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchStatusTarget {
    team_id: i64,
    team_name: String,
    channel_id: u64,
    query_channel_id: u64,
    query_message_id: u64,
    status_message_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
struct ClaimedMatchStatus {
    request_id: i64,
    team_a_name: String,
    team_b_name: Option<String>,
    released_slot: Value,
    teams: Vec<MatchStatusTeam>,
    targets: Vec<MatchStatusTarget>,
    missing_targets: Vec<String>,
}

struct MatchStatusPost {
    team_id: i64,
    channel_id: u64,
    message_id: u64,
}

struct MatchStatusSyncResult {
    posts: Vec<MatchStatusPost>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaimedMatchRequestReminder {
    reminder_id: i64,
    request_id: i64,
    team_id: i64,
    team_name: String,
    channel_id: u64,
    source_message_id: u64,
    target_kind: String,
    target_user_ids: Vec<u64>,
    target_role_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchRequestResponseInput {
    custom_id: String,
    user_id: u64,
    channel_id: u64,
    message_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MatchRequestResponseOutcome {
    SavedSlot(i32),
    SavedNone,
    InvalidAction,
    NotOpen,
    WrongMessage,
    WrongTeam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedMatchRequestResponse {
    request_id: i64,
    team_id: i64,
    slot_index: i32,
}

struct MatchRequestResponseHandler {
    pool: PgPool,
}

pub fn register(router: &mut dl_discord::InteractionRouter, pool: PgPool) {
    router.on_prefix(
        MATCH_REQUEST_RESPONSE_PREFIX,
        Arc::new(MatchRequestResponseHandler { pool }),
    );
}

#[async_trait::async_trait]
impl dl_discord::InteractionHandler for MatchRequestResponseHandler {
    async fn handle(&self, interaction: dl_discord::BridgeInteraction) -> dl_discord::BridgeReply {
        match record_match_request_response(
            &self.pool,
            MatchRequestResponseInput {
                custom_id: interaction.custom_id,
                user_id: interaction.user_id,
                channel_id: interaction.channel_id,
                message_id: interaction.message_id,
            },
        )
        .await
        {
            Ok(outcome) => match_request_response_reply(outcome),
            Err(err) => {
                tracing::warn!(%err, "Scrim-Terminantwort konnte nicht gespeichert werden");
                dl_discord::BridgeReply::ephemeral_text("Antwort konnte nicht gespeichert werden.")
            }
        }
    }
}

pub fn spawn(
    pool: PgPool,
    adapter: Arc<dl_discord::DiscordAdapter>,
    tempvoice: Arc<dl_voice::tempvoice::TempVoiceEngine>,
    voice_config: ScrimVoiceConfig,
    lagebild_ai: Option<Arc<dyn dl_ai::ChatProvider>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(err) = process_one_pending(
                &pool,
                adapter.as_ref(),
                tempvoice.as_ref(),
                voice_config,
                lagebild_ai.as_deref(),
            )
            .await
            {
                tracing::warn!(%err, "Scrim-Match-Treiber-Tick fehlgeschlagen");
            }
        }
    })
}

async fn process_one_pending(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    voice_config: ScrimVoiceConfig,
    lagebild_ai: Option<&dyn dl_ai::ChatProvider>,
) -> anyhow::Result<()> {
    cleanup_terminal_scrim_voice_channels(pool, tempvoice).await?;
    if let Some(claim) = claim_next_pending_match(pool).await? {
        handle_claimed_match(pool, adapter, tempvoice, voice_config, lagebild_ai, &claim).await?;
    }
    if let Some(reminder) = claim_next_pending_match_request_reminder(pool).await? {
        handle_match_request_reminder(pool, adapter, reminder).await?;
    } else if let Some(status) = claim_next_pending_match_status(pool).await? {
        handle_match_status(pool, adapter, status).await?;
    } else if let Some(batch) = claim_next_pending_match_request_batch(pool).await? {
        handle_match_request_batch(pool, adapter, batch).await?;
    } else if let Some(claim) = claim_next_pending_match_or_result_retry(pool).await? {
        handle_claimed_match(pool, adapter, tempvoice, voice_config, lagebild_ai, &claim).await?;
    }

    match dl_squads::lagebild::generate_due_lagebilder(pool, lagebild_ai, 1).await {
        Ok(generated) if generated > 0 => {
            tracing::info!(generated, "Scrim-Lagebilder im Wochenlauf erzeugt");
        }
        Ok(_) => {}
        Err(err) => {
            tracing::error!(%err, "Scrim-Lagebild-Wochenlauf fehlgeschlagen");
        }
    }
    Ok(())
}

async fn claim_next_pending_match(pool: &PgPool) -> anyhow::Result<Option<ClaimedMatch>> {
    claim_next_match(pool, false).await
}

async fn claim_next_pending_match_or_result_retry(
    pool: &PgPool,
) -> anyhow::Result<Option<ClaimedMatch>> {
    claim_next_match(pool, true).await
}

async fn claim_next_match(
    pool: &PgPool,
    include_result_retries: bool,
) -> anyhow::Result<Option<ClaimedMatch>> {
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT m.id,
                   CASE
                       WHEN m.lobby_state IN ($1, $2, $3) THEN m.lobby_state
                       ELSE $7
                   END AS requested_state,
                   ta.discord_channel_id AS team_a_channel_id,
                   tb.discord_channel_id AS team_b_channel_id
              FROM scrim.matches m
              LEFT JOIN scrim.teams ta ON ta.id = m.team_a_id
              LEFT JOIN scrim.teams tb ON tb.id = m.team_b_id
             WHERE m.lobby_state IN ($1, $2, $3)
                OR ($8::boolean AND (
                    (
                        m.lobby_state IN ($5, $7)
                        AND m.updated_at <= now() - interval '15 minutes'
                    )
                    OR EXISTS (
                        SELECT 1
                          FROM scrim.match_result_refs result_ref
                         WHERE result_ref.match_id = m.id
                           AND result_ref.fetch_status IN ($9, $10)
                           AND result_ref.updated_at <= now() - interval '15 minutes'
                    )
                ))
             ORDER BY CASE WHEN m.lobby_state IN ($1, $2, $3) THEN 0 ELSE 1 END,
                      COALESCE(m.updated_at, m.created_at),
                      m.id
             LIMIT 1
             FOR UPDATE OF m SKIP LOCKED
        )
        UPDATE scrim.matches m
	           SET lobby_state = CASE candidate.requested_state
	                    WHEN $1 THEN $4
	                    WHEN $2 THEN $5
	                    WHEN $3 THEN $6
	                    WHEN $7 THEN $5
	                    ELSE m.lobby_state
	               END,
               updated_at = now()
          FROM candidate
         WHERE m.id = candidate.id
        RETURNING m.id::bigint AS match_id,
                  candidate.requested_state,
                  candidate.team_a_channel_id,
                  candidate.team_b_channel_id
        "#,
    )
    .bind(STATE_START_REQUESTED)
    .bind(STATE_RESULT_REQUESTED)
    .bind(STATE_LOBBY_OPEN)
    .bind(STATE_STARTING)
    .bind(STATE_RESULT_FETCHING)
    .bind(STATE_LOBBY_POSTING)
    .bind(STATE_RESULT_FAILED)
    .bind(include_result_retries)
    .bind(MATCH_RESULT_REF_STATUS_FAILED)
    .bind(MATCH_RESULT_REF_STATUS_FETCHING)
    .fetch_optional(pool)
    .await
    .context("Scrim-Match-Claim fehlgeschlagen")?;

    let Some(row) = row else {
        return Ok(None);
    };
    let requested_state: String = row.get("requested_state");
    let action = ScrimDriverAction::from_requested_state(&requested_state)
        .ok_or_else(|| anyhow!("unerwarteter Scrim-Claim-State: {requested_state}"))?;
    Ok(Some(ClaimedMatch {
        match_id: row.get("match_id"),
        action,
        team_channels: target_channels(row.get("team_a_channel_id"), row.get("team_b_channel_id")),
    }))
}

async fn handle_claimed_match(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    voice_config: ScrimVoiceConfig,
    lagebild_ai: Option<&dyn dl_ai::ChatProvider>,
    claim: &ClaimedMatch,
) -> anyhow::Result<()> {
    match claim.action {
        ScrimDriverAction::PostLobbyCode => handle_lobby_code(pool, adapter, claim).await,
        ScrimDriverAction::Start => {
            handle_start(pool, adapter, tempvoice, voice_config, claim).await
        }
        ScrimDriverAction::FetchResult => {
            handle_result(pool, adapter, tempvoice, claim, lagebild_ai).await
        }
    }
}

async fn handle_lobby_code(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    claim: &ClaimedMatch,
) -> anyhow::Result<()> {
    let Some(code) = load_lobby_code(pool, claim.match_id).await? else {
        set_lobby_state(pool, claim.match_id, STATE_LOBBY_POST_FAILED).await?;
        post_log(
            adapter,
            lobby_code_failure_message(claim.match_id, "Lobbycode fehlt", line!()),
            claim.match_id,
        )
        .await;
        return Ok(());
    };
    if claim.team_channels.is_empty() {
        set_lobby_state(pool, claim.match_id, STATE_LOBBY_POST_FAILED).await?;
        post_log(
            adapter,
            missing_target_message(claim.match_id, "lobby_code", line!()),
            claim.match_id,
        )
        .await;
        return Ok(());
    }

    match sync_lobby_code_messages(pool, adapter, claim.match_id, &claim.team_channels, &code).await
    {
        Ok(result) => {
            let state = if result.errors.is_empty() {
                STATE_LOBBY_POSTED
            } else {
                STATE_LOBBY_POST_FAILED
            };
            save_lobby_code_message_ids(pool, claim.match_id, &result.message_ids, state).await?;
            if result.errors.is_empty() {
                post_log(
                    adapter,
                    lobby_code_success_log_message(
                        claim.match_id,
                        &code,
                        result.message_ids.len(),
                        line!(),
                    ),
                    claim.match_id,
                )
                .await;
            } else {
                post_log(
                    adapter,
                    lobby_code_failure_message(claim.match_id, &result.errors.join("; "), line!()),
                    claim.match_id,
                )
                .await;
            }
        }
        Err(err) => {
            set_lobby_state(pool, claim.match_id, STATE_LOBBY_POST_FAILED).await?;
            post_log(
                adapter,
                lobby_code_failure_message(claim.match_id, &err, line!()),
                claim.match_id,
            )
            .await;
        }
    }
    Ok(())
}

async fn claim_next_pending_match_request_batch(
    pool: &PgPool,
) -> anyhow::Result<Option<ClaimedMatchRequestBatch>> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT id
              FROM scrim.match_request_batches
             WHERE status = $1
               AND deadline_at > now()
             ORDER BY created_at, id
             LIMIT 1
             FOR UPDATE SKIP LOCKED
        )
        UPDATE scrim.match_request_batches b
           SET status = $2,
               updated_at = now()
          FROM candidate
         WHERE b.id = candidate.id
        RETURNING b.id::bigint AS batch_id,
                  b.template,
                  b.deadline_at
        "#,
    )
    .bind(MATCH_REQUEST_STATUS_DRAFT)
    .bind(MATCH_REQUEST_STATUS_POSTING)
    .fetch_optional(&mut *tx)
    .await
    .context("Scrim-Match-Abfrage-Claim fehlgeschlagen")?;

    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    let batch_id: i64 = row.get("batch_id");
    let request_rows = sqlx::query(
        r#"
        SELECT mr.id::bigint AS request_id,
               mr.slot_options,
               ta.id::bigint AS team_a_id,
               ta.name AS team_a_name,
               ta.discord_channel_id AS team_a_channel_id,
               tb.id::bigint AS team_b_id,
               tb.name AS team_b_name,
               tb.discord_channel_id AS team_b_channel_id
          FROM scrim.match_requests mr
          JOIN scrim.teams ta ON ta.id = mr.team_a_id
          LEFT JOIN scrim.teams tb ON tb.id = mr.team_b_id
         WHERE mr.batch_id = $1
           AND mr.status = $2
         ORDER BY mr.id
        "#,
    )
    .bind(i32::try_from(batch_id)?)
    .bind(MATCH_REQUEST_STATUS_DRAFT)
    .fetch_all(&mut *tx)
    .await
    .context("Scrim-Match-Abfragen laden")?;
    tx.commit().await?;

    let mut requests = Vec::with_capacity(request_rows.len());
    for row in request_rows {
        let team_a_id: i64 = row.get("team_a_id");
        let team_a_name: String = row.get("team_a_name");
        let team_b_id: Option<i64> = row.get("team_b_id");
        let team_b_name: Option<String> = row.get("team_b_name");
        let mut targets = Vec::new();
        let mut missing_targets = Vec::new();
        if let Some(channel_id) = valid_channel_id(row.get("team_a_channel_id")) {
            targets.push(MatchRequestTarget {
                team_id: team_a_id,
                team_name: team_a_name.clone(),
                channel_id,
                opponent_name: team_b_name.clone(),
            });
        } else {
            missing_targets.push(team_a_name.clone());
        }
        if let (Some(team_id), Some(team_name)) = (team_b_id, team_b_name.clone()) {
            if let Some(channel_id) = valid_channel_id(row.get("team_b_channel_id")) {
                targets.push(MatchRequestTarget {
                    team_id,
                    team_name,
                    channel_id,
                    opponent_name: Some(team_a_name),
                });
            } else {
                missing_targets.push(team_name);
            }
        }
        requests.push(ClaimedMatchRequest {
            request_id: row.get("request_id"),
            slot_options: row.get("slot_options"),
            targets,
            missing_targets,
        });
    }

    Ok(Some(ClaimedMatchRequestBatch {
        batch_id,
        template: row.get("template"),
        deadline_at: row.get("deadline_at"),
        requests,
    }))
}

async fn handle_match_request_batch(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    batch: ClaimedMatchRequestBatch,
) -> anyhow::Result<()> {
    let result = send_match_request_batch(adapter, &batch).await;
    if !result.posts.is_empty() {
        save_match_request_posts(
            pool,
            batch.batch_id,
            &result.posts,
            if result.errors.is_empty() {
                MATCH_REQUEST_STATUS_OPEN
            } else {
                MATCH_REQUEST_STATUS_POST_FAILED
            },
        )
        .await?;
    }
    if result.errors.is_empty() {
        post_log(
            adapter,
            match_request_success_log_message(batch.batch_id, result.posts.len(), line!()),
            batch.batch_id,
        )
        .await;
    } else {
        if result.posts.is_empty() {
            set_match_request_batch_status(pool, batch.batch_id, MATCH_REQUEST_STATUS_POST_FAILED)
                .await?;
        }
        post_log(
            adapter,
            match_request_failure_message(batch.batch_id, &result.errors.join("; "), line!()),
            batch.batch_id,
        )
        .await;
    }
    Ok(())
}

async fn claim_next_pending_match_request_reminder(
    pool: &PgPool,
) -> anyhow::Result<Option<ClaimedMatchRequestReminder>> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        r#"
        SELECT r.id::bigint AS reminder_id,
               r.request_id::bigint AS request_id,
               r.team_id::bigint AS team_id,
               r.target_participant_ids,
               r.target_role_id,
               r.discord_channel_id,
               r.source_message_id,
               mr.status AS request_status,
               t.name AS team_name
          FROM scrim.match_request_reminders r
          JOIN scrim.match_requests mr ON mr.id = r.request_id
          JOIN scrim.teams t ON t.id = r.team_id
         WHERE r.status = $1
           AND r.scheduled_for <= now()
         ORDER BY r.scheduled_for, r.id
         LIMIT 1
         FOR UPDATE OF r, mr SKIP LOCKED
        "#,
    )
    .bind(MATCH_REQUEST_REMINDER_STATUS_APPROVED)
    .fetch_optional(&mut *tx)
    .await
    .context("Scrim-Reminder-Claim fehlgeschlagen")?;

    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    let reminder_id = row.get::<i64, _>("reminder_id");
    let request_id = row.get::<i64, _>("request_id");
    let team_id = row.get::<i64, _>("team_id");
    let request_status = row.get::<String, _>("request_status");
    if !matches!(
        request_status.as_str(),
        MATCH_REQUEST_STATUS_OPEN | MATCH_REQUEST_STATUS_POST_FAILED
    ) {
        cancel_match_request_reminder(&mut tx, reminder_id, "Terminabfrage geschlossen").await?;
        tx.commit().await?;
        return Ok(None);
    }
    let approved_participant_ids = row.get::<Vec<i32>, _>("target_participant_ids");
    let missing_rows = sqlx::query(
        r#"
        SELECT p.id, p.discord_id
          FROM scrim.participants p
         WHERE p.id = ANY($1)
           AND NOT EXISTS (
                SELECT 1
                  FROM scrim.match_request_responses response
                 WHERE response.request_id = $2
                   AND response.team_id = $3
                   AND response.participant_id = p.id
           )
         ORDER BY array_position($1, p.id), p.id
        "#,
    )
    .bind(&approved_participant_ids)
    .bind(i32::try_from(request_id)?)
    .bind(i32::try_from(team_id)?)
    .fetch_all(&mut *tx)
    .await?;
    if missing_rows.is_empty() {
        cancel_match_request_reminder(&mut tx, reminder_id, "Keine offenen Antworten mehr").await?;
        tx.commit().await?;
        return Ok(None);
    }

    let target_participant_ids = missing_rows
        .iter()
        .map(|missing| missing.get::<i32, _>("id"))
        .collect::<Vec<_>>();
    let target_discord_user_ids = missing_rows
        .iter()
        .filter_map(|missing| missing.get::<Option<i64>, _>("discord_id"))
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    let target_kind = if target_discord_user_ids.len() == target_participant_ids.len() {
        "members"
    } else {
        "team"
    };
    let target_role_id = row
        .get::<Option<i64>, _>("target_role_id")
        .and_then(|id| u64::try_from(id).ok().filter(|id| *id > 0));
    let stored_target_user_ids = if target_kind == "members" {
        target_discord_user_ids
    } else {
        Vec::new()
    };
    if target_kind == "team" && target_role_id.is_none() {
        fail_unaddressable_match_request_reminder(
            &mut tx,
            reminder_id,
            &target_participant_ids,
            "Keine pingbare Teamrolle hinterlegt",
        )
        .await?;
        tx.commit().await?;
        tracing::warn!(
            reminder_id,
            request_id,
            team_id,
            target_kind = "team",
            action = "failed",
            "Scrim-Reminder ohne pingbares Ziel nicht gepostet"
        );
        return Ok(None);
    }
    sqlx::query(
        r#"
        UPDATE scrim.match_request_reminders
           SET status = $2,
               target_kind = $3,
               target_participant_ids = $4,
               target_discord_user_ids = $5,
               missing_count = $6,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(reminder_id)
    .bind(MATCH_REQUEST_REMINDER_STATUS_POSTING)
    .bind(target_kind)
    .bind(&target_participant_ids)
    .bind(&stored_target_user_ids)
    .bind(i32::try_from(target_participant_ids.len())?)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let target_user_ids = stored_target_user_ids
        .into_iter()
        .filter_map(|id| u64::try_from(id).ok().filter(|id| *id > 0))
        .collect();
    Ok(Some(ClaimedMatchRequestReminder {
        reminder_id,
        request_id,
        team_id,
        team_name: row.get("team_name"),
        channel_id: u64::try_from(row.get::<i64, _>("discord_channel_id"))
            .context("Scrim-Reminder-Channel-ID ungültig")?,
        source_message_id: u64::try_from(row.get::<i64, _>("source_message_id"))
            .context("Scrim-Reminder-Source-Message-ID ungültig")?,
        target_kind: target_kind.to_string(),
        target_user_ids,
        target_role_id,
    }))
}

async fn fail_unaddressable_match_request_reminder(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    reminder_id: i64,
    target_participant_ids: &[i32],
    reason: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE scrim.match_request_reminders
           SET status = $2,
               target_kind = 'team',
               target_participant_ids = $3,
               target_discord_user_ids = '{}',
               missing_count = $4,
               last_error = $5,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(reminder_id)
    .bind(MATCH_REQUEST_REMINDER_STATUS_FAILED)
    .bind(target_participant_ids)
    .bind(i32::try_from(target_participant_ids.len())?)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn cancel_match_request_reminder(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    reminder_id: i64,
    reason: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE scrim.match_request_reminders
           SET status = $2,
               missing_count = 0,
               target_discord_user_ids = '{}',
               last_error = $3,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(reminder_id)
    .bind(MATCH_REQUEST_REMINDER_STATUS_CANCELLED)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn handle_match_request_reminder(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    reminder: ClaimedMatchRequestReminder,
) -> anyhow::Result<()> {
    let content = match_request_reminder_message(
        &reminder.team_name,
        &reminder.target_user_ids,
        reminder.target_role_id,
    );
    let mut body = match_request_reminder_body(
        &content,
        reminder.source_message_id,
        &reminder.target_user_ids,
    );
    if reminder.target_user_ids.is_empty() {
        body.insert(
            "allowed_mentions".to_string(),
            reminder.target_role_id.map_or_else(
                || json!({ "parse": [], "replied_user": false }),
                |role_id| json!({ "parse": [], "roles": [role_id.to_string()], "replied_user": false }),
            ),
        );
    }

    match adapter.send_raw_public(reminder.channel_id, &body).await {
        Ok(message_id) => {
            save_match_request_reminder_success(pool, reminder.reminder_id, message_id).await?;
            post_log(
                adapter,
                match_request_reminder_success_log_message(
                    reminder.reminder_id,
                    reminder.request_id,
                    reminder.team_id,
                    &reminder.target_kind,
                    line!(),
                ),
                reminder.request_id,
            )
            .await;
        }
        Err(err) => {
            save_match_request_reminder_failure(pool, reminder.reminder_id, &err).await?;
            post_log(
                adapter,
                match_request_reminder_failure_log_message(
                    reminder.reminder_id,
                    reminder.request_id,
                    &err,
                    line!(),
                ),
                reminder.request_id,
            )
            .await;
        }
    }
    Ok(())
}

async fn claim_next_pending_match_status(
    pool: &PgPool,
) -> anyhow::Result<Option<ClaimedMatchStatus>> {
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT mr.id,
                   mr.team_a_id,
                   ta.name AS team_a_name,
                   ta.discord_channel_id AS team_a_channel_id,
                   mr.team_b_id,
                   tb.name AS team_b_name,
                   tb.discord_channel_id AS team_b_channel_id,
                   mr.slot_options,
                   mr.released_slot_index,
                   mr.released_slot,
                   mr.team_query_message_ids,
                   mr.team_status_message_ids
              FROM scrim.match_requests mr
              JOIN scrim.teams ta ON ta.id = mr.team_a_id
              LEFT JOIN scrim.teams tb ON tb.id = mr.team_b_id
             WHERE mr.status = $3
               AND mr.released_at IS NOT NULL
               AND mr.released_slot_index IS NOT NULL
               AND mr.status_message_state = $1
             ORDER BY mr.released_at, mr.id
             LIMIT 1
             FOR UPDATE OF mr SKIP LOCKED
        )
        UPDATE scrim.match_requests mr
           SET status_message_state = $2,
               status_message_last_error = NULL,
               updated_at = now()
         FROM candidate
         WHERE mr.id = candidate.id
        RETURNING mr.id::bigint AS request_id,
                  candidate.team_a_id::bigint AS team_a_id,
                  candidate.team_a_name,
                  candidate.team_a_channel_id,
                  candidate.team_b_id::bigint AS team_b_id,
                  candidate.team_b_name,
                  candidate.team_b_channel_id,
                  candidate.slot_options,
                  candidate.released_slot_index,
                  candidate.released_slot,
                  candidate.team_query_message_ids,
                  candidate.team_status_message_ids
        "#,
    )
    .bind(MATCH_STATUS_MESSAGE_STATE_PENDING)
    .bind(MATCH_STATUS_MESSAGE_STATE_POSTING)
    .bind(MATCH_REQUEST_STATUS_CLOSED)
    .fetch_optional(pool)
    .await
    .context("Scrim-Match-Status-Claim fehlgeschlagen")?;

    let Some(row) = row else {
        return Ok(None);
    };
    let request_id: i64 = row.get("request_id");
    let team_a_id: i64 = row.get("team_a_id");
    let team_a_name: String = row.get("team_a_name");
    let team_a_channel_id: Option<i64> = row.get("team_a_channel_id");
    let team_b_id: Option<i64> = row.get("team_b_id");
    let team_b_name: Option<String> = row.get("team_b_name");
    let team_b_channel_id: Option<i64> = row.get("team_b_channel_id");
    let released_slot_index: i32 = row
        .get::<Option<i32>, _>("released_slot_index")
        .ok_or_else(|| anyhow!("freigegebener Slot fehlt fuer Request {request_id}"))?;
    let slot_options: Value = row.get("slot_options");
    let released_slot: Value = row
        .get::<Option<Value>, _>("released_slot")
        .or_else(|| {
            usize::try_from(released_slot_index)
                .ok()
                .and_then(|index| slot_options.as_array()?.get(index).cloned())
        })
        .ok_or_else(|| anyhow!("freigegebener Slot ungueltig fuer Request {request_id}"))?;
    let query_message_ids: Value = row.get("team_query_message_ids");
    let status_message_ids: Value = row.get("team_status_message_ids");

    let mut team_infos = vec![(team_a_id, team_a_name.clone(), team_a_channel_id)];
    if let (Some(team_id), Some(team_name)) = (team_b_id, team_b_name.clone()) {
        team_infos.push((team_id, team_name, team_b_channel_id));
    }
    let team_ids = team_infos
        .iter()
        .map(|(team_id, _, _)| i32::try_from(*team_id))
        .collect::<Result<Vec<_>, _>>()?;

    let member_rows = sqlx::query(
        r#"
        SELECT tm.team_id,
               p.id AS participant_id,
               p.display_name,
               tm.is_bench
          FROM scrim.team_members tm
          JOIN scrim.participants p ON p.id = tm.participant_id
         WHERE tm.team_id = ANY($1)
         ORDER BY tm.team_id ASC, tm.is_bench ASC, p.display_name ASC, p.id ASC
        "#,
    )
    .bind(&team_ids)
    .fetch_all(pool)
    .await?;
    let mut members = BTreeMap::<i64, Vec<MatchStatusMember>>::new();
    for member in member_rows {
        members
            .entry(i64::from(member.get::<i32, _>("team_id")))
            .or_default()
            .push(MatchStatusMember {
                participant_id: i64::from(member.get::<i32, _>("participant_id")),
                display_name: member.get("display_name"),
                is_bench: member.get("is_bench"),
            });
    }

    let response_rows = sqlx::query(
        r#"
        SELECT team_id, participant_id, slot_index, response
          FROM scrim.match_request_responses
         WHERE request_id = $1
           AND team_id = ANY($2)
        "#,
    )
    .bind(i32::try_from(request_id)?)
    .bind(&team_ids)
    .fetch_all(pool)
    .await?;
    let mut responses = BTreeMap::<i64, Vec<MatchStatusResponse>>::new();
    for response in response_rows {
        responses
            .entry(i64::from(response.get::<i32, _>("team_id")))
            .or_default()
            .push(MatchStatusResponse {
                participant_id: i64::from(response.get::<i32, _>("participant_id")),
                slot_index: response.get("slot_index"),
                response: response.get("response"),
            });
    }

    let mut targets = Vec::new();
    let mut missing_targets = Vec::new();
    for (team_id, team_name, channel_id) in &team_infos {
        let Some(channel_id) = valid_channel_id(*channel_id) else {
            missing_targets.push(format!("{team_name}: Teamkanal fehlt"));
            continue;
        };
        let Some((query_channel_id, query_message_id)) =
            message_ids_for_team(&query_message_ids, *team_id)
        else {
            missing_targets.push(format!("{team_name}: Terminabfrage fehlt"));
            continue;
        };
        let status_message_id = message_ids_for_team(&status_message_ids, *team_id).and_then(
            |(stored_channel_id, message_id)| {
                (stored_channel_id == channel_id).then_some(message_id)
            },
        );
        targets.push(MatchStatusTarget {
            team_id: *team_id,
            team_name: team_name.clone(),
            channel_id,
            query_channel_id,
            query_message_id,
            status_message_id,
        });
    }

    let teams = team_infos
        .iter()
        .map(|(team_id, team_name, _)| {
            match_status_team(
                *team_id,
                team_name,
                members.get(team_id).map(Vec::as_slice).unwrap_or(&[]),
                responses.get(team_id).map(Vec::as_slice).unwrap_or(&[]),
                released_slot_index,
            )
        })
        .collect();

    Ok(Some(ClaimedMatchStatus {
        request_id,
        team_a_name,
        team_b_name,
        released_slot,
        teams,
        targets,
        missing_targets,
    }))
}

fn match_status_team(
    team_id: i64,
    team_name: &str,
    members: &[MatchStatusMember],
    responses: &[MatchStatusResponse],
    released_slot_index: i32,
) -> MatchStatusTeam {
    let mut confirmed = Vec::new();
    let mut needs = Vec::new();
    let mut no_slot_count = 0usize;
    for member in members.iter().filter(|member| !member.is_bench) {
        let member_responses = responses
            .iter()
            .filter(|response| response.participant_id == member.participant_id)
            .collect::<Vec<_>>();
        if member_responses.iter().any(|response| {
            response.response == "available" && response.slot_index == released_slot_index
        }) {
            confirmed.push(member.display_name.clone());
            continue;
        }
        let reason = if member_responses
            .iter()
            .any(|response| response.response == "unavailable" && response.slot_index == -1)
        {
            no_slot_count += 1;
            "Kein Slot passt"
        } else if member_responses.is_empty() {
            "Antwort fehlt"
        } else {
            "Für finalen Slot nicht zugesagt"
        };
        needs.push(MatchStatusNeed {
            display_name: member.display_name.clone(),
            reason: reason.to_string(),
        });
    }
    MatchStatusTeam {
        team_id,
        team_name: team_name.to_string(),
        confirmed,
        needs,
        no_slot_count,
    }
}

async fn handle_match_status(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    status: ClaimedMatchStatus,
) -> anyhow::Result<()> {
    let result = sync_match_status_messages(adapter, &status).await;
    let mut errors = status.missing_targets.clone();
    errors.extend(result.errors);
    let error = (!errors.is_empty()).then(|| errors.join("; "));
    save_match_status_messages(pool, status.request_id, &result.posts, error.as_deref()).await?;

    if let Some(reason) = error {
        post_log(
            adapter,
            match_status_failure_log_message(status.request_id, &reason, line!()),
            status.request_id,
        )
        .await;
    } else {
        post_log(
            adapter,
            match_status_success_log_message(status.request_id, result.posts.len(), line!()),
            status.request_id,
        )
        .await;
    }
    Ok(())
}

async fn sync_match_status_messages(
    adapter: &dl_discord::DiscordAdapter,
    status: &ClaimedMatchStatus,
) -> MatchStatusSyncResult {
    let mut posts = Vec::new();
    let mut errors = Vec::new();
    if status.targets.is_empty() {
        errors.push("kein Teamkanal mit Terminabfrage".to_string());
    }
    for target in &status.targets {
        let content = match match_status_message(status, target) {
            Ok(content) => content,
            Err(err) => {
                errors.push(format!("request {}: {err}", status.request_id));
                continue;
            }
        };
        let body = message_body(&content);
        if let Some(message_id) = target.status_message_id {
            match adapter
                .edit_raw_public(target.channel_id, message_id, &body)
                .await
            {
                Ok(()) => posts.push(MatchStatusPost {
                    team_id: target.team_id,
                    channel_id: target.channel_id,
                    message_id,
                }),
                Err(err) => errors.push(format!(
                    "edit {}/{}: {}",
                    target.channel_id, message_id, err
                )),
            }
        } else {
            match adapter.send_raw_public(target.channel_id, &body).await {
                Ok(message_id) => posts.push(MatchStatusPost {
                    team_id: target.team_id,
                    channel_id: target.channel_id,
                    message_id,
                }),
                Err(err) => errors.push(format!("post {}: {err}", target.channel_id)),
            }
        }
    }
    MatchStatusSyncResult { posts, errors }
}

async fn save_match_status_messages(
    pool: &PgPool,
    request_id: i64,
    posts: &[MatchStatusPost],
    error: Option<&str>,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let raw = sqlx::query_scalar::<_, Value>(
        r#"
        SELECT team_status_message_ids
          FROM scrim.match_requests
         WHERE id = $1
         FOR UPDATE
        "#,
    )
    .bind(i32::try_from(request_id)?)
    .fetch_one(&mut *tx)
    .await?;
    let mut value = raw.as_object().cloned().unwrap_or_default();
    for post in posts {
        value.insert(
            post.team_id.to_string(),
            json!({
                "channel_id": post.channel_id,
                "message_id": post.message_id,
            }),
        );
    }
    sqlx::query(
        r#"
        UPDATE scrim.match_requests
           SET team_status_message_ids = $2::jsonb,
               status_message_state = $3,
               status_message_last_error = $4,
               status_message_posted_at = CASE
                   WHEN $5 THEN COALESCE(status_message_posted_at, now())
                   ELSE status_message_posted_at
               END,
               status_message_updated_at = now(),
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(i32::try_from(request_id)?)
    .bind(Value::Object(value))
    .bind(if error.is_some() {
        MATCH_STATUS_MESSAGE_STATE_POST_FAILED
    } else {
        MATCH_STATUS_MESSAGE_STATE_POSTED
    })
    .bind(error)
    .bind(!posts.is_empty())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn send_match_request_batch(
    adapter: &dl_discord::DiscordAdapter,
    batch: &ClaimedMatchRequestBatch,
) -> MatchRequestSendResult {
    let mut posts = Vec::new();
    let mut errors = Vec::new();
    if batch.requests.is_empty() {
        errors.push("keine Match-Abfragen".to_string());
    }
    for request in &batch.requests {
        if !request.missing_targets.is_empty() {
            errors.push(format!(
                "request {}: fehlender Teamkanal fuer {}",
                request.request_id,
                request.missing_targets.join(", ")
            ));
        }
        if request.targets.is_empty() {
            errors.push(format!("request {}: kein Teamkanal", request.request_id));
            continue;
        }
        for target in &request.targets {
            let content = match match_request_message(
                batch.batch_id,
                &batch.template,
                &target.team_name,
                target.opponent_name.as_deref(),
                batch.deadline_at.timestamp(),
                &request.slot_options,
            ) {
                Ok(content) => content,
                Err(err) => {
                    errors.push(format!("request {}: {err}", request.request_id));
                    continue;
                }
            };
            let body = match match_request_body(
                request.request_id,
                target.team_id,
                &content,
                &request.slot_options,
            ) {
                Ok(body) => body,
                Err(err) => {
                    errors.push(format!("request {}: {err}", request.request_id));
                    continue;
                }
            };
            match adapter.send_raw_public(target.channel_id, &body).await {
                Ok(message_id) => posts.push(MatchRequestPost {
                    request_id: request.request_id,
                    team_id: target.team_id,
                    channel_id: target.channel_id,
                    message_id,
                }),
                Err(err) => errors.push(format!("post {}: {err}", target.channel_id)),
            }
        }
    }
    MatchRequestSendResult { posts, errors }
}

async fn save_match_request_posts(
    pool: &PgPool,
    batch_id: i64,
    posts: &[MatchRequestPost],
    status: &str,
) -> anyhow::Result<()> {
    let mut by_request = BTreeMap::<i64, Map<String, Value>>::new();
    for post in posts {
        by_request.entry(post.request_id).or_default().insert(
            post.team_id.to_string(),
            json!({
                "channel_id": post.channel_id,
                "message_id": post.message_id,
            }),
        );
    }

    let mut tx = pool.begin().await?;
    for (request_id, message_ids) in by_request {
        sqlx::query(
            r#"
            UPDATE scrim.match_requests
               SET team_query_message_ids = $2::jsonb,
                   status = $3,
                   posted_at = COALESCE(posted_at, now()),
                   updated_at = now()
             WHERE id = $1
            "#,
        )
        .bind(i32::try_from(request_id)?)
        .bind(Value::Object(message_ids))
        .bind(status)
        .execute(&mut *tx)
        .await?;
    }
    if status != MATCH_REQUEST_STATUS_OPEN {
        sqlx::query(
            r#"
            UPDATE scrim.match_requests
               SET status = $2,
                   updated_at = now()
             WHERE batch_id = $1
               AND status <> $2
            "#,
        )
        .bind(i32::try_from(batch_id)?)
        .bind(status)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        r#"
        UPDATE scrim.match_request_batches
           SET status = $2,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(i32::try_from(batch_id)?)
    .bind(status)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn set_match_request_batch_status(
    pool: &PgPool,
    batch_id: i64,
    status: &str,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE scrim.match_request_batches
           SET status = $2,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(i32::try_from(batch_id)?)
    .bind(status)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE scrim.match_requests
           SET status = $2,
               updated_at = now()
         WHERE batch_id = $1
        "#,
    )
    .bind(i32::try_from(batch_id)?)
    .bind(status)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn save_match_request_reminder_success(
    pool: &PgPool,
    reminder_id: i64,
    message_id: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE scrim.match_request_reminders
           SET status = $2,
               discord_message_id = $3,
               last_error = NULL,
               posted_at = now(),
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(reminder_id)
    .bind(MATCH_REQUEST_REMINDER_STATUS_POSTED)
    .bind(i64::try_from(message_id)?)
    .execute(pool)
    .await?;
    Ok(())
}

async fn save_match_request_reminder_failure(
    pool: &PgPool,
    reminder_id: i64,
    error: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE scrim.match_request_reminders
           SET status = $2,
               last_error = $3,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(reminder_id)
    .bind(MATCH_REQUEST_REMINDER_STATUS_FAILED)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

async fn handle_start(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    voice_config: ScrimVoiceConfig,
    claim: &ClaimedMatch,
) -> anyhow::Result<()> {
    match start_scrim_match(pool, claim.match_id).await {
        Ok(outcome) => {
            if let Err(err) =
                provision_scrim_voice_channels(pool, tempvoice, voice_config, claim.match_id).await
            {
                tracing::error!(%err, match_id = claim.match_id, action = "error", "Scrim-Voice-Erstellung fehlgeschlagen");
            }
            post_to_targets_or_log(
                adapter,
                &claim.team_channels,
                start_success_message(claim.match_id, &outcome, line!()),
                "start_success",
                claim.match_id,
            )
            .await;
            post_log(
                adapter,
                start_success_log_message(claim.match_id, &outcome.join_code, line!()),
                claim.match_id,
            )
            .await;
        }
        Err(err) => {
            tracing::warn!(%err, match_id = claim.match_id, "Scrim-Match-Start fehlgeschlagen");
            set_lobby_state(pool, claim.match_id, STATE_START_FAILED).await?;
            post_log(
                adapter,
                start_failure_message(claim.match_id, line!()),
                claim.match_id,
            )
            .await;
        }
    }
    Ok(())
}

async fn claim_next_pending_match_result_ref(
    pool: &PgPool,
    match_id: i64,
) -> anyhow::Result<Option<ClaimedMatchResultRef>> {
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT id, steam_match_id
              FROM scrim.match_result_refs
             WHERE match_id = $1
               AND (
                    fetch_status IN ($2, $3)
                    OR (fetch_status = $4 AND updated_at <= now() - interval '15 minutes')
               )
              ORDER BY CASE
                           WHEN fetch_status = $2 THEN 0
                           WHEN fetch_status = $3 THEN 1
                           ELSE 2
                       END,
                       CASE WHEN fetch_status = $2 THEN entered_at ELSE updated_at END ASC,
                       id ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
        )
        UPDATE scrim.match_result_refs r
           SET fetch_status = $4,
               last_error = NULL,
               updated_at = now()
          FROM candidate
         WHERE r.id = candidate.id
        RETURNING r.id::bigint AS ref_id,
                  r.steam_match_id
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .bind(MATCH_RESULT_REF_STATUS_PENDING)
    .bind(MATCH_RESULT_REF_STATUS_FAILED)
    .bind(MATCH_RESULT_REF_STATUS_FETCHING)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("Scrim-Match-ID-Claim fehlgeschlagen: {match_id}"))?;

    Ok(row.map(|row| ClaimedMatchResultRef {
        ref_id: row.get("ref_id"),
        steam_match_id: row.get("steam_match_id"),
    }))
}

async fn save_match_result_ref_success(
    pool: &PgPool,
    ref_id: i64,
    outcome: &ScrimMatchResultOutcome,
) -> anyhow::Result<()> {
    let normalized = normalize_match_result_payload(outcome);
    sqlx::query(
        r#"
        UPDATE scrim.match_result_refs
           SET fetch_status = $2,
               fetched_at = now(),
               last_error = NULL,
               winner_team_id = $3,
               raw_result_json = $4,
               normalized_result_json = $5,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(ref_id)
    .bind(MATCH_RESULT_REF_STATUS_FETCHED)
    .bind(outcome.winner_team_id.map(i32::try_from).transpose()?)
    .bind(&outcome.result_json)
    .bind(&normalized)
    .execute(pool)
    .await
    .with_context(|| format!("Scrim-Match-ID-Ergebnis-Speicherung fehlgeschlagen: {ref_id}"))?;
    Ok(())
}

async fn save_match_result_ref_failure(
    pool: &PgPool,
    ref_id: i64,
    error: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE scrim.match_result_refs
           SET fetch_status = $2,
               last_error = $3,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(ref_id)
    .bind(MATCH_RESULT_REF_STATUS_FAILED)
    .bind(error)
    .execute(pool)
    .await
    .with_context(|| format!("Scrim-Match-ID-Fehler-Speicherung fehlgeschlagen: {ref_id}"))?;
    Ok(())
}

fn normalize_match_result_payload(outcome: &ScrimMatchResultOutcome) -> Value {
    let raw = &outcome.result_json;
    let mut obj = Map::new();
    if let Some(steam_match_id) = outcome.steam_match_id {
        obj.insert("steam_match_id".to_string(), json!(steam_match_id));
    }
    if let Some(winner_team_id) = outcome.winner_team_id {
        obj.insert("winner_team_id".to_string(), json!(winner_team_id));
    }
    copy_first(raw, &mut obj, "winner", &["winner", "winning_team"]);
    copy_first(
        raw,
        &mut obj,
        "match_time",
        &["match_time", "match_started_at", "start_time", "started_at"],
    );
    copy_first(
        raw,
        &mut obj,
        "duration",
        &["duration", "duration_s", "duration_seconds"],
    );
    copy_first(raw, &mut obj, "teams", &["teams", "team_results"]);
    copy_first(raw, &mut obj, "players", &["players", "player_results"]);
    copy_first(raw, &mut obj, "lineups", &["lineups", "lineup"]);
    copy_first(raw, &mut obj, "substitutes", &["substitutes", "standins"]);
    copy_first(
        raw,
        &mut obj,
        "stats",
        &["stats", "statistics", "performance"],
    );
    Value::Object(obj)
}

fn copy_first(raw: &Value, obj: &mut Map<String, Value>, target: &str, keys: &[&str]) {
    if let Some(value) = keys.iter().find_map(|key| raw.get(*key)) {
        obj.insert(target.to_string(), value.clone());
    }
}

async fn handle_result(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    claim: &ClaimedMatch,
    lagebild_ai: Option<&dyn dl_ai::ChatProvider>,
) -> anyhow::Result<()> {
    let claimed_ref = claim_next_pending_match_result_ref(pool, claim.match_id).await?;
    let result = if let Some(result_ref) = claimed_ref.as_ref() {
        fetch_scrim_match_result_by_steam_match_id(pool, claim.match_id, result_ref.steam_match_id)
            .await
    } else {
        fetch_scrim_match_result(pool, claim.match_id).await
    };

    match result {
        Ok(outcome) => {
            if let Some(result_ref) = claimed_ref.as_ref() {
                save_match_result_ref_success(pool, result_ref.ref_id, &outcome).await?;
            }
            if let Err(err) = cleanup_scrim_voice_channels_for_match(
                pool,
                tempvoice,
                claim.match_id,
                "Scrim beendet",
            )
            .await
            {
                tracing::error!(%err, match_id = claim.match_id, action = "error", "Scrim-Voice-Cleanup fehlgeschlagen");
            }
            post_to_targets_or_log(
                adapter,
                &claim.team_channels,
                result_success_message(claim.match_id, &outcome, line!()),
                "result_success",
                claim.match_id,
            )
            .await;
            post_log(
                adapter,
                result_success_log_message(claim.match_id, &outcome, line!()),
                claim.match_id,
            )
            .await;
            if set_result_state_after_attempt(pool, claim.match_id, false).await?
                == ResultQueueState::Finished
            {
                generate_lagebilder_after_result(pool, lagebild_ai, claim.match_id).await;
            }
        }
        Err(err) => {
            tracing::warn!(%err, match_id = claim.match_id, "Scrim-Match-Ergebnis fehlgeschlagen");
            if let Some(result_ref) = claimed_ref.as_ref() {
                save_match_result_ref_failure(pool, result_ref.ref_id, &err.to_string()).await?;
            }
            let queue_state = set_result_state_after_attempt(pool, claim.match_id, true).await?;
            if queue_state == ResultQueueState::Finished
                && !has_match_lagebild_snapshot(pool, claim.match_id).await?
            {
                generate_lagebilder_after_result(pool, lagebild_ai, claim.match_id).await;
            }
            post_log(
                adapter,
                result_failure_message(claim.match_id, &err.to_string(), line!()),
                claim.match_id,
            )
            .await;
        }
    }
    Ok(())
}

async fn generate_lagebilder_after_result(
    pool: &PgPool,
    lagebild_ai: Option<&dyn dl_ai::ChatProvider>,
    match_id: i64,
) {
    match dl_squads::lagebild::generate_match_lagebilder(pool, lagebild_ai, match_id).await {
        Ok(generated) if generated > 0 => {
            tracing::info!(match_id, generated, "Scrim-Lagebilder nach Match erzeugt");
        }
        Ok(_) => {}
        Err(err) => {
            tracing::error!(%err, match_id, "Scrim-Lagebilder nach Match fehlgeschlagen");
        }
    }
}

async fn has_match_lagebild_snapshot(pool: &PgPool, match_id: i64) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM scrim.lagebild_snapshots WHERE generated_for = $1 AND source = 'match')",
    )
    .bind(format!("match:{match_id}"))
    .fetch_one(pool)
    .await?)
}

async fn set_result_state_after_attempt(
    pool: &PgPool,
    match_id: i64,
    failed: bool,
) -> anyhow::Result<ResultQueueState> {
    let match_id_i32 = i32::try_from(match_id)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM scrim.matches WHERE id = $1 FOR UPDATE")
        .bind(match_id_i32)
        .fetch_one(&mut *tx)
        .await?;
    let has_pending = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1
              FROM scrim.match_result_refs
             WHERE match_id = $1
               AND fetch_status = $2
        )
        "#,
    )
    .bind(match_id_i32)
    .bind(MATCH_RESULT_REF_STATUS_PENDING)
    .fetch_one(&mut *tx)
    .await?;
    let has_fetching = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1
              FROM scrim.match_result_refs
             WHERE match_id = $1
               AND fetch_status = $2
        )
        "#,
    )
    .bind(match_id_i32)
    .bind(MATCH_RESULT_REF_STATUS_FETCHING)
    .fetch_one(&mut *tx)
    .await?;
    let has_success = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT result_json IS NOT NULL OR EXISTS(
                   SELECT 1
                     FROM scrim.match_result_refs
                    WHERE match_id = $1
                      AND fetch_status = $2
               )
          FROM scrim.matches
         WHERE id = $1
        "#,
    )
    .bind(match_id_i32)
    .bind(MATCH_RESULT_REF_STATUS_FETCHED)
    .fetch_one(&mut *tx)
    .await?;
    let queue_state = if has_pending {
        ResultQueueState::Pending
    } else if has_fetching {
        ResultQueueState::Fetching
    } else if has_success || !failed {
        ResultQueueState::Finished
    } else {
        ResultQueueState::Failed
    };
    let next_state = match queue_state {
        ResultQueueState::Pending => STATE_RESULT_REQUESTED,
        ResultQueueState::Fetching => STATE_RESULT_FETCHING,
        ResultQueueState::Finished => STATE_FINISHED,
        ResultQueueState::Failed => STATE_RESULT_FAILED,
    };
    sqlx::query("UPDATE scrim.matches SET lobby_state = $2, updated_at = now() WHERE id = $1")
        .bind(match_id_i32)
        .bind(next_state)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(queue_state)
}

async fn set_lobby_state(pool: &PgPool, match_id: i64, lobby_state: &str) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET lobby_state = $2,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .bind(lobby_state)
    .execute(pool)
    .await
    .with_context(|| format!("Scrim-Match-State-Update fehlgeschlagen: {match_id}"))?;
    Ok(())
}

async fn load_lobby_code(pool: &PgPool, match_id: i64) -> anyhow::Result<Option<String>> {
    let code = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT join_code
          FROM scrim.matches
         WHERE id = $1
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("Scrim-Lobbycode-Lookup fehlgeschlagen: {match_id}"))?
    .flatten()
    .map(|value| value.trim().to_ascii_uppercase())
    .filter(|value| !value.is_empty());
    Ok(code)
}

async fn load_lobby_code_message_ids(
    pool: &PgPool,
    match_id: i64,
) -> anyhow::Result<BTreeMap<u64, u64>> {
    let raw = sqlx::query_scalar::<_, Option<Value>>(
        r#"
        SELECT lobby_code_message_ids
          FROM scrim.matches
         WHERE id = $1
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("Scrim-Lobbycode-Message-Lookup fehlgeschlagen: {match_id}"))?
    .flatten()
    .unwrap_or_else(|| json!({}));
    let mut ids = BTreeMap::new();
    if let Some(obj) = raw.as_object() {
        for (channel_id, message_id) in obj {
            if let (Ok(channel_id), Some(message_id)) = (channel_id.parse(), message_id.as_u64()) {
                ids.insert(channel_id, message_id);
            }
        }
    }
    Ok(ids)
}

async fn sync_lobby_code_messages(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    match_id: i64,
    team_channels: &[u64],
    code: &str,
) -> Result<LobbyCodeSyncResult, String> {
    let mut stored = load_lobby_code_message_ids(pool, match_id)
        .await
        .map_err(|err| err.to_string())?;
    let content = lobby_code_message(code);
    let body = message_body(&content);
    let mut errors = Vec::new();

    for &channel_id in team_channels {
        if let Some(message_id) = stored.get(&channel_id).copied() {
            if let Err(err) = adapter.edit_raw_public(channel_id, message_id, &body).await {
                errors.push(format!("edit {channel_id}/{message_id}: {err}"));
            }
        } else {
            match adapter.send_raw_public(channel_id, &body).await {
                Ok(message_id) => {
                    stored.insert(channel_id, message_id);
                }
                Err(err) => errors.push(format!("post {channel_id}: {err}")),
            }
        }
    }

    Ok(LobbyCodeSyncResult {
        message_ids: stored,
        errors,
    })
}

async fn save_lobby_code_message_ids(
    pool: &PgPool,
    match_id: i64,
    message_ids: &BTreeMap<u64, u64>,
    lobby_state: &str,
) -> anyhow::Result<()> {
    let mut value = Map::new();
    for (channel_id, message_id) in message_ids {
        value.insert(channel_id.to_string(), json!(message_id));
    }
    sqlx::query(
        r#"
        UPDATE scrim.matches
           SET lobby_code_message_ids = $2::jsonb,
               lobby_state = $3,
               updated_at = now()
         WHERE id = $1
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .bind(Value::Object(value))
    .bind(lobby_state)
    .execute(pool)
    .await
    .with_context(|| format!("Scrim-Lobbycode-Message-Speicherung fehlgeschlagen: {match_id}"))?;
    Ok(())
}

#[derive(Debug)]
struct ScrimVoiceTeam {
    team_id: i64,
    connect_user_ids: Vec<u64>,
}

#[derive(Debug)]
struct ScrimVoiceChannel {
    match_id: i64,
    team_id: i64,
    channel_id: u64,
}

async fn provision_scrim_voice_channels(
    pool: &PgPool,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    config: ScrimVoiceConfig,
    match_id: i64,
) -> anyhow::Result<()> {
    match decide_scrim_voice_action(config.enabled, config.category_id, false, false) {
        ScrimVoiceAction::SkipDisabled => {
            tracing::info!(
                match_id,
                action = "skipped",
                reason = "feature_disabled",
                "Scrim-Voice-Entscheidung"
            );
            return Ok(());
        }
        ScrimVoiceAction::SkipMissingCategory => {
            tracing::warn!(
                match_id,
                action = "skipped",
                reason = "category_missing",
                "Scrim-Voice-Entscheidung"
            );
            return Ok(());
        }
        _ => {}
    }
    let category_id = config
        .category_id
        .ok_or_else(|| anyhow!("Scrim-Voice-Kategorie fehlt"))?;
    let teams = load_scrim_voice_teams(pool, match_id).await?;
    if teams.is_empty() {
        tracing::warn!(
            match_id,
            action = "skipped",
            reason = "teams_missing",
            "Scrim-Voice-Entscheidung"
        );
        return Ok(());
    }

    for (team_index, team) in teams.into_iter().enumerate() {
        let channel_name = format!("{} {}", SCRIM_VOICE_CHANNEL_NAME, team_index + 1);
        let existing = stored_scrim_voice_channel(pool, match_id, team.team_id).await?;
        match decide_scrim_voice_action(true, Some(category_id), existing.is_some(), false) {
            ScrimVoiceAction::Keep => {
                tracing::info!(
                    match_id,
                    team_id = team.team_id,
                    channel_id = existing,
                    action = "skipped",
                    reason = "already_created",
                    "Scrim-Voice-Entscheidung"
                );
                continue;
            }
            ScrimVoiceAction::Create => {}
            action => {
                tracing::warn!(
                    match_id,
                    team_id = team.team_id,
                    decision = ?action,
                    action = "skipped",
                    reason = "unexpected_decision",
                    "Scrim-Voice-Entscheidung"
                );
                continue;
            }
        }

        let channel_id = match tempvoice
            .create_restricted_voice_channel(category_id, &channel_name, &team.connect_user_ids)
            .await
        {
            Ok(channel_id) => channel_id,
            Err(err) => {
                tracing::error!(%err, match_id, team_id = team.team_id, action = "error", reason = "discord_create_failed", "Scrim-Voice-Entscheidung");
                continue;
            }
        };

        match store_scrim_voice_channel(pool, match_id, team.team_id, channel_id).await {
            Ok(true) => tracing::info!(
                match_id,
                team_id = team.team_id,
                channel_id,
                action = "created",
                "Scrim-Voice-Entscheidung"
            ),
            Ok(false) => {
                tracing::info!(
                    match_id,
                    team_id = team.team_id,
                    channel_id,
                    action = "skipped",
                    reason = "concurrent_create",
                    "Scrim-Voice-Entscheidung"
                );
                if let Err(err) = tempvoice
                    .delete_managed_voice_channel(channel_id, "Scrim: doppelten Voice entfernen")
                    .await
                {
                    tracing::error!(%err, match_id, team_id = team.team_id, channel_id, action = "error", reason = "duplicate_cleanup_failed", "Scrim-Voice-Entscheidung");
                }
            }
            Err(err) => {
                tracing::error!(%err, match_id, team_id = team.team_id, channel_id, action = "error", reason = "persist_failed", "Scrim-Voice-Entscheidung");
                if let Err(cleanup_err) = tempvoice
                    .delete_managed_voice_channel(channel_id, "Scrim: ungetrackten Voice entfernen")
                    .await
                {
                    tracing::error!(%cleanup_err, match_id, team_id = team.team_id, channel_id, action = "error", reason = "rollback_cleanup_failed", "Scrim-Voice-Entscheidung");
                }
            }
        }
    }
    Ok(())
}

async fn load_scrim_voice_teams(
    pool: &PgPool,
    match_id: i64,
) -> anyhow::Result<Vec<ScrimVoiceTeam>> {
    let rows = sqlx::query(
        r#"
        SELECT t.id::bigint AS team_id,
               p.discord_id,
               m.coach_spectator_discord_id
          FROM scrim.matches m
          JOIN scrim.teams t ON t.id IN (m.team_a_id, m.team_b_id)
          LEFT JOIN scrim.team_members tm ON tm.team_id = t.id
          LEFT JOIN scrim.participants p ON p.id = tm.participant_id
         WHERE m.id = $1
         ORDER BY t.id, p.id
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .fetch_all(pool)
    .await
    .context("Scrim-Voice-Teams laden")?;
    let mut teams = BTreeMap::<i64, BTreeSet<u64>>::new();
    for row in rows {
        let team_id: i64 = row.get("team_id");
        let users = teams.entry(team_id).or_default();
        if let Some(user_id) = valid_channel_id(row.get("discord_id")) {
            users.insert(user_id);
        }
        if let Some(coach_id) = valid_channel_id(row.get("coach_spectator_discord_id")) {
            users.insert(coach_id);
        }
    }
    Ok(teams
        .into_iter()
        .map(|(team_id, connect_user_ids)| ScrimVoiceTeam {
            team_id,
            connect_user_ids: connect_user_ids.into_iter().collect(),
        })
        .collect())
}

async fn stored_scrim_voice_channel(
    pool: &PgPool,
    match_id: i64,
    team_id: i64,
) -> anyhow::Result<Option<u64>> {
    let channel_id = sqlx::query_scalar::<_, i64>(
        "SELECT channel_id FROM scrim.voice_channels WHERE match_id = $1 AND team_id = $2",
    )
    .bind(i32::try_from(match_id)?)
    .bind(i32::try_from(team_id)?)
    .fetch_optional(pool)
    .await?;
    Ok(channel_id.and_then(|id| u64::try_from(id).ok()))
}

async fn store_scrim_voice_channel(
    pool: &PgPool,
    match_id: i64,
    team_id: i64,
    channel_id: u64,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        r#"
        INSERT INTO scrim.voice_channels(match_id, team_id, channel_id)
        VALUES($1, $2, $3)
        ON CONFLICT (match_id, team_id) DO NOTHING
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .bind(i32::try_from(team_id)?)
    .bind(i64::try_from(channel_id)?)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

async fn cleanup_terminal_scrim_voice_channels(
    pool: &PgPool,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
) -> anyhow::Result<()> {
    let rows = sqlx::query(
        r#"
        SELECT vc.match_id::bigint AS match_id,
               vc.team_id::bigint AS team_id,
               vc.channel_id
          FROM scrim.voice_channels vc
          LEFT JOIN scrim.matches m ON m.id = vc.match_id
         WHERE m.id IS NULL
            OR lower(COALESCE(m.lobby_state, '')) IN ('finished', 'aborted', 'cancelled', 'canceled')
            OR lower(COALESCE(m.status, '')) IN ('finished', 'completed', 'aborted', 'cancelled', 'canceled')
         ORDER BY vc.match_id, vc.team_id
        "#,
    )
    .fetch_all(pool)
    .await
    .context("terminale Scrim-Voice-Channels laden")?;
    for row in rows {
        let channel_id: i64 = row.get("channel_id");
        let Ok(channel_id) = u64::try_from(channel_id) else {
            tracing::error!(
                match_id = row.get::<i64, _>("match_id"),
                team_id = row.get::<i64, _>("team_id"),
                channel_id,
                action = "error",
                reason = "invalid_channel_id",
                "Scrim-Voice-Cleanup"
            );
            continue;
        };
        cleanup_scrim_voice_channel(
            pool,
            tempvoice,
            ScrimVoiceChannel {
                match_id: row.get("match_id"),
                team_id: row.get("team_id"),
                channel_id,
            },
            "Scrim beendet oder abgebrochen",
        )
        .await?;
    }
    Ok(())
}

async fn cleanup_scrim_voice_channels_for_match(
    pool: &PgPool,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    match_id: i64,
    reason: &str,
) -> anyhow::Result<()> {
    let rows = sqlx::query(
        r#"
        SELECT team_id::bigint AS team_id, channel_id
          FROM scrim.voice_channels
         WHERE match_id = $1
         ORDER BY team_id
        "#,
    )
    .bind(i32::try_from(match_id)?)
    .fetch_all(pool)
    .await?;
    for row in rows {
        let channel_id: i64 = row.get("channel_id");
        let Ok(channel_id) = u64::try_from(channel_id) else {
            tracing::error!(
                match_id,
                team_id = row.get::<i64, _>("team_id"),
                channel_id,
                action = "error",
                reason = "invalid_channel_id",
                "Scrim-Voice-Cleanup"
            );
            continue;
        };
        cleanup_scrim_voice_channel(
            pool,
            tempvoice,
            ScrimVoiceChannel {
                match_id,
                team_id: row.get("team_id"),
                channel_id,
            },
            reason,
        )
        .await?;
    }
    Ok(())
}

async fn cleanup_scrim_voice_channel(
    pool: &PgPool,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    channel: ScrimVoiceChannel,
    reason: &str,
) -> anyhow::Result<()> {
    let delete_result = tempvoice
        .delete_managed_voice_channel(channel.channel_id, reason)
        .await;
    let already_missing = delete_result
        .as_ref()
        .is_err_and(|err| channel_already_missing(err));
    if let Err(err) = delete_result {
        if !already_missing {
            tracing::error!(%err, match_id = channel.match_id, team_id = channel.team_id, channel_id = channel.channel_id, action = "error", reason = "discord_delete_failed", "Scrim-Voice-Cleanup");
            return Ok(());
        }
    }
    if let Err(err) =
        sqlx::query("DELETE FROM scrim.voice_channels WHERE match_id = $1 AND team_id = $2")
            .bind(i32::try_from(channel.match_id)?)
            .bind(i32::try_from(channel.team_id)?)
            .execute(pool)
            .await
    {
        tracing::error!(%err, match_id = channel.match_id, team_id = channel.team_id, channel_id = channel.channel_id, action = "error", reason = "record_delete_failed", "Scrim-Voice-Cleanup");
        return Err(err.into());
    }
    tracing::info!(
        match_id = channel.match_id,
        team_id = channel.team_id,
        channel_id = channel.channel_id,
        action = "cleanup",
        already_missing,
        "Scrim-Voice-Cleanup"
    );
    Ok(())
}

fn channel_already_missing(err: &str) -> bool {
    err.contains("Unknown Channel") || err.contains("10003") || err.contains("404")
}

fn target_channels(team_a_channel_id: Option<i64>, team_b_channel_id: Option<i64>) -> Vec<u64> {
    [team_a_channel_id, team_b_channel_id]
        .into_iter()
        .filter_map(valid_channel_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn valid_channel_id(channel_id: Option<i64>) -> Option<u64> {
    u64::try_from(channel_id?).ok().filter(|id| *id > 0)
}

async fn post_to_targets_or_log(
    adapter: &dl_discord::DiscordAdapter,
    channel_ids: &[u64],
    content: String,
    message_kind: &'static str,
    match_id: i64,
) {
    if channel_ids.is_empty() {
        post_log(
            adapter,
            missing_target_message(match_id, message_kind, line!()),
            match_id,
        )
        .await;
        return;
    }

    for channel_id in channel_ids {
        if let Err(err) = send_content(adapter, *channel_id, &content).await {
            tracing::warn!(%err, match_id, channel_id, "Scrim-Match-Discord-Post fehlgeschlagen");
            post_log(
                adapter,
                target_post_failure_message(match_id, message_kind, *channel_id, &err, line!()),
                match_id,
            )
            .await;
        }
    }
}

async fn post_log(adapter: &dl_discord::DiscordAdapter, content: String, match_id: i64) {
    if let Err(err) = send_content(adapter, LOG_CHANNEL_ID, &content).await {
        tracing::warn!(%err, match_id, channel_id = LOG_CHANNEL_ID, "Scrim-Match-Log-Post fehlgeschlagen");
    }
}

async fn send_content(
    adapter: &dl_discord::DiscordAdapter,
    channel_id: u64,
    content: &str,
) -> Result<u64, String> {
    adapter
        .send_raw_public(channel_id, &message_body(content))
        .await
}

fn message_body(content: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("content".to_string(), json!(content));
    body.insert("allowed_mentions".to_string(), json!({ "parse": [] }));
    body
}

fn match_request_body(
    request_id: i64,
    team_id: i64,
    content: &str,
    slot_options: &Value,
) -> anyhow::Result<Map<String, Value>> {
    let mut body = message_body(content);
    body.insert(
        "components".to_string(),
        match_request_components(request_id, team_id, slot_options)?,
    );
    Ok(body)
}

fn start_success_message(
    _match_id: i64,
    outcome: &StartScrimMatchOutcome,
    _source_line: u32,
) -> String {
    lobby_code_message(&outcome.join_code)
}

fn lobby_code_message(code: &str) -> String {
    format!("Lobby Code: {}", code.trim().to_ascii_uppercase())
}

fn match_request_message(
    batch_id: i64,
    template: &str,
    team_name: &str,
    opponent_name: Option<&str>,
    deadline_ts: i64,
    slot_options: &Value,
) -> anyhow::Result<String> {
    let slots = slot_options
        .as_array()
        .ok_or_else(|| anyhow!("slot_options ist kein Array"))?;
    let mut lines = vec![
        format!("Scrim-Terminabfrage #{batch_id}"),
        format!("Team: {team_name}"),
        format!("Gegner: {}", opponent_name.unwrap_or("offen")),
        format!("Vorlage: {}", template_label(template)),
        format!("Antwortfrist: <t:{deadline_ts}:f>"),
        "Slots:".to_string(),
    ];
    for (index, slot) in slots.iter().enumerate() {
        lines.push(format!(
            "{}. {}",
            index + 1,
            format_match_request_slot(slot)?
        ));
    }
    lines.push(
        "Antwortet bitte über die Buttons. Freitext könnt ihr ergänzen, wenn etwas unklar ist."
            .to_string(),
    );
    Ok(lines.join("\n"))
}

fn match_status_message(
    status: &ClaimedMatchStatus,
    target: &MatchStatusTarget,
) -> anyhow::Result<String> {
    let target_team = status
        .teams
        .iter()
        .find(|team| team.team_id == target.team_id)
        .ok_or_else(|| anyhow!("Statusziel ohne Teamdaten: {}", target.team_id))?;
    let match_label = status.team_b_name.as_ref().map_or_else(
        || format!("{} vs Gegner offen", status.team_a_name),
        |team_b_name| format!("{} vs {}", status.team_a_name, team_b_name),
    );
    let vote_summary = status
        .teams
        .iter()
        .map(match_status_vote_summary)
        .collect::<Vec<_>>()
        .join(". ");
    let mut lines = vec![
        format!("Scrim-Status #{}", status.request_id),
        format!("Match: {match_label}"),
        format!(
            "Finaler Termin: {}",
            format_match_request_slot(&status.released_slot)?
        ),
        format!("Abstimmung: {vote_summary}."),
        format!(
            "Zusagen {}: {}",
            target.team_name,
            format_name_list(&target_team.confirmed)
        ),
        format!(
            "Fehlt/unsicher {}: {}",
            target.team_name,
            format_need_list(&target_team.needs)
        ),
    ];
    if target_team.needs.is_empty() {
        lines.push("Ersatzbedarf: keiner sichtbar.".to_string());
        lines.push("Nächste Aktion: Lineup bestätigen und Lobby vorbereiten.".to_string());
    } else {
        lines.push(format!(
            "Ersatzbedarf: {} Spieler klären: {}",
            target_team.needs.len(),
            format_need_list(&target_team.needs)
        ));
        lines.push("Nächste Aktion: Ersatz klären, dann Lineup bestätigen.".to_string());
    }
    lines.push(format!(
        "Terminabfrage: {}",
        discord_message_url(target.query_channel_id, target.query_message_id)
    ));
    Ok(lines.join("\n"))
}

fn match_status_vote_summary(team: &MatchStatusTeam) -> String {
    format!(
        "{}: {} {}, {} offen/unsicher, {} Kein Slot passt",
        team.team_name,
        team.confirmed.len(),
        if team.confirmed.len() == 1 {
            "Zusage"
        } else {
            "Zusagen"
        },
        team.needs.len(),
        team.no_slot_count
    )
}

fn format_name_list(names: &[String]) -> String {
    if names.is_empty() {
        "keine".to_string()
    } else {
        names.join(", ")
    }
}

fn format_need_list(needs: &[MatchStatusNeed]) -> String {
    if needs.is_empty() {
        return "keine".to_string();
    }
    needs
        .iter()
        .map(|need| format!("{} ({})", need.display_name, need.reason))
        .collect::<Vec<_>>()
        .join(", ")
}

fn discord_message_url(channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/channels/{MAIN_GUILD_ID}/{channel_id}/{message_id}")
}

fn match_request_reminder_message(
    team_name: &str,
    target_user_ids: &[u64],
    target_role_id: Option<u64>,
) -> String {
    let target = if target_user_ids.is_empty() {
        target_role_id.map_or_else(|| "euch".to_string(), |id| format!("<@&{id}>"))
    } else {
        target_user_ids
            .iter()
            .map(|id| format!("<@{id}>"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "Erinnerung für {team_name}: Es fehlen noch Antworten von {target}.\nBitte stimmt in der Terminabfrage oben ab."
    )
}

fn match_request_reminder_body(
    content: &str,
    source_message_id: u64,
    target_user_ids: &[u64],
) -> Map<String, Value> {
    let mut body = message_body(content);
    body.insert(
        "message_reference".to_string(),
        json!({ "message_id": source_message_id.to_string(), "fail_if_not_exists": false }),
    );
    body.insert(
        "allowed_mentions".to_string(),
        json!({
            "parse": [],
            "users": target_user_ids.iter().map(u64::to_string).collect::<Vec<_>>(),
            "replied_user": false,
        }),
    );
    body
}

fn match_request_components(
    request_id: i64,
    team_id: i64,
    slot_options: &Value,
) -> anyhow::Result<Value> {
    let slots = slot_options
        .as_array()
        .ok_or_else(|| anyhow!("slot_options ist kein Array"))?;
    let mut buttons = Vec::with_capacity(slots.len() + 1);
    for index in 0..slots.len() {
        buttons.push(json!({
            "type": 2,
            "style": 3,
            "label": format!("Slot {} passt", index + 1),
            "custom_id": format!("{MATCH_REQUEST_RESPONSE_PREFIX}slot:{request_id}:{team_id}:{index}"),
        }));
    }
    buttons.push(json!({
        "type": 2,
        "style": 2,
        "label": "Kein Slot passt",
        "custom_id": format!("{MATCH_REQUEST_RESPONSE_PREFIX}none:{request_id}:{team_id}"),
    }));

    Ok(Value::Array(
        buttons
            .chunks(5)
            .map(|chunk| {
                json!({
                    "type": 1,
                    "components": chunk,
                })
            })
            .collect(),
    ))
}

fn parse_match_request_response_custom_id(value: &str) -> Option<ParsedMatchRequestResponse> {
    let rest = value.strip_prefix(MATCH_REQUEST_RESPONSE_PREFIX)?;
    let mut parts = rest.split(':');
    let action = parts.next()?;
    let request_id = parts.next()?.parse().ok()?;
    let team_id = parts.next()?.parse().ok()?;
    let slot_index = match action {
        "slot" => parts.next()?.parse().ok()?,
        "none" => -1,
        _ => return None,
    };
    if parts.next().is_some() || request_id <= 0 || team_id <= 0 || slot_index < -1 {
        return None;
    }
    Some(ParsedMatchRequestResponse {
        request_id,
        team_id,
        slot_index,
    })
}

fn match_request_response_lock_keys(
    request_id: i64,
    participant_id: i64,
) -> anyhow::Result<(i32, i32)> {
    Ok((i32::try_from(request_id)?, i32::try_from(participant_id)?))
}

async fn record_match_request_response(
    pool: &PgPool,
    input: MatchRequestResponseInput,
) -> anyhow::Result<MatchRequestResponseOutcome> {
    let Some(parsed) = parse_match_request_response_custom_id(&input.custom_id) else {
        return Ok(MatchRequestResponseOutcome::InvalidAction);
    };
    let user_id = i64::try_from(input.user_id).context("Discord-User-ID zu gross")?;
    let channel_id = i64::try_from(input.channel_id).context("Discord-Channel-ID zu gross")?;
    let message_id = input
        .message_id
        .map(i64::try_from)
        .transpose()
        .context("Discord-Message-ID zu gross")?;

    let Some(row) = sqlx::query(
        r#"
        SELECT mr.status,
               mr.slot_options,
               mr.team_query_message_ids,
               tm.participant_id::bigint AS participant_id
          FROM scrim.match_requests mr
          LEFT JOIN scrim.participants p ON p.discord_id = $3
          LEFT JOIN scrim.team_members tm ON tm.participant_id = p.id AND tm.team_id = $2
         WHERE mr.id = $1
           AND (mr.team_a_id = $2 OR mr.team_b_id = $2)
        "#,
    )
    .bind(i32::try_from(parsed.request_id)?)
    .bind(i32::try_from(parsed.team_id)?)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(MatchRequestResponseOutcome::InvalidAction);
    };

    let status = row.get::<String, _>("status");
    if !matches!(
        status.as_str(),
        MATCH_REQUEST_STATUS_OPEN | MATCH_REQUEST_STATUS_POST_FAILED
    ) {
        return Ok(MatchRequestResponseOutcome::NotOpen);
    }
    let participant_id = row.get::<Option<i64>, _>("participant_id");
    let Some(participant_id) = participant_id else {
        return Ok(MatchRequestResponseOutcome::WrongTeam);
    };
    let slot_options: Value = row.get("slot_options");
    let slot_count = slot_options
        .as_array()
        .ok_or_else(|| anyhow!("slot_options ist kein Array"))?
        .len();
    if parsed.slot_index >= 0 && parsed.slot_index as usize >= slot_count {
        return Ok(MatchRequestResponseOutcome::InvalidAction);
    }
    let message_ids: Value = row.get("team_query_message_ids");
    if !posted_message_matches(&message_ids, parsed.team_id, channel_id, message_id) {
        return Ok(MatchRequestResponseOutcome::WrongMessage);
    }

    let mut tx = pool.begin().await?;
    let (lock_a, lock_b) = match_request_response_lock_keys(parsed.request_id, participant_id)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(lock_a)
        .bind(lock_b)
        .execute(&mut *tx)
        .await?;
    let current_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM scrim.match_requests WHERE id = $1 FOR UPDATE",
    )
    .bind(i32::try_from(parsed.request_id)?)
    .fetch_one(&mut *tx)
    .await?;
    if !matches!(
        current_status.as_str(),
        MATCH_REQUEST_STATUS_OPEN | MATCH_REQUEST_STATUS_POST_FAILED
    ) {
        tx.rollback().await?;
        return Ok(MatchRequestResponseOutcome::NotOpen);
    }
    if parsed.slot_index == -1 {
        sqlx::query(
            r#"
            DELETE FROM scrim.match_request_responses
             WHERE request_id = $1
               AND team_id = $2
               AND participant_id = $3
               AND slot_index <> -1
            "#,
        )
        .bind(i32::try_from(parsed.request_id)?)
        .bind(i32::try_from(parsed.team_id)?)
        .bind(i32::try_from(participant_id)?)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            r#"
            DELETE FROM scrim.match_request_responses
             WHERE request_id = $1
               AND team_id = $2
               AND participant_id = $3
               AND slot_index = -1
            "#,
        )
        .bind(i32::try_from(parsed.request_id)?)
        .bind(i32::try_from(parsed.team_id)?)
        .bind(i32::try_from(participant_id)?)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        r#"
        INSERT INTO scrim.match_request_responses(
            request_id, team_id, participant_id, discord_user_id, slot_index, response,
            source, message_id, channel_id, responded_at, updated_at
        )
        VALUES($1, $2, $3, $4, $5, $6, 'button', $7, $8, now(), now())
        ON CONFLICT(request_id, team_id, participant_id, slot_index) DO UPDATE SET
            discord_user_id = excluded.discord_user_id,
            response = excluded.response,
            source = excluded.source,
            message_id = excluded.message_id,
            channel_id = excluded.channel_id,
            responded_at = now(),
            updated_at = now()
        "#,
    )
    .bind(i32::try_from(parsed.request_id)?)
    .bind(i32::try_from(parsed.team_id)?)
    .bind(i32::try_from(participant_id)?)
    .bind(user_id)
    .bind(parsed.slot_index)
    .bind(if parsed.slot_index == -1 {
        "unavailable"
    } else {
        "available"
    })
    .bind(message_id)
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(if parsed.slot_index == -1 {
        MatchRequestResponseOutcome::SavedNone
    } else {
        MatchRequestResponseOutcome::SavedSlot(parsed.slot_index + 1)
    })
}

fn posted_message_matches(
    message_ids: &Value,
    team_id: i64,
    channel_id: i64,
    message_id: Option<i64>,
) -> bool {
    let Some(entry) = message_ids.get(team_id.to_string()) else {
        return false;
    };
    entry.get("channel_id").and_then(Value::as_i64) == Some(channel_id)
        && entry.get("message_id").and_then(Value::as_i64) == message_id
}

fn message_ids_for_team(message_ids: &Value, team_id: i64) -> Option<(u64, u64)> {
    let entry = message_ids.get(team_id.to_string())?;
    let channel_id = entry
        .get("channel_id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)?;
    let message_id = entry
        .get("message_id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)?;
    Some((channel_id, message_id))
}

fn match_request_response_reply(outcome: MatchRequestResponseOutcome) -> dl_discord::BridgeReply {
    let text = match outcome {
        MatchRequestResponseOutcome::SavedSlot(slot) => {
            format!("Antwort gespeichert: Slot {slot} passt.")
        }
        MatchRequestResponseOutcome::SavedNone => {
            "Antwort gespeichert: Kein Slot passt.".to_string()
        }
        MatchRequestResponseOutcome::InvalidAction => {
            "Diese Terminantwort ist ungültig.".to_string()
        }
        MatchRequestResponseOutcome::NotOpen => {
            "Diese Abstimmung ist nicht mehr offen.".to_string()
        }
        MatchRequestResponseOutcome::WrongMessage => {
            "Diese Antwort passt nicht zu dieser Terminabfrage.".to_string()
        }
        MatchRequestResponseOutcome::WrongTeam => {
            "Diese Terminabfrage gehört nicht zu deinem Team.".to_string()
        }
    };
    dl_discord::BridgeReply::ephemeral_text(text)
}

fn template_label(template: &str) -> &str {
    match template {
        "regular_scrim" => "Regulärer Scrim",
        "testmatch" => "Testmatch",
        "training" => "Training/Teamspiel",
        value => value,
    }
}

fn format_match_request_slot(slot: &Value) -> anyhow::Result<String> {
    let day = slot
        .get("day")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("slot day fehlt"))?;
    let from = slot
        .get("from")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("slot from fehlt"))?;
    let to = slot
        .get("to")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("slot to fehlt"))?;
    Ok(format!(
        "{} {}-{}",
        day_label(day),
        minute_label(from)?,
        minute_label(to)?
    ))
}

fn day_label(day: &str) -> &str {
    match day {
        "mon" => "Montag",
        "tue" => "Dienstag",
        "wed" => "Mittwoch",
        "thu" => "Donnerstag",
        "fri" => "Freitag",
        "sat" => "Samstag",
        "sun" => "Sonntag",
        value => value,
    }
}

fn minute_label(value: i64) -> anyhow::Result<String> {
    if !(0..=1440).contains(&value) {
        return Err(anyhow!("slot minute ausserhalb des Tages"));
    }
    Ok(format!("{:02}:{:02}", value / 60, value % 60))
}

fn lobby_code_success_log_message(
    match_id: i64,
    code: &str,
    target_count: usize,
    _source_line: u32,
) -> String {
    format!(
        "Scrim-Lobbycode gepostet (Match {match_id}, Ziele: {target_count}, Code: {}).",
        code
    )
}

fn start_success_log_message(match_id: i64, code: &str, _source_line: u32) -> String {
    format!(
        "Scrim-Lobby gestartet (Match {match_id}, Code: {}).",
        code.trim().to_ascii_uppercase()
    )
}

fn result_success_log_message(
    match_id: i64,
    outcome: &ScrimMatchResultOutcome,
    _source_line: u32,
) -> String {
    outcome.winner_team_id.map_or_else(
        || format!("Scrim-Ergebnis gespeichert (Match {match_id})."),
        |team_id| format!("Scrim-Ergebnis gespeichert (Match {match_id}, Sieger: Team {team_id})."),
    )
}

fn target_post_failure_message(
    match_id: i64,
    message_kind: &str,
    channel_id: u64,
    reason: &str,
    _source_line: u32,
) -> String {
    format!(
        "Scrim-Meldung konnte nicht gepostet werden (Match {match_id}, Typ {message_kind}, Kanal {channel_id}): {}.",
        short_error(reason)
    )
}

fn lobby_code_failure_message(match_id: i64, reason: &str, _source_line: u32) -> String {
    format!("Scrim-Lobbycode konnte nicht gepostet werden (Match {match_id}): {reason}.")
}

fn match_request_success_log_message(
    batch_id: i64,
    target_count: usize,
    _source_line: u32,
) -> String {
    format!("Scrim-Terminabfragen gepostet (Batch {batch_id}, Ziele: {target_count}).")
}

fn match_request_failure_message(batch_id: i64, reason: &str, _source_line: u32) -> String {
    format!("Scrim-Terminabfragen konnten nicht gepostet werden (Batch {batch_id}): {reason}.")
}

fn match_status_success_log_message(
    request_id: i64,
    target_count: usize,
    _source_line: u32,
) -> String {
    format!("Scrim-Match-Status gepostet (Abfrage {request_id}, Ziele: {target_count}).")
}

fn match_status_failure_log_message(request_id: i64, reason: &str, _source_line: u32) -> String {
    format!("Scrim-Match-Status konnte nicht gepostet werden (Abfrage {request_id}): {reason}.")
}

fn match_request_reminder_success_log_message(
    reminder_id: i64,
    request_id: i64,
    team_id: i64,
    target_kind: &str,
    _source_line: u32,
) -> String {
    format!(
        "Scrim-Reminder gepostet (Reminder {reminder_id}, Abfrage {request_id}, Team {team_id}, Ziel: {target_kind})."
    )
}

fn match_request_reminder_failure_log_message(
    reminder_id: i64,
    request_id: i64,
    reason: &str,
    _source_line: u32,
) -> String {
    format!(
        "Scrim-Reminder konnte nicht gepostet werden (Reminder {reminder_id}, Abfrage {request_id}): {reason}."
    )
}

fn start_failure_message(match_id: i64, _source_line: u32) -> String {
    format!(
        "Scrim-Lobby ließ sich nicht starten (Match {match_id}). Der Steam-GC war nicht erreichbar oder die Lobby-Erstellung schlug fehl. Details stehen im Bot-Log. Der Start lässt sich erneut anstoßen."
    )
}

// ponytail: Sieger als interne Team-ID; Auflösung auf den Team-Namen ist ein Follow-up, falls es stört.
fn result_success_message(
    match_id: i64,
    outcome: &ScrimMatchResultOutcome,
    _source_line: u32,
) -> String {
    match outcome.winner_team_id {
        Some(team_id) => {
            format!("Scrim beendet. Das Ergebnis ist eingetragen. Sieger: Team {team_id}.")
        }
        None => {
            format!("Scrim beendet. Das Ergebnis ist eingetragen (Match {match_id}).")
        }
    }
}

fn result_failure_message(match_id: i64, error: &str, _source_line: u32) -> String {
    format!(
        "Scrim-Ergebnis konnte noch nicht abgerufen werden (Match {match_id}). Fehler: {}. Der Abruf wird später erneut versucht und kann auch manuell angestoßen werden.",
        short_error(error)
    )
}

fn short_error(error: &str) -> String {
    let mut out = error.trim().chars().take(220).collect::<String>();
    if error.trim().chars().count() > 220 {
        out.push_str("...");
    }
    if out.is_empty() {
        "unbekannt".to_string()
    } else {
        out
    }
}

fn missing_target_message(match_id: i64, message_kind: &str, _source_line: u32) -> String {
    format!(
        "Scrim-Meldung ohne Zielkanal (Match {match_id}, Typ {message_kind}). Bitte die Teamkanäle prüfen."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    #[test]
    fn scrim_voice_entscheidung_deckt_create_skip_keep_und_cleanup_ab() {
        assert_eq!(
            decide_scrim_voice_action(false, Some(10), false, false),
            ScrimVoiceAction::SkipDisabled
        );
        assert_eq!(
            decide_scrim_voice_action(true, None, false, false),
            ScrimVoiceAction::SkipMissingCategory
        );
        assert_eq!(
            decide_scrim_voice_action(true, Some(10), false, false),
            ScrimVoiceAction::Create
        );
        assert_eq!(
            decide_scrim_voice_action(true, Some(10), true, false),
            ScrimVoiceAction::Keep
        );
        assert_eq!(
            decide_scrim_voice_action(false, None, true, true),
            ScrimVoiceAction::Cleanup
        );
        assert_eq!(
            decide_scrim_voice_action(true, Some(10), false, true),
            ScrimVoiceAction::SkipNoChannel
        );
    }

    #[tokio::test]
    async fn claim_next_pending_match_markiert_start_und_verhindert_doppelstart() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 12, STATE_STARTING).await?;
        insert_match(pool, 10, STATE_START_REQUESTED).await?;
        insert_match(pool, 11, STATE_RESULT_REQUESTED).await?;
        insert_match(pool, 13, "lobby_open").await?;

        let first = claim_next_pending_match(pool).await?;
        assert_eq!(
            first,
            Some(ClaimedMatch {
                match_id: 10,
                action: ScrimDriverAction::Start,
                team_channels: vec![100, 200],
            })
        );
        assert_eq!(lobby_state(pool, 10).await?, STATE_STARTING);

        let second = claim_next_pending_match(pool).await?;
        assert_eq!(
            second,
            Some(ClaimedMatch {
                match_id: 11,
                action: ScrimDriverAction::FetchResult,
                team_channels: vec![100, 200],
            })
        );
        assert_eq!(lobby_state(pool, 11).await?, STATE_RESULT_FETCHING);
        assert_eq!(lobby_state(pool, 12).await?, STATE_STARTING);
        let third = claim_next_pending_match(pool).await?;
        assert_eq!(
            third,
            Some(ClaimedMatch {
                match_id: 13,
                action: ScrimDriverAction::PostLobbyCode,
                team_channels: vec![100, 200],
            })
        );
        assert_eq!(lobby_state(pool, 13).await?, "lobby_posting");
        assert_eq!(claim_next_pending_match(pool).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn match_result_ref_status_speichert_fehler_raw_und_normaldaten() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_RESULT_REQUESTED).await?;
        insert_match_result_ref(pool, 70, 30, 987_654_321).await?;

        let claim = claim_next_pending_match_result_ref(pool, 30)
            .await?
            .expect("result ref claim");
        assert_eq!(claim.ref_id, 70);
        assert_eq!(claim.steam_match_id, 987_654_321);
        assert_eq!(
            match_result_ref_status(pool, 70).await?,
            ("fetching".to_string(), None)
        );

        save_match_result_ref_failure(pool, 70, "match data not ready").await?;
        assert_eq!(
            match_result_ref_status(pool, 70).await?,
            (
                "failed".to_string(),
                Some("match data not ready".to_string())
            )
        );

        insert_match_result_ref(pool, 71, 30, 987_654_322).await?;
        let pending = claim_next_pending_match_result_ref(pool, 30)
            .await?
            .expect("pending refs win over failed refs");
        assert_eq!(pending.ref_id, 71);
        assert_eq!(pending.steam_match_id, 987_654_322);
        save_match_result_ref_failure(pool, 71, "new match data not ready").await?;

        let retry = claim_next_pending_match_result_ref(pool, 30)
            .await?
            .expect("failed refs are retryable");
        assert_eq!(retry.ref_id, 70);
        save_match_result_ref_failure(pool, 70, "still not ready").await?;
        let next_retry = claim_next_pending_match_result_ref(pool, 30)
            .await?
            .expect("failed refs rotate by last attempt");
        assert_eq!(next_retry.ref_id, 71);

        let raw = json!({
            "match_id": 987654321,
            "winning_team": 0,
            "teams": [{"team": 0, "players": ["76561197960265729"]}],
            "players": [{"steam_id": "76561197960265729", "kills": 12, "deaths": 4}],
            "lineups": {"0": ["76561197960265729"]},
            "substitutes": [{"steam_id": "76561197960265730", "reason": "standin"}],
            "start_time": 1_785_000_000,
            "duration_s": 1810,
            "stats": {"team_damage": [12345, 11000]}
        });
        let outcome = ScrimMatchResultOutcome {
            steam_match_id: Some(987_654_321),
            winner_team_id: Some(1),
            result_json: raw.clone(),
        };
        save_match_result_ref_success(pool, 70, &outcome).await?;

        let row = sqlx::query(
            r#"
            SELECT fetch_status,
                   last_error,
                   raw_result_json,
                   normalized_result_json,
                   winner_team_id,
                   fetched_at
              FROM scrim.match_result_refs
             WHERE id = 70
            "#,
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(row.get::<String, _>("fetch_status"), "fetched");
        assert_eq!(row.get::<Option<String>, _>("last_error"), None);
        assert_eq!(row.get::<Option<Value>, _>("raw_result_json"), Some(raw));
        let normalized = row
            .get::<Option<Value>, _>("normalized_result_json")
            .expect("normalized");
        assert_eq!(normalized["steam_match_id"], 987_654_321);
        assert_eq!(normalized["winner_team_id"], 1);
        assert_eq!(normalized["teams"][0]["team"], 0);
        assert_eq!(normalized["players"][0]["steam_id"], "76561197960265729");
        assert_eq!(normalized["lineups"]["0"][0], "76561197960265729");
        assert_eq!(normalized["substitutes"][0]["reason"], "standin");
        assert_eq!(normalized["match_time"], 1_785_000_000);
        assert_eq!(normalized["stats"]["team_damage"][0], 12345);
        assert_eq!(row.get::<Option<i32>, _>("winner_team_id"), Some(1));
        assert!(row.get::<Option<DateTime<Utc>>, _>("fetched_at").is_some());
        Ok(())
    }

    #[tokio::test]
    async fn result_state_bleibt_bei_neuer_pending_id_angefordert() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_RESULT_FETCHING).await?;

        let mut tx = pool.begin().await?;
        sqlx::query("SELECT id FROM scrim.matches WHERE id = 30 FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;

        let pool_for_state = pool.clone();
        let mut update =
            tokio::spawn(
                async move { set_result_state_after_attempt(&pool_for_state, 30, true).await },
            );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut update)
                .await
                .is_err()
        );

        insert_match_result_ref_tx(&mut tx, 70, 30, 987_654_321).await?;
        sqlx::query("UPDATE scrim.matches SET lobby_state = $2 WHERE id = $1")
            .bind(30)
            .bind(STATE_RESULT_REQUESTED)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        assert_eq!(update.await??, ResultQueueState::Pending);
        assert_eq!(lobby_state(pool, 30).await?, STATE_RESULT_REQUESTED);
        Ok(())
    }

    #[tokio::test]
    async fn alter_result_fehler_wird_nach_backoff_neu_geclaimt() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_RESULT_FAILED).await?;
        insert_match_result_ref(pool, 70, 30, 987_654_321).await?;
        sqlx::query("UPDATE scrim.match_result_refs SET fetch_status = 'failed' WHERE id = 70")
            .execute(pool)
            .await?;
        sqlx::query(
            "UPDATE scrim.matches SET updated_at = now() - interval '16 minutes' WHERE id = 30",
        )
        .execute(pool)
        .await?;

        let claim = claim_next_pending_match_or_result_retry(pool)
            .await?
            .expect("stale result failure must be retried");
        assert_eq!(claim.match_id, 30);
        assert_eq!(claim.action, ScrimDriverAction::FetchResult);
        assert_eq!(lobby_state(pool, 30).await?, STATE_RESULT_FETCHING);
        Ok(())
    }

    #[tokio::test]
    async fn frische_lobby_aktion_hat_vorrang_vor_altem_result_retry() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_RESULT_FAILED).await?;
        sqlx::query(
            "UPDATE scrim.matches SET updated_at = now() - interval '16 minutes' WHERE id = 30",
        )
        .execute(pool)
        .await?;
        insert_match(pool, 31, STATE_LOBBY_OPEN).await?;

        let claim = claim_next_pending_match_or_result_retry(pool)
            .await?
            .expect("active lobby claim");
        assert_eq!(claim.match_id, 31);
        assert_eq!(claim.action, ScrimDriverAction::PostLobbyCode);
        Ok(())
    }

    #[tokio::test]
    async fn erfolgreiches_result_bleibt_finished_wenn_weitere_id_fehlschlaegt() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_RESULT_FETCHING).await?;
        insert_match_result_ref(pool, 70, 30, 987_654_321).await?;
        insert_match_result_ref(pool, 71, 30, 987_654_322).await?;
        sqlx::query(
            "UPDATE scrim.match_result_refs SET fetch_status = CASE id WHEN 70 THEN 'fetched' ELSE 'failed' END WHERE match_id = 30",
        )
        .execute(pool)
        .await?;

        assert_eq!(
            set_result_state_after_attempt(pool, 30, true).await?,
            ResultQueueState::Finished
        );

        assert_eq!(lobby_state(pool, 30).await?, "finished");
        Ok(())
    }

    #[tokio::test]
    async fn result_queue_bleibt_offen_solange_eine_weitere_id_fetching_ist() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_RESULT_FETCHING).await?;
        insert_match_result_ref(pool, 70, 30, 987_654_321).await?;
        insert_match_result_ref(pool, 71, 30, 987_654_322).await?;
        sqlx::query(
            "UPDATE scrim.match_result_refs SET fetch_status = CASE id WHEN 70 THEN 'fetched' ELSE 'fetching' END WHERE match_id = 30",
        )
        .execute(pool)
        .await?;

        assert_eq!(
            set_result_state_after_attempt(pool, 30, false).await?,
            ResultQueueState::Fetching
        );
        assert_eq!(lobby_state(pool, 30).await?, STATE_RESULT_FETCHING);
        Ok(())
    }

    #[tokio::test]
    async fn fehlgeschlagene_id_wird_trotz_finished_match_neu_geclaimt() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, "finished").await?;
        insert_match_result_ref(pool, 70, 30, 987_654_321).await?;
        sqlx::query(
            "UPDATE scrim.match_result_refs SET fetch_status = 'failed', updated_at = now() - interval '16 minutes' WHERE id = 70",
        )
        .execute(pool)
        .await?;

        let claim = claim_next_pending_match_or_result_retry(pool)
            .await?
            .expect("stale failed ref on finished match");
        assert_eq!(claim.match_id, 30);
        assert_eq!(claim.action, ScrimDriverAction::FetchResult);
        Ok(())
    }

    #[tokio::test]
    async fn haengender_result_abruf_wird_nach_backoff_neu_geclaimt() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_RESULT_FETCHING).await?;
        insert_match_result_ref(pool, 70, 30, 987_654_321).await?;
        sqlx::query(
            "UPDATE scrim.matches SET updated_at = now() - interval '16 minutes' WHERE id = 30",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE scrim.match_result_refs SET fetch_status = 'fetching', updated_at = now() - interval '16 minutes' WHERE id = 70",
        )
        .execute(pool)
        .await?;

        let claim = claim_next_pending_match_or_result_retry(pool)
            .await?
            .expect("stale fetching claim");
        assert_eq!(claim.match_id, 30);
        let result_ref = claim_next_pending_match_result_ref(pool, 30)
            .await?
            .expect("stale fetching ref");
        assert_eq!(result_ref.ref_id, 70);
        Ok(())
    }

    #[tokio::test]
    async fn operative_scrim_posts_bleiben_in_den_teamkanaelen() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 30, STATE_LOBBY_OPEN).await?;

        let claim = claim_next_pending_match(pool)
            .await?
            .expect("lobby post claim");
        assert_eq!(claim.team_channels, vec![100, 200]);
        Ok(())
    }

    #[test]
    fn lobby_code_message_ist_schlicht_und_pingfrei() {
        assert_eq!(lobby_code_message("ABC12"), "Lobby Code: ABC12");
        let body = message_body(&lobby_code_message("ABC12"));
        assert_eq!(body["allowed_mentions"], json!({ "parse": [] }));
    }

    #[test]
    fn match_request_message_ist_teambezogen_pingfrei_und_enthaelt_slots() -> TestResult {
        let content = match_request_message(
            7,
            "regular_scrim",
            "Team Alpha",
            Some("Team Beta"),
            1_783_354_800,
            &json!([
                { "day": "sat", "from": 1200, "to": 1320 },
                { "day": "sun", "from": 1200, "to": 1320 }
            ]),
        )?;
        assert!(content.contains("Team Alpha"));
        assert!(content.contains("Team Beta"));
        assert!(content.contains("Samstag 20:00-22:00"));
        assert!(content.contains("Regulärer Scrim"));
        assert!(!content.contains("Regulaerer"));
        assert!(content.contains("<t:1783354800:f>"));
        assert!(!content.contains("<@"));
        assert_eq!(
            message_body(&content)["allowed_mentions"],
            json!({ "parse": [] })
        );
        Ok(())
    }

    #[test]
    fn match_request_components_enthalten_slot_buttons() -> TestResult {
        let components = match_request_components(
            31,
            1,
            &json!([
                { "day": "sat", "from": 1200, "to": 1320 },
                { "day": "sun", "from": 1200, "to": 1320 }
            ]),
        )?;

        assert_eq!(
            components,
            json!([
                {
                    "type": 1,
                    "components": [
                        {
                            "type": 2,
                            "style": 3,
                            "label": "Slot 1 passt",
                            "custom_id": "scrimreq:v1:slot:31:1:0"
                        },
                        {
                            "type": 2,
                            "style": 3,
                            "label": "Slot 2 passt",
                            "custom_id": "scrimreq:v1:slot:31:1:1"
                        },
                        {
                            "type": 2,
                            "style": 2,
                            "label": "Kein Slot passt",
                            "custom_id": "scrimreq:v1:none:31:1"
                        }
                    ]
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn match_request_reminder_body_verlinkt_abfrage_und_pinged_nur_fehlende() {
        let content = match_request_reminder_message("Team Alpha", &[555, 666], None);
        assert!(content.contains("Team Alpha"));
        assert!(content.contains("<@555>"));
        assert!(content.contains("<@666>"));
        assert!(!content.contains("<@777>"));

        let body = match_request_reminder_body(&content, 9001, &[555, 666]);
        assert_eq!(
            body["message_reference"],
            json!({ "message_id": "9001", "fail_if_not_exists": false })
        );
        assert_eq!(
            body["allowed_mentions"],
            json!({ "parse": [], "users": ["555", "666"], "replied_user": false })
        );
    }

    #[test]
    fn match_request_reminder_nennt_teamrolle_wenn_einzelziele_fehlen() {
        let content = match_request_reminder_message("Team Alpha", &[], Some(888));
        assert!(content.contains("<@&888>"));
        assert!(!content.contains("von euch"));
    }

    #[test]
    fn sichtbare_scrim_ergebnistexte_sind_klar_und_ohne_standard_emojis() {
        let outcome = ScrimMatchResultOutcome {
            steam_match_id: Some(123),
            winner_team_id: Some(1),
            result_json: json!({}),
        };
        let success = result_success_message(30, &outcome, line!());
        let failure = result_failure_message(30, "noch nicht verfügbar", line!());

        for text in [success, failure] {
            assert!(!text.contains(['⚠', '🏁', '🤝', '—', '–']));
        }
    }

    #[test]
    fn match_status_message_enthaelt_freigabe_ergebnis_ersatzbedarf_link_und_bleibt_pingfrei(
    ) -> TestResult {
        let content = match_status_message(
            &ClaimedMatchStatus {
                request_id: 31,
                team_a_name: "team-1".to_string(),
                team_b_name: Some("team-2".to_string()),
                released_slot: json!({ "day": "sat", "from": 1200, "to": 1320 }),
                teams: vec![
                    MatchStatusTeam {
                        team_id: 1,
                        team_name: "team-1".to_string(),
                        confirmed: vec!["Alice".to_string(), "Bob".to_string()],
                        needs: vec![MatchStatusNeed {
                            display_name: "Cara".to_string(),
                            reason: "Antwort fehlt".to_string(),
                        }],
                        no_slot_count: 0,
                    },
                    MatchStatusTeam {
                        team_id: 2,
                        team_name: "team-2".to_string(),
                        confirmed: vec!["Dino".to_string()],
                        needs: Vec::new(),
                        no_slot_count: 0,
                    },
                ],
                targets: Vec::new(),
                missing_targets: Vec::new(),
            },
            &MatchStatusTarget {
                team_id: 1,
                team_name: "team-1".to_string(),
                channel_id: 100,
                query_channel_id: 100,
                query_message_id: 9001,
                status_message_id: None,
            },
        )?;

        assert!(content.contains("Scrim-Status #31"));
        assert!(content.contains("Match: team-1 vs team-2"));
        assert!(content.contains("Finaler Termin: Samstag 20:00-22:00"));
        assert!(content.contains("team-1: 2 Zusagen, 1 offen/unsicher"));
        assert!(content.contains("Zusagen team-1: Alice, Bob"));
        assert!(content.contains("Fehlt/unsicher team-1: Cara (Antwort fehlt)"));
        assert!(content.contains("Ersatzbedarf: 1 Spieler klären"));
        assert!(content
            .contains("Terminabfrage: https://discord.com/channels/1289721245281292288/100/9001"));
        assert!(!content.contains("<@"));
        assert_eq!(
            message_body(&content)["allowed_mentions"],
            json!({ "parse": [] })
        );
        Ok(())
    }

    #[tokio::test]
    async fn match_request_reminder_claim_und_result_speichern_ziel_zeitpunkt_und_fehler(
    ) -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_team_member(pool, 1, 502, 666).await?;
        insert_team_member(pool, 2, 601, 777).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;
        insert_match_request_response(pool, 31, 1, 501, 555, 0, (Some(9001), Some(100))).await?;
        insert_match_request_reminder(pool, 80, 31, 1, &[502], &[666]).await?;

        let claim = claim_next_pending_match_request_reminder(pool)
            .await?
            .expect("reminder claim");
        assert_eq!(claim.reminder_id, 80);
        assert_eq!(claim.request_id, 31);
        assert_eq!(claim.team_id, 1);
        assert_eq!(claim.channel_id, 100);
        assert_eq!(claim.source_message_id, 9001);
        assert_eq!(claim.target_user_ids, vec![666]);
        assert_eq!(claim.target_kind, "members");
        assert_eq!(match_request_reminder_status(pool, 80).await?, "posting");

        save_match_request_reminder_failure(pool, 80, "discord down").await?;
        let row = sqlx::query(
            r#"
            SELECT status, last_error, posted_at, discord_message_id
              FROM scrim.match_request_reminders
             WHERE id = 80
            "#,
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(row.get::<String, _>("status"), "failed");
        assert_eq!(
            row.get::<Option<String>, _>("last_error"),
            Some("discord down".to_string())
        );
        assert!(row
            .get::<Option<chrono::DateTime<Utc>>, _>("posted_at")
            .is_none());
        assert!(row.get::<Option<i64>, _>("discord_message_id").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn reminder_claim_entfernt_inzwischen_beantwortete_personen() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_team_member(pool, 1, 502, 666).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;
        insert_match_request_reminder(pool, 80, 31, 1, &[501, 502], &[555, 666]).await?;
        insert_match_request_response(pool, 31, 1, 501, 555, 0, (Some(9001), Some(100))).await?;

        let claim = claim_next_pending_match_request_reminder(pool)
            .await?
            .expect("one missing participant remains");
        assert_eq!(claim.target_user_ids, vec![666]);
        assert_eq!(claim.target_kind, "members");
        Ok(())
    }

    #[tokio::test]
    async fn reminder_claim_storniert_wenn_niemand_mehr_fehlt() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;
        insert_match_request_reminder(pool, 80, 31, 1, &[501], &[555]).await?;
        insert_match_request_response(pool, 31, 1, 501, 555, 0, (Some(9001), Some(100))).await?;

        assert_eq!(claim_next_pending_match_request_reminder(pool).await?, None);
        assert_eq!(match_request_reminder_status(pool, 80).await?, "cancelled");
        Ok(())
    }

    #[tokio::test]
    async fn reminder_claim_storniert_nach_freigabe_des_slots() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;
        insert_match_request_reminder(pool, 80, 31, 1, &[501], &[555]).await?;
        sqlx::query("UPDATE scrim.match_requests SET status = 'closed' WHERE id = 31")
            .execute(pool)
            .await?;

        assert_eq!(claim_next_pending_match_request_reminder(pool).await?, None);
        assert_eq!(match_request_reminder_status(pool, 80).await?, "cancelled");
        Ok(())
    }

    #[tokio::test]
    async fn reminder_ohne_pingbares_ziel_wird_nicht_gepostet() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        sqlx::query("UPDATE scrim.participants SET discord_id = NULL WHERE id = 501")
            .execute(pool)
            .await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;
        insert_match_request_reminder(pool, 80, 31, 1, &[501], &[]).await?;

        assert_eq!(claim_next_pending_match_request_reminder(pool).await?, None);
        assert_eq!(match_request_reminder_status(pool, 80).await?, "failed");
        Ok(())
    }

    #[tokio::test]
    async fn match_request_buttonantwort_validiert_team_und_speichert_slot() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;

        let outcome = record_match_request_response(
            pool,
            MatchRequestResponseInput {
                custom_id: "scrimreq:v1:slot:31:1:0".to_string(),
                user_id: 555,
                channel_id: 100,
                message_id: Some(9001),
            },
        )
        .await?;

        assert_eq!(outcome, MatchRequestResponseOutcome::SavedSlot(1));
        let row = sqlx::query(
            r#"
            SELECT request_id, team_id, participant_id, discord_user_id, slot_index, response
              FROM scrim.match_request_responses
             WHERE request_id = 31 AND team_id = 1 AND participant_id = 501
            "#,
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(row.get::<i32, _>("request_id"), 31);
        assert_eq!(row.get::<i32, _>("team_id"), 1);
        assert_eq!(row.get::<i32, _>("participant_id"), 501);
        assert_eq!(row.get::<i64, _>("discord_user_id"), 555);
        assert_eq!(row.get::<i32, _>("slot_index"), 0);
        assert_eq!(row.get::<String, _>("response"), "available");

        let wrong_team = record_match_request_response(
            pool,
            MatchRequestResponseInput {
                custom_id: "scrimreq:v1:slot:31:2:0".to_string(),
                user_id: 555,
                channel_id: 200,
                message_id: Some(9002),
            },
        )
        .await?;
        assert_eq!(wrong_team, MatchRequestResponseOutcome::WrongTeam);
        Ok(())
    }

    #[tokio::test]
    async fn match_request_buttonantwort_serialisiert_userwechsel() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;

        let (lock_a, lock_b) = match_request_response_lock_keys(31, 501)?;
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(lock_a)
            .bind(lock_b)
            .execute(&mut *tx)
            .await?;

        let pool_for_click = pool.clone();
        let mut click = tokio::spawn(async move {
            record_match_request_response(
                &pool_for_click,
                MatchRequestResponseInput {
                    custom_id: "scrimreq:v1:slot:31:1:0".to_string(),
                    user_id: 555,
                    channel_id: 100,
                    message_id: Some(9001),
                },
            )
            .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut click)
                .await
                .is_err(),
            "zweiter Klick darf denselben User/Request nicht parallel schreiben"
        );
        tx.commit().await?;
        assert_eq!(click.await??, MatchRequestResponseOutcome::SavedSlot(1));
        Ok(())
    }

    #[tokio::test]
    async fn match_request_buttonantwort_schreibt_nicht_nach_schliessung() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[MatchRequestPost {
                request_id: 31,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;

        let (lock_a, lock_b) = match_request_response_lock_keys(31, 501)?;
        let mut lock_tx = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(lock_a)
            .bind(lock_b)
            .execute(&mut *lock_tx)
            .await?;

        let pool_for_click = pool.clone();
        let mut click = tokio::spawn(async move {
            record_match_request_response(
                &pool_for_click,
                MatchRequestResponseInput {
                    custom_id: "scrimreq:v1:slot:31:1:0".to_string(),
                    user_id: 555,
                    channel_id: 100,
                    message_id: Some(9001),
                },
            )
            .await
        });
        assert!(tokio::time::timeout(Duration::from_millis(100), &mut click)
            .await
            .is_err());

        sqlx::query("UPDATE scrim.match_requests SET status = 'closed' WHERE id = 31")
            .execute(pool)
            .await?;
        lock_tx.commit().await?;

        assert_eq!(click.await??, MatchRequestResponseOutcome::NotOpen);
        let stored: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scrim.match_request_responses WHERE request_id = 31",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(stored, 0);
        Ok(())
    }

    #[tokio::test]
    async fn match_request_buttonantwort_akzeptiert_sichtbare_teilfehler_posts() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, None).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_match_request_batch(pool, 40).await?;
        save_match_request_posts(
            pool,
            40,
            &[MatchRequestPost {
                request_id: 41,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_POST_FAILED,
        )
        .await?;

        let outcome = record_match_request_response(
            pool,
            MatchRequestResponseInput {
                custom_id: "scrimreq:v1:slot:41:1:0".to_string(),
                user_id: 555,
                channel_id: 100,
                message_id: Some(9001),
            },
        )
        .await?;

        assert_eq!(outcome, MatchRequestResponseOutcome::SavedSlot(1));
        Ok(())
    }

    #[tokio::test]
    async fn match_status_claim_speichert_ids_und_liefert_edit_ziel() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_team_member(pool, 1, 501, 555).await?;
        insert_team_member(pool, 1, 502, 666).await?;
        insert_team_member(pool, 2, 601, 777).await?;
        insert_match_request_batch(pool, 30).await?;
        save_match_request_posts(
            pool,
            30,
            &[
                MatchRequestPost {
                    request_id: 31,
                    team_id: 1,
                    channel_id: 100,
                    message_id: 9001,
                },
                MatchRequestPost {
                    request_id: 31,
                    team_id: 2,
                    channel_id: 200,
                    message_id: 9002,
                },
            ],
            MATCH_REQUEST_STATUS_OPEN,
        )
        .await?;
        insert_match_request_response(pool, 31, 1, 501, 555, 0, (Some(9001), Some(100))).await?;
        insert_match_request_response(pool, 31, 2, 601, 777, 0, (Some(9002), Some(200))).await?;
        release_match_request_for_test(pool, 31).await?;

        let claim = claim_next_pending_match_status(pool).await?.expect("claim");
        assert_eq!(claim.request_id, 31);
        assert_eq!(claim.targets.len(), 2);
        assert_eq!(claim.targets[0].query_message_id, 9001);
        assert_eq!(claim.targets[0].status_message_id, None);
        assert_eq!(
            match_request_status_message_state(pool, 31).await?,
            MATCH_STATUS_MESSAGE_STATE_POSTING
        );

        save_match_status_messages(
            pool,
            31,
            &[
                MatchStatusPost {
                    team_id: 1,
                    channel_id: 100,
                    message_id: 9101,
                },
                MatchStatusPost {
                    team_id: 2,
                    channel_id: 200,
                    message_id: 9102,
                },
            ],
            None,
        )
        .await?;
        assert_eq!(
            match_request_status_message_state(pool, 31).await?,
            MATCH_STATUS_MESSAGE_STATE_POSTED
        );
        let ids = sqlx::query_scalar::<_, Value>(
            "SELECT team_status_message_ids FROM scrim.match_requests WHERE id = 31",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(
            ids,
            json!({
                "1": { "channel_id": 100, "message_id": 9101 },
                "2": { "channel_id": 200, "message_id": 9102 }
            })
        );

        sqlx::query("UPDATE scrim.match_requests SET status_message_state = $2 WHERE id = $1")
            .bind(31)
            .bind(MATCH_STATUS_MESSAGE_STATE_PENDING)
            .execute(pool)
            .await?;
        let edit_claim = claim_next_pending_match_status(pool)
            .await?
            .expect("edit claim");
        assert_eq!(edit_claim.targets[0].status_message_id, Some(9101));
        assert_eq!(edit_claim.targets[1].status_message_id, Some(9102));
        Ok(())
    }

    #[tokio::test]
    async fn claim_next_pending_match_request_batch_markiert_batch_als_posting() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match_request_batch(pool, 30).await?;

        let claim = claim_next_pending_match_request_batch(pool)
            .await?
            .expect("claim");
        assert_eq!(claim.batch_id, 30);
        assert_eq!(claim.requests.len(), 1);
        assert_eq!(claim.requests[0].request_id, 31);
        assert_eq!(
            claim.requests[0].targets,
            vec![
                MatchRequestTarget {
                    team_id: 1,
                    team_name: "team-1".to_string(),
                    channel_id: 100,
                    opponent_name: Some("team-2".to_string()),
                },
                MatchRequestTarget {
                    team_id: 2,
                    team_name: "team-2".to_string(),
                    channel_id: 200,
                    opponent_name: Some("team-1".to_string()),
                },
            ]
        );
        assert!(claim.requests[0].missing_targets.is_empty());
        assert_eq!(match_request_batch_status(pool, 30).await?, "posting");
        Ok(())
    }

    #[tokio::test]
    async fn claim_next_pending_match_request_batch_meldet_fehlenden_teamkanal() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, None).await?;
        insert_match_request_batch(pool, 50).await?;

        let claim = claim_next_pending_match_request_batch(pool)
            .await?
            .expect("claim");
        assert_eq!(claim.requests[0].targets.len(), 1);
        assert_eq!(claim.requests[0].missing_targets, vec!["team-2"]);
        Ok(())
    }

    #[tokio::test]
    async fn claim_next_pending_match_request_batch_ignoriert_abgelaufene_drafts() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match_request_batch_with_deadline_offset(pool, 70, -1).await?;

        assert!(claim_next_pending_match_request_batch(pool)
            .await?
            .is_none());
        assert_eq!(
            match_request_batch_status(pool, 70).await?,
            MATCH_REQUEST_STATUS_DRAFT
        );
        Ok(())
    }

    #[tokio::test]
    async fn save_match_request_posts_speichert_ids_auch_bei_post_failed() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match_request_batch(pool, 40).await?;

        save_match_request_posts(
            pool,
            40,
            &[MatchRequestPost {
                request_id: 41,
                team_id: 1,
                channel_id: 100,
                message_id: 9001,
            }],
            MATCH_REQUEST_STATUS_POST_FAILED,
        )
        .await?;

        assert_eq!(
            match_request_batch_status(pool, 40).await?,
            MATCH_REQUEST_STATUS_POST_FAILED
        );
        let row = sqlx::query(
            "SELECT status, team_query_message_ids FROM scrim.match_requests WHERE id = 41",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(
            row.get::<String, _>("status"),
            MATCH_REQUEST_STATUS_POST_FAILED
        );
        assert_eq!(
            row.get::<Value, _>("team_query_message_ids"),
            json!({ "1": { "channel_id": 100, "message_id": 9001 } })
        );
        Ok(())
    }

    #[tokio::test]
    async fn set_match_request_batch_status_markiert_requests_mit() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, None).await?;
        insert_match_request_batch(pool, 60).await?;

        set_match_request_batch_status(pool, 60, MATCH_REQUEST_STATUS_POST_FAILED).await?;

        assert_eq!(
            match_request_batch_status(pool, 60).await?,
            MATCH_REQUEST_STATUS_POST_FAILED
        );
        assert_eq!(
            match_request_status(pool, 61).await?,
            MATCH_REQUEST_STATUS_POST_FAILED
        );
        Ok(())
    }

    async fn release_match_request_for_test(pool: &PgPool, request_id: i32) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            UPDATE scrim.match_requests
               SET status = 'closed',
                   released_slot_index = 0,
                   released_slot = '{"day":"sat","from":1200,"to":1320}'::jsonb,
                   released_at = now(),
                   released_by_user_id = '42',
                   released_by_display_name = 'Coach',
                   status_message_state = 'pending',
                   updated_at = now()
             WHERE id = $1
            "#,
        )
        .bind(request_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn match_request_status_message_state(
        pool: &PgPool,
        request_id: i32,
    ) -> anyhow::Result<String> {
        Ok(sqlx::query_scalar(
            "SELECT status_message_state FROM scrim.match_requests WHERE id = $1",
        )
        .bind(request_id)
        .fetch_one(pool)
        .await?)
    }

    async fn insert_team(
        pool: &PgPool,
        team_id: i32,
        channel_id: Option<i64>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
            VALUES($1, $2, $3, now())
            "#,
        )
        .bind(team_id)
        .bind(format!("team-{team_id}"))
        .bind(channel_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_team_member(
        pool: &PgPool,
        team_id: i32,
        participant_id: i32,
        discord_user_id: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scrim.participants(
                id, discord_id, display_name, rank_source, status, source, created_at, updated_at
            )
            VALUES($1, $2, $3, 'manual', 'assigned', 'test', now(), now())
            "#,
        )
        .bind(participant_id)
        .bind(discord_user_id)
        .bind(format!("user-{discord_user_id}"))
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.team_members(team_id, participant_id, role, is_captain, is_bench)
            VALUES($1, $2, 'player', false, false)
            "#,
        )
        .bind(team_id)
        .bind(participant_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_match(pool: &PgPool, match_id: i32, lobby_state: &str) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scrim.matches(
                id, team_a_id, team_b_id, status, lobby_state, created_at, updated_at
            )
            VALUES($1, 1, 2, 'scheduled', $2, now(), now())
            "#,
        )
        .bind(match_id)
        .bind(lobby_state)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_match_result_ref(
        pool: &PgPool,
        ref_id: i64,
        match_id: i32,
        steam_match_id: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scrim.match_result_refs(
                id, match_id, steam_match_id, source_user_id, source_display_name,
                fetch_status, entered_at, updated_at
            )
            VALUES($1, $2, $3, '42', 'Coach', 'pending', now(), now())
            "#,
        )
        .bind(ref_id)
        .bind(match_id)
        .bind(steam_match_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_match_result_ref_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        ref_id: i64,
        match_id: i32,
        steam_match_id: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scrim.match_result_refs(
                id, match_id, steam_match_id, source_user_id, source_display_name,
                fetch_status, entered_at, updated_at
            )
            VALUES($1, $2, $3, '42', 'Coach', 'pending', now(), now())
            "#,
        )
        .bind(ref_id)
        .bind(match_id)
        .bind(steam_match_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn match_result_ref_status(
        pool: &PgPool,
        ref_id: i64,
    ) -> anyhow::Result<(String, Option<String>)> {
        let row = sqlx::query(
            r#"
            SELECT fetch_status, last_error
              FROM scrim.match_result_refs
             WHERE id = $1
            "#,
        )
        .bind(ref_id)
        .fetch_one(pool)
        .await?;
        Ok((row.get("fetch_status"), row.get("last_error")))
    }

    async fn insert_match_request_batch(pool: &PgPool, batch_id: i32) -> anyhow::Result<()> {
        insert_match_request_batch_with_deadline_offset(pool, batch_id, 48).await
    }

    async fn insert_match_request_batch_with_deadline_offset(
        pool: &PgPool,
        batch_id: i32,
        deadline_offset_hours: i32,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scrim.match_request_batches(
                id, template, deadline_at, status, created_by_user_id,
                created_by_display_name, created_at, updated_at
            )
            VALUES($1, 'regular_scrim', now() + ($2 * interval '1 hour'), 'draft', '42', 'Coach', now(), now())
            "#,
        )
        .bind(batch_id)
        .bind(deadline_offset_hours)
        .execute(pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO scrim.match_requests(
                id, batch_id, team_a_id, team_b_id, status, slot_options, created_at, updated_at
            )
            VALUES(
                $1, $2, 1, 2, 'draft',
                '[{"day":"sat","from":1200,"to":1320},{"day":"sun","from":1200,"to":1320}]'::jsonb,
                now(), now()
            )
            "#,
        )
        .bind(batch_id + 1)
        .bind(batch_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_match_request_response(
        pool: &PgPool,
        request_id: i32,
        team_id: i32,
        participant_id: i32,
        discord_user_id: i64,
        slot_index: i32,
        source_message: (Option<i64>, Option<i64>),
    ) -> anyhow::Result<()> {
        let (message_id, channel_id) = source_message;
        sqlx::query(
            r#"
            INSERT INTO scrim.match_request_responses(
                request_id, team_id, participant_id, discord_user_id, slot_index,
                response, source, message_id, channel_id, responded_at, updated_at
            )
            VALUES($1, $2, $3, $4, $5, $6, 'button', $7, $8, now(), now())
            "#,
        )
        .bind(request_id)
        .bind(team_id)
        .bind(participant_id)
        .bind(discord_user_id)
        .bind(slot_index)
        .bind(if slot_index == -1 {
            "unavailable"
        } else {
            "available"
        })
        .bind(message_id)
        .bind(channel_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_match_request_reminder(
        pool: &PgPool,
        reminder_id: i32,
        request_id: i32,
        team_id: i32,
        target_participant_ids: &[i32],
        target_user_ids: &[i64],
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scrim.match_request_reminders(
                id, request_id, team_id, template, target_kind, target_participant_ids,
                target_discord_user_ids, missing_count, approved_by_user_id,
                approved_by_display_name, approved_at, scheduled_for, status, discord_channel_id,
                source_message_id, created_at, updated_at
            )
            VALUES(
                $1, $2, $3, 'antwort_fehlt', 'members', $4, $5, 1, '42',
                'Coach', now(), now(), 'approved', 100, 9001, now(), now()
            )
            "#,
        )
        .bind(reminder_id)
        .bind(request_id)
        .bind(team_id)
        .bind(target_participant_ids)
        .bind(target_user_ids)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn match_request_reminder_status(
        pool: &PgPool,
        reminder_id: i32,
    ) -> anyhow::Result<String> {
        Ok(
            sqlx::query_scalar("SELECT status FROM scrim.match_request_reminders WHERE id = $1")
                .bind(reminder_id)
                .fetch_one(pool)
                .await?,
        )
    }

    async fn match_request_batch_status(pool: &PgPool, batch_id: i32) -> anyhow::Result<String> {
        Ok(
            sqlx::query_scalar("SELECT status FROM scrim.match_request_batches WHERE id = $1")
                .bind(batch_id)
                .fetch_one(pool)
                .await?,
        )
    }

    async fn match_request_status(pool: &PgPool, request_id: i32) -> anyhow::Result<String> {
        Ok(
            sqlx::query_scalar("SELECT status FROM scrim.match_requests WHERE id = $1")
                .bind(request_id)
                .fetch_one(pool)
                .await?,
        )
    }

    async fn lobby_state(pool: &PgPool, match_id: i32) -> anyhow::Result<String> {
        let state = sqlx::query_scalar::<_, String>(
            r#"
            SELECT lobby_state
              FROM scrim.matches
             WHERE id = $1
            "#,
        )
        .bind(match_id)
        .fetch_one(pool)
        .await?;
        Ok(state)
    }
}
