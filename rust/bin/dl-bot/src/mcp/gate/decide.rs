use super::policy::{Capability, Policy};
use crate::mcp::access::{role_can_view, Access, GuildRoles, ADMIN, VIEW};
use dl_access_gate::{ActionClass, Reason};
use serde_json::Value;

pub const MAX_MEMBER_OVERWRITES: usize = 25;

pub fn holds(capability: &Capability, roles: &[String], bits: u64) -> bool {
    !capability.owner_only
        && (bits & ADMIN != 0
            || bits & capability.mask != 0
            || roles.iter().any(|role| capability.roles.contains(role)))
}

pub fn grant(capability: Option<&Capability>, class: ActionClass, access: &Access) -> Reason {
    if access.timed_out && class != ActionClass::Read {
        return Reason::TimedOut;
    }
    if capability.is_some_and(|capability| capability.owner_only) {
        return Reason::OwnerOnly;
    }
    if access.base & ADMIN != 0 {
        return Reason::Allowed;
    }
    let Some(capability) = capability else {
        return Reason::Unmapped;
    };
    if capability.public || holds(capability, &access.roles, access.base) {
        Reason::Allowed
    } else {
        Reason::MissingRight
    }
}

pub fn outranks(asker: &Access, target_rank: i64) -> bool {
    asker.top_position > target_rank
}

pub fn dm_with(channel: &Value, user: u64) -> bool {
    channel["type"].as_u64() == Some(1)
        && channel["recipients"].as_array().is_some_and(|recipients| {
            recipients
                .iter()
                .any(|recipient| recipient["id"].as_str() == Some(user.to_string().as_str()))
        })
}

fn member_viewers(channel: &Value) -> Option<Vec<u64>> {
    let mut users = Vec::new();
    for overwrite in channel.get("permission_overwrites")?.as_array()? {
        if overwrite.get("type")?.as_u64()? != 1 {
            continue;
        }
        let allow = overwrite.get("allow")?.as_str()?.parse::<u64>().ok()?;
        if allow & VIEW != 0 {
            users.push(overwrite.get("id")?.as_str()?.parse::<u64>().ok()?);
        }
    }
    (users.len() <= MAX_MEMBER_OVERWRITES).then_some(users)
}

pub fn guild_place(
    policy: &Policy,
    capability: Option<&Capability>,
    channel_id: u64,
    channel: &Value,
    roles: &GuildRoles,
) -> Option<Vec<u64>> {
    let capability = capability?;
    if !policy
        .area_channels(capability)?
        .contains(&channel_id.to_string())
    {
        return None;
    }
    if channel["guild_id"].as_str() != Some(roles.guild.to_string().as_str()) {
        return None;
    }
    let verified = roles.verified.to_string();
    if role_can_view(channel, roles, None) || role_can_view(channel, roles, Some(&verified)) {
        return None;
    }
    let everyone = roles.guild.to_string();
    let outsider = roles.roles.iter().any(|(id, info)| {
        *id != everyone
            && role_can_view(channel, roles, Some(id))
            && !holds(
                capability,
                std::slice::from_ref(id),
                info.bits | roles.everyone,
            )
    });
    if outsider {
        return None;
    }
    member_viewers(channel)
}
