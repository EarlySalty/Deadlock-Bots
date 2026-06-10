//! Gateway-Cache-Anbindung des Voice-Trackers.

use std::sync::Arc;

use dl_discord::DiscordAdapter;
use serde_json::json;
use serenity::all::{ChannelId, GuildId, RoleId, UserId};

use crate::tempvoice::LanePort;
use crate::tracker::{VoiceMemberState, VoiceSnapshot};

/// Discord-Permission-Bit CONNECT (Voice).
const CONNECT_BIT: u64 = 1 << 20;

pub struct CacheSnapshot {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl VoiceSnapshot for CacheSnapshot {
    async fn channel_states(
        &self,
        guild_id: u64,
        channel_id: u64,
        grace_role_id: u64,
    ) -> Vec<VoiceMemberState> {
        let Some(guild) = self.adapter.cache.guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        let channel = ChannelId::new(channel_id);
        let is_afk_channel =
            guild.afk_metadata.as_ref().map(|afk| afk.afk_channel_id) == Some(channel);

        guild
            .voice_states
            .iter()
            .filter(|(_, vs)| vs.channel_id == Some(channel))
            .filter_map(|(user_id, vs)| {
                let member = guild.members.get(user_id)?;
                Some(VoiceMemberState {
                    user_id: user_id.get(),
                    display_name: member.display_name().to_string(),
                    is_bot: member.user.bot,
                    is_afk_channel,
                    muted_or_deaf: vs.mute || vs.deaf || vs.self_mute || vs.self_deaf,
                    has_grace_role: member.roles.contains(&RoleId::new(grace_role_id)),
                })
            })
            .collect()
    }

    async fn channel_name(&self, guild_id: u64, channel_id: u64) -> Option<String> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .map(|c| c.name.to_string())
    }
}

#[async_trait::async_trait]
impl LanePort for CacheSnapshot {
    async fn create_voice_channel(
        &self,
        guild_id: u64,
        category_id: Option<u64>,
        name: &str,
        user_limit: i64,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("name".into(), json!(name));
        body.insert("type".into(), json!(2));
        body.insert("user_limit".into(), json!(user_limit));
        if let Some(category) = category_id {
            body.insert("parent_id".into(), json!(category.to_string()));
        }
        self.adapter
            .http
            .create_channel(GuildId::new(guild_id), &body, Some("TempVoice: Auto-Lane"))
            .await
            .map(|c| c.id.get())
            .map_err(|e| e.to_string())
    }

    async fn delete_channel(&self, channel_id: u64, reason: &str) -> Result<(), String> {
        self.adapter
            .http
            .delete_channel(ChannelId::new(channel_id), Some(reason))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn move_member(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "channel_id": channel_id.to_string() }),
                Some(reason),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn rename_channel(
        &self,
        channel_id: u64,
        name: &str,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "name": name }),
                Some(reason),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn set_member_connect(
        &self,
        channel_id: u64,
        user_id: u64,
        connect: Option<bool>,
    ) -> Result<(), String> {
        let channel = ChannelId::new(channel_id);
        match connect {
            None => self
                .adapter
                .http
                .delete_permission(
                    channel,
                    serenity::all::TargetId::new(user_id),
                    Some("TempVoice: Bann aufgehoben"),
                )
                .await
                .map_err(|e| e.to_string()),
            Some(allow) => {
                let (allow_bits, deny_bits) = if allow {
                    (CONNECT_BIT, 0)
                } else {
                    (0, CONNECT_BIT)
                };
                self.adapter
                    .http
                    .create_permission(
                        channel,
                        serenity::all::TargetId::new(user_id),
                        &json!({
                            "type": 1,
                            "allow": allow_bits.to_string(),
                            "deny": deny_bits.to_string(),
                        }),
                        Some("TempVoice: Owner-Bann"),
                    )
                    .await
                    .map_err(|e| e.to_string())
            }
        }
    }

    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))?
            .voice_states
            .get(&UserId::new(user_id))?
            .channel_id
            .map(|c| c.get())
    }

    async fn member_role_names(&self, guild_id: u64, user_id: u64) -> Vec<String> {
        let Some(guild) = self.adapter.cache.guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        let Some(member) = guild.members.get(&UserId::new(user_id)) else {
            return Vec::new();
        };
        member
            .roles
            .iter()
            .filter_map(|role_id| guild.roles.get(role_id).map(|r| r.name.to_string()))
            .collect()
    }

    async fn channel_members(&self, guild_id: u64, channel_id: u64) -> Vec<u64> {
        let Some(guild) = self.adapter.cache.guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        guild
            .voice_states
            .iter()
            .filter(|(_, vs)| vs.channel_id == Some(ChannelId::new(channel_id)))
            .map(|(user_id, _)| user_id.get())
            .collect()
    }

    async fn channel_name(&self, guild_id: u64, channel_id: u64) -> Option<String> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .map(|c| c.name.to_string())
    }

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))?
            .parent_id
            .map(|p| p.get())
    }

    async fn category_voice_channel_names(&self, guild_id: u64, category_id: u64) -> Vec<String> {
        let Some(guild) = self.adapter.cache.guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        guild
            .channels
            .values()
            .filter(|c| {
                c.parent_id == Some(ChannelId::new(category_id))
                    && c.kind == serenity::all::ChannelType::Voice
            })
            .map(|c| c.name.to_string())
            .collect()
    }

    async fn channel_created_at(&self, channel_id: u64) -> Option<i64> {
        Some(ChannelId::new(channel_id).created_at().unix_timestamp())
    }
}
