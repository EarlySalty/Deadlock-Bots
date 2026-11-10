use super::access::{effective_permissions, Access, HISTORY, VIEW};
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

const SEND: u64 = 1 << 11;
const TTL: Duration = Duration::from_secs(60);
const MAX_TERMS: usize = 16;
const MAX_TERM_CHARS: usize = 64;
const MIN_MATCH_CHARS: usize = 4;
const MAX_MATCHED_CHANNELS: usize = 4;

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
    #[serde(default)]
    channel_terms: Vec<String>,
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

pub(super) struct CachedFacts {
    guild: String,
    channels: Value,
    created: Instant,
    facts: Facts,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum Order {
    #[default]
    Newest,
    Oldest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    channel_id: u64,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    order: Order,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    channel_id: u64,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    after: Option<Value>,
    #[serde(default)]
    before: Option<Value>,
    #[serde(default)]
    order: Order,
    #[serde(default)]
    limit: Option<u64>,
}

#[derive(Serialize)]
struct FoundMessage {
    id: String,
    timestamp: String,
    author: String,
    text: String,
    bot: bool,
}

#[derive(Serialize)]
struct ReadOutput {
    channel_id: String,
    messages: Vec<FoundMessage>,
}

#[derive(Serialize)]
struct SearchOutput {
    channel_id: String,
    order: Order,
    messages: Vec<FoundMessage>,
    scanned: usize,
    truncated: bool,
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

pub(super) fn check_arguments(tool: &str, args: &Value) -> Result<()> {
    match tool {
        "read_messages" => {
            let args: ReadArgs = serde_json::from_value(args.clone())?;
            message_limit(args.limit)?;
        }
        "search_messages" => {
            let args: SearchArgs = serde_json::from_value(args.clone())?;
            SearchFilter::new(&args)?;
        }
        _ => {}
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
            let limit = message_limit(args.limit)?;
            channel_permission(st, access, args.channel_id, VIEW | HISTORY).await?;
            let mut query = vec![("limit".to_owned(), limit.to_string())];
            if args.order == Order::Oldest {
                query.push(("after".into(), "0".into()));
            }
            let raw = discord_call(
                st,
                "GET",
                &format!("/channels/{}/messages", args.channel_id),
                &query,
                None,
                None,
            )
            .await?;
            let mut page = raw
                .as_array()
                .context("Nachrichtenformat ist unbekannt")?
                .clone();
            sort_page(&mut page, args.order)?;
            let mut messages = Vec::new();
            for message in &page {
                if let Some(found) = found_message(st, message)? {
                    messages.push(found);
                }
            }
            Ok(serde_json::to_value(ReadOutput {
                channel_id: args.channel_id.to_string(),
                messages,
            })?)
        }
        "search_messages" => {
            let args: SearchArgs = serde_json::from_value(args.clone())?;
            let filter = SearchFilter::new(&args)?;
            channel_permission(st, access, args.channel_id, VIEW | HISTORY).await?;
            Ok(serde_json::to_value(
                search_messages(st, args.channel_id, &filter).await?,
            )?)
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

const MAX_LIMIT: u64 = 50;
const DEFAULT_LIMIT: u64 = 20;
const SEARCH_PAGE: usize = 50;
const SEARCH_MAX_PAGES: usize = 10;
const SEARCH_MAX_TEXT: usize = 200;
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

fn message_limit(limit: Option<u64>) -> Result<u64> {
    match limit.unwrap_or(DEFAULT_LIMIT) {
        value @ 1..=MAX_LIMIT => Ok(value),
        _ => bail!("Nachrichtenlimit ist ungültig"),
    }
}

fn bound_snowflake(value: &Value) -> Result<u64> {
    let text = match value {
        Value::Number(number) => return number.as_u64().context("Zeitgrenze ist ungültig"),
        Value::String(text) => text.trim(),
        _ => bail!("Zeitgrenze ist ungültig"),
    };
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        return text.parse::<u64>().context("Zeitgrenze ist ungültig");
    }
    let millis = bound_millis(text)?;
    let millis = u64::try_from(millis).context("Zeitgrenze ist ungültig")?;
    let elapsed = millis.saturating_sub(DISCORD_EPOCH_MS);
    if elapsed > u64::MAX >> 22 {
        bail!("Zeitgrenze liegt zu weit in der Zukunft");
    }
    Ok(elapsed << 22)
}

fn bound_millis(text: &str) -> Result<i64> {
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(text) {
        return Ok(time.timestamp_millis());
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(time) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return Ok(time.and_utc().timestamp_millis());
        }
    }
    Ok(chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .context("Zeitgrenze ist ungültig")?
        .and_hms_opt(0, 0, 0)
        .context("Zeitgrenze ist ungültig")?
        .and_utc()
        .timestamp_millis())
}

fn message_id(message: &Value) -> Result<u64> {
    message["id"]
        .as_str()
        .context("Nachrichten-ID fehlt")?
        .parse::<u64>()
        .context("Nachrichten-ID ist ungültig")
}

fn sort_page(page: &mut [Value], order: Order) -> Result<()> {
    for message in page.iter() {
        message_id(message)?;
    }
    page.sort_by_key(|message| message_id(message).unwrap_or_default());
    if order == Order::Newest {
        page.reverse();
    }
    Ok(())
}

fn message_timestamp(message: &Value, id: u64) -> Result<String> {
    if let Some(time) = message["timestamp"]
        .as_str()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
    {
        return Ok(time.with_timezone(&chrono::Utc).to_rfc3339());
    }
    let millis =
        i64::try_from((id >> 22) + DISCORD_EPOCH_MS).context("Nachrichtenzeit ist ungültig")?;
    Ok(chrono::DateTime::from_timestamp_millis(millis)
        .context("Nachrichtenzeit ist ungültig")?
        .to_rfc3339())
}

fn cached_nick(st: &McpState, user: &str) -> Option<String> {
    let (adapter, _, guild) = st.public_source.as_ref()?;
    let user = serenity::all::UserId::new(user.parse::<u64>().ok().filter(|id| *id != 0)?);
    let guild = adapter.cache().guild(GuildId::new(*guild))?;
    guild.members.get(&user)?.nick.clone()
}

fn author_names(message: &Value, nick: Option<String>) -> Vec<String> {
    let author = &message["author"];
    [
        message["member"]["nick"].as_str().map(str::to_owned),
        nick,
        author["global_name"].as_str().map(str::to_owned),
        author["username"].as_str().map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    .map(|name| without_mentions(&name))
    .filter(|name| !name.is_empty())
    .collect()
}

fn message_names(st: &McpState, message: &Value) -> Vec<String> {
    let nick = message["author"]["id"]
        .as_str()
        .and_then(|user| cached_nick(st, user));
    author_names(message, nick)
}

fn found_with_names(message: &Value, names: &[String]) -> Result<Option<FoundMessage>> {
    let text = without_mentions(
        message["content"]
            .as_str()
            .context("Nachrichtentext fehlt")?,
    );
    if text.is_empty() {
        return Ok(None);
    }
    let id = message_id(message)?;
    Ok(Some(FoundMessage {
        id: id.to_string(),
        timestamp: message_timestamp(message, id)?,
        author: names
            .first()
            .cloned()
            .unwrap_or_else(|| "Unbekannt".to_owned()),
        text,
        bot: message["author"]["bot"].as_bool() == Some(true),
    }))
}

fn found_message(st: &McpState, message: &Value) -> Result<Option<FoundMessage>> {
    found_with_names(message, &message_names(st, message))
}

struct SearchFilter {
    query: Option<String>,
    author: Option<String>,
    after: Option<u64>,
    before: Option<u64>,
    order: Order,
    limit: usize,
}

fn search_text(value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.chars().count() > SEARCH_MAX_TEXT => bail!("Suchtext ist zu lang"),
        Some(value) => Ok(Some(value.to_lowercase())),
        None => Ok(None),
    }
}

impl SearchFilter {
    fn new(args: &SearchArgs) -> Result<Self> {
        let after = args.after.as_ref().map(bound_snowflake).transpose()?;
        let before = args.before.as_ref().map(bound_snowflake).transpose()?;
        if let (Some(after), Some(before)) = (after, before) {
            if after >= before {
                bail!("Zeitraum ist leer");
            }
        }
        Ok(Self {
            query: search_text(args.query.as_deref())?,
            author: search_text(args.author.as_deref())?,
            after,
            before,
            order: args.order,
            limit: usize::try_from(message_limit(args.limit)?)?,
        })
    }

