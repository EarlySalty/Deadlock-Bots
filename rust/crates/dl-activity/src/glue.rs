//! Gateway-Cache-Anbindung des Aktivitäts-Analyzers.

use std::collections::HashMap;
use std::sync::Arc;

use dl_discord::DiscordAdapter;
use serenity::all::{GuildId, UserId};

use crate::analyzer::VoiceGroups;
use crate::stats_cmd::NamePort;

pub struct CacheVoiceGroups {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl VoiceGroups for CacheVoiceGroups {
    async fn channel_groups(&self) -> Vec<Vec<(u64, String)>> {
        let mut groups = Vec::new();
        for guild_id in self.adapter.cache().guilds() {
            let Some(guild) = self.adapter.cache().guild(guild_id) else {
                continue;
            };
            let mut per_channel: std::collections::HashMap<u64, Vec<(u64, String)>> =
                std::collections::HashMap::new();
            for (user_id, voice_state) in &guild.voice_states {
                let Some(channel_id) = voice_state.channel_id else {
                    continue;
                };
                let Some(member) = guild.members.get(user_id) else {
                    continue;
                };
                if member.user.bot {
                    continue;
                }
                per_channel
                    .entry(channel_id.get())
                    .or_default()
                    .push((user_id.get(), member.display_name().to_string()));
            }
            groups.extend(per_channel.into_values().filter(|g| g.len() >= 2));
        }
        groups
    }
}

/// Namens-/Guild-Auflösung für die Aktivitäts-/Text-Stats-Befehle.
pub struct StatsNames {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl NamePort for StatsNames {
    async fn resolve_names(&self, user_ids: &[u64]) -> HashMap<u64, String> {
        let mut out = HashMap::new();
        let guilds = self.adapter.cache().guilds();
        for &user_id in user_ids {
            let target = UserId::new(user_id);
            for gid in &guilds {
                let Some(guild) = self.adapter.cache().guild(*gid) else {
                    continue;
                };
                if let Some(member) = guild.members.get(&target) {
                    out.insert(user_id, member.display_name().to_string());
                    break; // erste Fundstelle genügt
                }
            }
        }
        out
    }

    async fn guild_name(&self, guild_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| g.name.clone())
    }
}
