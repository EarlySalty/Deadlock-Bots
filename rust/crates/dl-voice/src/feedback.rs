//! Voice-Feedback-DMs — Port des Feedback-Systems aus
//! `cogs/voice_activity_tracker.py` (schließt die letzte 4a-Lücke).
//!
//! Erste Session ≥ 5 min mit Mitspielern → freundliche Feedback-DM mit
//! Button (`voice_feedback:start`, custom_id unverändert — alte DMs
//! funktionieren weiter) → 4-Fragen-Modal → Antwort wird gespeichert und
//! an den Owner weitergeleitet. Zweit-Feedback nach ≥ 4 verschiedenen
//! Voice-Tagen, einmalig.

use std::sync::Arc;

use dl_discord::interactions::{ChannelSender, ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler, InteractionRouter, MessageEvent,
};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};

use crate::db::{u64_to_i64, VoiceDbError, VoiceDbResult};

pub const MIN_SECONDS: i64 = 300;
pub const MAX_NAMES: usize = 10;
pub const SECOND_MIN_DAYS: i64 = 4;
pub const FORWARD_USER_ID: u64 = 662995601738170389;
pub const START_CUSTOM_ID: &str = "voice_feedback:start";
pub const MODAL_CUSTOM_ID: &str = "voice_feedback:modal";
pub const RESPONSE_WINDOW_SECONDS: i64 = 72 * 3600;
pub const FEEDBACK_BUTTON_LABEL: &str = "Feedback ausfüllen";
pub const FEEDBACK_ALREADY_RESPONDED_TEXT: &str =
    "Danke, dein Voice-Feedback ist schon angekommen. 👍";
pub const FEEDBACK_WINDOW_EXPIRED_TEXT: &str =
    "Dieses Feedback-Fenster ist abgelaufen. Schreib uns gern direkt, falls noch etwas offen ist.";
pub const ACK_TEXT: &str = "Danke für dein Feedback! 🙌\n\n\
Wenn sonst irgendwas sein sollte, kannst du dich jederzeit an unser Team wenden – hier beißt keiner und jeder hilft gerne! :) \
Falls es doch mal ein Problem geben sollte, wende dich bitte direkt an einen Community Moderator (bei kleineren Dingen), einen Moderator oder an den Owner. ❤️";
const FEEDBACK_REQUEST_ID_LOCK_KEY: i64 = -7_010_020_001;
const FEEDBACK_RESPONSE_ID_LOCK_KEY: i64 = -7_010_020_002;

struct FeedbackRequestResponse {
    id: i64,
    sent_at_ts: i64,
    status: String,
    request_type: String,
    co_player_names: String,
    channel_name: String,
    duration_seconds: i64,
}

/// DM-Text wie das Original (first/second).
pub fn build_message(display_name: &str, request_type: &str, co_player_names: &[String]) -> String {
    let mut names: Vec<String> = co_player_names.to_vec();
    let extra = names.len().saturating_sub(MAX_NAMES);
    names.truncate(MAX_NAMES);
    let mut co_text = names.join(", ");
    if extra > 0 {
        co_text = format!("{co_text} (+{extra} weitere)");
    }
    let mut lines: Vec<String> = if request_type == "second" {
        vec![
            format!("Hey {display_name}, danke für deine Voice-Runden."),
            "Kurzes Update: Was läuft gut, was nervt, was sollen wir fixen?".to_string(),
            "Button drücken und in 1-2 Sätzen Feedback dalassen.".to_string(),
        ]
    } else {
        vec![
            format!("Hey {display_name}!"),
            "Wie waren deine ersten Runden bei uns? Wir würden mega gern wissen, wie's dir gefallen hat :)".to_string(),
            "Hau einfach kurz auf den Button und lass uns wissen, was gut lief oder auch nicht und was vielleicht noch besser gehen könnte. Dauert nur ne Minute und wir freuen uns echt über deine Meinung ❤️".to_string(),
        ]
    };
    if !co_player_names.is_empty() {
        lines.push(format!("Mit im Call waren u.a.: {co_text}"));
    }
    lines.join("\n\n")
}

