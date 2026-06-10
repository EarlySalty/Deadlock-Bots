//! Discord-Glue für dl-moderation (ModPort + aimod:*-Review-Buttons).

use std::sync::Arc;

use dl_discord::{BridgeInteraction, BridgeReply, DiscordAdapter, InteractionHandler};
use serde_json::json;
use serenity::all::{ChannelId, GuildId, MessageId, UserId};

pub struct ModGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_moderation::ModPort for ModGlue {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn timeout_member(&self, guild_id: u64, user_id: u64, minutes: i64) -> bool {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": until.to_rfc3339() }),
                Some("AI-Moderation: Timeout"),
            )
            .await
            .is_ok()
    }

    async fn post_review(
        &self,
        case: &dl_moderation::store::CaseDraft,
        case_id: &str,
    ) -> Option<u64> {
        let preview: String = case.content.chars().take(900).collect();
        let embed = json!({
            "title": format!("🛡️ Moderationsvorschlag — {}", case.category),
            "description": format!(
                "**User:** <@{}> (`{}`)\n**Kanal:** <#{}>\n**Confidence:** {:.0}%\n**Begründung:** {}\n\n**Nachricht:**\n{}",
                case.user_id, case.user_tag, case.channel_id,
                case.confidence * 100.0, case.reason, preview
            ),
            "color": 0xE67E22,
        });
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 3, "label": "Accept (Delete + Timeout)",
              "custom_id": format!("aimod:accept:{case_id}") },
            { "type": 2, "style": 4, "label": "Ban",
              "custom_id": format!("aimod:ban:{case_id}") },
            { "type": 2, "style": 2, "label": "Deny",
              "custom_id": format!("aimod:deny:{case_id}") },
        ]}]);
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        self.adapter
            .send_raw_public(dl_moderation::MOD_REVIEW_CHANNEL_ID, &body)
            .await
            .ok()
    }

    async fn post_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &body)
            .await;
    }
}

/// Review-Buttons: aimod:accept|ban|deny:{case_id} (Mod-Guard via Rechte).
pub struct ReviewHandler {
    pub moderator: Arc<dl_moderation::AiModerator>,
}

#[async_trait::async_trait]
impl InteractionHandler for ReviewHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !interaction.author_can_manage_roles {
            return BridgeReply::ephemeral_text("Keine Berechtigung.");
        }
        let rest = interaction
            .custom_id
            .strip_prefix("aimod:")
            .unwrap_or_default();
        let Some((action, case_id)) = rest.split_once(':') else {
            return BridgeReply::ephemeral_text("Unbekannte Aktion.");
        };
        match action {
            "accept" => {
                self.moderator
                    .store
                    .resolve_case(case_id, "accepted", interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text(
                    "Akzeptiert — Nachricht löschen/Timeout bitte im Case prüfen.",
                )
            }
            "ban" => {
                self.moderator
                    .store
                    .resolve_case(case_id, "banned", interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text("Als Ban markiert — Ban bitte manuell ausführen.")
            }
            "deny" => {
                self.moderator
                    .store
                    .resolve_case(case_id, "denied", interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text("Vorschlag abgelehnt.")
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}
