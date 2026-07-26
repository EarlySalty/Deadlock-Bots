//! Anonymes Feedback (Port von `cogs/feedback_hub.py`) — Button + Modal.
//!
//! Das `!fhub`-Panel-Posten hängt an der noch fehlenden Prefix-Dispatch-Infra
//! und folgt dort. Der `feedback_hub:open_modal`-Button + der Modal-Submit
//! funktionieren unabhängig davon — ein bereits gepostetes Panel bleibt nach
//! dem Cutover nutzbar (persistente custom_ids). Das Feedback geht als DM an
//! den Empfänger; statt eines Embeds (das `send_dm` nicht kann) als
//! formatierter Text — der Inhalt ist identisch, die Anonymität bleibt.

use std::sync::Arc;

use dl_central_db::kv;
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Map, Value};
use sqlx::PgPool;

/// Empfänger der anonymen Feedback-DMs (wie `FEEDBACK_RECIPIENT_ID`).
pub const FEEDBACK_RECIPIENT_ID: u64 = 662995601738170389;

/// Kanal, in dem das `!fhub`-Panel lebt (wie `FEEDBACK_CHANNEL_ID`).
pub const FEEDBACK_CHANNEL_ID: u64 = 1289721245281292291;

/// KV-Ablage der Panel-Nachricht (Python: `kv("feedback_hub", ...)`),
/// damit `!fhub` ein bestehendes Panel editiert statt zu duplizieren.
const PANEL_KV_NS: &str = "feedback_hub";
const PANEL_KV_KEY: &str = "interface_message_id";

/// Minimaler Port: DM (Text) senden sowie eine Panel-Nachricht
/// (Embed + Button-Komponenten) posten bzw. editieren.
#[async_trait::async_trait]
pub trait FeedbackPort: Send + Sync {
    async fn send_dm_text(&self, user_id: u64, text: String) -> Result<(), String>;
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
            if let Err(err) = self
                .hub
                .port
                .send_dm_text(FEEDBACK_RECIPIENT_ID, text)
                .await
            {
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

/// Baut den Panel-Body (Embed + Button) — identisch zum Python-`FeedbackHubView`.
fn panel_body() -> Map<String, Value> {
    let embed = json!({
        "title": "Feedback Hub",
        "description":
            "Teile uns dein anonymes Feedback zu dem Server mit, zu den Spielern \
             oder deinem Spielerlebnis. Deine Antworten werden nur intern weitergegeben.",
        // Discord-Blurple (discord.Colour.blurple())
        "color": 0x5865F2,
        "fields": [{
            "name": "So funktioniert's",
            "value":
                "Klicke auf den Button, beantworte die Fragen im Formular und bestätige. \
                 Dein Feedback bleibt anonym und trägt zur Verbesserung des Servers bei.",
            "inline": false,
        }],
    });
    let components = json!([{
        "type": 1,
        "components": [{
            "type": 2,        // Button
            "style": 1,       // Primary
            "label": "Anonymes Feedback senden",
            "custom_id": "feedback_hub:open_modal",
        }],
    }]);
    let mut body = Map::new();
    body.insert("embeds".into(), json!([embed]));
    body.insert("components".into(), components);
    body
}

impl FeedbackHub {
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
