use chrono::{DateTime, Utc};
use serenity::all::{GuildId, Http, PermissionOverwrite, PermissionOverwriteType, Role};
use sqlx::PgPool;

use crate::db;
use crate::model::{
    CategorySpec, ChannelKind, ChannelSpec, GuildModel, OverwriteKey, PermissionOverwriteSpec,
    RoleSpec, TargetKind,
};
use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotImportReport {
    pub snapshot_id: i64,
    pub guild_id: u64,
    pub captured_at: DateTime<Utc>,
    pub categories: usize,
    pub channels: usize,
    pub roles: usize,
    pub overwrites: usize,
}

pub async fn import_live_guild_snapshot(
    pool: &PgPool,
    http: &Http,
    guild_id: u64,
) -> Result<SnapshotImportReport> {
    let model = fetch_live_guild_model(http, guild_id).await?;
    db::persist_snapshot_model(pool, &model, "serenity_http").await
}

pub async fn fetch_live_guild_model(http: &Http, guild_id: u64) -> Result<GuildModel> {
    let guild_id = GuildId::new(guild_id);
    let channels = guild_id.channels(http).await?;
    let roles = http.get_guild_roles(guild_id).await?;
    let mut model = GuildModel::new(guild_id.get());

    for channel in channels.values() {
        let kind = ChannelKind::from_serenity(channel.kind);
        if kind == ChannelKind::Category {
            model.categories.insert(
                channel.id.get(),
                CategorySpec {
                    guild_id: guild_id.get(),
                    category_id: channel.id.get(),
                    name: channel.name.clone(),
                    position: i32::from(channel.position),
                },
            );
        } else {
            model.channels.insert(
                channel.id.get(),
                ChannelSpec {
                    guild_id: guild_id.get(),
                    channel_id: channel.id.get(),
                    name: channel.name.clone(),
                    kind,
                    topic: channel.topic.clone(),
                    position: i32::from(channel.position),
                    parent_category_id: channel.parent_id.map(|id| id.get()),
                    nsfw: channel.nsfw,
                    bitrate: channel.bitrate.and_then(|value| i32::try_from(value).ok()),
                    user_limit: channel
                        .user_limit
                        .and_then(|value| i32::try_from(value).ok()),
                    rate_limit_per_user: channel.rate_limit_per_user.map(i32::from),
                    status: channel.status.clone(),
                },
            );
        }

        for overwrite in &channel.permission_overwrites {
            if let Some(spec) = overwrite_spec(guild_id.get(), channel.id.get(), overwrite) {
                model.overwrites.insert(spec.key.clone(), spec);
            }
        }
    }

    for role in roles {
        model
            .roles
            .insert(role.id.get(), role_spec(guild_id.get(), &role));
    }

    Ok(model)
}

fn role_spec(guild_id: u64, role: &Role) -> RoleSpec {
    RoleSpec {
        guild_id,
        role_id: role.id.get(),
        name: role.name.clone(),
        color: i32::try_from(role.colour.0).unwrap_or(0),
        hoist: role.hoist,
        mentionable: role.mentionable,
        managed: role.managed,
        permissions_bitmask: role.permissions.bits(),
        position: i32::from(role.position),
    }
}

fn overwrite_spec(
    guild_id: u64,
    channel_id: u64,
    overwrite: &PermissionOverwrite,
) -> Option<PermissionOverwriteSpec> {
    let (target_kind, target_id) = match overwrite.kind {
        PermissionOverwriteType::Role(role_id) => (TargetKind::Role, role_id.get()),
        PermissionOverwriteType::Member(user_id) => (TargetKind::Member, user_id.get()),
        _ => return None,
    };

    Some(PermissionOverwriteSpec {
        guild_id,
        key: OverwriteKey {
            channel_id,
            target_kind,
            target_id,
        },
        allow_bits: overwrite.allow.bits(),
        deny_bits: overwrite.deny.bits(),
    })
}
