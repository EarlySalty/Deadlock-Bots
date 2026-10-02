//! Bestehendes Feedbackformular und bestätigter Transport an das Moderatorenteam.
//! Der gemeinsame Guide-Kern entscheidet über Anliegen und Weitergabe.

use std::sync::Arc;

use dl_central_db::kv;
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

/// Kanal, in dem das `!fhub`-Panel lebt (wie `FEEDBACK_CHANNEL_ID`).
pub const FEEDBACK_CHANNEL_ID: u64 = 1289721245281292291;

/// KV-Ablage der Panel-Nachricht (Python: `kv("feedback_hub", ...)`),
/// damit `!fhub` ein bestehendes Panel editiert statt zu duplizieren.
const PANEL_KV_NS: &str = "feedback_hub";
const PANEL_KV_KEY: &str = "interface_message_id";

/// Port für Teamzustellung und das bestehende Formularpanel.
#[async_trait::async_trait]
pub trait FeedbackPort: Send + Sync {
    /// Postet eine Nachricht mit `embeds`/`components` und gibt die ID zurück.
    async fn post_rich(&self, channel_id: u64, body: Map<String, Value>) -> Result<u64, String>;
    /// Editiert eine bestehende Nachricht; `Err`, wenn sie nicht mehr existiert.
    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), String>;
}

pub struct FeedbackHub {
    pub port: Arc<dyn FeedbackPort>,
    pub pool: PgPool,
}

struct FeedbackHandler;

#[async_trait::async_trait]
impl InteractionHandler for FeedbackHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.custom_id == "feedback_hub:open_modal" {
            let field = |id: &str, label: &str, placeholder: &str, required: bool| ModalField {
                custom_id: id.to_string(),
                label: label.to_string(),
                placeholder: placeholder.to_string(),
                value: None,
                required,
                min_length: 0,
                max_length: 1024,
                paragraph: true,
            };
            return BridgeReply {
                modal: Some(ModalSpec {
                    custom_id: "feedback_hub:submit".to_string(),
                    title: "Deadlock Feedback".to_string(),
                    fields: vec![
                        field(
                            "experience",
                            "Wie war dein bisheriges Spielerlebnis?",
                            "Beschreibe dein Erlebnis so präzise wie möglich.",
                            true,
                        ),
                        field(
                            "server_usage",
                            "Wie gut kommst du mit dem Server zurecht?",
                            "Bots, Kanäle oder Möglichkeiten, die dir helfen oder fehlen?",
                            false,
                        ),
                        field(
                            "improvements",
                            "Wie können wir den Server verbessern?",
                            "Wünsche und Kritik sind willkommen.",
                            true,
                        ),
                        field("wish", "Hast du sonst noch einen Wunsch?", "", false),
                        field("additional", "Möchtest du noch etwas mitteilen?", "", false),
                    ],
                }),
                ..BridgeReply::default()
            };
        }

        BridgeReply::ephemeral_text("Unbekannte Aktion.")
    }
}

/// Das bestehende Formular bleibt erreichbar; der Guide-Kern steuert die Zustellung.
pub fn register_form(router: &mut InteractionRouter) {
    router.on_custom_id("feedback_hub:open_modal", Arc::new(FeedbackHandler));
}

/// Baut den Panel-Body (Embed + Button) — identisch zum Python-`FeedbackHubView`.
fn panel_body() -> Map<String, Value> {
    let embed = json!({
        "title": "Serverfeedback",
        "description":
            "Teile Wünsche, Kritik oder Verbesserungsvorschläge zum Server. \
             Dein konkretes Anliegen wird an das Moderatorenteam weitergeleitet.",
        // Discord-Blurple (discord.Colour.blurple())
        "color": 0x5865F2,
        "fields": [{
            "name": "So funktioniert's",
            "value":
                "Klicke auf den Button, beantworte die Fragen im Formular und bestätige. \
                Nach erfolgreicher Weiterleitung erhältst du eine Bestätigung.",
            "inline": false,
        }],
    });
    let components = json!([{
        "type": 1,
        "components": [{
            "type": 2,        // Button
            "style": 1,       // Primary
            "label": "Serverfeedback senden",
            "custom_id": "feedback_hub:open_modal",
        }],
    }]);
    let mut body = Map::new();
    body.insert("embeds".into(), json!([embed]));
    body.insert("components".into(), components);
    body
}

