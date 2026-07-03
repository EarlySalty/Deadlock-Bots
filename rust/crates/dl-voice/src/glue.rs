//! Gateway-Cache-Anbindung des Voice-Trackers.

use std::{collections::HashMap, sync::Arc};

use dl_discord::DiscordAdapter;
use serde_json::{json, Map, Value};
use serenity::all::{
    AutoArchiveDuration, ButtonStyle, ChannelId, CreateAllowedMentions, CreateForumPost,
    CreateMessage, GuildId, MessageId, PermissionOverwrite, PermissionOverwriteType, PremiumTier,
    RoleId, UserId,
};
use serenity::builder::{CreateActionRow, CreateButton, EditMessage, EditThread, GetMessages};

use crate::tempvoice::LanePort;
use crate::tracker::{VoiceMemberState, VoiceSnapshot};

/// Discord-Permission-Bit CONNECT (Voice).
const CONNECT_BIT: u64 = 1 << 20;
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const SERENITY_429_RETRY_AFTER_FALLBACK_SECONDS: f64 = 1.0;

pub fn merge_connect_overwrite(
    existing_allow: u64,
    existing_deny: u64,
    connect: Option<bool>,
) -> Option<(u64, u64)> {
    let mut allow_bits = existing_allow & !CONNECT_BIT;
    let mut deny_bits = existing_deny & !CONNECT_BIT;
    match connect {
        Some(true) => allow_bits |= CONNECT_BIT,
        Some(false) => deny_bits |= CONNECT_BIT,
        None => {}
    }
    (allow_bits != 0 || deny_bits != 0).then_some((allow_bits, deny_bits))
}

fn overwrites_to_json(overwrites: &[PermissionOverwrite]) -> Vec<Value> {
    overwrites
        .iter()
        .filter_map(|ow| {
            let (kind, target_id) = match ow.kind {
                PermissionOverwriteType::Role(role_id) => (0u8, role_id.get()),
                PermissionOverwriteType::Member(user_id) => (1u8, user_id.get()),
                _ => return None,
            };
            Some(json!({
                "id": target_id.to_string(),
                "type": kind,
                "allow": ow.allow.bits().to_string(),
                "deny": ow.deny.bits().to_string(),
            }))
        })
        .collect()
}

fn build_connect_batch_payload(
    adapter: &DiscordAdapter,
    guild_id: Option<u64>,
    channel_id: u64,
    role_changes: &HashMap<u64, Option<bool>>,
    member_changes: &HashMap<u64, Option<bool>>,
) -> Result<Vec<Value>, String> {
    fn push_changed_overwrites(
        overwrites: &mut Vec<Value>,
        kind: u8,
        changes: &HashMap<u64, Option<bool>>,
    ) {
        let mut changed: Vec<_> = changes.iter().collect();
        changed.sort_by_key(|(target_id, _)| **target_id);
        for (target_id, connect) in changed {
            let Some(connect) = *connect else {
                continue;
            };
            let (allow_bits, deny_bits) = if connect {
                (CONNECT_BIT, 0)
            } else {
                (0, CONNECT_BIT)
            };
            overwrites.push(json!({
                "id": target_id.to_string(),
                "type": kind,
                "allow": allow_bits.to_string(),
                "deny": deny_bits.to_string(),
            }));
        }
    }

    fn build_from_overwrites(
        existing: &[PermissionOverwrite],
        role_changes: &HashMap<u64, Option<bool>>,
        member_changes: &HashMap<u64, Option<bool>>,
    ) -> Vec<Value> {
        let mut overwrites = Vec::new();
        for ow in existing {
            let (kind, target_id) = match ow.kind {
                PermissionOverwriteType::Role(role_id) => (0u8, role_id.get()),
                PermissionOverwriteType::Member(user_id) => (1u8, user_id.get()),
                _ => continue,
            };
            if (kind == 0 && role_changes.contains_key(&target_id))
                || (kind == 1 && member_changes.contains_key(&target_id))
            {
                continue;
            }
            overwrites.push(json!({
                "id": target_id.to_string(),
                "type": kind,
                "allow": ow.allow.bits().to_string(),
                "deny": ow.deny.bits().to_string(),
            }));
        }

        push_changed_overwrites(&mut overwrites, 0, role_changes);
        push_changed_overwrites(&mut overwrites, 1, member_changes);
        overwrites
    }

    let channel_id = ChannelId::new(channel_id);
    if let Some(guild_id) = guild_id {
        let Some(guild) = adapter.cache().guild(GuildId::new(guild_id)) else {
            return Err("Guild nicht im Cache".to_string());
        };
        let Some(channel) = guild.channels.get(&channel_id) else {
            return Err("Channel nicht im Cache".to_string());
        };
        return Ok(build_from_overwrites(
            &channel.permission_overwrites,
            role_changes,
            member_changes,
        ));
    }

    for guild_id in adapter.cache().guilds() {
        let Some(guild) = adapter.cache().guild(guild_id) else {
            continue;
        };
        if let Some(channel) = guild.channels.get(&channel_id) {
            return Ok(build_from_overwrites(
                &channel.permission_overwrites,
                role_changes,
                member_changes,
            ));
        }
    }
    Err("Channel nicht im Cache".to_string())
}

