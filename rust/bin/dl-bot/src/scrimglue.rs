use std::collections::BTreeSet;
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
const STATE_START_REQUESTED: &str = "start_requested";
const STATE_STARTING: &str = "starting";
const STATE_START_FAILED: &str = "start_failed";
const STATE_RESULT_REQUESTED: &str = "result_requested";
const STATE_RESULT_FETCHING: &str = "result_fetching";
const STATE_RESULT_FAILED: &str = "result_failed";
const LOG_CHANNEL_ID: u64 = dl_moderation::LOG_CHANNEL_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrimDriverAction {
    Start,
    FetchResult,
}

impl ScrimDriverAction {
    fn from_requested_state(state: &str) -> Option<Self> {
        match state {
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
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(err) =
                process_one_pending(&pool, adapter.as_ref(), announcement_channel_id).await
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
) -> anyhow::Result<()> {
    let Some(claim) = claim_next_pending_match(pool, announcement_channel_id).await? else {
        return Ok(());
    };

    match claim.action {
        ScrimDriverAction::Start => handle_start(pool, adapter, &claim).await,
        ScrimDriverAction::FetchResult => handle_result(pool, adapter, &claim).await,
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
             WHERE m.lobby_state IN ($1, $2)
             ORDER BY COALESCE(m.updated_at, m.created_at), m.id
             LIMIT 1
             FOR UPDATE OF m SKIP LOCKED
        )
        UPDATE scrim.matches m
           SET lobby_state = CASE candidate.requested_state
                    WHEN $1 THEN $3
                    WHEN $2 THEN $4
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
    .bind(STATE_STARTING)
    .bind(STATE_RESULT_FETCHING)
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

async fn handle_start(
    pool: &PgPool,
    adapter: &dl_discord::DiscordAdapter,
    claim: &ClaimedMatch,
) -> anyhow::Result<()> {
    match start_scrim_match(pool, claim.match_id).await {
        Ok(outcome) => {
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
    claim: &ClaimedMatch,
) -> anyhow::Result<()> {
    match fetch_scrim_match_result(pool, claim.match_id).await {
        Ok(outcome) => {
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
    let invited = outcome.invited_participants.len();
    let unlinked = outcome.unlinked_participants.len();
    let mut msg = format!(
        "🎮 **Scrim-Lobby steht!**\nDie Custom-Lobby ist erstellt und die Einladungen sind raus.\n**Join-Code:** `{}`\n{invited} Spieler eingeladen.",
        outcome.join_code,
    );
    if unlinked > 0 {
        msg.push_str(&format!(
            "\n⚠️ {unlinked} Spieler ohne verknüpften Steam-Account — die kommen per Join-Code manuell rein."
        ));
    }
    msg.push_str("\nViel Erfolg! 🫡");
    msg
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

    #[tokio::test]
    async fn claim_next_pending_match_markiert_start_und_verhindert_doppelstart() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        insert_team(pool, 1, Some(100)).await?;
        insert_team(pool, 2, Some(200)).await?;
        insert_match(pool, 12, STATE_STARTING).await?;
        insert_match(pool, 10, STATE_START_REQUESTED).await?;
        insert_match(pool, 11, STATE_RESULT_REQUESTED).await?;

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
        assert_eq!(claim_next_pending_match(pool, None).await?, None);
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
