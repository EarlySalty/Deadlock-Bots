use super::{discord_call, McpState};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use serenity::all::{GuildId, RoleId};
use sqlx::Row;
use std::{
    collections::{BTreeSet, HashSet},
    time::{Duration, Instant},
};

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
    let allowed: HashSet<&str> = channels
        .iter()
        .filter(|c| {
            everyone_permissions(c, guild, base).is_some_and(|p| p & VIEW != 0)
                && matches!(c["type"].as_u64(), Some(0 | 2 | 4 | 5 | 13 | 15 | 16))
        })
        .filter_map(|c| c["id"].as_str())
        .collect();
    let mut result: Vec<Value> = channels
        .iter()
        .filter(|c| {
            c["id"].as_str().is_some_and(|id| allowed.contains(id))
                && (c["parent_id"].is_null()
                    || c["parent_id"]
                        .as_str()
                        .is_some_and(|id| allowed.contains(id)))
        })
        .map(|c| {
            json!({
                "id": c["id"], "name": c["name"], "type": c["type"],
                "topic": c["topic"], "position": c["position"], "parent_id": c["parent_id"]
            })
        })
        .collect();
    result.sort_by(|a, b| {
        (a["position"].as_i64(), a["id"].as_str()).cmp(&(b["position"].as_i64(), b["id"].as_str()))
    });
    result
}

fn info_key(key: &str) -> bool {
    matches!(
        key,
        "guide_lfg" | "spawn_manage" | "regelwerk" | "faq" | "rang-guide"
    ) || ["regelwerk:", "faq:", "rang-guide:"].iter().any(|prefix| {
        key.strip_prefix(prefix)
            .and_then(|suffix| suffix.parse::<usize>().ok())
            .is_some_and(|index| index > 0)
    })
}

struct StaticSource {
    channel_id: u64,
    prefix: String,
    format_key: String,
    format: &'static str,
}

impl StaticSource {
    fn message_id(&self, key: &str, value: &str) -> Option<String> {
        let suffix = key.strip_prefix(&self.prefix)?;
        let index = suffix.parse::<usize>().ok()?;
        let id = value.parse::<u64>().ok()?;
        (suffix == index.to_string() && id != 0).then(|| id.to_string())
    }
}