fn collect_component_custom_ids(value: &Value, custom_ids: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(custom_id) = map.get("custom_id").and_then(Value::as_str) {
                custom_ids.push(custom_id.to_string());
            }
            if let Some(components) = map.get("components").and_then(Value::as_array) {
                for component in components {
                    collect_component_custom_ids(component, custom_ids);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_component_custom_ids(item, custom_ids);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RouterPanelFile {
    id: u8,
    filename: String,
    bytes: Vec<u8>,
}

fn router_file_part_name(id: u8) -> String {
    format!("files[{id}]")
}

fn router_panel_files(
    attachments: &[crate::router::RouterPanelAttachment],
) -> Result<Vec<RouterPanelFile>, String> {
    let repo_root = crate::router::router_repo_root();
    attachments
        .iter()
        .map(|attachment| {
            let path = repo_root.join(&attachment.relative_path);
            let bytes = std::fs::read(&path).map_err(|err| {
                format!(
                    "Router-Banner `{}` konnte nicht gelesen werden: {err}",
                    path.display()
                )
            })?;
            Ok(RouterPanelFile {
                id: attachment.id,
                filename: attachment.filename.clone(),
                bytes,
            })
        })
        .collect()
}

fn lfg_panel_files(
    attachments: &[crate::lfg_panel::LfgPanelAttachment],
) -> Result<Vec<RouterPanelFile>, String> {
    let repo_root = crate::lfg_panel::lfg_repo_root();
    attachments
        .iter()
        .map(|attachment| {
            let path = repo_root.join(&attachment.relative_path);
            let bytes = std::fs::read(&path).map_err(|err| {
                format!(
                    "LFG-Banner `{}` konnte nicht gelesen werden: {err}",
                    path.display()
                )
            })?;
            Ok(RouterPanelFile {
                id: attachment.id,
                filename: attachment.filename.clone(),
                bytes,
            })
        })
        .collect()
}

#[derive(Debug, serde::Deserialize)]
struct DiscordMessageWriteResponse {
    id: String,
}

struct RouterDiscordResponse {
    status: reqwest::StatusCode,
    body: String,
}

async fn send_router_message_payload(
    adapter: &DiscordAdapter,
    method: &'static str,
    url: String,
    body: Map<String, Value>,
    files: Vec<RouterPanelFile>,
) -> Result<RouterDiscordResponse, String> {
    let payload_text = serde_json::to_string(&body).map_err(|err| err.to_string())?;
    let client = reqwest::Client::new();
    let request = match method {
        "POST" => client.post(url),
        "PATCH" => client.patch(url),
        other => unreachable!("unsupported Discord method {other}"),
    }
    .header(reqwest::header::AUTHORIZATION, adapter.http.token());

    let mut form = reqwest::multipart::Form::new().text("payload_json", payload_text);
    for file in files {
        let part = reqwest::multipart::Part::bytes(file.bytes)
            .file_name(file.filename)
            .mime_str("image/png")
            .map_err(|err| err.to_string())?;
        form = form.part(router_file_part_name(file.id), part);
    }

    let response = request
        .multipart(form)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    Ok(RouterDiscordResponse { status, body })
}

fn router_discord_body_preview(body: &str) -> String {
    body.chars().take(300).collect()
}

fn router_discord_json_response<T: serde::de::DeserializeOwned>(
    response: RouterDiscordResponse,
    method: &str,
) -> Result<T, String> {
    if !response.status.is_success() {
        return Err(format!(
            "Discord {method} fehlgeschlagen: HTTP {}: {}",
            response.status.as_u16(),
            router_discord_body_preview(&response.body)
        ));
    }
    serde_json::from_str::<T>(&response.body).map_err(|err| err.to_string())
}

fn router_discord_success(response: RouterDiscordResponse, method: &str) -> Result<(), String> {
    if !response.status.is_success() {
        return Err(format!(
            "Discord {method} fehlgeschlagen: HTTP {}: {}",
            response.status.as_u16(),
            router_discord_body_preview(&response.body)
        ));
    }
    Ok(())
}

fn parse_router_message_id(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| "Discord POST lieferte keine gueltige Message-ID".to_string())
}

fn guild_voice_bitrate_limit(tier: PremiumTier) -> u32 {
    match tier {
        PremiumTier::Tier3 => 384_000,
        PremiumTier::Tier2 => 256_000,
        PremiumTier::Tier1 => 128_000,
        _ => 96_000,
    }
}

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
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
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
            .cache()
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
            let (overwrites, bitrate) = self
                .adapter
                .cache()
                .guild(GuildId::new(guild_id))
                .map(|guild| {
                    let overwrites = guild
                        .channels
                        .get(&ChannelId::new(category))
                        .map(|channel| overwrites_to_json(&channel.permission_overwrites))
                        .unwrap_or_default();
                    (overwrites, guild_voice_bitrate_limit(guild.premium_tier))
                })
                .unwrap_or_else(|| (Vec::new(), 96_000));
            if !overwrites.is_empty() {
                body.insert("permission_overwrites".into(), json!(overwrites));
            }
            body.insert("bitrate".into(), json!(bitrate));
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
        crate::rename_queue::enqueue_or_direct(&self.adapter, channel_id, name, reason).await
    }

    async fn set_member_connect(
        &self,
        channel_id: u64,
        user_id: u64,
        connect: Option<bool>,
    ) -> Result<(), String> {
        let role_changes = HashMap::new();
        let member_changes = HashMap::from([(user_id, connect)]);
        let overwrites = build_connect_batch_payload(
            &self.adapter,
            None,
            channel_id,
            &role_changes,
            &member_changes,
        )?;
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "permission_overwrites": overwrites }),
                Some("TempVoice: Member-Batch"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn apply_member_connect_batch(
        &self,
        channel_id: u64,
        denied_user_ids: &std::collections::HashSet<u64>,
        clear_user_ids: &std::collections::HashSet<u64>,
    ) -> Result<(), String> {
        if denied_user_ids.is_empty() && clear_user_ids.is_empty() {
            return Ok(());
        }
        let role_changes = HashMap::new();
        let mut member_changes = HashMap::new();
        for user_id in clear_user_ids {
            member_changes.insert(*user_id, None);
        }
        for user_id in denied_user_ids {
            member_changes.insert(*user_id, Some(false));
        }
        let overwrites = build_connect_batch_payload(
            &self.adapter,
            None,
            channel_id,
            &role_changes,
            &member_changes,
        )?;
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "permission_overwrites": overwrites }),
                Some("TempVoice: Mitglieder-Batch"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn set_role_connect(
        &self,
        channel_id: u64,
        role_id: u64,
        connect: Option<bool>,
    ) -> Result<(), String> {
        let role_changes = HashMap::from([(role_id, connect)]);
        let member_changes = HashMap::new();
        let overwrites = build_connect_batch_payload(
            &self.adapter,
            None,
            channel_id,
            &role_changes,
            &member_changes,
        )?;
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "permission_overwrites": overwrites }),
                Some("TempVoice: Rollen-Batch"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn apply_role_connect_batch(
        &self,
        guild_id: u64,
        channel_id: u64,
        allowed_role_ids: &std::collections::HashSet<u64>,
        clear_role_ids: &std::collections::HashSet<u64>,
    ) -> Result<(), String> {
        if allowed_role_ids.is_empty() && clear_role_ids.is_empty() {
            return Ok(());
        }
        let mut changes = HashMap::new();
        for role_id in clear_role_ids {
            changes.insert(*role_id, None);
        }
        for role_id in allowed_role_ids {
            changes.insert(*role_id, Some(true));
        }
        let member_changes = HashMap::new();
        let overwrites = build_connect_batch_payload(
            &self.adapter,
            Some(guild_id),
            channel_id,
            &changes,
            &member_changes,
        )?;
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "permission_overwrites": overwrites }),
                Some("TempVoice: Rangrollen-Batch"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn set_user_limit(
        &self,
        channel_id: u64,
        limit: i64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "user_limit": limit }),
                Some(reason),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn disconnect_member(
        &self,
        guild_id: u64,
        user_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "channel_id": null }),
                Some(reason),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .members
            .get(&UserId::new(user_id))
            .map(|m| m.display_name().to_string())
    }

    async fn add_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some(reason),
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn remove_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some(reason),
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn set_nick(
        &self,
        guild_id: u64,
        user_id: u64,
        nick: Option<&str>,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "nick": nick }),
                Some(reason),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn member_nick(&self, guild_id: u64, user_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .members
            .get(&UserId::new(user_id))?
            .nick
            .as_ref()
            .map(|n| n.to_string())
    }

    async fn channel_user_limit(&self, guild_id: u64, channel_id: u64) -> Option<i64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .map(|c| c.user_limit.map(|l| l as i64).unwrap_or(0))
    }

    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .voice_states
            .get(&UserId::new(user_id))?
            .channel_id
            .map(|c| c.get())
    }

    async fn member_role_names(&self, guild_id: u64, user_id: u64) -> Vec<String> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
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

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|guild| {
                guild
                    .members
                    .get(&UserId::new(user_id))
                    .map(|member| member.roles.iter().map(|role_id| role_id.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn channel_members(&self, guild_id: u64, channel_id: u64) -> Vec<u64> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
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
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .map(|c| c.name.to_string())
    }

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))?
            .parent_id
            .map(|p| p.get())
    }

    async fn category_voice_channel_names(&self, guild_id: u64, category_id: u64) -> Vec<String> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
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

    async fn category_voice_channels(&self, guild_id: u64, category_id: u64) -> Vec<(u64, String)> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        guild
            .channels
            .values()
            .filter(|c| {
                c.parent_id == Some(ChannelId::new(category_id))
                    && c.kind == serenity::all::ChannelType::Voice
            })
            .map(|c| (c.id.get(), c.name.to_string()))
            .collect()
    }

    async fn channel_created_at(&self, channel_id: u64) -> Option<i64> {
        Some(ChannelId::new(channel_id).created_at().unix_timestamp())
    }

    async fn guild_role_names(&self, guild_id: u64) -> Vec<(u64, String)> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.roles
                    .iter()
                    .map(|(id, role)| (id.get(), role.name.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn set_channel_category(
        &self,
        channel_id: u64,
        category_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "parent_id": category_id.to_string() }),
                Some(reason),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl crate::tempvoice::interface::TempVoiceInterfacePort for CacheSnapshot {
    async fn post_rich(&self, channel_id: u64, body: Map<String, Value>) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                serenity::all::MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<crate::tempvoice::interface::TempVoicePanelMessage>, String> {
        let bot_id = self
            .adapter
            .http
            .get_current_user()
            .await
            .map_err(|err| err.to_string())?
            .id;
        let mut messages = ChannelId::new(channel_id)
            .messages(&self.adapter.http, GetMessages::new().limit(limit.min(100)))
            .await
            .map_err(|err| err.to_string())?;
        messages.sort_by_key(|message| message.id.get());
        Ok(messages
            .into_iter()
            .filter(|message| message.author.id == bot_id)
            .map(
                |message| crate::tempvoice::interface::TempVoicePanelMessage {
                    message_id: message.id.get(),
                    has_embeds: !message.embeds.is_empty(),
                    has_components: !message.components.is_empty(),
                    embed_titles: message
                        .embeds
                        .into_iter()
                        .filter_map(|embed| embed.title)
                        .collect(),
                },
            )
            .collect())
    }

    async fn member_can_manage_guild(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|guild| {
                let member = guild.members.get(&UserId::new(user_id))?;
                Some(guild.member_permissions(member))
            })
            .map(|perms| perms.manage_guild() || perms.administrator())
            .unwrap_or(false)
    }
}

