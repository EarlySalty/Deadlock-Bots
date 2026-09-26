//! Invites requested by Twitch viewers, restricted to public community voices.
//! This never changes overwrites or moves/disconnects members.
use std::future::Future;
use std::time::Duration;

use dl_broker::port::{PortError, StreamerVoiceInvite};
use serde_json::json;
use serenity::all::{
    ChannelId, ChannelType, GuildChannel, GuildId, PermissionOverwrite, PermissionOverwriteType,
    Permissions, RoleId, UserId,
};

use crate::adapter::DiscordAdapter;
use crate::community::{eligible, GUILD};

const REASON: &str = "Twitch: Zuschauer möchte beim Streamer mitspielen";
const INVITE_SECONDS: u64 = 600;
const EDIT_READBACK_ATTEMPTS: usize = 2;
const EDIT_READBACK_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy)]
struct EditReadback {
    channel_id: u64,
    limit: u32,
    members: usize,
    policy_safe: bool,
}

/// An edit may have reached Discord even if its HTTP acknowledgement was
/// lost. Read only; never issue an unsafe compensating channel PATCH.
async fn confirm_uncertain_edit<F, Fut>(channel_id: u64, target: u32, mut read: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<EditReadback, PortError>>,
{
    for attempt in 0..EDIT_READBACK_ATTEMPTS {
        match read().await {
            Ok(state) if !state.policy_safe || state.channel_id != channel_id => return false,
            Ok(state) if state.limit == target => return state.members < target as usize,
            _ => {}
        }
        if attempt + 1 < EDIT_READBACK_ATTEMPTS {
            tokio::time::sleep(EDIT_READBACK_DELAY).await;
        }
    }
    false
}

fn invite_options() -> serde_json::Value {
    json!({"max_age": INVITE_SECONDS, "max_uses": 1, "unique": true})
}

/// Return the existing limit if there is space, otherwise exactly one free
/// slot, including occupants that bypassed the old limit. Zero is unlimited.
fn target_limit(limit: u32, members: usize) -> Option<u32> {
    if limit == 0 || members < limit as usize {
        return Some(limit);
    }
    u32::try_from(members)
        .ok()?
        .checked_add(1)
        .filter(|n| *n <= 99)
}

/// Effective rights of an ordinary new member, not those of the streamer or
/// bot. Role-restricted and owner-locked channels must not get public invites.
fn everyone_permissions(
    base: Permissions,
    guild_id: u64,
    overwrites: &[PermissionOverwrite],
) -> Permissions {
    if base.contains(Permissions::ADMINISTRATOR) {
        return Permissions::all();
    }
    overwrites.iter().filter(|overwrite| {
        matches!(overwrite.kind, PermissionOverwriteType::Role(id) if id.get() == guild_id)
    }).fold(base, |permissions, overwrite| (permissions & !overwrite.deny) | overwrite.allow)
}

struct Snapshot {
    members: usize,
    can_manage: bool,
}

fn snapshot(
    adapter: &DiscordAdapter,
    channel: &GuildChannel,
    guild_id: u64,
    streamer_id: u64,
) -> Result<Option<Snapshot>, PortError> {
    if guild_id != GUILD
        || !adapter.community_gateway.is_ready()
        || adapter.voice_cache_snapshot(guild_id).is_none()
    {
        return Err(PortError::GuildUnavailable);
    }
    if channel.guild_id.get() != guild_id || channel.kind != ChannelType::Voice || channel.nsfw {
        return Ok(None);
    }
    let guild = adapter
        .cache()
        .guild(GuildId::new(guild_id))
        .ok_or(PortError::GuildUnavailable)?;
    let uid = UserId::new(streamer_id);
    if guild
        .afk_metadata
        .as_ref()
        .is_some_and(|afk| afk.afk_channel_id == channel.id)
        || guild
            .voice_states
            .get(&uid)
            .and_then(|state| state.channel_id)
            != Some(channel.id)
    {
        return Ok(None);
    }
    let Some(streamer) = guild.members.get(&uid) else {
        return Ok(None);
    };
    if streamer.user.bot
        || streamer.communication_disabled_until.is_some_and(|until| {
            until.unix_timestamp() > serenity::all::Timestamp::now().unix_timestamp()
        })
    {
        return Ok(None);
    }
    let Some(everyone) = guild.roles.get(&RoleId::new(guild_id)) else {
        return Ok(None);
    };
    let public = everyone_permissions(
        everyone.permissions,
        guild_id,
        &channel.permission_overwrites,
    );
    if !eligible(
        channel.id.get(),
        channel.parent_id.map(|id| id.get()),
        public,
    ) {
        return Ok(None);
    }
    let required = Permissions::VIEW_CHANNEL | Permissions::CONNECT;
    if !guild
        .user_permissions_in(channel, streamer)
        .contains(required)
    {
        return Ok(None);
    }
    let bot_id = adapter.cache().current_user().id;
    let Some(bot) = guild.members.get(&bot_id) else {
        return Ok(None);
    };
    let permissions = guild.user_permissions_in(channel, bot);
    if !permissions.contains(required | Permissions::CREATE_INSTANT_INVITE) {
        return Ok(None);
    }
    Ok(Some(Snapshot {
        members: guild
            .voice_states
            .values()
            .filter(|state| state.channel_id == Some(channel.id))
            .count(),
        can_manage: permissions.contains(Permissions::MANAGE_CHANNELS),
    }))
}

async fn fresh_channel(
    adapter: &DiscordAdapter,
    channel_id: u64,
) -> Result<GuildChannel, PortError> {
    adapter
        .http
        .get_channel(ChannelId::new(channel_id))
        .await
        .map_err(|_| PortError::GuildUnavailable)?
        .guild()
        .ok_or(PortError::ChannelNotFound)
}

async fn read_after_edit_error(
    adapter: &DiscordAdapter,
    guild_id: u64,
    streamer_id: u64,
    channel_id: u64,
) -> Result<EditReadback, PortError> {
    if guild_id != GUILD || !adapter.community_gateway.is_ready() {
        return Err(PortError::GuildUnavailable);
    }
    let channel = fresh_channel(adapter, channel_id).await?;
    let state = snapshot(adapter, &channel, guild_id, streamer_id)?;
    Ok(EditReadback {
        channel_id: channel.id.get(),
        limit: channel.user_limit.unwrap_or(0),
        members: state.as_ref().map_or(0, |state| state.members),
        // The broker's channel allowlist is immutable for this request and
        // approved this exact channel ID before entering the adapter.
        policy_safe: state.is_some(),
    })
}

/// A failed response must not leave a usable public invite behind. Never
/// restore a channel limit here: Discord has no conditional edit, so a
/// read-then-write rollback could overwrite a concurrent owner change.
async fn discard_invite(adapter: &DiscordAdapter, code: &str) {
    if !code.is_empty() {
        if let Err(error) = adapter.http.delete_invite(code, Some(REASON)).await {
            tracing::error!(%error, "Streamer-Voice-Invite konnte nicht zurückgenommen werden");
        }
    }
}

pub(crate) async fn invite_still_valid(
    adapter: &DiscordAdapter,
    guild_id: u64,
    streamer_id: u64,
    channel_id: u64,
    invite_url: &str,
) -> Result<bool, PortError> {
    let _guard = adapter.streamer_voice_lock.lock().await;
    if guild_id != GUILD || !adapter.community_gateway.is_ready() {
        return Err(PortError::GuildUnavailable);
    }
    let channel = fresh_channel(adapter, channel_id).await?;
    let Some(state) = snapshot(adapter, &channel, guild_id, streamer_id)? else {
        return Ok(false);
    };
    let limit = channel.user_limit.unwrap_or(0);
    if limit != 0 && state.members >= limit as usize {
        return Ok(false);
    }
    let Some(code) = invite_url.strip_prefix("https://discord.gg/") else {
        return Ok(false);
    };
    if code.is_empty()
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Ok(false);
    }
    let invite = adapter
        .http
        .get_invite(code, false, false, None)
        .await
        .map_err(|_| PortError::GuildUnavailable)?;
    Ok(invite.code == code
        && invite.channel.id == channel.id
        && invite
            .guild
            .as_ref()
            .is_some_and(|guild| guild.id.get() == guild_id))
}

