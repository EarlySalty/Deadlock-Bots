use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use dl_squads::scrim_match::{
    fetch_scrim_match_result, start_scrim_match, ScrimMatchResultOutcome, StartScrimMatchOutcome,
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
const MATCH_REQUEST_STATUS_DRAFT: &str = "draft";
const MATCH_REQUEST_STATUS_POSTING: &str = "posting";
const MATCH_REQUEST_STATUS_OPEN: &str = "open";
const MATCH_REQUEST_STATUS_POST_FAILED: &str = "post_failed";
const MATCH_REQUEST_RESPONSE_PREFIX: &str = "scrimreq:v1:";
const LOG_CHANNEL_ID: u64 = dl_moderation::LOG_CHANNEL_ID;
const SCRIM_VOICE_CHANNEL_NAME: &str = "⚔️ Scrim läuft · Team";

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

impl ScrimDriverAction {
    fn from_requested_state(state: &str) -> Option<Self> {
        match state {
            STATE_LOBBY_OPEN => Some(Self::PostLobbyCode),
            STATE_START_REQUESTED => Some(Self::Start),
            STATE_RESULT_REQUESTED => Some(Self::FetchResult),
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
    announcement_channel_id: Option<u64>,
    tempvoice: Arc<dl_voice::tempvoice::TempVoiceEngine>,
    voice_config: ScrimVoiceConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(err) = process_one_pending(
                &pool,
                adapter.as_ref(),
                announcement_channel_id,
                tempvoice.as_ref(),
                voice_config,
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
    announcement_channel_id: Option<u64>,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    voice_config: ScrimVoiceConfig,
) -> anyhow::Result<()> {
    cleanup_terminal_scrim_voice_channels(pool, tempvoice).await?;
    if let Some(claim) = claim_next_pending_match(pool, announcement_channel_id).await? {
        match claim.action {
            ScrimDriverAction::PostLobbyCode => handle_lobby_code(pool, adapter, &claim).await,
            ScrimDriverAction::Start => {
                handle_start(pool, adapter, tempvoice, voice_config, &claim).await
            }
            ScrimDriverAction::FetchResult => handle_result(pool, adapter, tempvoice, &claim).await,
        }?;
        return Ok(());
    }

    if let Some(batch) = claim_next_pending_match_request_batch(pool).await? {
        handle_match_request_batch(pool, adapter, batch).await?;
    }
    Ok(())
}

async fn claim_next_pending_match(
    pool: &PgPool,
    announcement_channel_id: Option<u64>,
) -> anyhow::Result<Option<ClaimedMatch>> {
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT m.id,
                   m.lobby_state AS requested_state,
                   ta.discord_channel_id AS team_a_channel_id,
                   tb.discord_channel_id AS team_b_channel_id
              FROM scrim.matches m
              LEFT JOIN scrim.teams ta ON ta.id = m.team_a_id
              LEFT JOIN scrim.teams tb ON tb.id = m.team_b_id
	             WHERE m.lobby_state IN ($1, $2, $3)
             ORDER BY COALESCE(m.updated_at, m.created_at), m.id
             LIMIT 1
             FOR UPDATE OF m SKIP LOCKED
        )
        UPDATE scrim.matches m
	           SET lobby_state = CASE candidate.requested_state
	                    WHEN $1 THEN $4
	                    WHEN $2 THEN $5
	                    WHEN $3 THEN $6
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
        team_channels: target_channels(
            announcement_channel_id,
            row.get("team_a_channel_id"),
            row.get("team_b_channel_id"),
        ),
    }))
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
        Ok(message_ids) => {
            save_lobby_code_message_ids(pool, claim.match_id, &message_ids, STATE_LOBBY_POSTED)
                .await?;
            post_log(
                adapter,
                lobby_code_success_log_message(claim.match_id, &code, message_ids.len(), line!()),
                claim.match_id,
            )
            .await;
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

async fn handle_result(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    tempvoice: &dl_voice::tempvoice::TempVoiceEngine,
    claim: &ClaimedMatch,
) -> anyhow::Result<()> {
    match fetch_scrim_match_result(pool, claim.match_id).await {
        Ok(outcome) => {
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
        }
        Err(err) => {
            tracing::warn!(%err, match_id = claim.match_id, "Scrim-Match-Ergebnis fehlgeschlagen");
            set_lobby_state(pool, claim.match_id, STATE_RESULT_FAILED).await?;
            post_log(
                adapter,
                result_failure_message(claim.match_id, line!()),
                claim.match_id,
            )
            .await;
        }
    }
    Ok(())
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
) -> Result<BTreeMap<u64, u64>, String> {
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

    if errors.is_empty() {
        Ok(stored)
    } else {
        Err(errors.join("; "))
    }
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

fn target_channels(
    announcement_channel_id: Option<u64>,
    team_a_channel_id: Option<i64>,
    team_b_channel_id: Option<i64>,
) -> Vec<u64> {
    if let Some(channel_id) = announcement_channel_id.filter(|id| *id > 0) {
        return vec![channel_id];
    }

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
        "regular_scrim" => "Regulaerer Scrim",
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

fn lobby_code_failure_message(match_id: i64, reason: &str, _source_line: u32) -> String {
    format!("⚠️ Scrim-Lobbycode konnte nicht gepostet werden (Match {match_id}): {reason}.")
}

fn match_request_success_log_message(
    batch_id: i64,
    target_count: usize,
    _source_line: u32,
) -> String {
    format!("Scrim-Terminabfragen gepostet (Batch {batch_id}, Ziele: {target_count}).")
}

fn match_request_failure_message(batch_id: i64, reason: &str, _source_line: u32) -> String {
    format!("⚠️ Scrim-Terminabfragen konnten nicht gepostet werden (Batch {batch_id}): {reason}.")
}

fn start_failure_message(match_id: i64, _source_line: u32) -> String {
    format!(
        "⚠️ Scrim-Lobby ließ sich nicht starten (Match {match_id}). Der Steam-GC war nicht erreichbar oder die Lobby-Erstellung schlug fehl — Details im Bot-Log. Der Start lässt sich erneut anstoßen."
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
            format!(
                "🏁 **Scrim beendet!** Ergebnis ist eingetragen — Sieger: Team {team_id}. GG! 🤝"
            )
        }
        None => {
            format!("🏁 **Scrim beendet!** Ergebnis ist eingetragen (Match {match_id}). GG! 🤝")
        }
    }
}

fn result_failure_message(match_id: i64, _source_line: u32) -> String {
    format!(
        "⚠️ Scrim-Ergebnis konnte noch nicht abgerufen werden (Match {match_id}). Vielleicht läuft das Match noch — der Abruf lässt sich später erneut anstoßen. Details im Bot-Log."
    )
}

fn missing_target_message(match_id: i64, message_kind: &str, _source_line: u32) -> String {
    format!(
        "⚠️ Scrim-Meldung ohne Zielkanal (Match {match_id}, Typ {message_kind}): weder ein Team-Channel noch DL_SCRIM_ANNOUNCEMENT_CHANNEL_ID ist gesetzt — bitte Channel-Zuordnung prüfen."
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

        let first = claim_next_pending_match(pool, None).await?;
        assert_eq!(
            first,
            Some(ClaimedMatch {
                match_id: 10,
                action: ScrimDriverAction::Start,
                team_channels: vec![100, 200],
            })
        );
        assert_eq!(lobby_state(pool, 10).await?, STATE_STARTING);

        let second = claim_next_pending_match(pool, None).await?;
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
        let third = claim_next_pending_match(pool, None).await?;
        assert_eq!(
            third,
            Some(ClaimedMatch {
                match_id: 13,
                action: ScrimDriverAction::PostLobbyCode,
                team_channels: vec![100, 200],
            })
        );
        assert_eq!(lobby_state(pool, 13).await?, "lobby_posting");
        assert_eq!(claim_next_pending_match(pool, None).await?, None);
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
