//! serenity-Anbindung der Matcher-Ports an den DiscordAdapter.

use std::sync::Arc;

use dl_discord::DiscordAdapter;
use serde_json::{json, Map, Value};
use serenity::all::{ChannelId, GuildId, MessageId, RoleId, UserId};

use crate::matcher::{GuildPort, MemberLite, Notifier};

pub struct AdapterGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub notify_channel_id: u64,
}

#[async_trait::async_trait]
impl GuildPort for AdapterGlue {
    async fn members(&self, guild_id: u64) -> Option<Vec<MemberLite>> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        Some(
            guild
                .members
                .values()
                .map(|member| MemberLite {
                    user_id: member.user.id.get(),
                    name: member.user.name.to_string(),
                    global_name: member.user.global_name.as_ref().map(|n| n.to_string()),
                    nick: member.nick.as_ref().map(|n| n.to_string()),
                    is_bot: member.user.bot,
                })
                .collect(),
        )
    }

    async fn grant_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> String {
        // Status-Texte sind Vertrag (identisch zu _grant_role im Original).
        let role_exists = self
            .adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| g.roles.contains_key(&RoleId::new(role_id)))
            .unwrap_or(false);
        if !role_exists {
            return "⚠️ Streamer-Rolle nicht gefunden – nur verknüpft.".to_string();
        }
        let already = self
            .adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.contains(&RoleId::new(role_id)))
            })
            .unwrap_or(false);
        if already {
            return "Streamer-Rolle war bereits vergeben.".to_string();
        }
        match self
            .adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some("Auto-Match Twitch↔Discord"),
            )
            .await
        {
            Ok(()) => "Streamer-Rolle vergeben.".to_string(),
            Err(err) => {
                tracing::error!(%err, user_id, "Matcher: add_member_role fehlgeschlagen");
                "⚠️ Rolle konnte nicht vergeben werden.".to_string()
            }
        }
    }
}

#[async_trait::async_trait]
impl Notifier for AdapterGlue {
    async fn notify(&self, embed: Value, components: Option<Value>) -> Option<(u64, u64)> {
        let mut body = Map::new();
        body.insert("embeds".into(), json!([embed]));
        if let Some(components) = components {
            body.insert("components".into(), components);
        }
        match self
            .adapter
            .send_raw_public(self.notify_channel_id, &body)
            .await
        {
            Ok(message_id) => Some((self.notify_channel_id, message_id)),
            Err(err) => {
                tracing::warn!(%err, "Matcher: Notify fehlgeschlagen");
                None
            }
        }
    }

    async fn finalize_review(&self, channel_id: u64, message_id: u64, status: String, color: u32) {
        // Original-Embed holen, Status-Feld anhängen, Buttons entfernen.
        let original = self
            .adapter
            .http
            .get_message(ChannelId::new(channel_id), MessageId::new(message_id))
            .await;
        let mut embed = match &original {
            Ok(message) => message
                .embeds
                .first()
                .and_then(|e| serde_json::to_value(e).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default(),
            Err(_) => Map::new(),
        };
        embed.insert("color".into(), json!(color));
        let mut fields = embed
            .get("fields")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        fields.push(json!({ "name": "Status", "value": status, "inline": false }));
        embed.insert("fields".into(), json!(fields));

        let body = json!({ "embeds": [embed], "components": [] });
        if let Err(err) = self
            .adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
        {
            tracing::warn!(%err, message_id, "Matcher: Review-Finalisierung fehlgeschlagen");
        }
    }

    async fn send_text(&self, channel_id: u64, text: String) {
        let mut body = Map::new();
        body.insert("content".into(), json!(text));
        if let Err(err) = self.adapter.send_raw_public(channel_id, &body).await {
            tracing::warn!(%err, channel_id, "Matcher: Text-Antwort fehlgeschlagen");
        }
    }
}