pub(crate) async fn invite(
    adapter: &DiscordAdapter,
    guild_id: u64,
    streamer_id: u64,
    expected_channel_id: u64,
) -> Result<Option<StreamerVoiceInvite>, PortError> {
    // A REST read inside this lock, not a possibly lagging gateway user_limit,
    // makes simultaneous viewers and retrying requests safe.
    let _guard = adapter.streamer_voice_lock.lock().await;
    if guild_id != GUILD || !adapter.community_gateway.is_ready() {
        return Err(PortError::GuildUnavailable);
    }
    let channel = fresh_channel(adapter, expected_channel_id).await?;
    let Some(state) = snapshot(adapter, &channel, guild_id, streamer_id)? else {
        return Ok(None);
    };
    let limit = channel.user_limit.unwrap_or(0);
    let Some(target) = target_limit(limit, state.members) else {
        return Ok(None);
    };
    if target != limit && !state.can_manage {
        return Ok(None);
    }

    // Invite failure must not leave an expanded channel behind. Short-lived,
    // channel-specific invites also work for people not yet on the server.
    let invite = adapter
        .http
        .create_invite(channel.id, &invite_options(), Some(REASON))
        .await
        .map_err(|_| PortError::GuildUnavailable)?;
    let result = async {
        if invite.channel.id != channel.id
            || invite
                .guild
                .as_ref()
                .is_none_or(|guild| guild.id.get() != guild_id)
            || invite.code.is_empty()
            || !invite
                .code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(PortError::GuildUnavailable);
        }

        // The owner may have changed the limit or locked the channel while
        // creating the invite. Re-read before writing only user_limit.
        let channel = fresh_channel(adapter, expected_channel_id).await?;
        let Some(state) = snapshot(adapter, &channel, guild_id, streamer_id)? else {
            return Ok(None);
        };
        let limit = channel.user_limit.unwrap_or(0);
        let Some(target) = target_limit(limit, state.members) else {
            return Ok(None);
        };
        let slot_added = target != limit;
        if slot_added {
            if !state.can_manage {
                return Ok(None);
            }
            // This is the final mutating step. The REST response confirms the
            // target; a later read/rollback cannot be atomic with an owner edit.
            match adapter
                .http
                .edit_channel(channel.id, &json!({"user_limit": target}), Some(REASON))
                .await
            {
                Ok(edited) if edited.id == channel.id && edited.user_limit.unwrap_or(0) == target => {}
                Ok(_) => return Err(PortError::GuildUnavailable),
                Err(error) => {
                    tracing::warn!(%error, channel_id = expected_channel_id, target, "Streamer-Voice-Edit-ACK unklar; REST-Zustand wird geprüft");
                    if !confirm_uncertain_edit(expected_channel_id, target, || {
                        read_after_edit_error(adapter, guild_id, streamer_id, expected_channel_id)
                    })
                    .await
                    {
                        return Err(PortError::GuildUnavailable);
                    }
                }
            }
        }
        tracing::info!(
            guild_id,
            streamer_id,
            channel_id = expected_channel_id,
            slot_added,
            "Streamer-Voice-Invite erstellt"
        );
        Ok(Some(StreamerVoiceInvite {
            invite_url: format!("https://discord.gg/{}", invite.code),
            channel_id: expected_channel_id.to_string(),
            slot_added,
        }))
    }
    .await;
    if !matches!(result, Ok(Some(_))) {
        discard_invite(adapter, &invite.code).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn readback(limit: u32, policy_safe: bool) -> Result<EditReadback, PortError> {
        Ok(EditReadback {
            channel_id: 42,
            limit,
            members: 8,
            policy_safe,
        })
    }

    #[tokio::test]
    async fn verlorenes_edit_ack_mit_wirksamem_limit_wird_bestaetigt() {
        let mut reads = VecDeque::from([readback(9, true)]);
        assert!(
            confirm_uncertain_edit(42, 9, || {
                std::future::ready(reads.pop_front().expect("readback"))
            })
            .await
        );
    }

    #[tokio::test]
    async fn noch_nicht_sichtbares_edit_wird_begrenzt_nachgelesen() {
        let mut reads = VecDeque::from([readback(8, true), readback(9, true)]);
        assert!(
            confirm_uncertain_edit(42, 9, || {
                std::future::ready(reads.pop_front().expect("readback"))
            })
            .await
        );
        assert!(reads.is_empty());
    }

    #[tokio::test]
    async fn nicht_wirksames_edit_oder_get_fehler_bleibt_gesperrt() {
        let mut unchanged = VecDeque::from([readback(8, true), readback(8, true)]);
        assert!(
            !confirm_uncertain_edit(42, 9, || {
                std::future::ready(unchanged.pop_front().expect("readback"))
            })
            .await
        );
        let mut unavailable = VecDeque::from([
            Err(PortError::GuildUnavailable),
            Err(PortError::GuildUnavailable),
        ]);
        assert!(
            !confirm_uncertain_edit(42, 9, || {
                std::future::ready(unavailable.pop_front().expect("readback"))
            })
            .await
        );
    }

    #[tokio::test]
    async fn nach_rechteentzug_oder_vollem_ziel_keine_bestaetigung() {
        let mut revoked = VecDeque::from([readback(9, false)]);
        assert!(
            !confirm_uncertain_edit(42, 9, || {
                std::future::ready(revoked.pop_front().expect("readback"))
            })
            .await
        );
        let mut full = VecDeque::from([Ok(EditReadback {
            channel_id: 42,
            limit: 9,
            members: 9,
            policy_safe: true,
        })]);
        assert!(
            !confirm_uncertain_edit(42, 9, || {
                std::future::ready(full.pop_front().expect("readback"))
            })
            .await
        );
    }

    #[test]
    fn public_invite_can_be_used_once() {
        assert_eq!(
            invite_options(),
            json!({"max_age": 600, "max_uses": 1, "unique": true})
        );
    }

    #[test]
    fn capacity_changes_only_when_full() {
        assert_eq!(target_limit(8, 8), Some(9));
        assert_eq!(target_limit(9, 8), Some(9));
        assert_eq!(target_limit(8, 7), Some(8));
        assert_eq!(target_limit(8, 10), Some(11));
        assert_eq!(target_limit(0, 100), Some(0));
        assert_eq!(target_limit(99, 99), None);
        assert_eq!(target_limit(8, usize::MAX), None);
    }

    #[test]
    fn owner_or_role_access_does_not_make_a_locked_voice_public() {
        let required = Permissions::VIEW_CHANNEL | Permissions::CONNECT;
        let overwrites: Vec<PermissionOverwrite> = serde_json::from_value(json!([
            {"id": GUILD.to_string(), "type": 0, "allow": "0", "deny": Permissions::CONNECT.bits().to_string()},
            {"id": "42", "type": 0, "allow": required.bits().to_string(), "deny": "0"},
            {"id": "43", "type": 1, "allow": required.bits().to_string(), "deny": "0"}
        ])).expect("overwrites");
        assert!(!everyone_permissions(required, GUILD, &overwrites).contains(required));
        assert!(everyone_permissions(required, GUILD, &[]).contains(required));
    }
}