/// Cache-Anbindung für die Voice-Statistik-Befehle (`!vstats`/`!vleaderboard`).
#[async_trait::async_trait]
impl crate::stats::StatsPort for CacheSnapshot {
    async fn resolve_names(&self, user_ids: &[u64]) -> std::collections::HashMap<u64, String> {
        let mut out = std::collections::HashMap::new();
        let guilds = self.adapter.cache().guilds();
        for &user_id in user_ids {
            let target = UserId::new(user_id);
            for gid in &guilds {
                let Some(guild) = self.adapter.cache().guild(*gid) else {
                    continue;
                };
                if let Some(member) = guild.members.get(&target) {
                    out.insert(user_id, member.display_name().to_string());
                    break; // erste Fundstelle genügt (wie resolve_names)
                }
            }
        }
        out
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn guild_name(&self, guild_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| g.name.clone())
    }
}

/// Nudge-Anbindung: Cache + REST + Steam-Bot-Client.
pub struct NudgeGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub steam: Arc<dl_bridges::steam::SteamBotClient>,
    pub log_channel_id: u64,
}

#[async_trait::async_trait]
impl crate::nudge::NudgePort for NudgeGlue {
    async fn is_in_voice(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.voice_states
                    .get(&UserId::new(user_id))
                    .and_then(|vs| vs.channel_id)
            })
            .is_some()
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn send_dm(
        &self,
        user_id: u64,
        embeds: &[serde_json::Value],
        components: &serde_json::Value,
    ) -> Result<(u64, u64), String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|e| e.to_string())?;
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!(embeds));
        body.insert("components".into(), components.clone());
        let message_id = self
            .adapter
            .send_raw_public(channel.id.get(), &body)
            .await?;
        Ok((channel.id.get(), message_id))
    }

    async fn send_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(self.log_channel_id, &body)
            .await;
    }

    async fn fetch_steam_link_url(&self, user_id: u64) -> Option<String> {
        self.steam.fetch_steam_link_url(user_id).await
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) {
        let _ = self
            .adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                serenity::all::MessageId::new(message_id),
                Some("Nudge: vom User geschlossen"),
            )
            .await;
    }

    async fn refresh_dm(
        &self,
        channel_id: u64,
        message_id: u64,
        embeds: &[serde_json::Value],
        components: &serde_json::Value,
    ) -> Result<bool, String> {
        let body = json!({ "embeds": embeds, "components": components });
        match self
            .adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                serenity::all::MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(err) if is_missing_or_forbidden(&err) => Ok(false),
            Err(err) => Err(err.to_string()),
        }
    }
}

