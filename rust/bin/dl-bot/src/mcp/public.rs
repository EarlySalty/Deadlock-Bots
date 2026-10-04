use super::{discord_call, resolve_guild, McpState};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use serenity::all::{GuildId, RoleId};
use sqlx::Row;
use std::{collections::HashSet, time::{Duration, Instant}};

const VIEW: u64 = 1 << 10;
const HISTORY: u64 = 1 << 16;
const ADMIN: u64 = 1 << 3;
const TTL: Duration = Duration::from_secs(60);

pub(super) struct CachedFacts {
    guild: String,
    channels: Value,
    created: Instant,
    facts: Value,
}

fn everyone_permissions(channel: &Value, guild: &str, base: u64) -> Option<u64> {
    if base & ADMIN != 0 {
        return None;
    }
    let mut permissions = base;
    for overwrite in channel.get("permission_overwrites")?.as_array()? {
        let kind = overwrite.get("type")?.as_u64()?;
        let id = overwrite.get("id")?.as_str()?;
        if kind == 0 && id == guild {
            let deny = overwrite.get("deny")?.as_str()?.parse::<u64>().ok()?;
            let allow = overwrite.get("allow")?.as_str()?.parse::<u64>().ok()?;
            permissions = (permissions & !deny) | allow;
        }
    }
    Some(permissions)
}

fn public_channels(channels: &[Value], guild: &str, base: u64) -> Vec<Value> {
    let allowed: HashSet<&str> = channels.iter().filter(|c| {
        everyone_permissions(c, guild, base).is_some_and(|p| p & VIEW != 0)
            && matches!(c["type"].as_u64(), Some(0 | 2 | 4 | 5 | 13 | 15 | 16))
    }).filter_map(|c| c["id"].as_str()).collect();
    let mut result: Vec<Value> = channels.iter().filter(|c| {
        c["id"].as_str().is_some_and(|id| allowed.contains(id))
            && (c["parent_id"].is_null() || c["parent_id"].as_str().is_some_and(|id| allowed.contains(id)))
    }).map(|c| json!({
        "id": c["id"], "name": c["name"], "type": c["type"],
        "topic": c["topic"], "position": c["position"], "parent_id": c["parent_id"]
    })).collect();
    result.sort_by(|a,b| (a["position"].as_i64(), a["id"].as_str()).cmp(&(b["position"].as_i64(), b["id"].as_str())));
    result
}

fn info_key(key: &str) -> bool {
    matches!(key, "guide_lfg" | "spawn_manage" | "regelwerk" | "faq" | "rang-guide")
        || key.starts_with("regelwerk:") || key.starts_with("faq:") || key.starts_with("rang-guide:")
}

fn component_text(components: &Value, out: &mut Vec<String>) {
    if let Some(items) = components.as_array() {
        for item in items {
            if item["type"].as_u64() == Some(10) {
                if let Some(text) = item["content"].as_str() {
                    out.push(text.to_owned());
                }
            }
            component_text(&item["components"], out);
        }
    }
}

fn own_info(message: &Value, bot_id: &str) -> Option<String> {
    if message["author"]["id"].as_str()? != bot_id
        || message["author"]["bot"].as_bool() != Some(true)
        || !message["webhook_id"].is_null()
        || !message["message_reference"].is_null()
        || message["type"].as_u64() != Some(0)
        || message["mentions"].as_array().is_none_or(|items| !items.is_empty()) {
        return None;
    }
    let mut texts = Vec::new();
    if let Some(content) = message["content"].as_str().filter(|s| !s.is_empty()) {
        texts.push(content.to_owned());
    }
    component_text(&message["components"], &mut texts);
    let text = texts.join("\n");
    if text.is_empty() || text.contains("<@") || text.contains("discord.com/users/") {
        return None;
    }
    Some(text)
}

