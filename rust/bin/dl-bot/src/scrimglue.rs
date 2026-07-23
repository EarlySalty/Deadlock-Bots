use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
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
    let Some(claim) = claim_next_pending_match(pool, announcement_channel_id).await? else {
        return Ok(());
    };

    match claim.action {
        ScrimDriverAction::PostLobbyCode => handle_lobby_code(pool, adapter, &claim).await,
        ScrimDriverAction::Start => {
            handle_start(pool, adapter, tempvoice, voice_config, &claim).await
        }
        ScrimDriverAction::FetchResult => handle_result(pool, adapter, tempvoice, &claim).await,
    }
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