fn is_missing_or_forbidden(err: &serenity::Error) -> bool {
    matches!(
        err,
        serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp))
            if matches!(resp.status_code.as_u16(), 403 | 404)
    )
}

/// Voice-Status-Anbindung (Kategorien-Scan + Rename).
pub struct StatusGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub tempvoice: Option<Arc<crate::tempvoice::TempVoiceEngine>>,
}

#[async_trait::async_trait]
impl crate::status::StatusPort for StatusGlue {
    async fn monitored_channels(&self) -> Vec<(u64, u64, String, Vec<u64>)> {
        let mut result = Vec::new();
        for guild_id in self.adapter.cache().guilds() {
            let Some(guild) = self.adapter.cache().guild(guild_id) else {
                continue;
            };
            for (channel_id, channel) in &guild.channels {
                if channel.kind != serenity::all::ChannelType::Voice {
                    continue;
                }
                if crate::status::EXCLUDED_CHANNEL_IDS.contains(&channel_id.get()) {
                    continue;
                }
                let in_target = channel
                    .parent_id
                    .map(|p| crate::status::TARGET_CATEGORY_IDS.contains(&p.get()))
                    .unwrap_or(false);
                if !in_target {
                    continue;
                }
                let members: Vec<u64> = guild
                    .voice_states
                    .iter()
                    .filter(|(_, vs)| vs.channel_id == Some(*channel_id))
                    .filter(|(user_id, _)| {
                        guild
                            .members
                            .get(user_id)
                            .map(|m| !m.user.bot)
                            .unwrap_or(true)
                    })
                    .map(|(user_id, _)| user_id.get())
                    .collect();
                result.push((
                    guild_id.get(),
                    channel_id.get(),
                    channel.name.to_string(),
                    members,
                ));
            }
        }
        result
    }

    async fn channel_info(&self, channel_id: u64) -> Option<(u64, String, Vec<u64>)> {
        for guild_id in self.adapter.cache().guilds() {
            let Some(guild) = self.adapter.cache().guild(guild_id) else {
                continue;
            };
            if let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) {
                let members: Vec<u64> = guild
                    .voice_states
                    .iter()
                    .filter(|(_, vs)| vs.channel_id == Some(ChannelId::new(channel_id)))
                    .map(|(user_id, _)| user_id.get())
                    .collect();
                return Some((guild_id.get(), channel.name.to_string(), members));
            }
        }
        None
    }

    async fn resolved_base_name(
        &self,
        guild_id: u64,
        channel_id: u64,
        fallback_base: &str,
    ) -> Option<String> {
        let Some(engine) = &self.tempvoice else {
            return None;
        };
        engine
            .status_base_name(guild_id, channel_id)
            .await
            .filter(|base| !base.trim().is_empty())
            .or_else(|| Some(fallback_base.to_string()))
    }

    async fn rename(&self, channel_id: u64, name: &str) -> Result<(), String> {
        crate::rename_queue::enqueue_or_direct(
            &self.adapter,
            channel_id,
            name,
            "Deadlock Voice Status Update",
        )
        .await
    }
}

