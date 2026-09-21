//! Vollständige Voice-Momentaufnahmen für konservative Reconcile-Läufe.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serenity::all::{Cache, ChannelType, GuildId};

/// Ein Guild-Cache nach GUILD_CREATE oder vollständig abgespieltem RESUMED.
/// Ein fehlender Snapshot bedeutet unbekannt, nicht leer.
#[derive(Debug, Clone)]
pub struct GuildVoiceSnapshot {
    pub guild_id: u64,
    pub observed_at: DateTime<Utc>,
    /// Voice-/Stage-Kanal -> Kategorie, einschließlich fester Kanäle.
    pub channels: HashMap<u64, Option<u64>>,
    /// Mitglied -> Voice-/Stage-Kanal, einschließlich Bots.
    pub members: HashMap<u64, u64>,
}

impl GuildVoiceSnapshot {
    pub fn channel_is_empty(&self, channel_id: u64) -> bool {
        !self.members.values().any(|channel| *channel == channel_id)
    }
}

#[derive(Default)]
struct ShardCache {
    connected: bool,
    expected: HashSet<u64>,
    loaded: HashSet<u64>,
}

/// Getrennt vom allgemeinen READY-Flag: READY allein enthält keine Voice-States.
#[derive(Default)]
pub(crate) struct VoiceCacheHealth {
    shards: Mutex<HashMap<u32, ShardCache>>,
}

impl VoiceCacheHealth {
    pub(crate) fn ready(&self, shard_id: u32, guilds: impl IntoIterator<Item = u64>) {
        let Ok(mut shards) = self.shards.lock() else {
            return;
        };
        shards.insert(
            shard_id,
            ShardCache {
                connected: true,
                expected: guilds.into_iter().collect(),
                loaded: HashSet::new(),
            },
        );
    }

    pub(crate) fn guild_loaded(&self, shard_id: u32, guild_id: u64) {
        let Ok(mut shards) = self.shards.lock() else {
            return;
        };
        let shard = shards.entry(shard_id).or_default();
        shard.expected.insert(guild_id);
        shard.loaded.insert(guild_id);
    }

    pub(crate) fn guild_unavailable(&self, guild_id: u64) {
        let Ok(mut shards) = self.shards.lock() else {
            return;
        };
        for shard in shards.values_mut() {
            shard.loaded.remove(&guild_id);
        }
    }

    pub(crate) fn disconnected(&self, shard_id: u32) {
        let Ok(mut shards) = self.shards.lock() else {
            return;
        };
        if let Some(shard) = shards.get_mut(&shard_id) {
            shard.connected = false;
        }
    }

    pub(crate) fn resumed(&self, shard_id: u32) -> Vec<u64> {
        let Ok(mut shards) = self.shards.lock() else {
            return Vec::new();
        };
        let Some(shard) = shards.get_mut(&shard_id) else {
            return Vec::new();
        };
        shard.connected = true;
        shard.loaded.iter().copied().collect()
    }

    pub(crate) fn snapshot(&self, cache: &Cache, guild_id: u64) -> Option<GuildVoiceSnapshot> {
        let shards = self.shards.lock().ok()?;
        if !shards
            .values()
            .any(|shard| shard.connected && shard.loaded.contains(&guild_id))
        {
            return None;
        }
        let guild = cache.guild(GuildId::new(guild_id))?;
        if guild.unavailable {
            return None;
        }
        Some(GuildVoiceSnapshot {
            guild_id,
            observed_at: Utc::now(),
            channels: guild
                .channels
                .values()
                .filter(|channel| matches!(channel.kind, ChannelType::Voice | ChannelType::Stage))
                .map(|channel| (channel.id.get(), channel.parent_id.map(|id| id.get())))
                .collect(),
            members: guild
                .voice_states
                .iter()
                .filter_map(|(user, state)| {
                    state.channel_id.map(|channel| (user.get(), channel.get()))
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available(health: &VoiceCacheHealth, guild_id: u64) -> bool {
        health
            .shards
            .lock()
            .expect("lock")
            .values()
            .any(|shard| shard.connected && shard.loaded.contains(&guild_id))
    }

    #[test]
    fn ready_und_resumed_unterscheiden_frischen_und_wiederaufgenommenen_cache() {
        let health = VoiceCacheHealth::default();
        health.ready(0, [42]);
        assert!(!available(&health, 42));
        health.guild_loaded(0, 42);
        assert!(available(&health, 42));
        health.disconnected(0);
        assert!(!available(&health, 42));
        assert_eq!(health.resumed(0), vec![42]);
        assert!(available(&health, 42));
        // Eine neue Sitzung darf den alten Guild-Cache nicht freigeben.
        health.ready(0, [42]);
        assert!(!available(&health, 42));
        assert!(health.resumed(0).is_empty());
        health.guild_loaded(0, 42);
        assert!(available(&health, 42));
    }

    #[test]
    fn unavailable_und_fremde_shards_geben_keine_leeren_snapshots_frei() {
        let health = VoiceCacheHealth::default();
        health.ready(0, [42]);
        health.guild_loaded(0, 42);
        health.ready(1, [43]);
        health.guild_loaded(1, 43);
        health.disconnected(1);
        assert!(available(&health, 42));
        assert!(!available(&health, 43));
        health.guild_unavailable(42);
        assert!(!available(&health, 42));
        assert!(!health.resumed(0).contains(&42));
    }
}
