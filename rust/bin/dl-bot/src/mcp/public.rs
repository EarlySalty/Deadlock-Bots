use super::{discord_call, McpState};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serenity::all::GuildId;
use sqlx::Row;
use std::{
    collections::{BTreeSet, HashSet},
    time::{Duration, Instant},
};

const VIEW: u64 = 1 << 10;
const HISTORY: u64 = 1 << 16;
const ADMIN: u64 = 1 << 3;
const SEND: u64 = 1 << 11;
const TTL: Duration = Duration::from_secs(60);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Facts {
    schema: String,
    guild_id: String,
    observed_at: String,
    cache_seconds: u64,
    audience: String,
    channels: Vec<Channel>,
    voice_counts: Vec<VoiceCount>,
    bot_infos: Vec<BotInfo>,
    tempvoice: TempVoice,
    messages: Vec<ReadResult>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsArgs {
    #[serde(default)]
    channel_id: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Channel {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: u64,
    topic: Option<String>,
    position: i64,
    parent_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VoiceCount {
    channel_id: String,
    count: usize,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BotInfo {
    channel_id: String,
    message_id: String,
    text: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TempVoice {
    category_ids: Vec<String>,
    join_channel_ids: Vec<String>,
    open_lanes: Vec<OpenLane>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OpenLane {
    channel_id: String,
    count: usize,
    mode: String,
}

pub(super) struct Access {
    pub user_id: Option<u64>,
    guild: String,
    roles: Vec<String>,
    base: u64,
    timed_out: bool,
}

pub(super) async fn access(st: &McpState, requested_user: Option<u64>) -> Result<Access> {
    let (_, _, guild_id) = st.public_source.as_ref().context("Gateway-Quelle fehlt")?;
    let verified = st
        .public_policy
        .mcp_verified_role_id
        .filter(|id| *id != 0 && *id != *guild_id)
        .context("Mitgliederrolle ist nicht konfiguriert")?;
    let member = if let Some(user) = requested_user {
        discord_call(
            st,
            "GET",
            &format!("/guilds/{guild_id}/members/{user}"),
            &[],
            None,
            None,
        )
        .await
        .ok()
        .filter(|member| member["user"]["id"].as_str() == Some(user.to_string().as_str()))
    } else {
        None
    };
    let member_roles = member
        .as_ref()
        .and_then(|member| member["roles"].as_array())
        .and_then(|roles| {
            roles
                .iter()
                .map(|role| role.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        });
    let raw_roles = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}/roles"),
        &[],
        None,
        None,
    )
    .await?;
    let mut role_bits = std::collections::BTreeMap::new();
    for role in raw_roles.as_array().context("Rollenformat ist unbekannt")? {
        let id = role["id"].as_str().context("Rollen-ID fehlt")?.to_owned();
        let bits = role["permissions"]
            .as_str()
            .context("Rollenrechte fehlen")?
            .parse::<u64>()?;
        if role_bits.insert(id, bits).is_some() {
            bail!("Rollen-Zulassung ist uneindeutig");
        }
    }
    let everyone = *role_bits
        .get(&guild_id.to_string())
        .context("everyone-Rolle fehlt")?;
    let verified_bits = *role_bits
        .get(&verified.to_string())
        .context("Mitgliederrolle ist unbekannt")?;
    if verified_bits & ADMIN != 0 {
        bail!("Mitgliederrolle ist nicht eindeutig zulässig");
    }
    let user_id = member_roles.as_ref().and(requested_user);
    let timed_out = match member
        .as_ref()
        .filter(|_| user_id.is_some())
        .and_then(|member| member.get("communication_disabled_until"))
    {
        None | Some(Value::Null) => false,
        Some(until) => {
            chrono::DateTime::parse_from_rfc3339(
                until.as_str().context("Timeout-Zeitpunkt ist ungültig")?,
            )
            .context("Timeout-Zeitpunkt ist ungültig")?
                > chrono::Utc::now()
        }
    };
    let roles = member_roles.unwrap_or_else(|| vec![verified.to_string()]);
    let mut base = everyone;
    for role in &roles {
        role.parse::<u64>()?;
        base |= role_bits
            .get(role)
            .context("Mitgliedsrolle ist unbekannt")?;
    }
    Ok(Access {
        user_id,
        guild: guild_id.to_string(),
        roles,
        base,
        timed_out,
    })
}

fn effective_permissions(channel: &Value, access: &Access) -> Option<u64> {
    if access.base & ADMIN != 0 && access.user_id.is_some() {
        return Some(u64::MAX);
    }
    if access.base & ADMIN != 0 {
        return None;
    }
    let mut permissions = access.base;
    let mut role_deny = 0;
    let mut role_allow = 0;
    let mut member = None;
    let mut seen = HashSet::new();
    for overwrite in channel.get("permission_overwrites")?.as_array()? {
        let kind = overwrite.get("type")?.as_u64()?;
        let id = overwrite.get("id")?.as_str()?;
        if kind > 1 || !seen.insert((kind, id)) {
            return None;
        }
        let deny = overwrite.get("deny")?.as_str()?.parse::<u64>().ok()?;
        let allow = overwrite.get("allow")?.as_str()?.parse::<u64>().ok()?;
        if kind == 0 && id == access.guild {
            permissions = (permissions & !deny) | allow;
        } else if kind == 0 && access.roles.iter().any(|role| role == id) {
            role_deny |= deny;
            role_allow |= allow;
        } else if kind == 1 && access.user_id.is_some_and(|user| user.to_string() == id) {
            member = Some((deny, allow));
        }
    }
    permissions = (permissions & !role_deny) | role_allow;
    if let Some((deny, allow)) = member {
        permissions = (permissions & !deny) | allow;
    }
    if access.timed_out {
        permissions &= VIEW | HISTORY;
    }
    Some(permissions)
}

pub(super) struct CachedFacts {
    guild: String,
    channels: Value,
    created: Instant,
    facts: Facts,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    channel_id: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendArgs {
    channel_id: u64,
    content: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct ReadResult {
    channel_id: String,
    messages: Vec<Message>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Message {
    id: String,
    text: String,
}

#[derive(Serialize)]
struct SendResult {
    channel_id: String,
    message_id: String,
}

async fn channel_permission(
    st: &McpState,
    access: &Access,
    channel_id: u64,
    required: u64,
) -> Result<()> {
    let raw = discord_call(
        st,
        "GET",
        &format!("/channels/{channel_id}"),
        &[],
        None,
        None,
    )
    .await?;
    if raw["guild_id"].as_str() != Some(access.guild.as_str())
        || !matches!(raw["type"].as_u64(), Some(0 | 5))
    {
        bail!("Kanaltyp ist nicht freigegeben");
    }
    if !effective_permissions(&raw, access).is_some_and(|p| p & required == required) {
        bail!("Kanalrecht fehlt");
    }
    Ok(())
}

pub(super) async fn request_tool(
    st: &McpState,
    tool: &str,
    args: &Value,
    access: &Access,
) -> Result<Value> {
    match tool {
        "read_messages" => {
            let args: ReadArgs = serde_json::from_value(args.clone())?;
            channel_permission(st, access, args.channel_id, VIEW | HISTORY).await?;
            let raw = discord_call(
                st,
                "GET",
                &format!("/channels/{}/messages", args.channel_id),
                &[("limit".into(), "20".into())],
                None,
                None,
            )
            .await?;
            let mut messages = Vec::new();
            for message in raw.as_array().context("Nachrichtenformat ist unbekannt")? {
                messages.push(Message {
                    id: message["id"]
                        .as_str()
                        .context("Nachrichten-ID fehlt")?
                        .into(),
                    text: message["content"]
                        .as_str()
                        .context("Nachrichtentext fehlt")?
                        .into(),
                });
            }
            Ok(serde_json::to_value(ReadResult {
                channel_id: args.channel_id.to_string(),
                messages,
            })?)
        }
        "send_message" => {
            if access.user_id.is_none() {
                bail!("Schreiben benötigt eine bekannte Identität");
            }
            let args: SendArgs = serde_json::from_value(args.clone())?;
            if args.content.is_empty() || args.content.chars().count() > 2000 {
                bail!("Nachrichtenlänge ist ungültig");
            }
            channel_permission(st, access, args.channel_id, VIEW | SEND).await?;
            let raw = discord_call(
                st,
                "POST",
                &format!("/channels/{}/messages", args.channel_id),
                &[],
                Some(&json!({"content":args.content,"allowed_mentions":{"parse":[]}})),
                None,
            )
            .await?;
            Ok(serde_json::to_value(SendResult {
                channel_id: args.channel_id.to_string(),
                message_id: raw["id"].as_str().context("Nachrichten-ID fehlt")?.into(),
            })?)
        }
        _ => bail!("Werkzeug ist nicht freigegeben"),
    }
}

fn public_channels(channels: &[Value], access: &Access) -> Vec<Value> {
    let allowed: HashSet<&str> = channels
        .iter()
        .filter(|c| {
            effective_permissions(c, access).is_some_and(|p| p & VIEW != 0)
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

fn join_channel_ids(
    channels: &[Value],
    config: &dl_voice::tempvoice::TempVoiceConfig,
) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for channel in channels {
        let id = channel["id"]
            .as_str()
            .context("Kanal-ID fehlt")?
            .parse::<u64>()?;
        if config.staging_channels.contains(&id) || id == dl_voice::router::ROUTER_VC_ID {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
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

pub(super) async fn facts(st: &McpState, args: &Value) -> Result<Facts> {
    let access = access(st, None).await?;
    facts_for(st, args, &access).await
}

pub(super) async fn facts_for(st: &McpState, args: &Value, access: &Access) -> Result<Facts> {
    let arguments: FactsArgs = serde_json::from_value(args.clone())?;
    let mut messages: Vec<ReadResult> = Vec::new();
    if let Some(channel_id) = arguments.channel_id {
        messages.push(serde_json::from_value(
            request_tool(
                st,
                "read_messages",
                &json!({"channel_id":channel_id}),
                access,
            )
            .await?,
        )?);
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
    let (channels, raw) = {
        let guild = adapter
            .cache()
            .guild(GuildId::new(guild_number))
            .context("Guild-Cache fehlt")?;
        let raw: Vec<Value> = guild
            .channels
            .values()
            .map(serde_json::to_value)
            .collect::<std::result::Result<_, _>>()?;
        (public_channels(&raw, access), raw)
    };
    let readable: Vec<Value> = channels
        .iter()
        .filter(|c| {
            raw.iter()
                .find(|raw| raw["id"] == c["id"])
                .and_then(|raw| effective_permissions(raw, access))
                .is_some_and(|p| p & HISTORY != 0)
        })
        .map(|c| c["id"].clone())
        .collect();
    let channels_value = json!({"channels": channels, "readable": readable});
    let mut cache = st.public_cache.lock().await;
    if let Some(cached) = cache.as_ref().filter(|c| {
        access.user_id.is_none()
            && arguments.channel_id.is_none()
            && c.guild == guild_id
            && c.channels == channels_value
            && c.created.elapsed() < TTL
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
    let mut tempvoice = TempVoice::default();
    if let Some(engine) = &st.public_tempvoice {
        tempvoice.join_channel_ids = join_channel_ids(&channels, &engine.config)?;
        for channel in &channels {
            let id = channel["id"]
                .as_str()
                .context("Kanal-ID fehlt")?
                .parse::<u64>()?;
            if engine.config.tempvoice_categories.contains(&id) {
                tempvoice.category_ids.push(id.to_string());
            }
            if let Some(mode) = engine.lane_mode(id).await {
                let count = voice
                    .members
                    .values()
                    .filter(|channel| **channel == id)
                    .count();
                tempvoice.open_lanes.push(OpenLane {
                    channel_id: id.to_string(),
                    count,
                    mode: mode.to_owned(),
                });
            }
        }
    }
    let result: Facts = serde_json::from_value(
        json!({"schema": "discord.public-facts.v2", "guild_id": guild_id, "observed_at": chrono::Utc::now().to_rfc3339(), "cache_seconds": 60, "audience": if access.user_id.is_some() { "requester" } else { "verified_members" }, "channels": channels, "voice_counts": voice_counts, "bot_infos": infos, "tempvoice":tempvoice,"messages":messages}),
    )?;
    if access.user_id.is_none() && arguments.channel_id.is_none() {
        *cache = Some(CachedFacts {
            guild: guild_id,
            channels: channels_value,
            created: Instant::now(),
            facts: result.clone(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_und_staging_einstiege_verlangen_erlaubte_sicht() {
        let config = dl_voice::tempvoice::TempVoiceConfig::production();
        let staging = *config.staging_channels.iter().min().expect("Staging-ID");
        let router = dl_voice::router::ROUTER_VC_ID;
        let channels = vec![
            json!({"id":router.to_string(),"type":2,"position":0,"permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"},{"id":"3","type":0,"deny":"0","allow":VIEW.to_string()}]}),
            json!({"id":staging.to_string(),"type":2,"position":1,"permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"},{"id":"3","type":0,"deny":"0","allow":VIEW.to_string()}]}),
            json!({"id":"20","type":2,"position":2,"permission_overwrites":[{"id":"3","type":0,"deny":"0","allow":VIEW.to_string()}]}),
        ];
        for user_id in [None, Some(42)] {
            let access = Access {
                user_id,
                guild: "1".into(),
                roles: vec!["3".into()],
                base: HISTORY,
                timed_out: false,
            };
            assert_eq!(
                join_channel_ids(&public_channels(&channels, &access), &config)
                    .expect("Erlaubte Einstiege"),
                [router.to_string(), staging.to_string()]
            );
            assert!(join_channel_ids(
                &public_channels(
                    &channels,
                    &Access {
                        roles: Vec::new(),
                        ..access
                    }
                ),
                &config
            )
            .expect("Fehlende Sicht")
            .is_empty());
        }
    }

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
        let access = Access {
            user_id: None,
            guild: "1".into(),
            roles: vec!["3".into()],
            base: VIEW | HISTORY,
            timed_out: false,
        };
        let result = public_channels(&channels, &access);
        assert_eq!(
            result
                .iter()
                .map(|c| c["id"].as_str().expect("Kanal-ID"))
                .collect::<Vec<_>>(),
            ["10", "11"]
        );
        assert!(public_channels(
            &channels,
            &Access {
                base: ADMIN | VIEW,
                ..access
            }
        )
        .is_empty());
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
            .expect("Registrierte Regelwerk-Quelle");
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