/// Rank-Manager-Anbindung (Cache-Reads + Batch-Overwrites).
pub struct RankGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl crate::rank::RankPort for RankGlue {
    async fn channel_members_with_roles(
        &self,
        guild_id: u64,
        channel_id: u64,
    ) -> Vec<(u64, Vec<(u64, String)>)> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        guild
            .voice_states
            .iter()
            .filter(|(_, vs)| vs.channel_id == Some(ChannelId::new(channel_id)))
            .filter_map(|(user_id, _)| {
                let member = guild.members.get(user_id)?;
                if member.user.bot {
                    return None;
                }
                let roles = member
                    .roles
                    .iter()
                    .filter_map(|rid| {
                        guild
                            .roles
                            .get(rid)
                            .map(|r| (rid.get(), r.name.to_string()))
                    })
                    .collect();
                Some((user_id.get(), roles))
            })
            .collect()
    }

    async fn guild_roles(&self, guild_id: u64) -> Vec<(u64, String)> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.roles
                    .iter()
                    .map(|(id, role)| (id.get(), role.name.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn current_role_overwrites(&self, guild_id: u64, channel_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.channels.get(&ChannelId::new(channel_id)).map(|c| {
                    c.permission_overwrites
                        .iter()
                        .filter_map(|ow| match ow.kind {
                            serenity::all::PermissionOverwriteType::Role(role_id) => {
                                Some(role_id.get())
                            }
                            _ => None,
                        })
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    async fn apply_overwrites(
        &self,
        guild_id: u64,
        channel_id: u64,
        allowed_role_ids: &std::collections::HashSet<u64>,
        removes: &std::collections::HashSet<u64>,
    ) -> Result<(), String> {
        const VIEW: u64 = 1 << 10;
        const CONNECT: u64 = 1 << 20;
        const SPEAK: u64 = 1 << 21;

        // Bestehende Overwrites übernehmen, Rang-Batch einmischen (wie channel.edit)
        let mut overwrites: Vec<serde_json::Value> = Vec::new();
        let mut handled: std::collections::HashSet<u64> = std::collections::HashSet::new();
        if let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) {
            if let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) {
                for ow in &channel.permission_overwrites {
                    let (kind, target_id) = match ow.kind {
                        serenity::all::PermissionOverwriteType::Role(role_id) => {
                            (0u8, role_id.get())
                        }
                        serenity::all::PermissionOverwriteType::Member(user_id) => {
                            (1u8, user_id.get())
                        }
                        _ => continue,
                    };
                    if kind == 0
                        && (removes.contains(&target_id) || allowed_role_ids.contains(&target_id))
                    {
                        continue; // wird unten neu gesetzt bzw. entfernt
                    }
                    if kind == 0 && target_id == guild_id {
                        continue; // @everyone wird unten gesetzt
                    }
                    handled.insert(target_id);
                    overwrites.push(json!({
                        "id": target_id.to_string(),
                        "type": kind,
                        "allow": ow.allow.bits().to_string(),
                        "deny": ow.deny.bits().to_string(),
                    }));
                }
            }
        }
        // @everyone: deny connect, allow view (Rolle = guild_id)
        overwrites.push(json!({
            "id": guild_id.to_string(),
            "type": 0,
            "allow": VIEW.to_string(),
            "deny": CONNECT.to_string(),
        }));
        for role_id in allowed_role_ids {
            overwrites.push(json!({
                "id": role_id.to_string(),
                "type": 0,
                "allow": (VIEW | CONNECT | SPEAK).to_string(),
                "deny": "0",
            }));
        }
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "permission_overwrites": overwrites }),
                Some("Rank System: Batch Permission Update"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn rename(&self, channel_id: u64, name: &str) -> Result<(), String> {
        crate::rename_queue::enqueue_or_direct(
            &self.adapter,
            channel_id,
            name,
            "Rank Voice Manager Rename",
        )
        .await
    }

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))?
            .parent_id
            .map(|p| p.get())
    }

    async fn caller_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .voice_states
            .get(&UserId::new(user_id))?
            .channel_id
            .map(|c| c.get())
    }

    async fn channel_name(&self, guild_id: u64, channel_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .map(|c| c.name.to_string())
    }

    async fn category_name(&self, guild_id: u64, channel_id: u64) -> Option<String> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let parent = guild.channels.get(&ChannelId::new(channel_id))?.parent_id?;
        guild.channels.get(&parent).map(|c| c.name.to_string())
    }

    async fn channel_member_count(&self, guild_id: u64, channel_id: u64) -> usize {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.voice_states
                    .iter()
                    .filter(|(_, vs)| vs.channel_id == Some(ChannelId::new(channel_id)))
                    .filter(|(uid, _)| g.members.get(uid).map(|m| !m.user.bot).unwrap_or(true))
                    .count()
            })
            .unwrap_or(0)
    }

    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .members
            .get(&UserId::new(user_id))
            .map(|m| m.display_name().to_string())
    }

    async fn member_roles(&self, guild_id: u64, user_id: u64) -> Vec<(u64, String)> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members.get(&UserId::new(user_id)).map(|m| {
                    m.roles
                        .iter()
                        .filter_map(|rid| g.roles.get(rid).map(|r| (rid.get(), r.name.to_string())))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    async fn guild_member_roles(&self, guild_id: u64, user_id: u64) -> Option<Vec<(u64, String)>> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let member = guild.members.get(&UserId::new(user_id))?;
        if member.user.bot {
            return None;
        }
        Some(
            member
                .roles
                .iter()
                .filter_map(|rid| {
                    guild
                        .roles
                        .get(rid)
                        .map(|r| (rid.get(), r.name.to_string()))
                })
                .collect(),
        )
    }

    async fn role_member_count(&self, guild_id: u64, role_id: u64) -> usize {
        let role = RoleId::new(role_id);
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.members
                    .iter()
                    .filter(|(_, m)| m.roles.contains(&role))
                    .count()
            })
            .unwrap_or(0)
    }

    async fn category_voice_channels(&self, guild_id: u64, category_id: u64) -> Vec<(u64, String)> {
        let parent = ChannelId::new(category_id);
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.channels
                    .iter()
                    .filter(|(_, c)| {
                        c.kind == serenity::all::ChannelType::Voice && c.parent_id == Some(parent)
                    })
                    .map(|(id, c)| (id.get(), c.name.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Feedback-DM-Anbindung.
pub struct FeedbackGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl crate::feedback::FeedbackPort for FeedbackGlue {
    async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>) {
        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) => {
                return if err.to_string().contains("50007") {
                    ("forbidden".to_string(), None)
                } else {
                    ("error".to_string(), None)
                }
            }
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        body.insert(
            "components".into(),
            crate::feedback::feedback_button_components(),
        );
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(message_id) => ("sent".to_string(), Some(message_id)),
            Err(err) if err.to_string().contains("50007") => ("forbidden".to_string(), None),
            Err(_) => ("error".to_string(), None),
        }
    }

    async fn forward_to_owner(&self, owner_id: u64, text: String) {
        if let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": owner_id.to_string() }))
            .await
        {
            let mut body = serde_json::Map::new();
            body.insert("content".into(), json!(text));
            let _ = self.adapter.send_raw_public(channel.id.get(), &body).await;
        }
    }

    async fn delete_feedback_prompt(&self, user_id: u64, message_id: u64) {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return;
        };
        let _ = self
            .adapter
            .http
            .delete_message(
                channel.id,
                serenity::all::MessageId::new(message_id),
                Some("Voice feedback: stale prompt replaced"),
            )
            .await;
    }

    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .members
            .get(&UserId::new(user_id))
            .map(|m| m.display_name().to_string())
    }
}