pub(super) async fn facts(st: &McpState, args: &Value) -> Result<Value> {
    let guild_id = resolve_guild(st, args).await?;
    if st.default_guild.as_deref() != Some(&guild_id) {
        bail!("Öffentliche Live-Fakten benötigen die bestehende feste MCP-Guild.");
    }
    let (adapter, pool) = st.public_source.as_ref().context("Öffentliche Gateway-Quelle fehlt")?;
    let guild_number: u64 = guild_id.parse().context("Guild-ID ungültig")?;
    let voice = adapter.voice_cache_snapshot(guild_number).context("Gateway-Momentaufnahme ist derzeit unbekannt")?;
    let (channels, raw, base) = {
        let guild = adapter.cache().guild(GuildId::new(guild_number)).context("Guild-Cache fehlt")?;
        let base = guild.roles.get(&RoleId::new(guild_number)).context("everyone-Rolle fehlt")?.permissions.bits();
        let raw: Vec<Value> = guild.channels.values().map(serde_json::to_value).collect::<std::result::Result<_,_>>()?;
        (public_channels(&raw, &guild_id, base), raw, base)
    };
    let channels_value = Value::Array(channels.clone());
    let mut cache = st.public_cache.lock().await;
    if let Some(cached) = cache.as_ref().filter(|c| c.guild == guild_id && c.channels == channels_value && c.created.elapsed() < TTL) {
        return Ok(cached.facts.clone());
    }
    let mut voice_counts = Vec::new();
    for channel in &channels {
        if matches!(channel["type"].as_u64(), Some(2 | 13)) {
            let id = channel["id"].as_str().context("Kanal-ID fehlt")?.parse::<u64>()?;
            voice_counts.push(json!({"channel_id": channel["id"], "count": voice.members.values().filter(|c| **c == id).count()}));
        }
    }
    let bot_id = adapter.bot_user_id_cell().get().copied().context("Gateway-Botidentität fehlt")?.to_string();
    let refs = sqlx::query("SELECT channel_id, message_id, message_key FROM server_config.desired_bot_messages WHERE guild_id = $1 AND message_kind = 'panel' AND message_id IS NOT NULL ORDER BY channel_id, message_key LIMIT 65")
        .bind(i64::try_from(guild_number)?).fetch_all(pool).await?;
    if refs.len() > 64 {
        bail!("Zu viele registrierte Bot-Infotexte für den begrenzten Live-Aufruf");
    }
    let mut infos = Vec::new();
    for reference in refs {
        let key: String = reference.try_get("message_key")?;
        if !info_key(&key) { continue; }
        let id = reference.try_get::<i64,_>("channel_id")?.to_string();
        if !channels.iter().any(|c| c["id"].as_str() == Some(&id)) { continue; }
        let Some(channel) = raw.iter().find(|c| c["id"].as_str() == Some(&id)) else { continue; };
        if !everyone_permissions(channel, &guild_id, base).is_some_and(|p| p & HISTORY != 0) { continue; }
        let message_id = reference.try_get::<i64,_>("message_id")?.to_string();
        let message = discord_call(st, "GET", &format!("/channels/{id}/messages/{message_id}"), &[], None, None).await?;
        if let Some(text) = own_info(&message, &bot_id) {
            infos.push(json!({"channel_id": id, "message_id": message_id, "text": text}));
        }
    }
    let result = json!({"guild_id": guild_id, "observed_at": chrono::Utc::now().to_rfc3339(), "cache_seconds": 60, "audience": "everyone", "channels": channels, "voice_counts": voice_counts, "bot_infos": infos});
    *cache = Some(CachedFacts { guild: guild_id, channels: channels_value, created: Instant::now(), facts: result.clone() });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oeffentlicher_filter_schliesst_ticket_mod_und_private_kategorie_aus() {
        let channels = vec![
            json!({"id":"10","name":"Lanes","type":4,"position":0,"permission_overwrites":[]}),
            json!({"id":"11","name":"Lane","type":2,"parent_id":"10","position":1,"permission_overwrites":[]}),
            json!({"id":"12","name":"Ticket","type":0,"position":2,"permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"},{"id":"99","type":1,"deny":"0","allow":VIEW.to_string()}]}),
            json!({"id":"13","name":"Mod","type":0,"position":3,"permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"},{"id":"2","type":0,"deny":"0","allow":VIEW.to_string()}]}),
            json!({"id":"14","name":"Team","type":4,"position":4,"permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"}]}),
            json!({"id":"15","name":"Teamkind","type":0,"parent_id":"14","position":5,"permission_overwrites":[]}),
            json!({"id":"16","name":"Unbekannt","type":0,"position":6}),
        ];
        let result = public_channels(&channels, "1", VIEW | HISTORY);
        assert_eq!(result.iter().map(|c| c["id"].as_str().unwrap()).collect::<Vec<_>>(), ["10","11"]);
        assert!(public_channels(&channels, "1", ADMIN | VIEW).is_empty());
    }

    #[test]
    fn nur_eigener_registrierter_infotext_ohne_nutzerdaten() {
        let mut message = json!({"author":{"id":"2","bot":true,"username":"geheim"},"type":0,"mentions":[],"components":[{"type":17,"components":[{"type":10,"content":"So erstellst du eine Lane."}]}]});
        assert_eq!(own_info(&message,"2").as_deref(),Some("So erstellst du eine Lane."));
        assert!(own_info(&message,"3").is_none());
        message["mentions"] = json!([{"id":"4","username":"privat"}]);
        assert!(own_info(&message,"2").is_none());
        assert!(!info_key("welcome:team"));
        assert!(!info_key("ticket"));
    }
}