fn static_sources(guild: u64) -> Vec<StaticSource> {
    use crate::serversync::{faq_publish, rang_guide_publish, regelwerk_publish, voice_ux_publish};
    if guild != crate::serversync::GUILD_ID {
        return Vec::new();
    }
    let mut sources = vec![
        StaticSource {
            channel_id: regelwerk_publish::REGELWERK_CHANNEL_ID,
            prefix: regelwerk_publish::REGELWERK_MESSAGE_ID_PREFIX.into(),
            format_key: regelwerk_publish::REGELWERK_PAYLOAD_FORMAT_KEY.into(),
            format: regelwerk_publish::REGELWERK_PAYLOAD_FORMAT,
        },
        StaticSource {
            channel_id: faq_publish::FAQ_CHANNEL_ID,
            prefix: faq_publish::FAQ_MESSAGE_ID_PREFIX.into(),
            format_key: faq_publish::FAQ_PAYLOAD_FORMAT_KEY.into(),
            format: faq_publish::FAQ_PAYLOAD_FORMAT,
        },
        StaticSource {
            channel_id: rang_guide_publish::RANG_GUIDE_CHANNEL_ID,
            prefix: rang_guide_publish::RANG_GUIDE_MESSAGE_ID_PREFIX.into(),
            format_key: rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT_KEY.into(),
            format: rang_guide_publish::RANG_GUIDE_PAYLOAD_FORMAT,
        },
    ];
    sources.extend(
        voice_ux_publish::VOICE_UX_TARGET_CHANNEL_IDS.map(|channel_id| StaticSource {
            channel_id,
            prefix: voice_ux_publish::voice_ux_message_id_prefix(channel_id),
            format_key: voice_ux_publish::voice_ux_payload_format_key(channel_id),
            format: voice_ux_publish::VOICE_UX_PAYLOAD_FORMAT,
        }),
    );
    sources
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
        || message["mentions"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
    {
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

fn registered_info(message: &Value, bot_id: &str, channel: &str, id: &str) -> Option<String> {
    if message["id"].as_str() != Some(id) || message["channel_id"].as_str() != Some(channel) {
        return None;
    }
    own_info(message, bot_id)
}

pub(super) async fn facts(st: &McpState, args: &Value) -> Result<Value> {
    if args.as_object().is_none_or(|args| !args.is_empty()) {
        bail!(
            "Öffentliche Live-Fakten akzeptieren keine abweichende Guild oder Nachrichtenauswahl."
        );
    }
    let (adapter, pool, guild_number) = st
        .public_source
        .as_ref()
        .context("Öffentliche Gateway-Quelle fehlt")?;
    let guild_number = *guild_number;
    let guild_id = guild_number.to_string();
    let voice = adapter
        .voice_cache_snapshot(guild_number)
        .context("Gateway-Momentaufnahme ist derzeit unbekannt")?;
    let (channels, raw, base) = {
        let guild = adapter
            .cache()
            .guild(GuildId::new(guild_number))
            .context("Guild-Cache fehlt")?;
        let base = guild
            .roles
            .get(&RoleId::new(guild_number))
            .context("everyone-Rolle fehlt")?
            .permissions
            .bits();
        let raw: Vec<Value> = guild
            .channels
            .values()
            .map(serde_json::to_value)
            .collect::<std::result::Result<_, _>>()?;
        (public_channels(&raw, &guild_id, base), raw, base)
    };
    let readable: Vec<Value> = channels
        .iter()
        .filter(|c| {
            raw.iter()
                .find(|raw| raw["id"] == c["id"])
                .and_then(|raw| everyone_permissions(raw, &guild_id, base))
                .is_some_and(|p| p & HISTORY != 0)
        })
        .map(|c| c["id"].clone())
        .collect();
    let channels_value = json!({"channels": channels, "readable": readable});
    let mut cache = st.public_cache.lock().await;
    if let Some(cached) = cache.as_ref().filter(|c| {
        c.guild == guild_id && c.channels == channels_value && c.created.elapsed() < TTL
    }) {
        return Ok(cached.facts.clone());
    }
    let mut voice_counts = Vec::new();
    for channel in &channels {
        if matches!(channel["type"].as_u64(), Some(2 | 13)) {
            let id = channel["id"]
                .as_str()
                .context("Kanal-ID fehlt")?
                .parse::<u64>()?;
            voice_counts.push(json!({"channel_id": channel["id"], "count": voice.members.values().filter(|c| **c == id).count()}));
        }
    }
    let bot_id = adapter
        .bot_user_id_cell()
        .get()
        .copied()
        .context("Gateway-Botidentität fehlt")?
        .to_string();
    let refs = sqlx::query("SELECT channel_id, message_id, message_key FROM server_config.desired_bot_messages WHERE guild_id = $1 AND message_kind = 'panel' AND message_id IS NOT NULL ORDER BY channel_id, message_key LIMIT 65")
        .bind(i64::try_from(guild_number)?).fetch_all(pool).await?;
    if refs.len() > 64 {
        bail!("Zu viele registrierte Bot-Infotexte für den begrenzten Live-Aufruf");
    }
    let mut references = BTreeSet::new();
    for reference in refs {
        let key: String = reference.try_get("message_key")?;
        if !info_key(&key) {
            continue;
        }
        let id = reference.try_get::<i64, _>("channel_id")?.to_string();
        if !channels.iter().any(|c| c["id"].as_str() == Some(&id)) {
            continue;
        }
        if !readable.iter().any(|c| c.as_str() == Some(&id)) {
            continue;
        }
        let message_id = reference.try_get::<i64, _>("message_id")?.to_string();
        references.insert((id, message_id));
    }
    for source in static_sources(guild_number) {
        let id = source.channel_id.to_string();
        if !readable.iter().any(|c| c.as_str() == Some(&id)) {
            continue;
        }
        if dl_central_db::kv::get(pool, "serversync", &source.format_key)
            .await?
            .as_deref()
            != Some(source.format)
        {
            continue;
        }
        let stored = sqlx::query("SELECT k, v FROM bot.kv_store WHERE ns = 'serversync' AND starts_with(k, $1) ORDER BY k LIMIT 65")
            .bind(&source.prefix).fetch_all(pool).await?;
        if stored.len() > 64 {
            bail!("Zu viele publizierte statische Bot-Infotexte");
        }
        for row in stored {
            if let Some(message_id) = source.message_id(row.try_get("k")?, row.try_get("v")?) {
                references.insert((id.clone(), message_id));
            }
        }
    }
    if references.len() > 64 {
        bail!("Zu viele zugelassene Bot-Infotexte für den begrenzten Live-Aufruf");
    }
    let mut infos = Vec::new();
    for (id, message_id) in references {
        let message = discord_call(
            st,
            "GET",
            &format!("/channels/{id}/messages/{message_id}"),
            &[],
            None,
            None,
        )
        .await?;
        if let Some(text) = registered_info(&message, &bot_id, &id, &message_id) {
            infos.push(json!({"channel_id": id, "message_id": message_id, "text": text}));
        }
    }
    let result = json!({"schema": "discord.public-facts.v1", "guild_id": guild_id, "observed_at": chrono::Utc::now().to_rfc3339(), "cache_seconds": 60, "audience": "everyone", "channels": channels, "voice_counts": voice_counts, "bot_infos": infos});
    *cache = Some(CachedFacts {
        guild: guild_id,
        channels: channels_value,
        created: Instant::now(),
        facts: result.clone(),
    });
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
        assert_eq!(
            result
                .iter()
                .map(|c| c["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["10", "11"]
        );
        assert!(public_channels(&channels, "1", ADMIN | VIEW).is_empty());
    }

    #[test]
    fn nur_eigener_registrierter_infotext_ohne_nutzerdaten() {
        let mut message = json!({"author":{"id":"2","bot":true,"username":"geheim"},"type":0,"mentions":[],"components":[{"type":17,"components":[{"type":10,"content":"So erstellst du eine Lane."}]}]});
        assert_eq!(
            own_info(&message, "2").as_deref(),
            Some("So erstellst du eine Lane.")
        );
        assert!(own_info(&message, "3").is_none());
        message["mentions"] = json!([{"id":"4","username":"privat"}]);
        assert!(own_info(&message, "2").is_none());
        assert!(!info_key("welcome:team"));
        assert!(!info_key("ticket"));
    }

    #[test]
    fn statische_kv_ids_sind_an_publisher_guild_und_kanal_gebunden() {
        let sources = static_sources(crate::serversync::GUILD_ID);
        assert_eq!(sources.len(), 5);
        assert!(static_sources(1).is_empty());
        let source = sources
            .iter()
            .find(|s| s.prefix == "regelwerk_v2_message_id_")
            .unwrap();
        assert_eq!(
            source.channel_id,
            crate::serversync::regelwerk_publish::REGELWERK_CHANNEL_ID
        );
        assert_eq!(
            source.message_id("regelwerk_v2_message_id_0", "12"),
            Some("12".into())
        );
        for key in [
            "regelwerk_v2_message_id_antwort",
            "regelwerk_v2_message_id_0_user",
            "welcome_message_id_0",
            "regelwerk_v2_message_id_01",
        ] {
            assert_eq!(source.message_id(key, "12"), None);
        }
        assert_eq!(source.message_id("regelwerk_v2_message_id_0", "0"), None);
        assert_eq!(
            source.message_id("regelwerk_v2_message_id_0", "Nutzertext"),
            None
        );
        let mut message = json!({"id":"12","channel_id":source.channel_id.to_string(),"author":{"id":"2","bot":true},"type":0,"mentions":[],"content":"Öffentliches Regelwerk"});
        assert!(registered_info(&message, "2", &source.channel_id.to_string(), "12").is_some());
        assert!(registered_info(&message, "2", "99", "12").is_none());
        assert!(registered_info(&message, "2", &source.channel_id.to_string(), "13").is_none());
        message["message_reference"] = json!({"message_id":"privat"});
        assert!(registered_info(&message, "2", &source.channel_id.to_string(), "12").is_none());
    }
}