/// Router-Anbindung.
pub struct RouterGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl crate::router::RouterPort for RouterGlue {
    async fn category_lanes(&self, guild_id: u64, category_id: u64) -> Vec<(u64, Vec<u64>)> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        guild
            .channels
            .values()
            .filter(|c| {
                c.kind == serenity::all::ChannelType::Voice
                    && c.parent_id == Some(ChannelId::new(category_id))
            })
            .map(|c| {
                let members: Vec<u64> = guild
                    .voice_states
                    .iter()
                    .filter(|(_, vs)| vs.channel_id == Some(c.id))
                    .map(|(user_id, _)| user_id.get())
                    .collect();
                (c.id.get(), members)
            })
            .collect()
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .voice_states
            .get(&UserId::new(user_id))?
            .channel_id
            .map(|c| c.get())
    }

    async fn move_member(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "channel_id": channel_id.to_string() }),
                Some("Router: passende Lane gefunden"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn send_dm(&self, user_id: u64, text: String) {
        if let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            let mut body = serde_json::Map::new();
            body.insert("content".into(), json!(text));
            let _ = self.adapter.send_raw_public(channel.id.get(), &body).await;
        }
    }
}

#[async_trait::async_trait]
impl crate::router::RouterInterfacePort for RouterGlue {
    async fn post_rich(
        &self,
        channel_id: u64,
        body: Map<String, Value>,
        attachments: &[crate::router::RouterPanelAttachment],
    ) -> Result<u64, String> {
        let files = router_panel_files(attachments)?;
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages");
        let response = send_router_message_payload(&self.adapter, "POST", url, body, files).await?;
        let message: DiscordMessageWriteResponse = router_discord_json_response(response, "POST")?;
        parse_router_message_id(&message.id)
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
        attachments: &[crate::router::RouterPanelAttachment],
    ) -> Result<(), String> {
        let files = router_panel_files(attachments)?;
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages/{message_id}");
        let response =
            send_router_message_payload(&self.adapter, "PATCH", url, body, files).await?;
        router_discord_success(response, "PATCH")
    }

