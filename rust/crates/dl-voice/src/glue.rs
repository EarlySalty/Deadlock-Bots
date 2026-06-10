//! Gateway-Cache-Anbindung des Voice-Trackers.

use std::sync::Arc;

use dl_discord::DiscordAdapter;
use serenity::all::{ChannelId, GuildId, RoleId};

use crate::tracker::{VoiceMemberState, VoiceSnapshot};

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
