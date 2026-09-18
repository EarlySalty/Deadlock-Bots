//! Aggregate-only lobby directory for the authenticated Twitch dashboard.
//! Never returns names/IDs of voice members, private channels, or voice audio.
use dl_broker::port::{CommunityLobby, PortError};
use serenity::all::{ChannelType, GuildId, Permissions, UserId};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Separate from legacy bot readiness: a disconnected shard must not publish
/// an old permission/voice cache as a newly captured live snapshot.
pub(crate) struct GatewayFreshness { shard: AtomicU32, ready: AtomicBool }
impl Default for GatewayFreshness {
    fn default() -> Self { Self { shard: AtomicU32::new(u32::MAX), ready: AtomicBool::new(false) } }
}
impl GatewayFreshness {
    pub(crate) fn is_ready(&self) -> bool { self.ready.load(Ordering::Acquire) }
    pub(crate) fn ready(&self, shard: u32, contains_community: bool) {
        if contains_community {
            self.shard.store(shard, Ordering::Release);
            self.ready.store(true, Ordering::Release);
        }
    }
    pub(crate) fn resume(&self, shard: u32) {
        if self.shard.load(Ordering::Acquire) == shard { self.ready.store(true, Ordering::Release); }
    }
    pub(crate) fn stage(&self, shard: u32, connected: bool) {
        if !connected && self.shard.load(Ordering::Acquire) == shard { self.ready.store(false, Ordering::Release); }
    }
}

const GUILD: u64 = 1289721245281292288;
const STREAMER_VC: u64 = 1326984426906714236;
// Same categories as dl-voice::status::TARGET_CATEGORY_IDS. No dependency on
// dl-voice here (it already depends on dl-discord).
const CATEGORIES: [u64; 3] = [1289721245281292290, 1412804540994162789, 1357422957017698478];
const EXCLUDED: [u64; 4] = [1493690350580138114, 1501089974093873232, 1412804671432818890, 1357422958544420944];

fn eligible(channel: u64, parent: Option<u64>, permissions: Permissions) -> bool {
    !EXCLUDED.contains(&channel)
        && (channel == STREAMER_VC || parent.is_some_and(|id| CATEGORIES.contains(&id)))
        && permissions.contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT)
}

fn role_rank(name: &str) -> Option<f64> {
    let lower = name.to_lowercase();
    let tokens: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|s| !s.is_empty()).collect();
    const RANKS: [&str; 11] = ["initiate", "seeker", "alchemist", "arcanist", "ritualist", "emissary", "archon", "oracle", "phantom", "ascendant", "eternus"];
    let (index, tier) = tokens.iter().enumerate().find_map(|(i, token)| RANKS.iter().position(|r| r == token).map(|tier| (i, tier + 1)))?;
    let sub = tokens.get(index + 1).and_then(|s| match *s {
        "i" => Some(1), "ii" => Some(2), "iii" => Some(3), "iv" => Some(4), "v" => Some(5), "vi" => Some(6),
        _ => s.parse::<u8>().ok().filter(|v| (1..=6).contains(v)),
    });
    Some(tier as f64 + sub.unwrap_or(3) as f64 / 10.0)
}

pub(crate) fn directory(adapter: &crate::adapter::DiscordAdapter, guild_id: u64, user_id: u64) -> Result<Vec<CommunityLobby>, PortError> {
    if guild_id != GUILD || !adapter.community_gateway.is_ready() { return Err(PortError::GuildUnavailable); }
    let guild = adapter.cache().guild(GuildId::new(guild_id)).ok_or(PortError::GuildNotFound)?;
    let requester = guild.members.get(&UserId::new(user_id)).ok_or(PortError::MemberNotFound)?;
    let mut result = Vec::new();
    for channel in guild.channels.values() {
        if channel.kind != ChannelType::Voice { continue; }
        let permissions = guild.user_permissions_in(channel, requester);
        if !eligible(channel.id.get(), channel.parent_id.map(|p| p.get()), permissions) { continue; }
        // Timed-out members cannot connect even if cached role permissions allow it.
        if requester.communication_disabled_until.is_some_and(|until| until.unix_timestamp() > serenity::all::Timestamp::now().unix_timestamp()) { continue; }
        let members: Vec<_> = guild.voice_states.iter()
            .filter(|(_, state)| state.channel_id == Some(channel.id))
            .filter_map(|(id, _)| guild.members.get(id))
            .filter(|member| !member.user.bot).collect();
        if members.is_empty() { continue; }
        let ranks: Vec<f64> = members.iter().filter_map(|member| member.roles.iter()
            .filter_map(|role| guild.roles.get(role).and_then(|r| role_rank(&r.name)))
            .max_by(f64::total_cmp)).collect();
        let streamer = channel.id.get() == STREAMER_VC;
        let brawl = channel.parent_id.is_some_and(|id| id.get() == CATEGORIES[2]);
        let competitive = channel.parent_id.is_some_and(|id| id.get() == CATEGORIES[1]);
        result.push(CommunityLobby {
            channel_id: channel.id.get().to_string(), name: channel.name.chars().take(100).collect(),
            // Bots also consume Discord voice capacity even though they do not
            // make an otherwise empty lobby an active player group.
            member_count: guild.voice_states.values().filter(|state| state.channel_id == Some(channel.id)).count(),
            user_limit: channel.user_limit.filter(|n| *n > 0),
            mode: (!streamer).then(|| if brawl { "street_brawl" } else { "normal" }.to_string()),
            intent: (!streamer).then(|| if competitive { "competitive" } else { "casual" }.to_string()),
            rank_average: (!ranks.is_empty()).then(|| ranks.iter().sum::<f64>() / ranks.len() as f64),
            rank_samples: ranks.len(), requester_present: members.iter().any(|member| member.user.id.get() == user_id),
            is_streamer_vc: streamer,
        });
    }
    result.sort_by(|a, b| b.is_streamer_vc.cmp(&a.is_streamer_vc).then(b.member_count.cmp(&a.member_count)).then(a.channel_id.cmp(&b.channel_id)));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnect_closes_directory_until_matching_shard_resumes() {
        let state = GatewayFreshness::default();
        assert!(!state.is_ready());
        state.ready(0, true); assert!(state.is_ready());
        state.stage(0, false); assert!(!state.is_ready());
        state.stage(0, true); assert!(!state.is_ready());
        state.resume(1); assert!(!state.is_ready());
        state.resume(0); assert!(state.is_ready());
    }
    #[test]
    fn directory_requires_view_and_connect_and_scope() {
        let both = Permissions::VIEW_CHANNEL | Permissions::CONNECT;
        assert!(eligible(42, Some(CATEGORIES[0]), both));
        assert!(!eligible(42, Some(CATEGORIES[0]), Permissions::VIEW_CHANNEL));
        assert!(!eligible(42, Some(CATEGORIES[0]), Permissions::CONNECT));
        assert!(!eligible(42, Some(123), both));
        assert!(!eligible(EXCLUDED[0], Some(CATEGORIES[0]), both));
        assert!(eligible(STREAMER_VC, None, both));
        assert!(!eligible(STREAMER_VC, None, Permissions::empty()));
    }
    #[test]
    fn rank_roles_are_optional_and_never_substring_matched() {
        assert_eq!(role_rank("Oracle III"), Some(8.3));
        assert_eq!(role_rank("Emissary 6"), Some(6.6));
        assert_eq!(role_rank("Unranked"), None);
        assert_eq!(role_rank("PhantomFan"), None);
    }
}