    async fn delete_message(
        &self,
        channel_id: u64,
        message_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                serenity::all::MessageId::new(message_id),
                Some(reason),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<crate::router::RouterPanelMessage>, String> {
        let bot_id = self
            .adapter
            .http
            .get_current_user()
            .await
            .map_err(|err| err.to_string())?
            .id;
        let mut messages = ChannelId::new(channel_id)
            .messages(&self.adapter.http, GetMessages::new().limit(limit.min(100)))
            .await
            .map_err(|err| err.to_string())?;
        messages.sort_by_key(|message| message.id.get());
        Ok(messages
            .into_iter()
            .filter(|message| message.author.id == bot_id)
            .map(|message| {
                let mut custom_ids = Vec::new();
                if let Ok(components) = serde_json::to_value(&message.components) {
                    collect_component_custom_ids(&components, &mut custom_ids);
                }
                crate::router::RouterPanelMessage {
                    message_id: message.id.get(),
                    has_embeds: !message.embeds.is_empty(),
                    has_components: !message.components.is_empty(),
                    custom_ids,
                }
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl crate::lfg_panel::LfgPanelPort for RouterGlue {
    async fn post_rich(
        &self,
        channel_id: u64,
        body: Map<String, Value>,
        attachments: &[crate::lfg_panel::LfgPanelAttachment],
    ) -> Result<u64, String> {
        let files = lfg_panel_files(attachments)?;
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages");
        let response = send_router_message_payload(&self.adapter, "POST", url, body, files).await?;
        let message: DiscordMessageWriteResponse = router_discord_json_response(response, "POST")?;
        parse_router_message_id(&message.id)
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
        attachments: &[crate::lfg_panel::LfgPanelAttachment],
    ) -> Result<(), String> {
        let files = lfg_panel_files(attachments)?;
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages/{message_id}");
        let response =
            send_router_message_payload(&self.adapter, "PATCH", url, body, files).await?;
        router_discord_success(response, "PATCH")
    }

    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<crate::lfg_panel::LfgPanelMessage>, String> {
        let bot_id = self
            .adapter
            .http
            .get_current_user()
            .await
            .map_err(|err| err.to_string())?
            .id;
        let mut messages = ChannelId::new(channel_id)
            .messages(&self.adapter.http, GetMessages::new().limit(limit.min(100)))
            .await
            .map_err(|err| err.to_string())?;
        messages.sort_by_key(|message| message.id.get());
        Ok(messages
            .into_iter()
            .filter(|message| message.author.id == bot_id)
            .map(|message| {
                let mut custom_ids = Vec::new();
                if let Ok(components) = serde_json::to_value(&message.components) {
                    collect_component_custom_ids(&components, &mut custom_ids);
                }
                crate::lfg_panel::LfgPanelMessage {
                    message_id: message.id.get(),
                    has_embeds: !message.embeds.is_empty(),
                    has_components: !message.components.is_empty(),
                    custom_ids,
                }
            })
            .collect())
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn create_forum_post(
        &self,
        forum_channel_id: u64,
        draft: crate::lfg_panel::LfgForumPostDraft,
    ) -> Result<crate::lfg_panel::LfgCreatedForumPost, String> {
        let message = CreateMessage::new()
            .content(draft.body)
            .allowed_mentions(CreateAllowedMentions::new())
            .components(vec![CreateActionRow::Buttons(vec![CreateButton::new(
                crate::lfg_panel::lfg_join_custom_id(draft.post_id),
            )
            .label(crate::lfg_panel::LFG_BTN_BEITRETEN)
            .style(ButtonStyle::Success)])]);
        let builder = CreateForumPost::new(draft.title, message)
            .auto_archive_duration(AutoArchiveDuration::OneDay)
            .audit_log_reason("LFG: Forum-Post");
        let channel = ChannelId::new(forum_channel_id)
            .create_forum_post(&self.adapter.http, builder)
            .await
            .map_err(|err| err.to_string())?;
        Ok(crate::lfg_panel::LfgCreatedForumPost {
            thread_id: channel.id.get(),
        })
    }

    async fn first_thread_message_id(&self, thread_id: u64) -> Result<Option<u64>, String> {
        let mut messages = ChannelId::new(thread_id)
            .messages(
                &self.adapter.http,
                GetMessages::new().after(MessageId::new(0)).limit(1),
            )
            .await
            .map_err(|err| err.to_string())?;
        messages.sort_by_key(|message| message.id.get());
        Ok(messages.first().map(|message| message.id.get()))
    }

    async fn archive_and_lock_thread(&self, thread_id: u64) -> Result<(), String> {
        ChannelId::new(thread_id)
            .edit_thread(
                &self.adapter.http,
                EditThread::new()
                    .archived(true)
                    .locked(true)
                    .audit_log_reason("LFG: Persistenzfehler-Cleanup"),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn edit_forum_starter_message(
        &self,
        thread_id: u64,
        starter_message_id: u64,
        body: String,
    ) -> Result<(), crate::lfg_panel::LfgEditError> {
        ChannelId::new(thread_id)
            .edit_message(
                &self.adapter.http,
                MessageId::new(starter_message_id),
                EditMessage::new().content(body),
            )
            .await
            .map(|_| ())
            .map_err(lfg_edit_error_from_serenity)
    }

    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .voice_states
            .get(&UserId::new(user_id))?
            .channel_id
            .map(|c| c.get())
    }

    async fn move_member(&self, guild_id: u64, user_id: u64, lane_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "channel_id": lane_id.to_string() }),
                Some("LFG: Beitreten"),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn lane_occupancy(
        &self,
        guild_id: u64,
        lane_id: u64,
    ) -> Option<crate::lfg_panel::LfgLaneOccupancy> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let channel_id = ChannelId::new(lane_id);
        let channel = guild.channels.get(&channel_id)?;
        let member_count = guild
            .voice_states
            .values()
            .filter(|voice_state| voice_state.channel_id == Some(channel_id))
            .count();
        Some(crate::lfg_panel::LfgLaneOccupancy {
            member_count: i64::try_from(member_count).ok()?,
            user_limit: channel.user_limit.map(i64::from).unwrap_or(0),
        })
    }

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))?
            .parent_id
            .map(|id| id.get())
    }
}

#[async_trait::async_trait]
impl crate::adaptive::AdaptivePort for CacheSnapshot {
    async fn category_channels(
        &self,
        guild_id: u64,
        category_id: u64,
    ) -> Vec<(u64, String, usize, i64)> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        guild
            .channels
            .values()
            .filter(|c| {
                c.kind == serenity::all::ChannelType::Voice
                    && c.parent_id == Some(ChannelId::new(category_id))
            })
            .map(|c| {
                let members = guild
                    .voice_states
                    .values()
                    .filter(|vs| vs.channel_id == Some(c.id))
                    .count();
                (c.id.get(), c.name.to_string(), members, c.position as i64)
            })
            .collect()
    }

    async fn member_role_pairs(&self, guild_id: u64, user_id: u64) -> Vec<(u64, String)> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        guild
            .members
            .get(&UserId::new(user_id))
            .map(|m| {
                m.roles
                    .iter()
                    .filter_map(|rid| {
                        guild
                            .roles
                            .get(rid)
                            .map(|r| (rid.get(), r.name.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn move_member(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "channel_id": channel_id.to_string() }),
                Some("Neue Spieler Lane: Anfänger einsortiert"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn rename_channel(&self, channel_id: u64, name: &str) -> Result<(), String> {
        crate::rename_queue::enqueue_or_direct(
            &self.adapter,
            channel_id,
            name,
            "Adaptive Lanes: Name nachgezogen",
        )
        .await
    }

    async fn delete_channel(&self, channel_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_channel(
                ChannelId::new(channel_id),
                Some("Adaptive Lanes: Lane leer"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn create_voice_channel(
        &self,
        guild_id: u64,
        category_id: u64,
        anchor_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("name".into(), json!(name));
        body.insert("type".into(), json!(2));
        body.insert("parent_id".into(), json!(category_id.to_string()));
        let (overwrites, user_limit, bitrate) = {
            let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
                return Err("Guild nicht im Cache".to_string());
            };
            let Some(anchor) = guild.channels.get(&ChannelId::new(anchor_id)) else {
                return Err("Adaptive-Lane-Anker nicht im Cache".to_string());
            };
            let overwrites: Vec<serde_json::Value> = anchor
                .permission_overwrites
                .iter()
                .filter_map(|ow| {
                    let (kind, target_id) = match ow.kind {
                        serenity::all::PermissionOverwriteType::Role(role_id) => {
                            (0u8, role_id.get())
                        }
                        serenity::all::PermissionOverwriteType::Member(user_id) => {
                            (1u8, user_id.get())
                        }
                        _ => return None,
                    };
                    Some(json!({
                        "id": target_id.to_string(),
                        "type": kind,
                        "allow": ow.allow.bits().to_string(),
                        "deny": ow.deny.bits().to_string(),
                    }))
                })
                .collect();
            (
                overwrites,
                anchor.user_limit.map(|limit| limit as i64).unwrap_or(0),
                anchor.bitrate,
            )
        };
        body.insert("permission_overwrites".into(), json!(overwrites));
        body.insert("user_limit".into(), json!(user_limit));
        if let Some(bitrate) = bitrate {
            body.insert("bitrate".into(), json!(bitrate));
        }
        self.adapter
            .http
            .create_channel(
                GuildId::new(guild_id),
                &body,
                Some("Adaptive Lanes: Lane nachgelegt"),
            )
            .await
            .map(|c| c.id.get())
            .map_err(|e| e.to_string())
    }

    async fn set_channel_position(&self, channel_id: u64, position: i64) -> Result<(), String> {
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "position": position }),
                Some("Lane-Sortierung: Rang-Reihenfolge"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Worker-seitige Anbindung der Rename-Queue: liest den aktuellen Channel-Namen
/// aus dem Gateway-Cache und führt die eigentliche Discord-Umbenennung aus.
pub struct RenameExecGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl crate::rename_queue::RenameExec for RenameExecGlue {
    async fn current_name(&self, channel_id: u64) -> Option<String> {
        let cid = ChannelId::new(channel_id);
        for guild_id in self.adapter.cache().guilds() {
            let Some(guild) = self.adapter.cache().guild(guild_id) else {
                continue;
            };
            if let Some(channel) = guild.channels.get(&cid) {
                return Some(channel.name.to_string());
            }
        }
        None
    }

    async fn edit_name(
        &self,
        channel_id: u64,
        name: &str,
        reason: &str,
    ) -> Result<(), crate::rename_queue::RenameError> {
        self.adapter
            .http
            .edit_channel(
                ChannelId::new(channel_id),
                &json!({ "name": name }),
                Some(reason),
            )
            .await
            .map(|_| ())
            .map_err(rename_error_from_serenity)
    }
}

fn rename_error_from_serenity(err: serenity::Error) -> crate::rename_queue::RenameError {
    if let serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp)) = &err {
        if resp.status_code.as_u16() == 429 {
            // Serenity 0.12.5 exposes neither response headers nor Discord's
            // JSON `retry_after` field on ErrorResponse; fall back like Python
            // only when the concrete value is unavailable here.
            return crate::rename_queue::RenameError::rate_limited(
                SERENITY_429_RETRY_AFTER_FALLBACK_SECONDS,
            );
        }
    }
    crate::rename_queue::RenameError::from(err.to_string())
}

fn lfg_edit_error_from_serenity(err: serenity::Error) -> crate::lfg_panel::LfgEditError {
    if let serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp)) = &err {
        return match resp.status_code.as_u16() {
            429 => crate::lfg_panel::LfgEditError::rate_limited(
                SERENITY_429_RETRY_AFTER_FALLBACK_SECONDS,
            ),
            404 => crate::lfg_panel::LfgEditError::NotFound(err.to_string()),
            _ => crate::lfg_panel::LfgEditError::Other(err.to_string()),
        };
    }
    crate::lfg_panel::LfgEditError::Other(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_429_fallback_is_python_default_when_serenity_drops_retry_after() {
        let mapped = crate::rename_queue::RenameError::rate_limited(
            SERENITY_429_RETRY_AFTER_FALLBACK_SECONDS,
        );

        assert_eq!(mapped.retry_after_seconds(), Some(1.0));
        assert_eq!(mapped.to_string(), "HTTP 429 (retry_after=1)");
    }

    #[test]
    fn member_connect_merge_erhaelt_fremde_bits() {
        let view_channel = 1 << 10;
        let speak = 1 << 21;

        assert_eq!(
            merge_connect_overwrite(view_channel, speak, Some(false)),
            Some((view_channel, speak | CONNECT_BIT))
        );
        assert_eq!(
            merge_connect_overwrite(view_channel | CONNECT_BIT, speak, None),
            Some((view_channel, speak))
        );
        assert_eq!(merge_connect_overwrite(CONNECT_BIT, 0, None), None);
    }

    #[test]
    fn router_multipart_part_names_nutzen_attachment_ids() {
        assert_eq!(router_file_part_name(0), "files[0]");
        assert_eq!(router_file_part_name(1), "files[1]");
        assert_eq!(router_file_part_name(2), "files[2]");
    }
}