    fn in_range(&self, id: u64) -> bool {
        self.after.is_none_or(|after| id > after) && self.before.is_none_or(|before| id < before)
    }

    fn advances(&self, cursor: Option<u64>, id: u64) -> bool {
        match (self.order, cursor) {
            (_, None) => true,
            (Order::Newest, Some(cursor)) => id < cursor,
            (Order::Oldest, Some(cursor)) => id > cursor,
        }
    }

    fn text_matches(&self, text: &str) -> bool {
        let Some(query) = &self.query else {
            return true;
        };
        let text = text.to_lowercase();
        text.contains(query.as_str()) || query.split_whitespace().all(|word| text.contains(word))
    }

    fn author_matches(&self, names: &[String]) -> bool {
        let Some(author) = &self.author else {
            return true;
        };
        names
            .iter()
            .any(|name| name.to_lowercase().contains(author.as_str()))
    }

    fn page_query(&self, cursor: Option<u64>, size: usize) -> Vec<(String, String)> {
        let mut query = vec![("limit".to_owned(), size.to_string())];
        let key = match self.order {
            Order::Newest => "before",
            Order::Oldest => "after",
        };
        if let Some(cursor) = cursor {
            query.push((key.to_owned(), cursor.to_string()));
        }
        query
    }

    fn matched(&self, st: &McpState, message: &Value) -> Result<Option<FoundMessage>> {
        let names = message_names(st, message);
        if !self.author_matches(&names) {
            return Ok(None);
        }
        Ok(found_with_names(message, &names)?.filter(|found| self.text_matches(&found.text)))
    }
}

async fn search_messages(
    st: &McpState,
    channel_id: u64,
    filter: &SearchFilter,
) -> Result<SearchOutput> {
    let mut cursor = match filter.order {
        Order::Newest => filter.before,
        Order::Oldest => Some(filter.after.unwrap_or(0)),
    };
    let mut messages = Vec::new();
    let mut scanned = 0;
    let mut truncated = true;
    'pages: for round in 0..SEARCH_MAX_PAGES {
        let raw = match discord_call(
            st,
            "GET",
            &format!("/channels/{channel_id}/messages"),
            &filter.page_query(cursor, SEARCH_PAGE),
            None,
            None,
        )
        .await
        {
            Ok(raw) => raw,
            Err(error) if round == 0 => return Err(error),
            Err(_) => break,
        };
        let mut page = raw
            .as_array()
            .context("Nachrichtenformat ist unbekannt")?
            .clone();
        sort_page(&mut page, filter.order)?;
        let last_page = page.len() < SEARCH_PAGE;
        for (index, message) in page.iter().enumerate() {
            let id = message_id(message)?;
            if !filter.advances(cursor, id) {
                break 'pages;
            }
            if !filter.in_range(id) {
                truncated = false;
                break 'pages;
            }
            scanned += 1;
            cursor = Some(id);
            if let Some(found) = filter.matched(st, message)? {
                messages.push(found);
                if messages.len() >= filter.limit {
                    truncated = !(last_page && index + 1 == page.len());
                    break 'pages;
                }
            }
        }
        if last_page {
            truncated = false;
            break;
        }
    }
    Ok(SearchOutput {
        channel_id: channel_id.to_string(),
        order: filter.order,
        messages,
        scanned,
        truncated,
    })
}

fn without_mentions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let body = tail.trim_start_matches(['!', '&']);
        let digits = body.len() - body.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && body[digits..].starts_with('>') {
            rest = &body[digits + 1..];
        } else {
            out.push_str("<@");
            rest = tail;
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn name_tokens(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= MIN_MATCH_CHARS)
        .map(str::to_lowercase)
        .collect()
}

fn normalized_terms(terms: &[String]) -> Result<Vec<String>> {
    if terms.len() > MAX_TERMS {
        bail!("Zu viele Kanalbegriffe");
    }
    let mut result = Vec::new();
    for term in terms {
        if term.chars().count() > MAX_TERM_CHARS {
            bail!("Kanalbegriff ist zu lang");
        }
        result.extend(name_tokens(term));
    }
    Ok(result)
}

fn term_matches(term: &str, token: &str) -> bool {
    term.starts_with(token) || token.starts_with(term)
}

fn matching_channels(raw: &[Value], readable: &[Value], terms: &[String]) -> Vec<u64> {
    let parent_name = |channel: &Value| {
        raw.iter()
            .find(|parent| !channel["parent_id"].is_null() && parent["id"] == channel["parent_id"])
            .and_then(|parent| parent["name"].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let mut matched: Vec<(u64, usize, u64)> = raw
        .iter()
        .filter(|c| matches!(c["type"].as_u64(), Some(0 | 5)) && readable.contains(&c["id"]))
        .filter_map(|c| {
            let id = c["id"].as_str()?.parse::<u64>().ok()?;
            let mut tokens = name_tokens(c["name"].as_str().unwrap_or_default());
            tokens.extend(name_tokens(&parent_name(c)));
            let score = terms
                .iter()
                .filter(|term| tokens.iter().any(|token| term_matches(term, token)))
                .count();
            let recency = c["last_message_id"]
                .as_str()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            (score > 0).then_some((recency, score, id))
        })
        .collect();
    matched.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    matched
        .into_iter()
        .take(MAX_MATCHED_CHANNELS)
        .map(|(_, _, id)| id)
        .collect()
}

fn requested_channels(
    channel_id: Option<u64>,
    access: &Access,
    raw: &[Value],
    readable: &[Value],
    terms: &[String],
) -> Vec<u64> {
    let mut ids: Vec<u64> = channel_id
        .filter(|id| {
            access.user_id.is_none()
                && raw.iter().any(|c| {
                    c["id"].as_str() == Some(id.to_string().as_str())
                        && matches!(c["type"].as_u64(), Some(0 | 5))
                        && readable.contains(&c["id"])
                })
        })
        .into_iter()
        .collect();
    ids.extend(
        matching_channels(raw, readable, terms)
            .into_iter()
            .filter(|id| channel_id != Some(*id)),
    );
    ids
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
    let access = super::access::access(st, None).await?;
    facts_for(st, args, &access).await
}

pub(super) async fn facts_for(st: &McpState, args: &Value, access: &Access) -> Result<Facts> {
    let arguments: FactsArgs = serde_json::from_value(args.clone())?;
    let terms = normalized_terms(&arguments.channel_terms)?;
    let personal = arguments.channel_id.is_some() || !arguments.channel_terms.is_empty();
    let mut messages: Vec<ReadResult> = Vec::new();
    if let Some(channel_id) = arguments.channel_id.filter(|_| access.user_id.is_some()) {
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
    for channel_id in requested_channels(arguments.channel_id, access, &raw, &readable, &terms) {
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
    let channels_value = json!({"channels": channels, "readable": readable});
    let mut cache = st.public_cache.lock().await;
    if let Some(cached) = cache.as_ref().filter(|c| {
        access.user_id.is_none()
            && !personal
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
    if access.user_id.is_none() && !personal {
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
    use crate::mcp::access::ADMIN;

    #[test]
    fn suchgrenzen_und_anzeigenamen_folgen_dem_vertrag() {
        assert_eq!(
            bound_snowflake(&json!("2015-01-01T00:00:01Z")).expect("ISO"),
            1000 << 22
        );
        assert_eq!(
            bound_snowflake(&json!("2015-01-02")).expect("Datum"),
            86_400_000 << 22
        );
        assert_eq!(
            bound_snowflake(&json!("4194304000")).expect("ID"),
            4_194_304_000
        );
        assert!(bound_snowflake(&json!("gestern")).is_err());
        assert!(message_limit(Some(0)).is_err());
        assert!(message_limit(Some(51)).is_err());
        assert_eq!(message_limit(None).expect("Standard"), 20);
        let message = json!({"id":"4194304000","content":"Hallo <@123> Welt","author":{"id":"77","username":"nutzer","global_name":"Globaler Name","bot":true}});
        let names = author_names(&message, Some("Spitzname".into()));
        let found = found_with_names(&message, &names)
            .expect("Treffer")
            .expect("Text");
        assert_eq!(found.author, "Spitzname");
        assert_eq!(found.text, "Hallo Welt");
        assert!(found.bot);
        assert_eq!(found.timestamp, "2015-01-01T00:00:01+00:00");
        let anonymous = json!({"id":"1","content":"x","author":{"id":"77"}});
        assert_eq!(
            found_with_names(&anonymous, &author_names(&anonymous, None))
                .expect("Treffer")
                .expect("Text")
                .author,
            "Unbekannt"
        );
        let filter = SearchFilter::new(&SearchArgs {
            channel_id: 1,
            query: Some("Welt hallo".into()),
            author: Some("GLOBAL".into()),
            after: Some(json!("2015-01-01")),
            before: None,
            order: Order::Oldest,
            limit: None,
        })
        .expect("Filter");
        assert!(filter.author_matches(&names));
        assert!(filter.text_matches(&found.text));
        assert!(!filter.text_matches("nur hallo"));
        assert!(filter.in_range(4_194_304_000));
        assert!(!filter.in_range(0));
        assert!(SearchFilter::new(&SearchArgs {
            channel_id: 1,
            query: None,
            author: None,
            after: Some(json!("2015-01-02")),
            before: Some(json!("2015-01-01")),
            order: Order::Newest,
            limit: None,
        })
        .is_err());
    }

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
                top_position: 0,
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
            top_position: 0,
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
    fn kanalsuche_trifft_nur_lesbare_kanaele_neueste_zuerst() {
        let raw = vec![
            json!({"id":"10","name":"Scrims","type":4}),
            json!({"id":"11","name":"scrim-channel","type":0,"parent_id":"10","last_message_id":"500"}),
            json!({"id":"12","name":"announcement-scrims","type":0,"parent_id":"10","last_message_id":"300"}),
            json!({"id":"13","name":"urn-runners","type":0,"parent_id":"10","last_message_id":"900"}),
            json!({"id":"14","name":"allgemein","type":0,"last_message_id":"999"}),
            json!({"id":"15","name":"Scrim Voice","type":2,"parent_id":"10","last_message_id":"950"}),
        ];
        let terms = normalized_terms(&["Was ist bei den Scrims von uns passiert".into()])
            .expect("Begriffe");
        let readable = vec![
            json!("11"),
            json!("12"),
            json!("13"),
            json!("14"),
            json!("15"),
        ];
        assert_eq!(matching_channels(&raw, &readable, &terms), [13, 11, 12]);
        let hidden = vec![json!("14")];
        assert!(matching_channels(&raw, &hidden, &terms).is_empty());
        assert!(normalized_terms(&vec!["a".to_owned(); MAX_TERMS + 1]).is_err());
    }

    #[test]
    fn anonyme_kanalsuche_liest_nur_oeffentlich_lesbare_kanaele() {
        let raw = vec![
            json!({"id":"20","name":"frag-die-community","type":0,"position":0,"last_message_id":"100","permission_overwrites":[]}),
            json!({"id":"21","name":"mod-community","type":0,"position":1,"last_message_id":"900","permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"},{"id":"2","type":0,"deny":"0","allow":VIEW.to_string()}]}),
            json!({"id":"22","name":"ticket-community","type":0,"position":2,"last_message_id":"800","permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"},{"id":"99","type":1,"deny":"0","allow":(VIEW | HISTORY).to_string()}]}),
            json!({"id":"23","name":"Team","type":4,"position":3,"permission_overwrites":[{"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"}]}),
            json!({"id":"24","name":"team-community","type":0,"parent_id":"23","position":4,"last_message_id":"700","permission_overwrites":[]}),
        ];
        let access = Access {
            user_id: None,
            guild: "1".into(),
            roles: vec!["3".into()],
            base: VIEW | HISTORY,
            timed_out: false,
            top_position: 0,
        };
        let readable: Vec<Value> = public_channels(&raw, &access)
            .iter()
            .filter(|c| {
                raw.iter()
                    .find(|r| r["id"] == c["id"])
                    .and_then(|r| effective_permissions(r, &access))
                    .is_some_and(|p| p & HISTORY != 0)
            })
            .map(|c| c["id"].clone())
            .collect();
        let terms = normalized_terms(&["frag die community".into()]).expect("Begriffe");
        assert_eq!(
            requested_channels(None, &access, &raw, &readable, &terms),
            [20]
        );
        for private in [21, 22, 24] {
            assert_eq!(
                requested_channels(Some(private), &access, &raw, &readable, &[]),
                Vec::<u64>::new()
            );
        }
        assert_eq!(
            requested_channels(Some(20), &access, &raw, &readable, &[]),
            [20]
        );
        let requester = Access {
            user_id: Some(99),
            ..access
        };
        assert_eq!(
            requested_channels(Some(20), &requester, &raw, &readable, &terms),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn nachrichtentext_ohne_personen_und_rollenerwaehnungen() {
        assert_eq!(
            without_mentions("Match 1 <@&1520849762851618817> \n\nSieger <@!42> und <@7> <@x>"),
            "Match 1 Sieger und <@x>"
        );
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