impl FeedbackHub {
    /// Reservierte Zustellungen werden nach einem unklaren Versand nicht erneut gestartet.
    pub async fn deliver_guide_feedback(
        &self,
        delivery_id: &str,
        guild_id: &str,
        user_id: &str,
        channel_id: u64,
        text: &str,
    ) -> Result<u64, String> {
        if delivery_id.is_empty() || text.trim().is_empty() || text.chars().count() > 1800 {
            return Err("Feedback ist nicht sicher zustellbar".into());
        }
        let db_user = user_id
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or("Ungültiger Feedbackabsender")?;
        let mut privacy_tx = self
            .pool
            .begin()
            .await
            .map_err(|_| "Datenschutzprüfung nicht erreichbar")?;
        dl_central_db::lock_user_privacy(&mut privacy_tx, db_user)
            .await
            .map_err(|_| "Datenschutzprüfung nicht erreichbar")?;
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM brain.guide_feedback_outbox WHERE guild_id=$1 AND user_id=$2 AND delivery_id=$3 AND text=$4 AND destination_channel_id=$5 AND state='pending')")
            .bind(guild_id).bind(user_id).bind(delivery_id).bind(text).bind(channel_id.to_string()).fetch_one(&mut *privacy_tx).await.map_err(|_| "Feedbackfreigabe konnte nicht geprüft werden")?;
        if !pending {
            return Err("Die Feedbackfreigabe ist nicht mehr gültig".into());
        }
        let reserved = sqlx::query("INSERT INTO bot.serverguide_feedback_deliveries(delivery_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(delivery_id).bind(db_user).execute(&self.pool).await.map_err(|_| "Feedbackzustellung konnte nicht reserviert werden")?;
        if reserved.rows_affected() == 0 {
            let sent: Option<i64> = sqlx::query_scalar("SELECT sent_message_id FROM bot.serverguide_feedback_deliveries WHERE delivery_id = $1")
                .bind(delivery_id).fetch_one(&self.pool).await.map_err(|_| "Feedbackzustellung konnte nicht geprüft werden")?;
            return sent
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| "Die frühere Zustellung ist nicht bestätigt".into());
        }
        let nonce = hex::encode(Sha256::digest(delivery_id.as_bytes()));
        let body = json!({"content": text, "nonce": &nonce[..24], "enforce_nonce": true, "allowed_mentions": {"parse": []}})
            .as_object().cloned().ok_or("Feedback konnte nicht vorbereitet werden")?;
        let message_id = self.port.post_rich(channel_id, body).await?;
        let db_message = i64::try_from(message_id).map_err(|_| "Ungültiger Zustellungsnachweis")?;
        sqlx::query("UPDATE bot.serverguide_feedback_deliveries SET sent_message_id=$2 WHERE delivery_id=$1")
            .bind(delivery_id).bind(db_message).execute(&self.pool).await.map_err(|_| "Feedbackzustellung ist noch nicht sicher gespeichert")?;
        privacy_tx
            .commit()
            .await
            .map_err(|_| "Feedback-Datenschutzprüfung konnte nicht abgeschlossen werden")?;
        Ok(message_id)
    }

    /// Postet (oder editiert) das Feedback-Panel im `FEEDBACK_CHANNEL_ID`.
    /// Idempotent: ist eine Panel-Nachricht gemerkt, wird sie editiert; nur
    /// wenn das fehlschlägt (z. B. gelöscht), wird eine neue gepostet.
    async fn post_panel(&self) {
        let body = panel_body();
        let stored = kv::get(&self.pool, PANEL_KV_NS, PANEL_KV_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok());
        if let Some(message_id) = stored {
            if self
                .port
                .edit_rich(FEEDBACK_CHANNEL_ID, message_id, body.clone())
                .await
                .is_ok()
            {
                return;
            }
        }
        match self.port.post_rich(FEEDBACK_CHANNEL_ID, body).await {
            Ok(message_id) => {
                if let Err(err) = kv::set(
                    &self.pool,
                    PANEL_KV_NS,
                    PANEL_KV_KEY,
                    &message_id.to_string(),
                )
                .await
                {
                    tracing::warn!(%err, "Feedback-Panel-ID konnte nicht gespeichert werden");
                }
            }
            Err(err) => tracing::warn!(%err, "Feedback-Panel konnte nicht gepostet werden"),
        }
    }
}

/// Message-Listener für das `!fhub`-Panel (Python: `@commands.command("fhub")`,
/// `manage_guild`). Folgt dem `spawn_admin_command_listener`-Muster: eigener
/// `MessageEvent`-Subscriber, admin-gated, postet bzw. editiert das Panel.
///
/// Hinweis zur Treue: Python gated auf `manage_guild`; hier wird `author_is_admin`
/// (Administrator) genutzt — konsistent mit den anderen Admin-Command-Listenern.
/// Nicht-Admins werden still ignoriert (kein „fehlende Berechtigung"-Reply).
pub fn spawn(
    hub: Arc<FeedbackHub>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    if event.guild_id.is_none() || !event.author_is_admin {
                        continue;
                    }
                    let first = event
                        .content
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_lowercase();
                    if first == "!fhub" {
                        hub.post_panel().await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
