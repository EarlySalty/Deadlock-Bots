use dl_broker::port::{PortError, StreamerVoiceInvite};
use serenity::all::{ChannelId, ChannelType, Guild, GuildChannel, GuildId, Permissions, RoleId};

use crate::adapter::DiscordAdapter;
use crate::community::{eligible, GUILD};
use crate::streamer_voice::{
    current_invite, discard_invite, everyone_permissions, fresh_channel, invite_options,
    unless_gone, valid_code,
};

const REASON: &str = "Twitch: Zuschauer möchte in einen offenen Community-Call";

fn with_guild<T>(
    adapter: &DiscordAdapter,
    guild_id: u64,
    read: impl FnOnce(&Guild) -> T,
) -> Result<T, PortError> {
    if guild_id != GUILD
        || !adapter.community_gateway.is_ready()
        || adapter.voice_cache_snapshot(guild_id).is_none()
    {
        return Err(PortError::GuildUnavailable);
    }
    let guild = adapter
        .cache()
        .guild(GuildId::new(guild_id))
        .ok_or(PortError::GuildUnavailable)?;
    Ok(read(&guild))
}

fn humans_in(guild: &Guild, channel: ChannelId) -> usize {
    guild
        .voice_states
        .values()
        .filter(|state| state.channel_id == Some(channel))
        .filter(|state| {
            guild
                .members
                .get(&state.user_id)
                .is_some_and(|member| !member.user.bot)
        })
        .count()
}

fn occupants_in(guild: &Guild, channel: ChannelId) -> usize {
    guild
        .voice_states
        .values()
        .filter(|state| state.channel_id == Some(channel))
        .count()
}

fn open_room(adapter: &DiscordAdapter, guild: &Guild, channel: &GuildChannel) -> Option<usize> {
    if channel.guild_id != guild.id || channel.kind != ChannelType::Voice || channel.nsfw {
        return None;
    }
    if guild
        .afk_metadata
        .as_ref()
        .is_some_and(|afk| afk.afk_channel_id == channel.id)
    {
        return None;
    }
    let everyone = guild.roles.get(&RoleId::new(guild.id.get()))?;
    let public = everyone_permissions(
        everyone.permissions,
        guild.id.get(),
        &channel.permission_overwrites,
    );
    if !eligible(
        channel.id.get(),
        channel.parent_id.map(|id| id.get()),
        public,
    ) {
        return None;
    }
    let bot = guild.members.get(&adapter.cache().current_user().id)?;
    let required =
        Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::CREATE_INSTANT_INVITE;
    if !guild.user_permissions_in(channel, bot).contains(required) {
        return None;
    }
    let humans = humans_in(guild, channel.id);
    let limit = channel.user_limit.unwrap_or(0) as usize;
    let has_space = limit == 0 || occupants_in(guild, channel.id) < limit;
    (humans > 0 && has_space).then_some(humans)
}

pub(crate) fn candidates(adapter: &DiscordAdapter, guild_id: u64) -> Result<Vec<u64>, PortError> {
    let mut rooms: Vec<(usize, u64)> = with_guild(adapter, guild_id, |guild| {
        guild
            .channels
            .values()
            .filter_map(|channel| open_room(adapter, guild, channel).map(|n| (n, channel.id.get())))
            .collect()
    })?;
    rooms.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(rooms.into_iter().map(|(_, id)| id).collect())
}

pub(crate) async fn invite(
    adapter: &DiscordAdapter,
    guild_id: u64,
    channel_id: u64,
) -> Result<Option<StreamerVoiceInvite>, PortError> {
    let _guard = adapter.streamer_voice_lock.lock().await;
    let Some(channel) = unless_gone(fresh_channel(adapter, channel_id).await)? else {
        return Ok(None);
    };
    if with_guild(adapter, guild_id, |guild| {
        open_room(adapter, guild, &channel)
    })?
    .is_none()
    {
        return Ok(None);
    }
    let invite = adapter
        .http
        .create_invite(channel.id, &invite_options(), Some(REASON))
        .await
        .map_err(|_| PortError::GuildUnavailable)?;
    let matches = invite.channel.id == channel.id
        && invite
            .guild
            .as_ref()
            .is_some_and(|guild| guild.id.get() == guild_id)
        && valid_code(&invite.code);
    if !matches {
        discard_invite(adapter, &invite.code, REASON).await;
        return Err(PortError::GuildUnavailable);
    }
    tracing::info!(guild_id, channel_id, "Community-Voice-Invite erstellt");
    Ok(Some(StreamerVoiceInvite {
        invite_url: format!("https://discord.gg/{}", invite.code),
        channel_id: channel_id.to_string(),
        slot_added: false,
    }))
}

pub(crate) async fn invite_still_valid(
    adapter: &DiscordAdapter,
    guild_id: u64,
    channel_id: u64,
    invite_url: &str,
) -> Result<bool, PortError> {
    let _guard = adapter.streamer_voice_lock.lock().await;
    let Some(channel) = unless_gone(fresh_channel(adapter, channel_id).await)? else {
        return Ok(false);
    };
    if with_guild(adapter, guild_id, |guild| {
        open_room(adapter, guild, &channel)
    })?
    .is_none()
    {
        return Ok(false);
    }
    let Some(code) = invite_url
        .strip_prefix("https://discord.gg/")
        .filter(|code| valid_code(code))
    else {
        return Ok(false);
    };
    let Some(invite) = current_invite(adapter, code).await? else {
        return Ok(false);
    };
    Ok(invite.code == code
        && invite.channel.id == channel.id
        && invite
            .guild
            .as_ref()
            .is_some_and(|guild| guild.id.get() == guild_id))
}
