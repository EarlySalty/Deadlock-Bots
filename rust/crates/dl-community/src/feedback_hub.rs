//! Anonymes Feedback (Port von `cogs/feedback_hub.py`) — Button + Modal.
//!
//! Das `!fhub`-Panel-Posten hängt an der noch fehlenden Prefix-Dispatch-Infra
//! und folgt dort. Der `feedback_hub:open_modal`-Button + der Modal-Submit
//! funktionieren unabhängig davon — ein bereits gepostetes Panel bleibt nach
//! dem Cutover nutzbar (persistente custom_ids). Das Feedback geht als DM an
//! den Empfänger; statt eines Embeds (das `send_dm` nicht kann) als
//! formatierter Text — der Inhalt ist identisch, die Anonymität bleibt.

use std::sync::Arc;

use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::Value;

/// Empfänger der anonymen Feedback-DMs (wie `FEEDBACK_RECIPIENT_ID`).
pub const FEEDBACK_RECIPIENT_ID: u64 = 662995601738170389;

/// Minimaler Port: eine DM (Text) an einen User senden.
#[async_trait::async_trait]
pub trait FeedbackPort: Send + Sync {
    async fn send_dm_text(&self, user_id: u64, text: String) -> Result<(), String>;
}

pub struct FeedbackHub {
    pub port: Arc<dyn FeedbackPort>,
}

struct FeedbackHandler {
    hub: Arc<FeedbackHub>,
}

#[async_trait::async_trait]
impl InteractionHandler for FeedbackHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.custom_id == "feedback_hub:open_modal" {
            let field = |id: &str, label: &str, placeholder: &str, required: bool| ModalField {
                custom_id: id.to_string(),
                label: label.to_string(),
                placeholder: placeholder.to_string(),
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
                            "Jeder Wunsch ist willkommen – egal ob umsetzbar oder nicht.",
                            true,
                        ),
                        field("wish", "Hast du sonst noch einen Wunsch?", "", false),
                        field("additional", "Möchtest du noch etwas mitteilen?", "", false),
                    ],
                }),
                ..BridgeReply::default()
            };
        }

        if interaction.custom_id == "feedback_hub:submit" {
            let val = |key: &str| -> String {
                interaction
                    .options
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| "—".to_string())
            };
            let text = format!(
                "**Neues anonymes Feedback** (#{})\nQuelle: <#{}>\n\n\
                 **Spielerlebnis:** {}\n\
                 **Server & Möglichkeiten:** {}\n\
                 **Verbesserungsvorschläge:** {}\n\
                 **Detaillierter Wunsch:** {}\n\
                 **Weitere Mitteilungen:** {}",
                interaction.interaction_id,
                interaction.channel_id,
                val("experience"),
                val("server_usage"),
                val("improvements"),
                val("wish"),
                val("additional"),
            );
            if let Err(err) = self.hub.port.send_dm_text(FEEDBACK_RECIPIENT_ID, text).await {
                tracing::warn!(%err, "Feedback-DM-Versand fehlgeschlagen");
            }
            return BridgeReply::ephemeral_text(
                "Vielen Dank für dein Feedback! Es wurde anonym weitergeleitet.",
            );
        }

        BridgeReply::ephemeral_text("Unbekannte Aktion.")
    }
}

/// Registriert den Feedback-Button und den Modal-Submit.
pub fn register(router: &mut InteractionRouter, hub: Arc<FeedbackHub>) {
    let handler = Arc::new(FeedbackHandler { hub });
    router.on_custom_id("feedback_hub:open_modal", handler.clone());
    router.on_custom_id("feedback_hub:submit", handler);
}