/// Das 4-Fragen-Modal (Labels/Placeholder wortgleich).
pub fn feedback_modal(request_id: i64) -> ModalSpec {
    let field =
        |custom_id: &str, label: &str, placeholder: &str, required: bool, max: u16| ModalField {
            custom_id: custom_id.to_string(),
            label: label.to_string(),
            placeholder: placeholder.to_string(),
            required,
            min_length: 0,
            max_length: max,
            paragraph: true,
        };
    ModalSpec {
        custom_id: format!("{MODAL_CUSTOM_ID}:{request_id}"),
        title: "Kurzes Voice-Feedback".to_string(),
        fields: vec![
            field(
                "q1",
                "Wie war dein Eindruck?",
                "Kurz bewerten (1-10) und warum: Stimmung, Ablauf, Technik",
                true,
                500,
            ),
            field(
                "q2",
                "Was sollen wir verbessern?",
                "1-2 klare Punkte: Moderation, Themen, Ablauf, Technik, Verhalten",
                false,
                900,
            ),
            field(
                "q3",
                "Wie lief es mit den anderen?",
                "Highlights oder Probleme im Miteinander (gern mit Namen, falls relevant)",
                false,
                900,
            ),
            field(
                "q4",
                "Noch etwas, das wir wissen sollten?",
                "Wünsche, Probleme, Ideen, was dir aufgefallen ist",
                false,
                900,
            ),
        ],
    }
}

pub fn feedback_button_components() -> Value {
    json!([{ "type": 1, "components": [{
        "type": 2, "style": 1, "label": FEEDBACK_BUTTON_LABEL, "emoji": {"name": "📝"},
        "custom_id": START_CUSTOM_ID,
    }]}])
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait FeedbackPort: Send + Sync {
    /// DM mit Feedback-Button → (status, message_id):
    /// "sent" | "forbidden" (DMs zu) | "error".
    async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>);
    async fn forward_to_owner(&self, owner_id: u64, text: String);
    async fn delete_feedback_prompt(&self, user_id: u64, message_id: u64);
    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String>;
}

pub struct VoiceFeedback {
    pub pool: PgPool,
    pub port: Arc<dyn FeedbackPort>,
}

impl VoiceFeedback {
    pub fn new(pool: PgPool, port: Arc<dyn FeedbackPort>) -> Arc<Self> {
        Arc::new(Self { pool, port })
    }

    /// Vom Tracker nach jeder finalisierten Session gerufen.
    #[allow(clippy::too_many_arguments)]
    pub async fn on_session_end(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        channel_name: String,
        co_player_ids: Vec<u64>,
        seconds: i64,
        was_first_session: bool,
    ) {
        if was_first_session && seconds >= MIN_SECONDS && !co_player_ids.is_empty() {
            self.send_request(
                guild_id,
                user_id,
                channel_id,
                &channel_name,
                &co_player_ids,
                seconds,
                "first",
            )
            .await;
            return;
        }
        // Zweit-Feedback: hatte first, noch kein second, ≥ 4 Voice-Tage
        if !self.had_request(user_id, "first").await || self.had_request(user_id, "second").await {
            return;
        }
        if self.distinct_voice_days(user_id).await < SECOND_MIN_DAYS {
            return;
        }
        self.send_request(
            guild_id,
            user_id,
            channel_id,
            &channel_name,
            &co_player_ids,
            seconds,
            "second",
        )
        .await;
    }

    async fn had_request(&self, user_id: u64, request_type: &str) -> bool {
        let Ok(user_id) = u64_to_i64("voice_feedback_requests.user_id", user_id) else {
            return false;
        };
        sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                  FROM activity.voice_feedback_requests
                 WHERE user_id = $1
                   AND request_type = $2
                 LIMIT 1
            ) AS "exists!"
            "#,
            user_id,
            request_type,
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(false)
    }

    async fn distinct_voice_days(&self, user_id: u64) -> i64 {
        let Ok(user_id) = u64_to_i64("voice_session_log.user_id", user_id) else {
            return 0;
        };
        sqlx::query_scalar!(
            r#"
            SELECT COUNT(DISTINCT (started_at AT TIME ZONE 'UTC')::date) AS "days!"
              FROM activity.voice_session_log
             WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_request(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        channel_name: &str,
        co_player_ids: &[u64],
        seconds: i64,
        request_type: &str,
    ) {
        let mut names: Vec<String> = Vec::new();
        for co_id in co_player_ids.iter().take(MAX_NAMES + 5) {
            if let Some(name) = self.port.display_name(guild_id, *co_id).await {
                names.push(name);
            }
        }
        let display = self
            .port
            .display_name(guild_id, user_id)
            .await
            .unwrap_or_else(|| format!("User {user_id}"));
        let text = build_message(&display, request_type, &names);
        let co_text = if names.is_empty() {
            "-".to_string()
        } else {
            names.join(", ")
        };

        self.purge_feedback_requests(user_id).await;

        // Request anlegen.
        let (channel_name_owned, request_type_owned, co_text_owned) =
            (channel_name.to_string(), request_type.to_string(), co_text);
        let request_id = self
            .insert_feedback_request(
                user_id,
                guild_id,
                channel_id,
                channel_name_owned,
                co_text_owned,
                seconds,
                request_type_owned,
            )
            .await
            .ok();
        let Some(request_id) = request_id else { return };

        let (status, message_id) = self.port.send_feedback_dm(user_id, text).await;
        let prompt_message_id = message_id
            .map(|id| u64_to_i64("voice_feedback_requests.prompt_message_id", id))
            .transpose();
        let Ok(prompt_message_id) = prompt_message_id else {
            return;
        };
        let _ = sqlx::query!(
            r#"
            UPDATE activity.voice_feedback_requests
               SET status = $1,
                   prompt_message_id = $2
             WHERE id = $3
            "#,
            status,
            prompt_message_id,
            request_id,
        )
        .execute(&self.pool)
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_feedback_request(
        &self,
        user_id: u64,
        guild_id: u64,
        channel_id: u64,
        channel_name: String,
        co_player_names: String,
        duration_seconds: i64,
        request_type: String,
    ) -> VoiceDbResult<i64> {
        let user_id = u64_to_i64("voice_feedback_requests.user_id", user_id)?;
        let guild_id = u64_to_i64("voice_feedback_requests.guild_id", guild_id)?;
        let channel_id = u64_to_i64("voice_feedback_requests.channel_id", channel_id)?;
        let mut tx = self.pool.begin().await?;
        lock_feedback_request_ids(&mut tx).await?;
        let id = next_feedback_request_id(&mut tx).await?;
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_feedback_requests (
                id, user_id, guild_id, channel_id, channel_name, co_player_names,
                duration_seconds, request_type, status, sent_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', NOW())
            "#,
            id,
            user_id,
            guild_id,
            channel_id,
            channel_name,
            co_player_names,
            duration_seconds,
            request_type,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_test_request(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        channel_name: &str,
        co_player_ids: &[u64],
        seconds: i64,
        request_type: &str,
    ) {
        self.send_request(
            guild_id,
            user_id,
            channel_id,
            channel_name,
            co_player_ids,
            seconds,
            request_type,
        )
        .await;
    }

    async fn purge_feedback_requests(&self, user_id: u64) {
        let prompt_ids: Vec<u64> = self.prompt_message_ids(user_id).await.unwrap_or_default();
        for message_id in prompt_ids {
            self.port.delete_feedback_prompt(user_id, message_id).await;
        }
        let result = self.delete_feedback_requests_for_user(user_id).await;
        if let Err(err) = result {
            tracing::debug!(%err, user_id, "VoiceFeedback: alte Requests konnten nicht geloescht werden");
        }
    }

    async fn prompt_message_ids(&self, user_id: u64) -> VoiceDbResult<Vec<u64>> {
        let user_id = u64_to_i64("voice_feedback_requests.user_id", user_id)?;
        let rows = sqlx::query!(
            r#"
            SELECT prompt_message_id
              FROM activity.voice_feedback_requests
             WHERE user_id = $1
               AND prompt_message_id IS NOT NULL
            "#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.prompt_message_id)
            .filter_map(|id| u64::try_from(id).ok())
            .collect())
    }

    async fn delete_feedback_requests_for_user(&self, user_id: u64) -> VoiceDbResult<()> {
        let user_id = u64_to_i64("voice_feedback_requests.user_id", user_id)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            r#"
            DELETE FROM activity.voice_feedback_responses
             WHERE request_id IN (
                SELECT id
                  FROM activity.voice_feedback_requests
                 WHERE user_id = $1
             )
            "#,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            r#"
            DELETE FROM activity.voice_feedback_requests
             WHERE user_id = $1
            "#,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Freitext-Antworten auf die Feedback-DM (Python `on_message` in DMs).
    /// Gibt den Dankestext zurück, wenn der User noch keine Antwort bestätigt
    /// bekommen hat.
    pub async fn handle_dm_message(self: &Arc<Self>, event: MessageEvent) -> Option<&'static str> {
        if event.guild_id.is_some() {
            return None;
        }
        let content = event.content.trim().to_string();
        let Ok(author_id) = u64_to_i64("core.user_privacy.user_id", event.author_id) else {
            return None;
        };
        if content.is_empty() || dl_community::privacy::is_opted_out(&self.pool, author_id).await {
            return None;
        }

        let user_id = event.author_id;
        let row: Option<FeedbackRequestResponse> =
            self.latest_request_for_user(user_id).await.ok().flatten();
        let row = row?;

        let now = chrono::Utc::now().timestamp();
        if row.sent_at_ts > 0 && now.saturating_sub(row.sent_at_ts) > RESPONSE_WINDOW_SECONDS {
            return None;
        }

        let should_ack = row.status != "responded";
        let request_id = row.id;
        let message_id = event.message_id;
        let content_for_db = content.clone();
        let stored = async {
            insert_feedback_response(
                &self.pool,
                request_id,
                user_id,
                Some(message_id),
                content_for_db,
            )
            .await?;
            update_feedback_request_status(&self.pool, request_id, "responded").await?;
            Ok::<(), VoiceDbError>(())
        }
        .await;
        if let Err(err) = stored {
            tracing::warn!(%err, request_id, user_id, "VoiceFeedback: Freitext-Antwort speichern fehlgeschlagen");
            return None;
        }

        let duration_min = if row.duration_seconds > 0 {
            std::cmp::max(1, row.duration_seconds / 60).to_string()
        } else {
            "?".to_string()
        };
        let co_players = if row.co_player_names.trim().is_empty() {
            "—".to_string()
        } else {
            row.co_player_names
        };
        self.port
            .forward_to_owner(
                FORWARD_USER_ID,
                format_feedback_forward(
                    row.id,
                    &event.author_display_name,
                    event.author_id,
                    &content,
                    &row.request_type,
                    &co_players,
                    &row.channel_name,
                    &duration_min,
                ),
            )
            .await;

        should_ack.then_some(ACK_TEXT)
    }

    async fn latest_request_for_user(
        &self,
        user_id: u64,
    ) -> VoiceDbResult<Option<FeedbackRequestResponse>> {
        let user_id = u64_to_i64("voice_feedback_requests.user_id", user_id)?;
        let row = sqlx::query!(
            r#"
            SELECT id,
                   EXTRACT(EPOCH FROM sent_at)::int8 AS "sent_at_ts!",
                   COALESCE(status, '') AS "status!",
                   COALESCE(request_type, 'first') AS "request_type!",
                   COALESCE(co_player_names, '') AS "co_player_names!",
                   COALESCE(channel_name, 'Voice') AS "channel_name!",
                   COALESCE(duration_seconds, 0) AS "duration_seconds!"
              FROM activity.voice_feedback_requests
             WHERE user_id = $1
             ORDER BY sent_at DESC
             LIMIT 1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| FeedbackRequestResponse {
            id: row.id,
            sent_at_ts: row.sent_at_ts,
            status: row.status,
            request_type: row.request_type,
            co_player_names: row.co_player_names,
            channel_name: row.channel_name,
            duration_seconds: row.duration_seconds,
        }))
    }

    async fn latest_request_for_button(
        &self,
        user_id: u64,
    ) -> VoiceDbResult<Option<(i64, i64, String)>> {
        let user_id = u64_to_i64("voice_feedback_requests.user_id", user_id)?;
        let row = sqlx::query!(
            r#"
            SELECT id,
                   EXTRACT(EPOCH FROM sent_at)::int8 AS "sent_at_ts!",
                   COALESCE(status, '') AS "status!"
              FROM activity.voice_feedback_requests
             WHERE user_id = $1
             ORDER BY id DESC
             LIMIT 1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| (row.id, row.sent_at_ts, row.status)))
    }

    async fn request_by_id(
        &self,
        request_id: i64,
    ) -> VoiceDbResult<Option<FeedbackRequestResponse>> {
        let row = sqlx::query!(
            r#"
            SELECT id,
                   EXTRACT(EPOCH FROM sent_at)::int8 AS "sent_at_ts!",
                   COALESCE(status, '') AS "status!",
                   COALESCE(request_type, 'first') AS "request_type!",
                   COALESCE(co_player_names, '') AS "co_player_names!",
                   COALESCE(channel_name, 'Voice') AS "channel_name!",
                   COALESCE(duration_seconds, 0) AS "duration_seconds!"
              FROM activity.voice_feedback_requests
             WHERE id = $1
            "#,
            request_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| FeedbackRequestResponse {
            id: row.id,
            sent_at_ts: row.sent_at_ts,
            status: row.status,
            request_type: row.request_type,
            co_player_names: row.co_player_names,
            channel_name: row.channel_name,
            duration_seconds: row.duration_seconds,
        }))
    }
}

async fn insert_feedback_response(
    pool: &PgPool,
    request_id: i64,
    user_id: u64,
    message_id: Option<u64>,
    content: String,
) -> VoiceDbResult<()> {
    let user_id = u64_to_i64("voice_feedback_responses.user_id", user_id)?;
    let message_id = message_id
        .map(|id| u64_to_i64("voice_feedback_responses.message_id", id))
        .transpose()?;
    let mut tx = pool.begin().await?;
    lock_feedback_response_ids(&mut tx).await?;
    let id = next_feedback_response_id(&mut tx).await?;
    sqlx::query!(
        r#"
        INSERT INTO activity.voice_feedback_responses (
            id, request_id, user_id, message_id, content, received_at
        )
        VALUES ($1, $2, $3, $4, $5, NOW())
        "#,
        id,
        request_id,
        user_id,
        message_id,
        content,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn update_feedback_request_status(
    pool: &PgPool,
    request_id: i64,
    status: &str,
) -> VoiceDbResult<()> {
    sqlx::query!(
        r#"
        UPDATE activity.voice_feedback_requests
           SET status = $1
         WHERE id = $2
        "#,
        status,
        request_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn lock_feedback_request_ids(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        SELECT 1 AS "locked!"
          FROM pg_advisory_xact_lock($1)
        "#,
        FEEDBACK_REQUEST_ID_LOCK_KEY,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

async fn next_feedback_request_id(tx: &mut Transaction<'_, Postgres>) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int8 AS "next_id!"
          FROM activity.voice_feedback_requests
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

async fn lock_feedback_response_ids(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        SELECT 1 AS "locked!"
          FROM pg_advisory_xact_lock($1)
        "#,
        FEEDBACK_RESPONSE_ID_LOCK_KEY,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

async fn next_feedback_response_id(tx: &mut Transaction<'_, Postgres>) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int8 AS "next_id!"
          FROM activity.voice_feedback_responses
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

#[allow(clippy::too_many_arguments)]
fn format_feedback_forward(
    request_id: i64,
    author_name: &str,
    author_id: u64,
    content: &str,
    request_type: &str,
    co_players: &str,
    channel_name: &str,
    duration_min: &str,
) -> String {
    format!(
        "📩 Neues Voice-Feedback (Req #{request_id}, Typ: {request_type})\nVon: {author_name} ({author_id})\nKanal: {channel_name}\nDauer: {duration_min} Min\nMit im Call: {co_players}\n\nAntwort:\n{content}",
    )
}

/// Button + Modal-Submit (custom_ids: voice_feedback:start / :modal:{id}).
struct FeedbackHandler {
    feedback: Arc<VoiceFeedback>,
}

#[async_trait::async_trait]
impl InteractionHandler for FeedbackHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.custom_id == START_CUSTOM_ID {
            // Neuester gesendeter Request des klickenden Users
            let user_id = interaction.user_id;
            let request: Option<(i64, i64, String)> = self
                .feedback
                .latest_request_for_button(user_id)
                .await
                .ok()
                .flatten();
            let Some((request_id, sent_at_ts, status)) = request else {
                return BridgeReply::ephemeral_text(
                    "Diese Feedback-Anfrage ist nicht mehr offen — trotzdem danke!",
                );
            };
            if status == "responded" {
                return BridgeReply::ephemeral_text(FEEDBACK_ALREADY_RESPONDED_TEXT);
            }
            let now = chrono::Utc::now().timestamp();
            if sent_at_ts > 0 && now.saturating_sub(sent_at_ts) > RESPONSE_WINDOW_SECONDS {
                return BridgeReply::ephemeral_text(FEEDBACK_WINDOW_EXPIRED_TEXT);
            }
            if !matches!(status.as_str(), "sent" | "pending") {
                return BridgeReply::ephemeral_text(
                    "Diese Feedback-Anfrage ist nicht mehr offen — trotzdem danke!",
                );
            }
            return BridgeReply {
                modal: Some(feedback_modal(request_id)),
                ..BridgeReply::default()
            };
        }

        // Modal-Submit: voice_feedback:modal:{request_id}
        let request_id: i64 = interaction
            .custom_id
            .rsplit(':')
            .next()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0);
        let answers: Vec<String> = ["q1", "q2", "q3", "q4"]
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let value = interaction
                    .options
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if value.is_empty() {
                    format!("{}) —", index + 1)
                } else {
                    format!("{}) {value}", index + 1)
                }
            })
            .collect();
        let combined = answers.join("\n");
        let user_id = interaction.user_id;
        let combined_clone = combined.clone();
        let _ = async {
            insert_feedback_response(
                &self.feedback.pool,
                request_id,
                user_id,
                None,
                combined_clone,
            )
            .await?;
            update_feedback_request_status(&self.feedback.pool, request_id, "responded").await?;
            Ok::<(), VoiceDbError>(())
        }
        .await;

        let request_meta: Option<FeedbackRequestResponse> =
            self.feedback.request_by_id(request_id).await.ok().flatten();
        let (request_type, co_players, channel_name, duration_min) = request_meta
            .map(|row| {
                let duration_min = if row.duration_seconds > 0 {
                    std::cmp::max(1, row.duration_seconds / 60).to_string()
                } else {
                    "?".to_string()
                };
                let co_players = if row.co_player_names.trim().is_empty() {
                    "—".to_string()
                } else {
                    row.co_player_names
                };
                (row.request_type, co_players, row.channel_name, duration_min)
            })
            .unwrap_or_else(|| {
                (
                    "first".to_string(),
                    "—".to_string(),
                    "Voice".to_string(),
                    "?".to_string(),
                )
            });

        // An den Owner weiterleiten (best-effort)
        self.feedback
            .port
            .forward_to_owner(
                FORWARD_USER_ID,
                format_feedback_forward(
                    request_id,
                    &interaction.author_name,
                    user_id,
                    &combined,
                    &request_type,
                    &co_players,
                    &channel_name,
                    &duration_min,
                ),
            )
            .await;

        BridgeReply::ephemeral_text(ACK_TEXT)
    }
}

pub fn register(router: &mut InteractionRouter, feedback: Arc<VoiceFeedback>) {
    let handler = Arc::new(FeedbackHandler { feedback });
    router.on_custom_id(START_CUSTOM_ID, handler.clone());
    router.on_prefix(format!("{MODAL_CUSTOM_ID}:"), handler);
}

pub fn spawn_dm_responses(
    feedback: Arc<VoiceFeedback>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let channel_id = event.channel_id;
                    let Some(ack) = feedback.handle_dm_message(event).await else {
                        continue;
                    };
                    let _ = sender.send_to_channel(channel_id, Some(ack), &[]).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn texte_wie_python() {
        let names: Vec<String> = (1..=12).map(|i| format!("Spieler{i}")).collect();
        let text = build_message("Anna", "first", &names);
        assert!(text.starts_with("Hey Anna!"));
        assert!(text.contains("Wie waren deine ersten Runden bei uns?"));
        assert!(text.contains("(+2 weitere)")); // 12 Namen → 10 + 2
        let text = build_message("Ben", "second", &[]);
        assert!(text.contains("danke für deine Voice-Runden"));
        assert!(!text.contains("Mit im Call"));
    }

    #[test]
    fn modal_aufbau() {
        let modal = feedback_modal(42);
        assert_eq!(modal.custom_id, "voice_feedback:modal:42");
        assert_eq!(modal.fields.len(), 4);
        assert!(modal.fields[0].required);
        assert!(!modal.fields[1].required);
        assert!(modal.fields.iter().all(|f| f.paragraph));
    }

    #[test]
    fn feedback_button_label_kommt_aus_platzhalter_konstante() {
        let components = feedback_button_components();
        assert_eq!(
            components[0]["components"][0]["label"],
            FEEDBACK_BUTTON_LABEL
        );
    }

    struct MockPort {
        dms: StdMutex<Vec<(u64, String)>>,
        forwards: StdMutex<Vec<String>>,
        deletes: StdMutex<Vec<(u64, u64)>>,
    }

    #[async_trait::async_trait]
    impl FeedbackPort for MockPort {
        async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>) {
            self.dms.lock().expect("lock").push((user_id, text));
            ("sent".to_string(), Some(777))
        }
        async fn forward_to_owner(&self, _owner_id: u64, text: String) {
            self.forwards.lock().expect("lock").push(text);
        }
        async fn delete_feedback_prompt(&self, user_id: u64, message_id: u64) {
            self.deletes
                .lock()
                .expect("lock")
                .push((user_id, message_id));
        }
        async fn display_name(&self, _guild_id: u64, user_id: u64) -> Option<String> {
            Some(format!("User {user_id}"))
        }
    }

    async fn setup() -> (dl_central_db::TestDb, Arc<VoiceFeedback>, Arc<MockPort>) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockPort {
            dms: StdMutex::new(Vec::new()),
            forwards: StdMutex::new(Vec::new()),
            deletes: StdMutex::new(Vec::new()),
        });
        let feedback = VoiceFeedback::new(db.pool().clone(), port.clone());
        (db, feedback, port)
    }

    fn ts(raw: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
            .expect("test timestamp")
            .and_utc()
    }

    fn now_from_unix(value: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(value, 0).expect("valid unix timestamp")
    }

    fn dm_event(user_id: u64, message_id: u64, content: &str) -> MessageEvent {
        MessageEvent {
            guild_id: None,
            channel_id: 900,
            message_id,
            author_id: user_id,
            author_display_name: "FeedbackUser".to_string(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.to_string(),
            message_created_at: chrono::Utc::now().timestamp(),
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 0,
            author_joined_at: None,
        }
    }

    #[tokio::test]
    async fn erste_session_sendet_dm_und_persistiert() {
        let (_dir, feedback, port) = setup().await;
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200, 300], 600, true)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);
        let row = sqlx::query!(
            r#"
            SELECT COALESCE(status, '') AS "status!",
                   COALESCE(request_type, '') AS "request_type!"
              FROM activity.voice_feedback_requests
             WHERE user_id = 100
            "#
        )
        .fetch_one(&feedback.pool)
        .await
        .expect("request");
        assert_eq!(
            (row.status.as_str(), row.request_type.as_str()),
            ("sent", "first")
        );

        // Kurze Session / allein → kein weiterer Request
        feedback
            .on_session_end(1, 400, 10, "Lane 1".into(), vec![], 600, true)
            .await;
        feedback
            .on_session_end(1, 500, 10, "Lane 1".into(), vec![200], 100, true)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn neuer_prompt_loescht_alle_alten_requests_und_responses() {
        let (_dir, feedback, port) = setup().await;
        let now = chrono::Utc::now();
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_feedback_requests (
                id, user_id, status, prompt_message_id, sent_at
            )
            VALUES (10, 100, 'sent', 900, $1), (11, 100, 'responded', 901, $1)
            "#,
            now,
        )
        .execute(&feedback.pool)
        .await
        .expect("seed");
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_feedback_responses (
                id, request_id, user_id, content, received_at
            )
            VALUES (20, 10, 100, 'alt', $1), (21, 11, 100, 'alt2', $1)
            "#,
            now,
        )
        .execute(&feedback.pool)
        .await
        .expect("seed responses");

        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, true)
            .await;

        let old_requests = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.voice_feedback_requests
             WHERE id = ANY($1)
            "#,
            &[10_i64, 11_i64],
        )
        .fetch_one(&feedback.pool)
        .await
        .expect("old requests");
        let old_responses = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM activity.voice_feedback_responses"#
        )
        .fetch_one(&feedback.pool)
        .await
        .expect("old responses");
        let total_requests = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.voice_feedback_requests
             WHERE user_id = 100
            "#
        )
        .fetch_one(&feedback.pool)
        .await
        .expect("total requests");
        assert_eq!(old_requests, 0);
        assert_eq!(old_responses, 0);
        assert_eq!(total_requests, 1);
        assert_eq!(
            port.deletes.lock().expect("lock").clone(),
            vec![(100, 900), (100, 901)]
        );
    }

    #[tokio::test]
    async fn zweit_feedback_erst_nach_vier_tagen() {
        let (_dir, feedback, port) = setup().await;
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, true)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);

        // Nur 2 verschiedene Tage → noch kein second
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_session_log (
                id, user_id, started_at, ended_at, duration_seconds, points
            )
            VALUES
                (1001, 100, $1, $1, 60, 1),
                (1002, 100, $2, $2, 60, 1)
            "#,
            ts("2026-06-01 18:00:00"),
            ts("2026-06-02 18:00:00"),
        )
        .execute(&feedback.pool)
        .await
        .expect("seed");
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, false)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);

        // 4 Tage → second kommt genau einmal
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_session_log (
                id, user_id, started_at, ended_at, duration_seconds, points
            )
            VALUES
                (1003, 100, $1, $1, 60, 1),
                (1004, 100, $2, $2, 60, 1)
            "#,
            ts("2026-06-03 18:00:00"),
            ts("2026-06-04 18:00:00"),
        )
        .execute(&feedback.pool)
        .await
        .expect("seed");
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, false)
            .await;
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, false)
            .await;
        let dms = port.dms.lock().expect("lock");
        assert_eq!(dms.len(), 2);
        assert!(dms[1].1.contains("danke für deine Voice-Runden"));
    }

    #[tokio::test]
    async fn dm_freitext_antwort_wie_python() {
        let (_dir, feedback, port) = setup().await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_feedback_requests (
                id, user_id, guild_id, channel_id, channel_name, co_player_names,
                duration_seconds, request_type, status, sent_at
            )
            VALUES (2001, 100, 1, 10, 'Lane 1', 'Alice, Bob', 601, 'first', 'sent', $1)
            "#,
            now_from_unix(now),
        )
        .execute(&feedback.pool)
        .await
        .expect("seed");

        let ack = feedback
            .handle_dm_message(dm_event(100, 700, "War gut, aber bitte mehr Moderation."))
            .await;
        assert_eq!(ack, Some(ACK_TEXT));
        let row = sqlx::query!(
            r#"
            SELECT COALESCE(r.status, '') AS "status!",
                   a.message_id AS "message_id!",
                   COALESCE(a.content, '') AS "content!"
              FROM activity.voice_feedback_requests r
              JOIN activity.voice_feedback_responses a ON a.request_id = r.id
             WHERE r.user_id = 100
             ORDER BY a.id
             LIMIT 1
            "#
        )
        .fetch_one(&feedback.pool)
        .await
        .expect("response");
        assert_eq!(row.status, "responded");
        assert_eq!(row.message_id, 700);
        assert_eq!(row.content, "War gut, aber bitte mehr Moderation.");
        let forwards = port.forwards.lock().expect("lock");
        assert_eq!(forwards.len(), 1);
        assert!(forwards[0].contains("📩 Neues Voice-Feedback (Req #"));
        assert!(forwards[0].contains("Mit im Call: Alice, Bob"));

        drop(forwards);
        let ack = feedback
            .handle_dm_message(dm_event(100, 701, "Nachtrag"))
            .await;
        assert_eq!(ack, None);
    }

    #[tokio::test]
    async fn feedback_button_respektiert_responded_und_ablauf() {
        let (_dir, feedback, _port) = setup().await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_feedback_requests (id, user_id, status, sent_at)
            VALUES (21, 100, 'responded', $1), (22, 200, 'sent', $2)
            "#,
            now_from_unix(now),
            now_from_unix(now - RESPONSE_WINDOW_SECONDS - 5),
        )
        .execute(&feedback.pool)
        .await
        .expect("seed");
        let handler = FeedbackHandler {
            feedback: feedback.clone(),
        };

        let responded = handler
            .handle(BridgeInteraction {
                custom_id: START_CUSTOM_ID.to_string(),
                user_id: 100,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(
            responded.content.as_deref(),
            Some(FEEDBACK_ALREADY_RESPONDED_TEXT)
        );
        assert!(responded.modal.is_none());

        let expired = handler
            .handle(BridgeInteraction {
                custom_id: START_CUSTOM_ID.to_string(),
                user_id: 200,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(
            expired.content.as_deref(),
            Some(FEEDBACK_WINDOW_EXPIRED_TEXT)
        );
        assert!(expired.modal.is_none());
    }

    #[tokio::test]
    async fn modal_forward_enthaelt_python_kontext() {
        let (_dir, feedback, port) = setup().await;
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_feedback_requests (
                id, user_id, guild_id, channel_id, channel_name, co_player_names,
                duration_seconds, request_type, status, sent_at
            )
            VALUES (30, 100, 1, 10, 'Lane 7', 'Alice, Bob', 725, 'second', 'sent', $1)
            "#,
            chrono::Utc::now(),
        )
        .execute(&feedback.pool)
        .await
        .expect("seed");

        let handler = FeedbackHandler {
            feedback: feedback.clone(),
        };
        let mut options = std::collections::HashMap::new();
        options.insert("q1".to_string(), serde_json::json!("Alles gut"));
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "voice_feedback:modal:30".to_string(),
                user_id: 100,
                author_name: "FeedbackUser".to_string(),
                options,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(reply.content.as_deref(), Some(ACK_TEXT));
        let forwards = port.forwards.lock().expect("lock");
        assert_eq!(forwards.len(), 1);
        assert!(forwards[0].contains("Req #30"));
        assert!(forwards[0].contains("Typ: second"));
        assert!(forwards[0].contains("Lane 7"));
        assert!(forwards[0].contains("12 Min"));
        assert!(forwards[0].contains("Alice, Bob"));
    }
}
