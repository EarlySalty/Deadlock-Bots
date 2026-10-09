//! Discord-Glue für dl-moderation (ModPort + aimod:*-Review-Buttons).

use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use dl_community::concierge::CONCIERGE_OWNER_TOPIC_PREFIX;
use dl_discord::{BridgeInteraction, BridgeReply, CommandSpec, DiscordAdapter, InteractionHandler};
use serde_json::{json, Map, Value};
use serenity::all::{
    ChannelId, CreateAttachment, GuildId, Http, Message, MessageId, PermissionOverwriteType,
    Permissions, ReactionType, RoleId, UserId,
};
use serenity::builder::GetMessages;
use serenity::http::HttpError;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

const INVITE_CACHE_TTL_SECONDS: i64 = 3600;
const INVITE_CACHE_MAX_ENTRIES: usize = 256;
const DISCORD_FIELD_LIMIT: usize = 1024;
const CASE_IMAGE_ATTACHMENT_EXTENSIONS: &[&str] =
    &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif"];
const MODERATION_EVIDENCE_IMAGE_LIMIT: usize = 10;
const MODERATION_EVIDENCE_IMAGE_MAX_BYTES: usize = 7_000_000;
const MODERATION_EVIDENCE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5);
const DISCORD_MESSAGE_SAFE_LIMIT: usize = 1800;
const AUTO_RAGEBAITER_TAG_SET_BY: u64 = 0;
const BRAIN_USAGE: &str = "🧠 Nutze `/brain frage:<deine Frage>`. Build-Fragen erzeugen einen markierten Review-Build für die In-Game-Prüfung.";
const BRAIN_DAILY_LIMIT: &str =
    "Deine Brain-Fragen für heute sind aufgebraucht. Morgen geht es weiter.";
const BRAIN_TOO_LONG: &str =
    "Das ist ja ein halber Roman 😅. Pack deine Frage in unter {max} Zeichen.";
#[allow(dead_code)]
const BRAIN_WORKING: &str = "🧠 Moment, ich wühl kurz im Brain…";
const BRAIN_THINKING_FRAMES: [&str; 3] = ["💭 .", "💭 . .", "💭 . . ."];
const BRAIN_THINKING_INTERVAL: Duration = Duration::from_millis(1200);
const BRAIN_THINKING_MAX_TICKS: usize = 40;
const BRAIN_CONVERSATION_TTL: Duration = Duration::from_secs(10 * 60);
const BRAIN_BACKEND_ERR: &str = "🧠 Mein Hirn hakt grad. Probier's in ein paar Sekunden nochmal.";
const BRAIN_PRIVATE_HELP: &str = "Private Fragen kann ich gerade nicht sicher beantworten. Für persönliche Hilfe öffne ein Ticket in <#1459628609705738539>. Allgemeine Fragen zum Server kannst du in <#1426220702054355077> stellen, ohne persönliche Angaben.";
const BRAIN_NO_ANSWER: &str =
    "🧠 Dazu find ich grad nichts Handfestes. Frag mal konkreter nach Held, Item oder Fähigkeit.";
const BRAIN_OUT_OF_DOMAIN: &str =
    "🧠 Klingt nicht nach Deadlock. Dazu hab ich keine gesicherten Infos. Frag mich was zum Spiel: Held, Item, Build oder Mechanik.";
const BRAIN_EMBED_FOOTER: &str = "Deadlock Brain";
const BRAIN_EMBED_COLOR: u32 = 0xE0A340;
const BRAIN_EMBED_TITLE_QUESTION_LIMIT: usize = 250;
const BRAIN_EMBED_DESCRIPTION_LIMIT: usize = 4096;
const BRAIN_EMBED_DESCRIPTION_TRUNCATE_AT: usize = 4080;

#[derive(Debug, Clone, Default)]
pub struct BrainEmojiIndex {
    entries: Vec<(String, String)>,
}

impl BrainEmojiIndex {
    #[cfg(test)]
    fn markup_for(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, markup)| markup.as_str())
    }

    #[cfg(test)]
    fn decorate_name(&self, name: &str) -> String {
        self.markup_for(name)
            .map(|markup| format!("{markup} {name}"))
            .unwrap_or_else(|| name.to_string())
    }

    fn annotate_inline(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (name, markup) in &self.entries {
            if name.chars().count() < 6 && !name.contains(' ') {
                continue;
            }
            let decorated = format!("{markup} {name}");
            if result.contains(&decorated) {
                continue;
            }
            result = result.replace(name, &decorated);
        }
        result
    }
}

#[cfg(test)]
#[derive(Debug, serde::Deserialize)]
struct BrainReviewBuildItem {
    name: String,
}

#[cfg(test)]
#[derive(Debug, serde::Deserialize)]
struct BrainReviewBuildSituation {
    label: String,
    items: Vec<BrainReviewBuildItem>,
}

#[cfg(test)]
#[derive(Debug, serde::Deserialize)]
struct BrainReviewBuildReceipt {
    task_id: i64,
    hero_build_id: Option<i64>,
    version: Option<i64>,
    hero_name: String,
    core: Vec<BrainReviewBuildItem>,
    situations: Vec<BrainReviewBuildSituation>,
}

pub struct LfgFreetextGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn lfg_freetext_question_body(message_id: u64, text: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("content".into(), json!(text));
    body.insert(
        "message_reference".into(),
        json!({ "message_id": message_id.to_string() }),
    );
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    body
}

#[async_trait::async_trait]
impl dl_activity::lfg_freetext::FreetextLfgPort for LfgFreetextGlue {
    async fn ask_start_window(
        &self,
        channel_id: u64,
        message_id: u64,
        text: &'static str,
    ) -> Result<(), String> {
        self.adapter
            .send_raw_public(channel_id, &lfg_freetext_question_body(message_id, text))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn private_channel_overwrites_are_owner_only(
    guild_id: u64,
    owner_id: u64,
    bot_id: u64,
    overwrites: &[serenity::all::PermissionOverwrite],
) -> bool {
    let mut everyone_denied = false;
    let mut owner_allowed = false;
    let mut bot_allowed = false;

    for overwrite in overwrites {
        match overwrite.kind {
            PermissionOverwriteType::Role(role_id) if role_id == RoleId::new(guild_id) => {
                if overwrite.allow.contains(Permissions::VIEW_CHANNEL) {
                    return false;
                }
                everyone_denied |= overwrite.deny.contains(Permissions::VIEW_CHANNEL);
            }
            PermissionOverwriteType::Role(_) => {
                if overwrite.allow.contains(Permissions::VIEW_CHANNEL) {
                    return false;
                }
            }
            PermissionOverwriteType::Member(member_id) => {
                let member_id = member_id.get();
                if member_id != owner_id
                    && member_id != bot_id
                    && overwrite.allow.contains(Permissions::VIEW_CHANNEL)
                {
                    return false;
                }
                let required = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
                if member_id == owner_id {
                    if overwrite.deny.intersects(required) {
                        return false;
                    }
                    owner_allowed |= overwrite.allow.contains(required);
                }
                if member_id == bot_id {
                    if overwrite.deny.intersects(required) {
                        return false;
                    }
                    bot_allowed |= overwrite.allow.contains(required);
                }
            }
            _ => {}
        }
    }

    everyone_denied && owner_allowed && bot_allowed
}

pub struct ModGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub tags: Arc<dl_community::tags::TagService>,
}

#[cfg(test)]
pub use dl_answer::game::CliRetriever as BrainRetrieverGlue;

#[cfg(test)]
fn format_review_build_receipt(
    receipt: &BrainReviewBuildReceipt,
    emoji_index: &BrainEmojiIndex,
) -> String {
    let published = receipt
        .hero_build_id
        .map(|id| format!("✅ Im Spiel veröffentlicht · Build-ID `{id}`"))
        .unwrap_or_else(|| format!("⏳ Veröffentlichung läuft · Task `{}`", receipt.task_id));
    let core = receipt
        .core
        .iter()
        .take(12)
        .map(|item| emoji_index.decorate_name(&item.name))
        .collect::<Vec<_>>()
        .join(" → ");
    let situations = receipt
        .situations
        .iter()
        .take(4)
        .map(|block| {
            let items = block
                .items
                .iter()
                .take(5)
                .map(|item| emoji_index.decorate_name(&item.name))
                .collect::<Vec<_>>()
                .join(", ");
            format!("**{}:** {}", block.label, items)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let version = receipt
        .version
        .map(|version| format!(" · v{version}"))
        .unwrap_or_default();
    format!(
        "🧪 **{} Review-Build**{version}\n{published}\n\n**Kern:** {core}\n{}",
        emoji_index.decorate_name(&receipt.hero_name),
        situations
    )
}

pub struct BrainHandler {
    pub adapter: Arc<DiscordAdapter>,
    pub config: Arc<dl_brain::BrainConfig>,
    pub cooldowns: Arc<dl_brain::BrainCooldowns>,
    pub answerer: Arc<dyn dl_brain::AiAnswerer>,
    pub channel_allowlist: Option<HashSet<u64>>,
    pub all_guild_channels: bool,
    pub emoji_index: Arc<BrainEmojiIndex>,
    pub guide_pending: tokio::sync::Mutex<Option<(u64, u64)>>,
    pub conversations: Mutex<BrainConversations>,
}

#[derive(Default)]
pub struct BrainConversations {
    entries: HashMap<(u64, u64, u64), BrainConversation>,
}

struct BrainConversation {
    turns: Vec<(u64, String)>,
    last_reply: Instant,
}

impl BrainConversations {
    fn prune(&mut self, now: Instant) {
        self.entries.retain(|_, conversation| {
            now.saturating_duration_since(conversation.last_reply) < BRAIN_CONVERSATION_TTL
        });
    }

    fn history(&mut self, event: &dl_discord::MessageEvent, now: Instant) -> Option<Vec<String>> {
        self.prune(now);
        let guild_id = event.guild_id?;
        let conversation = self
            .entries
            .get(&(guild_id, event.channel_id, event.author_id))?;
        if event
            .reply_channel_id
            .is_some_and(|channel| channel != event.channel_id)
        {
            return None;
        }
        let end = if event.is_reply
            || event.reply_message_id.is_some()
            || event.reply_channel_id.is_some()
        {
            let reply = event.reply_message_id?;
            conversation
                .turns
                .iter()
                .position(|(message_id, _)| *message_id == reply)?
                + 1
        } else {
            conversation.turns.len()
        };
        Some(
            conversation.turns[..end]
                .iter()
                .map(|(_, question)| question.clone())
                .collect(),
        )
    }

    fn question(&mut self, event: &dl_discord::MessageEvent, now: Instant) -> Option<String> {
        self.history(event, now)?;
        let question = event.content.trim();
        (!question.is_empty()).then(|| question.to_owned())
    }

    fn record(&mut self, event: &dl_discord::MessageEvent, message_id: u64, now: Instant) {
        self.prune(now);
        if message_id == 0 {
            return;
        }
        if let Some(guild_id) = event.guild_id {
            let key = (guild_id, event.channel_id, event.author_id);
            let history = self.history(event, now);
            let mut conversation = self.entries.remove(&key).unwrap_or(BrainConversation {
                turns: Vec::new(),
                last_reply: now,
            });
            if let Some(history) = history {
                conversation.turns.truncate(history.len());
            } else {
                conversation.turns.clear();
            }
            conversation
                .turns
                .push((message_id, event.content.trim().to_owned()));
            let mut characters: usize = conversation
                .turns
                .iter()
                .map(|(_, question)| question.chars().count())
                .sum();
            while conversation.turns.len() > 4 || characters > 4000 {
                characters -= conversation.turns.remove(0).1.chars().count();
            }
            conversation.last_reply = now;
            self.entries.insert(key, conversation);
        }
    }
}

#[async_trait::async_trait]
trait BrainDirectReplyPort: Send + Sync {
    async fn is_human(&self, event: &dl_discord::MessageEvent) -> bool;
    async fn can_reply(&self, event: &dl_discord::MessageEvent) -> bool;
    fn is_public(&self, event: &dl_discord::MessageEvent) -> bool;
    fn answer_context(
        &self,
        _event: &dl_discord::MessageEvent,
    ) -> Option<dl_brain::DiscordAnswerContext> {
        None
    }
    async fn reaction(
        &self,
        event: &dl_discord::MessageEvent,
        emoji: &str,
        present: bool,
    ) -> Result<(), String>;
    async fn reply(
        &self,
        event: &dl_discord::MessageEvent,
        body: &Map<String, Value>,
    ) -> Result<u64, String>;
}

#[async_trait::async_trait]
impl BrainDirectReplyPort for DiscordAdapter {
    async fn is_human(&self, event: &dl_discord::MessageEvent) -> bool {
        event.guild_id.is_some_and(|guild_id| {
            self.cache()
                .guild(GuildId::new(guild_id))
                .is_some_and(|guild| {
                    guild
                        .members
                        .get(&UserId::new(event.author_id))
                        .is_some_and(|member| !member.user.bot)
                })
        })
    }

    async fn can_reply(&self, event: &dl_discord::MessageEvent) -> bool {
        let Some(guild_id) = event.guild_id else {
            return true;
        };
        let Some(guild) = self.cache().guild(GuildId::new(guild_id)) else {
            return false;
        };
        let Some(channel) = guild.channels.get(&ChannelId::new(event.channel_id)) else {
            return false;
        };
        let Some(member) = guild.members.get(&UserId::new(event.author_id)) else {
            return false;
        };
        if member
            .communication_disabled_until
            .is_some_and(|until| until > serenity::all::Timestamp::now())
        {
            return false;
        }
        guild.user_permissions_in(channel, member).contains(
            Permissions::VIEW_CHANNEL
                | Permissions::READ_MESSAGE_HISTORY
                | Permissions::SEND_MESSAGES,
        )
    }

    fn answer_context(
        &self,
        event: &dl_discord::MessageEvent,
    ) -> Option<dl_brain::DiscordAnswerContext> {
        let mention = self.bot_user_id_cell().get().is_some_and(|id| {
            event.content.contains(&format!("<@{id}>"))
                || event.content.contains(&format!("<@!{id}>"))
        });
        Some(brain_answer_context(
            self,
            event.guild_id,
            event.channel_id,
            event.author_id,
            if mention {
                dl_brain::AnswerInputKind::Mention
            } else {
                dl_brain::AnswerInputKind::Message
            },
        ))
    }

    fn is_public(&self, event: &dl_discord::MessageEvent) -> bool {
        event
            .guild_id
            .is_some_and(|guild_id| brain_channel_is_public(self, guild_id, event.channel_id))
    }

    async fn reaction(
        &self,
        event: &dl_discord::MessageEvent,
        emoji: &str,
        present: bool,
    ) -> Result<(), String> {
        if present {
            dl_broker::DiscordPort::add_reaction(self, event.channel_id, event.message_id, emoji)
                .await
                .map_err(|err| err.to_string())
        } else {
            self.http
                .delete_reaction_me(
                    ChannelId::new(event.channel_id),
                    MessageId::new(event.message_id),
                    &ReactionType::Unicode(emoji.to_owned()),
                )
                .await
                .map_err(|err| err.to_string())
        }
    }

    async fn reply(
        &self,
        event: &dl_discord::MessageEvent,
        body: &Map<String, Value>,
    ) -> Result<u64, String> {
        self.send_raw_public(event.channel_id, body).await
    }
}

fn brain_answer_context(
    adapter: &DiscordAdapter,
    guild_id: Option<u64>,
    channel_id: u64,
    user_id: u64,
    input_kind: dl_brain::AnswerInputKind,
) -> dl_brain::DiscordAnswerContext {
    let mut context = dl_brain::DiscordAnswerContext {
        is_direct_message: Some(guild_id.is_none()),
        input_kind: Some(input_kind),
        ..Default::default()
    };
    let Some(guild_id) = guild_id else {
        context.is_thread = Some(false);
        return context;
    };
    let Some(guild) = adapter.cache().guild(GuildId::new(guild_id)) else {
        return context;
    };
    let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) else {
        return context;
    };
    let thread = matches!(
        channel.kind,
        serenity::all::ChannelType::PublicThread
            | serenity::all::ChannelType::PrivateThread
            | serenity::all::ChannelType::NewsThread
    );
    context.is_thread = Some(thread);
    let Some(member) = guild.members.get(&UserId::new(user_id)) else {
        return context;
    };
    if thread
        || !guild
            .user_permissions_in(channel, member)
            .contains(Permissions::VIEW_CHANNEL)
        || !brain_channel_is_public(adapter, guild_id, channel_id)
    {
        return context;
    }
    context.channel_name = Some(channel.name.clone());
    context.topic = channel.topic.clone().filter(|text| !text.trim().is_empty());
    if let Some(category) = channel.parent_id.and_then(|id| guild.channels.get(&id)) {
        if guild
            .user_permissions_in(category, member)
            .contains(Permissions::VIEW_CHANNEL)
        {
            context.category_name = Some(category.name.clone());
        }
    }
    context
}

fn brain_channel_is_public(adapter: &DiscordAdapter, guild_id: u64, channel_id: u64) -> bool {
    if guild_id == 0 || channel_id == 0 {
        return false;
    }
    let Some(guild) = adapter.cache().guild(GuildId::new(guild_id)) else {
        return false;
    };
    let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) else {
        return false;
    };
    if !matches!(
        channel.kind,
        serenity::all::ChannelType::Text | serenity::all::ChannelType::News
    ) {
        return false;
    }
    let Some(everyone) = guild.roles.get(&RoleId::new(guild_id)) else {
        return false;
    };
    let mut visible = everyone.permissions.contains(Permissions::VIEW_CHANNEL);
    for overwrite in &channel.permission_overwrites {
        if overwrite.kind == PermissionOverwriteType::Role(RoleId::new(guild_id)) {
            if overwrite.deny.contains(Permissions::VIEW_CHANNEL) {
                visible = false;
            }
            if overwrite.allow.contains(Permissions::VIEW_CHANNEL) {
                visible = true;
            }
        }
    }
    visible
}

fn direct_brain_question(event: &dl_discord::MessageEvent, bot_id: u64) -> Option<String> {
    if bot_id == 0
        || event.author_id == 0
        || event.author_id == bot_id
        || event.message_id == 0
        || event.channel_id == 0
    {
        return None;
    }
    let mentions = [format!("<@{bot_id}>"), format!("<@!{bot_id}>")];
    if event.guild_id.is_some()
        && !mentions
            .iter()
            .any(|mention| event.content.contains(mention))
    {
        return None;
    }
    let mut question = event.content.clone();
    for mention in mentions {
        question = question.replace(&mention, " ");
    }
    Some(question.trim().to_owned())
}

fn guide_channel(event: &dl_discord::MessageEvent) -> bool {
    event.guild_id == Some(1289721245281292288) && event.channel_id == 1426220702054355077
}

fn guide_question(event: &dl_discord::MessageEvent, bot_id: u64) -> bool {
    let age = chrono::Utc::now()
        .signed_duration_since(event.message_created_at)
        .num_seconds();
    let text = event.content.trim().to_lowercase();
    guide_channel(event)
        && bot_id != 0
        && event.author_id != 0
        && event.author_id != bot_id
        && event.message_id != 0
        && !event.is_reply
        && event.reply_message_id.is_none()
        && event.reply_channel_id.is_none()
        && (0..=60).contains(&age)
        && !text.is_empty()
        && !text.contains("<@")
        && !text.starts_with('>')
        && !["danke", "erledigt", "hat sich", "alles klar"]
            .iter()
            .any(|start| text.starts_with(start))
        && ((text.ends_with('?')
            && [
                "wie ", "wo ", "wer ", "was ", "warum ", "wann ", "kann ", "könnte ", "gibt ",
                "hat ", "ist ", "sind ", "welche ", "welcher ",
            ]
            .iter()
            .any(|start| text.starts_with(start)))
            || [
                "wie kann ich ",
                "wo finde ich ",
                "kann mir jemand ",
                "ich brauche hilfe",
            ]
            .iter()
            .any(|start| text.starts_with(start)))
}

fn direct_brain_reply_body(event: &dl_discord::MessageEvent, text: &str) -> Map<String, Value> {
    let mut body = if text.encode_utf16().count() <= dl_brain::DISCORD_MESSAGE_LIMIT {
        brain_public_message_body(text)
    } else {
        let mut body = brain_public_message_body("");
        body.insert("embeds".into(), json!([{"description": text}]));
        body
    };
    body.insert(
        "message_reference".into(),
        json!({
            "message_id": event.message_id.to_string(),
            "channel_id": event.channel_id.to_string(),
            "fail_if_not_exists": true,
        }),
    );
    body
}

impl BrainHandler {
    async fn conversation_question(
        &self,
        event: &dl_discord::MessageEvent,
        bot_id: u64,
    ) -> Option<String> {
        if let Some(question) = direct_brain_question(event, bot_id) {
            return Some(question);
        }
        if bot_id == 0
            || event.author_id == 0
            || event.author_id == bot_id
            || event.message_id == 0
            || event.channel_id == 0
            || guide_channel(event)
            || parse_brain_question(&event.content).is_some()
        {
            return None;
        }
        self.conversations
            .lock()
            .await
            .question(event, Instant::now())
    }

    fn channel_allowed(&self, channel_id: u64) -> bool {
        self.all_guild_channels
            || self
                .channel_allowlist
                .as_ref()
                .map(|allowlist| allowlist.contains(&channel_id))
                .unwrap_or(false)
    }

    async fn outcome_for_question(
        &self,
        question: &str,
        context: &dl_brain::DiscordQueryContext,
        channel_id: u64,
    ) -> Option<dl_brain::BrainOutcome> {
        dl_brain::handle_discord_query_with_context(
            question,
            channel_id,
            self.config.max_question_len,
            self.cooldowns.as_ref(),
            self.answerer.as_ref(),
            context,
        )
        .await
    }

    async fn reserved_outcome_for_question(
        &self,
        question: &str,
        context: &dl_brain::DiscordQueryContext,
    ) -> dl_brain::BrainOutcome {
        dl_brain::answer_discord_query_with_context(
            question,
            self.config.max_question_len,
            self.answerer.as_ref(),
            context,
        )
        .await
    }

    fn public_bodies_for_outcome(
        &self,
        question: &str,
        outcome: dl_brain::BrainOutcome,
    ) -> Vec<Map<String, Value>> {
        match outcome {
            dl_brain::BrainOutcome::Usage => vec![brain_public_message_body(BRAIN_USAGE)],
            dl_brain::BrainOutcome::TooLong { .. } => vec![brain_public_message_body(
                &BRAIN_TOO_LONG.replace("{max}", &self.config.max_question_len.to_string()),
            )],
            dl_brain::BrainOutcome::DailyLimit => {
                vec![brain_public_message_body(BRAIN_DAILY_LIMIT)]
            }
            dl_brain::BrainOutcome::Cooldown { .. } => Vec::new(),
            dl_brain::BrainOutcome::Answer(answer) => {
                match brain_answer_embed_body(question, &answer, self.emoji_index.as_ref()) {
                    Some(body) => vec![body],
                    None => vec![brain_public_message_body(BRAIN_NO_ANSWER)],
                }
            }
            dl_brain::BrainOutcome::OutOfDomain => {
                vec![brain_public_message_body(BRAIN_OUT_OF_DOMAIN)]
            }
            dl_brain::BrainOutcome::NoAnswer => vec![brain_public_message_body(BRAIN_NO_ANSWER)],
            dl_brain::BrainOutcome::BackendError => {
                vec![brain_public_message_body(BRAIN_BACKEND_ERR)]
            }
        }
    }

    fn public_body_for_outcome(
        &self,
        question: &str,
        outcome: dl_brain::BrainOutcome,
    ) -> Map<String, Value> {
        let mut bodies = self.public_bodies_for_outcome(question, outcome);
        bodies
            .pop()
            .unwrap_or_else(|| brain_public_message_body(BRAIN_NO_ANSWER))
    }

    async fn send_public_bodies(&self, channel_id: u64, bodies: &[Map<String, Value>]) {
        for body in bodies {
            if let Err(err) = self.adapter.send_raw_public(channel_id, body).await {
                tracing::warn!(%err, channel_id, "Brain-Antwort konnte nicht gesendet werden");
                break;
            }
        }
    }

    async fn handle_brain_question(
        &self,
        channel_id: u64,
        context: &dl_brain::DiscordQueryContext,
        question: &str,
    ) {
        match self
            .cooldowns
            .reserve_discord(context.user_id, channel_id)
            .await
        {
            dl_brain::DiscordReservation::Accepted => {}
            dl_brain::DiscordReservation::DailyLimit => {
                self.send_public_bodies(
                    channel_id,
                    &[brain_public_message_body(BRAIN_DAILY_LIMIT)],
                )
                .await;
                return;
            }
            dl_brain::DiscordReservation::Suppressed => return,
        }
        if question.trim().is_empty() {
            let bodies = vec![brain_public_message_body(BRAIN_USAGE)];
            self.send_public_bodies(channel_id, &bodies).await;
            return;
        }

        let placeholder = brain_public_message_body(thinking_frame(0));
        let message_id = match self.adapter.send_raw_public(channel_id, &placeholder).await {
            Ok(message_id) => message_id,
            Err(err) => {
                tracing::warn!(%err, channel_id, "Brain-Denk-Platzhalter konnte nicht gesendet werden");
                let outcome = self.reserved_outcome_for_question(question, context).await;
                let bodies = self.public_bodies_for_outcome(question, outcome);
                self.send_public_bodies(channel_id, &bodies).await;
                return;
            }
        };

        let cancelled = Arc::new(AtomicBool::new(false));
        let animation = spawn_brain_thinking_animation(
            self.adapter.clone(),
            channel_id,
            message_id,
            cancelled.clone(),
        );

        let outcome = self.reserved_outcome_for_question(question, context).await;
        cancelled.store(true, Ordering::SeqCst);
        if let Err(err) = animation.await {
            tracing::warn!(%err, channel_id, message_id, "Brain-Denk-Animation Task fehlgeschlagen");
        }

        let body = self.public_body_for_outcome(question, outcome);
        if let Err(err) = self
            .adapter
            .edit_raw_public(channel_id, message_id, &body)
            .await
        {
            tracing::warn!(%err, channel_id, message_id, "Brain-Antwort konnte nicht editiert werden");
            self.send_public_bodies(channel_id, &[body]).await;
        }
    }

    #[cfg(test)]
    pub async fn handle_message_event(&self, event: &dl_discord::MessageEvent) {
        let Some(bot_id) = self.adapter.bot_user_id_cell().get().copied() else {
            return;
        };
        self.handle_message_event_with_replies(event, bot_id, self.adapter.as_ref())
            .await;
    }

    #[cfg(test)]
    async fn handle_message_event_with_replies(
        &self,
        event: &dl_discord::MessageEvent,
        bot_id: u64,
        replies: &dyn BrainDirectReplyPort,
    ) {
        let Some(proactive) = self.prepare_guide_event(event, bot_id, replies).await else {
            return;
        };
        self.handle_prepared_message_event(event, bot_id, replies, proactive)
            .await;
    }

    async fn prepare_guide_event(
        &self,
        event: &dl_discord::MessageEvent,
        bot_id: u64,
        replies: &dyn BrainDirectReplyPort,
    ) -> Option<bool> {
        if event.author_id == bot_id {
            return None;
        }
        let proactive = if guide_channel(event) {
            if !replies.is_human(event).await {
                return None;
            }
            let proactive = guide_question(event, bot_id);
            *self.guide_pending.lock().await =
                proactive.then_some((event.author_id, event.message_id));
            proactive
        } else {
            false
        };
        Some(proactive)
    }

    async fn handle_prepared_message_event(
        &self,
        event: &dl_discord::MessageEvent,
        bot_id: u64,
        replies: &dyn BrainDirectReplyPort,
        proactive: bool,
    ) {
        if self
            .handle_direct_message_event(event, bot_id, replies)
            .await
        {
            return;
        }
        if proactive {
            self.answer_discord_event(event, replies, event.content.trim(), true)
                .await;
            return;
        }
        if event.guild_id.is_none() {
            return;
        }
        if !self.channel_allowed(event.channel_id) {
            return;
        }
        let Some(question) = parse_brain_question(&event.content) else {
            return;
        };
        if !replies.is_public(event) {
            self.answer_discord_event(event, replies, &question, false)
                .await;
            return;
        }
        let context = dl_brain::DiscordQueryContext {
            user_id: event.author_id,
            allow_discord_reads: true,
            answer_context: replies.answer_context(event),
        };
        self.handle_brain_question(event.channel_id, &context, &question)
            .await;
    }

    async fn handle_direct_message_event(
        &self,
        event: &dl_discord::MessageEvent,
        bot_id: u64,
        replies: &dyn BrainDirectReplyPort,
    ) -> bool {
        let Some(question) = self.conversation_question(event, bot_id).await else {
            return false;
        };
        self.answer_discord_event(event, replies, &question, false)
            .await;
        true
    }

    async fn answer_discord_event(
        &self,
        event: &dl_discord::MessageEvent,
        replies: &dyn BrainDirectReplyPort,
        question: &str,
        proactive: bool,
    ) {
        if proactive
            && *self.guide_pending.lock().await != Some((event.author_id, event.message_id))
        {
            return;
        }
        if !replies.can_reply(event).await {
            self.clear_guide_pending(event, proactive).await;
            return;
        }
        let context = dl_brain::DiscordQueryContext {
            user_id: event.author_id,
            allow_discord_reads: replies.is_public(event),
            answer_context: replies.answer_context(event),
        };
        let reservation = self
            .cooldowns
            .reserve_discord(context.user_id, event.channel_id)
            .await;
        if matches!(reservation, dl_brain::DiscordReservation::Suppressed) {
            self.clear_guide_pending(event, proactive).await;
            return;
        }
        if !proactive {
            let reacted = replies.reaction(event, "👀", true).await.is_ok();
            tracing::info!(
                message_id = event.message_id,
                channel_id = event.channel_id,
                reacted,
                "brain_direct_reaction_started"
            );
        }
        let outcome = match reservation {
            dl_brain::DiscordReservation::Accepted => {
                let history = if proactive {
                    Vec::new()
                } else {
                    self.conversations
                        .lock()
                        .await
                        .history(event, Instant::now())
                        .unwrap_or_default()
                };
                let history = dl_brain::bounded_user_questions(&history, question);
                dl_brain::answer_discord_query_with_history(
                    question,
                    self.config.max_question_len,
                    self.answerer.as_ref(),
                    &context,
                    &history,
                )
                .await
            }
            dl_brain::DiscordReservation::DailyLimit => dl_brain::BrainOutcome::DailyLimit,
            dl_brain::DiscordReservation::Suppressed => unreachable!(),
        };
        let mut failed = matches!(outcome, dl_brain::BrainOutcome::BackendError);
        let extend_conversation = !matches!(outcome, dl_brain::BrainOutcome::DailyLimit);
        let text = match outcome {
            dl_brain::BrainOutcome::DailyLimit => BRAIN_DAILY_LIMIT.to_owned(),
            dl_brain::BrainOutcome::Answer(answer) if !answer.trim().is_empty() => answer,
            dl_brain::BrainOutcome::NoAnswer
            | dl_brain::BrainOutcome::OutOfDomain
            | dl_brain::BrainOutcome::Answer(_) => BRAIN_NO_ANSWER.to_owned(),
            dl_brain::BrainOutcome::Usage => "Was möchtest du wissen?".to_owned(),
            dl_brain::BrainOutcome::TooLong { .. } => {
                BRAIN_TOO_LONG.replace("{max}", &self.config.max_question_len.to_string())
            }
            dl_brain::BrainOutcome::BackendError => BRAIN_BACKEND_ERR.to_owned(),
            dl_brain::BrainOutcome::Cooldown { .. } => {
                if !proactive {
                    let _ = replies.reaction(event, "👀", false).await;
                }
                self.clear_guide_pending(event, proactive).await;
                return;
            }
        };
        if replies.can_reply(event).await {
            let mut pending = if proactive {
                Some(self.guide_pending.lock().await)
            } else {
                None
            };
            if pending
                .as_ref()
                .is_some_and(|pending| **pending != Some((event.author_id, event.message_id)))
            {
                return;
            }
            let body = direct_brain_reply_body(event, &text);
            match replies.reply(event, &body).await {
                Ok(message_id) if extend_conversation && !proactive && !guide_channel(event) => {
                    self.conversations
                        .lock()
                        .await
                        .record(event, message_id, Instant::now());
                }
                Ok(_) => {}
                Err(err) => {
                    failed = true;
                    tracing::warn!(%err, "Discord-Brain-Antwort konnte nicht zugestellt werden");
                }
            }
            if let Some(pending) = pending.as_mut() {
                **pending = None;
            }
        } else {
            failed = true;
        }
        if !proactive {
            let _ = replies.reaction(event, "👀", false).await;
            if failed {
                let _ = replies.reaction(event, "❌", true).await;
            }
            tracing::info!(
                message_id = event.message_id,
                channel_id = event.channel_id,
                failed,
                "brain_direct_reaction_finished"
            );
        }
        self.clear_guide_pending(event, proactive).await;
    }

    async fn clear_guide_pending(&self, event: &dl_discord::MessageEvent, proactive: bool) {
        if proactive {
            let mut pending = self.guide_pending.lock().await;
            if *pending == Some((event.author_id, event.message_id)) {
                *pending = None;
            }
        }
    }
}

#[async_trait::async_trait]
impl InteractionHandler for BrainHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.guild_id == 0 {
            return BridgeReply::ephemeral_text("Der Brain-Test ist nur auf dem Server verfügbar.");
        }
        if !self.channel_allowed(interaction.channel_id) {
            return BridgeReply::ephemeral_text(
                "Der Brain-Test ist in diesem Kanal nicht freigeschaltet.",
            );
        }
        if !brain_channel_is_public(
            self.adapter.as_ref(),
            interaction.guild_id,
            interaction.channel_id,
        ) {
            return BridgeReply::ephemeral_text(BRAIN_PRIVATE_HELP);
        }
        let question = interaction
            .options
            .get("frage")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| parse_brain_question(&interaction.content))
            .unwrap_or_default();
        let context = dl_brain::DiscordQueryContext {
            user_id: interaction.user_id,
            allow_discord_reads: true,
            answer_context: Some(brain_answer_context(
                self.adapter.as_ref(),
                Some(interaction.guild_id),
                interaction.channel_id,
                interaction.user_id,
                dl_brain::AnswerInputKind::SlashCommand,
            )),
        };
        let Some(outcome) = self
            .outcome_for_question(&question, &context, interaction.channel_id)
            .await
        else {
            return BridgeReply::ephemeral_text(BRAIN_DAILY_LIMIT);
        };
        brain_bridge_reply_from_body(self.public_body_for_outcome(&question, outcome))
    }
}

fn brain_bridge_reply_from_body(mut body: Map<String, Value>) -> BridgeReply {
    let content = body
        .remove("content")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .filter(|value| !value.is_empty());
    let embeds = body
        .remove("embeds")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let allowed_mentions = body.remove("allowed_mentions");
    BridgeReply {
        content,
        embeds,
        allowed_mentions,
        ..BridgeReply::default()
    }
}

fn brain_public_message_body(message: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("content".into(), json!(message));
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    body
}

fn brain_answer_embed_body(
    question: &str,
    raw_answer: &str,
    emoji_index: &BrainEmojiIndex,
) -> Option<Map<String, Value>> {
    let cleaned = clean_brain_markdown(raw_answer);
    let description = truncate_brain_description(&emoji_index.annotate_inline(&cleaned));
    if description.trim().is_empty() {
        return None;
    }

    let embed = json!({
        "title": brain_embed_title(question),
        "description": description,
        "color": BRAIN_EMBED_COLOR,
        "footer": { "text": BRAIN_EMBED_FOOTER },
    });
    let mut body = Map::new();
    body.insert("content".into(), json!(""));
    body.insert("embeds".into(), json!([embed]));
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    Some(body)
}

fn spawn_brain_thinking_animation(
    adapter: Arc<DiscordAdapter>,
    channel_id: u64,
    message_id: u64,
    cancelled: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        for tick in 1..=BRAIN_THINKING_MAX_TICKS {
            sleep(BRAIN_THINKING_INTERVAL).await;
            if cancelled.load(Ordering::SeqCst) {
                break;
            }

            let body = brain_public_message_body(thinking_frame(tick));
            if let Err(err) = adapter.edit_raw_public(channel_id, message_id, &body).await {
                tracing::warn!(%err, channel_id, message_id, "Brain-Denk-Animation konnte nicht editiert werden");
                break;
            }
        }
    })
}

fn thinking_frame(tick: usize) -> &'static str {
    BRAIN_THINKING_FRAMES[tick % BRAIN_THINKING_FRAMES.len()]
}

fn brain_embed_title(question: &str) -> String {
    format!(
        "🧠 {}",
        truncate_brain_chars(question.trim(), BRAIN_EMBED_TITLE_QUESTION_LIMIT, "…")
    )
}

fn truncate_brain_description(description: &str) -> String {
    let description = description.trim();
    if description.chars().count() <= BRAIN_EMBED_DESCRIPTION_LIMIT {
        return description.to_string();
    }

    let mut truncated = description
        .chars()
        .take(BRAIN_EMBED_DESCRIPTION_TRUNCATE_AT)
        .collect::<String>();
    let trimmed_len = truncated.trim_end().len();
    truncated.truncate(trimmed_len);
    truncated.push_str(" …");
    truncated
}

fn truncate_brain_chars(value: &str, max_chars: usize, suffix: &str) -> String {
    if value.encode_utf16().count() <= max_chars {
        return value.to_string();
    }

    let suffix_len = suffix.encode_utf16().count();
    let mut remaining = max_chars.saturating_sub(suffix_len);
    let mut truncated: String = value
        .chars()
        .take_while(|ch| {
            let units = ch.len_utf16();
            if units > remaining {
                return false;
            }
            remaining -= units;
            true
        })
        .collect();
    truncated.push_str(suffix);
    truncated
}

fn clean_brain_markdown(input: &str) -> String {
    let mut lines = Vec::new();
    let mut blank_count = 0usize;

    for raw_line in input.lines() {
        let line = raw_line.trim_end();
        let line = brain_heading_as_bold(line).unwrap_or_else(|| line.to_string());
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                lines.push(String::new());
            }
        } else {
            blank_count = 0;
            lines.push(line);
        }
    }

    lines.join("\n").trim().to_string()
}

fn brain_heading_as_bold(line: &str) -> Option<String> {
    let line = line.trim_start();
    let heading = line.strip_prefix("### ")?.trim();
    if heading.is_empty() {
        Some(String::new())
    } else {
        Some(format!("**{heading}**"))
    }
}

fn parse_brain_question(content: &str) -> Option<String> {
    let trimmed = content.trim();
    let rest = trimmed.strip_prefix("!brain")?;
    if !rest.is_empty() {
        let first = rest.chars().next()?;
        if !first.is_whitespace() {
            return None;
        }
    }
    Some(rest.trim().to_string())
}

pub fn brain_command_spec(max_question_len: usize) -> CommandSpec {
    CommandSpec {
        definition: json!({
            "name": "brain",
            "description": "Deadlock Brain fragen und Review-Builds fürs Spiel erzeugen",
            "dm_permission": false,
            "options": [{
                "type": 3,
                "name": "frage",
                "description": "Was möchtest du wissen?",
                "required": true,
                "max_length": max_question_len.min(4000),
            }],
        }),
    }
}

#[cfg(test)]
pub fn parse_brain_channel_allowlist(raw: &str) -> Option<HashSet<u64>> {
    if raw.trim().is_empty() {
        return None;
    }
    raw.split([',', ';', '\n', '\r', '\t', ' '])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().ok().filter(|id| *id > 0))
        .collect::<Option<HashSet<_>>>()
        .filter(|ids| !ids.is_empty())
}

pub fn spawn_brain_command(
    handler: Arc<BrainHandler>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let replies = handler.adapter.clone();
    spawn_brain_command_with_replies(handler, dispatcher, replies)
}

fn spawn_brain_command_with_replies(
    handler: Arc<BrainHandler>,
    dispatcher: &dl_discord::Dispatcher,
    replies: Arc<dyn BrainDirectReplyPort>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        let mut answers = tokio::task::JoinSet::new();
        let mut guide_task: Option<((u64, u64), tokio::task::AbortHandle)> = None;
        loop {
            tokio::select! {
                result = answers.join_next(), if !answers.is_empty() => {
                    if let Some(Err(err)) = result {
                        if !err.is_cancelled() {
                            tracing::warn!(%err, "Discord-Brain-Antworttask fehlgeschlagen");
                        }
                    }
                }
                message = messages.recv() => match message {
                    Ok(event) => {
                        let Some(bot_id) = handler.adapter.bot_user_id_cell().get().copied() else {
                            continue;
                        };
                        let prepared = handler.prepare_guide_event(&event, bot_id, replies.as_ref()).await;
                        let pending = *handler.guide_pending.lock().await;
                        if guide_task.as_ref().is_some_and(|(key, _)| pending != Some(*key)) {
                            if let Some((_, task)) = guide_task.take() {
                                task.abort();
                            }
                        }
                        let Some(proactive) = prepared else {
                            continue;
                        };
                        if !proactive
                            && handler.conversation_question(&event, bot_id).await.is_none()
                            && !(handler.channel_allowed(event.channel_id) && parse_brain_question(&event.content).is_some())
                        {
                            continue;
                        }
                        while let Some(result) = answers.try_join_next() {
                            if let Err(err) = result {
                                if !err.is_cancelled() {
                                    tracing::warn!(%err, "Discord-Brain-Antworttask fehlgeschlagen");
                                }
                            }
                        }
                        if answers.len() >= 500 {
                            handler.clear_guide_pending(&event, proactive).await;
                            tracing::warn!("Discord-Brain-Aufgabenbudget erreicht");
                            continue;
                        }
                        let key = (event.author_id, event.message_id);
                        let handler = handler.clone();
                        let replies = replies.clone();
                        let task = answers.spawn(async move {
                            handler.handle_prepared_message_event(&event, bot_id, replies.as_ref(), proactive).await;
                        });
                        if proactive {
                            guide_task = Some((key, task));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        *handler.guide_pending.lock().await = None;
                        if let Some((_, task)) = guide_task.take() {
                            task.abort();
                        }
                        tracing::warn!(missed, "Brain-Command: Message-Events verpasst");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    })
}

#[async_trait::async_trait]
impl dl_moderation::ModPort for ModGlue {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn timeout_member(&self, guild_id: u64, user_id: u64, minutes: i64) -> bool {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": until.to_rfc3339() }),
                Some("AI-Moderation: Timeout"),
            )
            .await
            .is_ok()
    }

    async fn ban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .ban_user(
                GuildId::new(guild_id),
                UserId::new(user_id),
                1, // 1 Tag Nachrichten löschen (wie delete_message_days=1)
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn post_review(
        &self,
        case: &dl_moderation::store::CaseDraft,
        case_id: &str,
    ) -> Option<u64> {
        let preview: String = case.content.chars().take(900).collect();
        let mut embed = json!({
            "title": format!("🛡️ Moderationsvorschlag — {}", case.category),
            "description": format!(
                "**User:** <@{}> (`{}`)\n**Kanal:** <#{}>\n**Sicherheit:** {:.0}%\n**Begründung:** {}\n\n**Nachricht:**\n{}",
                case.user_id, case.user_tag, case.channel_id,
                case.confidence * 100.0, case.reason, preview
            ),
            "color": 0xE67E22,
        });
        apply_case_attachment_rendering(&mut embed, &case.attachments);
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 3, "label": "Annehmen (Löschen + Timeout)",
              "custom_id": format!("aimod:accept:{case_id}") },
            { "type": 2, "style": 4, "label": "Ban",
              "custom_id": format!("aimod:ban:{case_id}") },
            { "type": 2, "style": 2, "label": "Ablehnen",
              "custom_id": format!("aimod:deny:{case_id}") },
        ]}]);
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        self.adapter
            .send_raw_public(dl_moderation::MOD_REVIEW_CHANNEL_ID, &body)
            .await
            .ok()
    }

    async fn post_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &body)
            .await;
    }

    async fn post_case_log(
        &self,
        case: &dl_moderation::store::CaseDraft,
        case_id: &str,
        action: &str,
    ) -> Option<u64> {
        let mut embed = build_case_log_embed(case, case_id, action);
        apply_case_attachment_rendering(&mut embed, &case.attachments);
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        let log_message_id = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &body)
            .await
            .ok();

        let original = safe_message_text(&case.content, DISCORD_MESSAGE_SAFE_LIMIT);
        let mut content_message = format!(">>> {original}");
        if case.content.chars().count() > DISCORD_MESSAGE_SAFE_LIMIT {
            content_message.push_str("\nNachricht gekuerzt");
        }
        let mut original_body = serde_json::Map::new();
        original_body.insert("content".into(), json!(content_message));
        original_body.insert("allowed_mentions".into(), json!({ "parse": [] }));
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &original_body)
            .await;
        log_message_id
    }

    async fn send_dm(&self, user_id: u64, text: String) {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return;
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self.adapter.send_raw_public(channel.id.get(), &body).await;
    }

    async fn add_mod_tag(&self, user_id: u64, tag: &str, reason: &str) -> bool {
        self.tags
            .add_mod_tag(
                user_id,
                tag,
                AUTO_RAGEBAITER_TAG_SET_BY,
                Some(reason.to_string()),
                None,
            )
            .await
            .is_ok()
    }

    async fn fetch_context_lines(
        &self,
        guild_id: u64,
        channel_id: u64,
        before_message_id: u64,
        author_id: u64,
        message_created_at: i64,
        limit: usize,
    ) -> Vec<String> {
        let fetch_limit = limit.saturating_add(1).min(100) as u8;
        let Ok(mut messages) = ChannelId::new(channel_id)
            .messages(
                &self.adapter.http,
                GetMessages::new()
                    .before(MessageId::new(before_message_id))
                    .limit(fetch_limit),
            )
            .await
        else {
            return Vec::new();
        };
        messages.sort_by_key(|message| message.timestamp.unix_timestamp());

        let mut lines = Vec::new();
        for previous in messages {
            let mut preview = strip_mentions(&previous.content);
            if preview.is_empty() && !previous.attachments.is_empty() {
                preview = "[Anhang]".to_string();
            }
            if preview.is_empty() {
                continue;
            }

            let delta_s = message_created_at - previous.timestamp.unix_timestamp();
            let mins = (delta_s.max(0)) / 60;
            let time_tag = if mins < 60 {
                format!("[{mins}min ago]")
            } else {
                format!("[{}h ago]", mins / 60)
            };
            let prefix = if previous.author.id.get() == author_id {
                ">>>"
            } else {
                "   "
            };
            let display_name = self.display_name_for_message(guild_id, &previous);
            lines.push(format!(
                "{prefix} {time_tag} {display_name}: {}",
                truncate_chars(&preview, 150)
            ));
        }
        if lines.len() > limit {
            lines.split_off(lines.len() - limit)
        } else {
            lines
        }
    }

    async fn fetch_reply_context(
        &self,
        guild_id: u64,
        channel_id: u64,
        reply_channel_id: Option<u64>,
        reply_message_id: Option<u64>,
    ) -> Option<dl_moderation::ReplyContext> {
        let message_id = reply_message_id?;
        let channel_id = reply_channel_id.unwrap_or(channel_id);
        let message = ChannelId::new(channel_id)
            .message(&self.adapter.http, MessageId::new(message_id))
            .await
            .ok()?;
        let mut content = strip_mentions(&message.content);
        if content.is_empty() {
            content = if message.attachments.is_empty() {
                "[kein Text]".to_string()
            } else {
                "[Anhang]".to_string()
            };
        }
        Some(dl_moderation::ReplyContext {
            author: truncate_chars(&self.display_name_for_message(guild_id, &message), 60),
            content: truncate_chars(&content, 300),
        })
    }
}

#[async_trait::async_trait]
impl dl_moderation::moderation_system::ModerationPort for ModGlue {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn send_moderation_notice(&self, channel_id: u64, user_id: u64, text: &str) -> bool {
        let mut body = serde_json::Map::new();
        body.insert(
            "content".into(),
            Value::String(format!("<@{user_id}> {text}")),
        );
        body.insert(
            "allowed_mentions".into(),
            json!({
                "parse": [],
                "users": [user_id.to_string()],
            }),
        );
        self.adapter
            .http
            .send_message(ChannelId::new(channel_id), Vec::new(), &body)
            .await
            .is_ok()
    }

    async fn mirror_evidence_images(
        &self,
        image_urls: &[String],
    ) -> Vec<dl_moderation::moderation_system::ModerationEvidenceFile> {
        let client = reqwest::Client::new();
        let mut files = Vec::new();
        for url in image_urls.iter().take(MODERATION_EVIDENCE_IMAGE_LIMIT) {
            let Ok(Ok(response)) =
                timeout(MODERATION_EVIDENCE_DOWNLOAD_TIMEOUT, client.get(url).send()).await
            else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_string();
            if response
                .content_length()
                .is_some_and(|size| size > MODERATION_EVIDENCE_IMAGE_MAX_BYTES as u64)
            {
                continue;
            }
            let Ok(Ok(bytes)) =
                timeout(MODERATION_EVIDENCE_DOWNLOAD_TIMEOUT, response.bytes()).await
            else {
                continue;
            };
            if bytes.len() > MODERATION_EVIDENCE_IMAGE_MAX_BYTES {
                continue;
            }
            files.push(dl_moderation::moderation_system::ModerationEvidenceFile {
                filename: moderation_evidence_filename(files.len(), &content_type, url),
                data: bytes.to_vec(),
            });
        }
        files
    }

    async fn timeout_member(
        &self,
        guild_id: u64,
        user_id: u64,
        minutes: i64,
        reason: &str,
    ) -> bool {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": until.to_rfc3339() }),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn ban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .ban_user(
                GuildId::new(guild_id),
                UserId::new(user_id),
                1,
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn untimeout_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": null }),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn unban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .remove_ban(GuildId::new(guild_id), UserId::new(user_id), Some(reason))
            .await
            .is_ok()
    }

    async fn post_moderation_case(
        &self,
        channel_id: u64,
        embed: Value,
        components: Value,
        files: Vec<dl_moderation::moderation_system::ModerationEvidenceFile>,
    ) -> Option<u64> {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        let attachments = files
            .into_iter()
            .map(|file| CreateAttachment::bytes(file.data, file.filename))
            .collect::<Vec<_>>();
        self.adapter
            .http
            .send_message(ChannelId::new(channel_id), attachments, &body)
            .await
            .ok()
            .map(|message| message.id.get())
    }
}

fn moderation_evidence_filename(index: usize, content_type: &str, url: &str) -> String {
    let ext = if content_type.contains("png") {
        "png"
    } else if content_type.contains("gif") {
        "gif"
    } else if content_type.contains("webp") {
        "webp"
    } else if content_type.contains("avif") {
        "avif"
    } else {
        url.rsplit('?')
            .next()
            .and_then(|clean| clean.rsplit('.').next())
            .filter(|ext| matches!(*ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif"))
            .unwrap_or("jpg")
    };
    format!("moderation-evidence-{}.{}", index + 1, ext)
}

impl ModGlue {
    fn display_name_for_message(&self, guild_id: u64, message: &Message) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|guild| {
                guild
                    .members
                    .get(&message.author.id)
                    .map(|member| member.display_name().to_string())
            })
            .unwrap_or_else(|| message.author.name.to_string())
    }
}

/// Review-Buttons: aimod:accept|ban|deny:{case_id} (Mod-Guard via Rechte).
/// `deny` öffnet ein Modal (Pflicht-Grund) → Submit kommt als
/// `aimod:denysubmit:{case_id}` über dieselbe Prefix-Route zurück.
#[async_trait::async_trait]
trait AimodReviewActions: Send + Sync {
    async fn accept_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
    async fn ban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
    async fn deny_case(
        &self,
        case_id: &str,
        mod_id: u64,
        reason: &str,
    ) -> dl_moderation::ReviewOutcome;
    async fn untimeout_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
    async fn unban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
}

#[async_trait::async_trait]
impl AimodReviewActions for dl_moderation::ModerationSystem {
    async fn accept_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.accept_case(case_id, mod_id).await
    }

    async fn ban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.ban_case(case_id, mod_id).await
    }

    async fn deny_case(
        &self,
        case_id: &str,
        mod_id: u64,
        reason: &str,
    ) -> dl_moderation::ReviewOutcome {
        self.deny_case(case_id, mod_id, reason).await
    }

    async fn untimeout_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.untimeout_case(case_id, mod_id).await
    }

    async fn unban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.unban_case(case_id, mod_id).await
    }
}

pub struct ReviewHandler {
    moderator: Arc<dyn AimodReviewActions>,
}

impl ReviewHandler {
    pub fn new(moderator: Arc<dl_moderation::ModerationSystem>) -> Self {
        Self { moderator }
    }

    #[cfg(test)]
    fn with_actions(moderator: Arc<dyn AimodReviewActions>) -> Self {
        Self { moderator }
    }

    fn outcome_reply(outcome: dl_moderation::ReviewOutcome) -> BridgeReply {
        use dl_moderation::ReviewOutcome;
        match outcome {
            ReviewOutcome::NotFound => BridgeReply::ephemeral_text("Fall nicht gefunden."),
            ReviewOutcome::AlreadyHandled => {
                BridgeReply::ephemeral_text("Dieser Fall wurde bereits bearbeitet.")
            }
            ReviewOutcome::Done(text) => BridgeReply::ephemeral_text(text),
        }
    }
}

#[async_trait::async_trait]
impl InteractionHandler for ReviewHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let rest = interaction
            .custom_id
            .strip_prefix("aimod:")
            .unwrap_or_default();
        let Some((action, case_id)) = rest.split_once(':') else {
            return BridgeReply::ephemeral_text("Unbekannte Aktion.");
        };
        let authorized = match action {
            "accept" | "deny" | "denysubmit" | "untimeout" => {
                interaction.author_can_moderate_members
            }
            "ban" | "unban" => interaction.author_can_ban_members,
            _ => true,
        };
        if !authorized {
            return BridgeReply::ephemeral_text("Dir fehlt die Berechtigung für diese Aktion.");
        }
        match action {
            "accept" => {
                let outcome = self
                    .moderator
                    .accept_case(case_id, interaction.user_id)
                    .await;
                Self::outcome_reply(outcome)
            }
            "ban" => {
                let outcome = self.moderator.ban_case(case_id, interaction.user_id).await;
                Self::outcome_reply(outcome)
            }
            // Button: Modal mit Pflicht-Grund öffnen (Original: DenyReasonModal,
            // required, min 4 / max 500, mehrzeilig).
            "deny" => BridgeReply {
                modal: Some(dl_discord::ModalSpec {
                    custom_id: format!("aimod:denysubmit:{case_id}"),
                    title: "Fall verwerfen".to_string(),
                    fields: vec![dl_discord::ModalField {
                        custom_id: "reason".to_string(),
                        label: "Grund".to_string(),
                        placeholder: "Warum ist das kein Verstoß?".to_string(),
                        value: None,
                        required: true,
                        min_length: 4,
                        max_length: 500,
                        paragraph: true,
                    }],
                }),
                ..BridgeReply::default()
            },
            // Modal-Submit: Grund speichern + Log posten.
            "denysubmit" => {
                let reason = interaction
                    .options
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let outcome = self
                    .moderator
                    .deny_case(case_id, interaction.user_id, &reason)
                    .await;
                Self::outcome_reply(outcome)
            }
            "untimeout" => {
                let outcome = self
                    .moderator
                    .untimeout_case(case_id, interaction.user_id)
                    .await;
                Self::outcome_reply(outcome)
            }
            "unban" => {
                let outcome = self
                    .moderator
                    .unban_case(case_id, interaction.user_id)
                    .await;
                Self::outcome_reply(outcome)
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

// ── Moderation-Hilfen ──────────────────────────────────────────────────────

fn normalize_text(value: &str) -> String {
    value
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_discord_mention(candidate: &str) -> bool {
    let Some(first) = candidate.chars().next() else {
        return false;
    };
    if first != '@' && first != '#' {
        return false;
    }
    let rest = &candidate[first.len_utf8()..];
    let rest = rest
        .strip_prefix('!')
        .or_else(|| rest.strip_prefix('&'))
        .unwrap_or(rest);
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())
}

fn strip_mentions(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('>') else {
            out.push_str(&rest[open..]);
            return normalize_text(&out);
        };
        let candidate = &after_open[..close];
        if !is_discord_mention(candidate) {
            out.push('<');
            out.push_str(candidate);
            out.push('>');
        }
        rest = &after_open[close + 1..];
    }
    out.push_str(rest);
    normalize_text(&out)
}

fn category_label(category: &str) -> &'static str {
    match category {
        "nsfw_explicit" => "NSFW",
        "csam" => "CSAM",
        "raping" => "Raping",
        "epstein_child" => "Epstein/Child",
        "racism" => "Racism",
        "harassment" => "Harassment",
        "hate_speech" => "Hate Speech",
        "ragebait_ok" => "Ragebait OK",
        "game_related_ok" => "Game Related OK",
        "scam" => "Scam",
        "persistent_ragebait" => "Persistent Ragebait",
        _ => "Other",
    }
}

fn case_jump_url(guild_id: u64, channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
}

fn safe_message_text(value: &str, limit: usize) -> String {
    let text = value.trim();
    if text.is_empty() {
        return "[kein Text]".to_string();
    }
    truncate_chars(text, limit)
}

fn embed_fields_mut(embed: &mut Value) -> Option<&mut Vec<Value>> {
    let object = embed.as_object_mut()?;
    object
        .entry("fields")
        .or_insert_with(|| json!([]))
        .as_array_mut()
}

fn push_embed_field(embed: &mut Value, name: &str, value: String, inline: bool) {
    if value.is_empty() {
        return;
    }
    let Some(fields) = embed_fields_mut(embed) else {
        return;
    };
    fields.push(json!({
        "name": name,
        "value": truncate_chars(&value, DISCORD_FIELD_LIMIT),
        "inline": inline,
    }));
}

fn case_attachment_is_image(attachment: &dl_moderation::store::CaseAttachment) -> bool {
    attachment.content_type.to_lowercase().starts_with("image/")
        || CASE_IMAGE_ATTACHMENT_EXTENSIONS
            .iter()
            .any(|ext| attachment.filename.to_lowercase().ends_with(ext))
}

fn apply_case_attachment_rendering(
    embed: &mut Value,
    attachments: &[dl_moderation::store::CaseAttachment],
) {
    if attachments.is_empty() {
        return;
    }
    let image_urls: Vec<&str> = attachments
        .iter()
        .filter(|attachment| case_attachment_is_image(attachment))
        .map(|attachment| attachment.url.as_str())
        .collect();
    let other_urls: Vec<String> = attachments
        .iter()
        .filter(|attachment| !case_attachment_is_image(attachment))
        .map(|attachment| {
            let filename = if attachment.filename.trim().is_empty() {
                "attachment"
            } else {
                attachment.filename.as_str()
            };
            format!("[{}]({})", truncate_chars(filename, 80), attachment.url)
        })
        .collect();

    if let Some(first_image) = image_urls.first() {
        if let Some(object) = embed.as_object_mut() {
            object.insert("image".into(), json!({ "url": first_image }));
        }
    }
    if image_urls.len() > 1 {
        push_embed_field(embed, "Weitere Bilder", image_urls[1..].join("\n"), false);
    }
    if !other_urls.is_empty() {
        push_embed_field(embed, "Attachments", other_urls.join("\n"), false);
    }
}

fn build_case_log_embed(
    case: &dl_moderation::store::CaseDraft,
    case_id: &str,
    action: &str,
) -> Value {
    let title = match action {
        "auto_delete" => format!(
            "🚨 Auto-Delete: {} ({:.2})",
            category_label(&case.category),
            case.confidence
        ),
        "auto_delete_failed" => format!(
            "⚠️ Auto-Delete fehlgeschlagen: {} ({:.2})",
            category_label(&case.category),
            case.confidence
        ),
        "proposed" => format!(
            "📝 Vorschlag: {} ({:.2})",
            category_label(&case.category),
            case.confidence
        ),
        "ragebait_escalated" => "📝 Ragebait eskaliert".to_string(),
        other => format!("📝 KI-Moderation: {other}"),
    };
    let color = match action {
        "auto_delete" => 0xE74C3C,
        "auto_delete_failed" => 0x992D22,
        "proposed" | "ragebait_escalated" => 0xE67E22,
        _ => 0x5865F2,
    };
    let mut embed = json!({
        "title": title,
        "color": color,
        "fields": [
            { "name": "Author", "value": format!("<@{}> (`{}`)", case.user_id, case.user_tag), "inline": false },
            { "name": "Channel", "value": format!("<#{}> | [Jump]({})", case.channel_id, case_jump_url(case.guild_id, case.channel_id, case.message_id)), "inline": false },
            { "name": "Kategorie", "value": category_label(&case.category), "inline": true },
            { "name": "Confidence", "value": format!("{:.2}", case.confidence), "inline": true },
            { "name": "AI-Reason", "value": truncate_chars(&case.reason, DISCORD_FIELD_LIMIT), "inline": false },
            { "name": "Aktion", "value": action, "inline": true },
            { "name": "Case-ID", "value": case_id, "inline": true }
        ]
    });
    if case.escalated_with_context {
        push_embed_field(
            &mut embed,
            "Detail",
            "context_escalation".to_string(),
            false,
        );
    }
    embed
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

#[derive(Debug, Clone)]
struct InviteCacheEntry {
    guild_id: u64,
    expires_at: i64,
    last_seen_at: i64,
}

struct BehaviorInviteResolver {
    our_guild_id: u64,
    fallback_codes: HashSet<String>,
    allowlist: RwLock<HashSet<String>>,
    cache: Mutex<HashMap<String, InviteCacheEntry>>,
}

impl BehaviorInviteResolver {
    fn new(our_guild_id: u64, fallback_codes: Vec<String>) -> Self {
        let fallback_codes: HashSet<String> = fallback_codes.into_iter().collect();
        Self {
            our_guild_id,
            allowlist: RwLock::new(fallback_codes.clone()),
            fallback_codes,
            cache: Mutex::new(HashMap::new()),
        }
    }

    async fn refresh_allowlist(&self, http: &Http) {
        let mut next = self.fallback_codes.clone();
        match http
            .get_guild_invites(GuildId::new(self.our_guild_id))
            .await
        {
            Ok(invites) => {
                for invite in invites {
                    next.insert(invite.code);
                }
            }
            Err(err) => {
                tracing::warn!(%err, guild_id = self.our_guild_id, "BehaviorDetector: eigene Invites nicht abrufbar");
            }
        }
        match http
            .get_guild_vanity_url(GuildId::new(self.our_guild_id))
            .await
        {
            Ok(code) if !code.trim().is_empty() => {
                next.insert(code.trim().to_string());
            }
            Ok(_) => {}
            Err(err) => {
                tracing::debug!(%err, guild_id = self.our_guild_id, "BehaviorDetector: Vanity-Invite nicht abrufbar");
            }
        }
        *self.allowlist.write().await = next;
    }

    async fn cached_guild_id(&self, code: &str, now: i64) -> Option<u64> {
        let mut cache = self.cache.lock().await;
        prune_invite_cache(&mut cache, now);
        let entry = cache.get_mut(code).filter(|entry| entry.expires_at > now)?;
        entry.last_seen_at = now;
        Some(entry.guild_id)
    }

    async fn remember_resolve_result(&self, code: &str, guild_id: Option<u64>, now: i64) {
        let Some(guild_id) = guild_id else {
            return;
        };
        let mut cache = self.cache.lock().await;
        prune_invite_cache(&mut cache, now);
        cache.insert(
            code.to_string(),
            InviteCacheEntry {
                guild_id,
                expires_at: now + INVITE_CACHE_TTL_SECONDS,
                last_seen_at: now,
            },
        );
        prune_invite_cache(&mut cache, now);
    }

    async fn resolve(&self, http: &Http, code: &str) -> Option<u64> {
        let code = code.trim();
        if code.is_empty() {
            return None;
        }
        if self.allowlist.read().await.contains(code) {
            return Some(self.our_guild_id);
        }

        let now = chrono::Utc::now().timestamp();
        if let Some(guild_id) = self.cached_guild_id(code, now).await {
            return Some(guild_id);
        }

        let guild_id = match http.get_invite(code, false, false, None).await {
            Ok(invite) => invite.guild.map(|guild| guild.id.get()),
            Err(err) => {
                tracing::debug!(%err, %code, "BehaviorDetector: Invite nicht aufloesbar");
                None
            }
        };
        if guild_id == Some(self.our_guild_id) {
            self.allowlist.write().await.insert(code.to_string());
        }
        self.remember_resolve_result(code, guild_id, now).await;
        guild_id
    }
}

fn prune_invite_cache(cache: &mut HashMap<String, InviteCacheEntry>, now: i64) {
    cache.retain(|_, entry| entry.expires_at > now);
    while cache.len() > INVITE_CACHE_MAX_ENTRIES {
        let Some(victim) = cache
            .iter()
            .min_by_key(|(code, entry)| (entry.last_seen_at, entry.expires_at, (*code).clone()))
            .map(|(code, _)| code.clone())
        else {
            break;
        };
        cache.remove(&victim);
    }
}

fn looks_like_invite_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

pub fn parse_invite_allowlist_fallback(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for token in raw.split([',', ';', '\n', '\r', '\t', ' ']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let extracted = dl_moderation::behavior_detector::extract_invite_codes(token);
        if extracted.is_empty() && looks_like_invite_code(token) {
            if seen.insert(token.to_string()) {
                out.push(token.to_string());
            }
            continue;
        }
        for code in extracted {
            if seen.insert(code.clone()) {
                out.push(code);
            }
        }
    }
    out
}

pub struct BehaviorDetectorGlue {
    pub adapter: Arc<DiscordAdapter>,
    invite_resolver: Arc<BehaviorInviteResolver>,
}

impl BehaviorDetectorGlue {
    pub fn new(
        adapter: Arc<DiscordAdapter>,
        our_guild_id: u64,
        fallback_codes: Vec<String>,
    ) -> Self {
        Self {
            adapter,
            invite_resolver: Arc::new(BehaviorInviteResolver::new(our_guild_id, fallback_codes)),
        }
    }

    pub async fn refresh_invite_allowlist(&self) {
        self.invite_resolver
            .refresh_allowlist(&self.adapter.http)
            .await;
    }
}

#[async_trait::async_trait]
impl dl_moderation::behavior_detector::BehaviorDetectorPort for BehaviorDetectorGlue {
    async fn resolve_invite_guild(&self, code: &str) -> Option<u64> {
        self.invite_resolver.resolve(&self.adapter.http, code).await
    }
}

// ── Coaching-Plattform-Anbindung ───────────────────────────────────────────

pub struct CoachingGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub guild_id: u64,
}

fn coach_member_tuple(member: &serenity::all::Member) -> (u64, String, String, String) {
    let avatar = member
        .user
        .avatar_url()
        .unwrap_or_else(|| member.user.default_avatar_url());
    let avatar = if avatar.contains('?') {
        format!("{avatar}&size=256")
    } else {
        format!("{avatar}?size=256")
    };
    (
        member.user.id.get(),
        member.user.name.to_string(),
        member.display_name().to_string(),
        avatar,
    )
}

#[async_trait::async_trait]
impl dl_community::coaching::CoachingPort for CoachingGlue {
    async fn coach_members(&self, role_id: u64) -> Vec<(u64, String, String, String)> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(self.guild_id)) else {
            return Vec::new();
        };
        let role = serenity::all::RoleId::new(role_id);
        guild
            .members
            .values()
            .filter(|member| member.roles.contains(&role))
            .map(coach_member_tuple)
            .collect()
    }

    async fn coach_members_fetch_fallback(
        &self,
        role_id: u64,
    ) -> Vec<(u64, String, String, String)> {
        let role = serenity::all::RoleId::new(role_id);
        let guild_id = GuildId::new(self.guild_id);
        let mut after = None;
        let mut coaches = Vec::new();
        loop {
            let page = match self
                .adapter
                .http
                .get_guild_members(guild_id, Some(1000), after)
                .await
            {
                Ok(page) => page,
                Err(err) => {
                    tracing::warn!(%err, guild_id = self.guild_id, "Coach-Member-Fetch-Fallback fehlgeschlagen");
                    break;
                }
            };
            if page.is_empty() {
                break;
            }
            after = page.last().map(|member| member.user.id.get());
            coaches.extend(
                page.iter()
                    .filter(|member| member.roles.contains(&role))
                    .map(coach_member_tuple),
            );
            if page.len() < 1000 || after.is_none() {
                break;
            }
        }
        coaches
    }

    async fn send_dm(&self, user_id: u64, text: String) -> Result<bool, String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|e| e.to_string())?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => Ok(true),
            // 50007 = Cannot send messages to this user (DMs zu) → ackbar
            Err(err) if err.to_string().contains("50007") => Ok(false),
            Err(err) => Err(err.to_string()),
        }
    }
}

// ── Website-Invite-Anbindung ───────────────────────────────────────────────

pub struct InviteGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::invites::InvitePort for InviteGlue {
    async fn guild_invite_codes(&self, guild_id: u64) -> Vec<String> {
        self.adapter
            .http
            .get_guild_invites(GuildId::new(guild_id))
            .await
            .map(|invites| invites.into_iter().map(|i| i.code).collect())
            .unwrap_or_default()
    }

    async fn create_permanent_invite(
        &self,
        channel_id: u64,
        reason: &str,
    ) -> Result<String, String> {
        self.adapter
            .http
            .create_invite(
                ChannelId::new(channel_id),
                &json!({ "max_age": 0, "max_uses": 0, "unique": true }),
                Some(reason),
            )
            .await
            .map(|invite| invite.code)
            .map_err(|e| e.to_string())
    }
}

// ── Leave-Survey-Anbindung ─────────────────────────────────────────────────

pub struct SurveyGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn is_discord_cannot_send_messages(err: &serenity::Error) -> bool {
    matches!(
        err,
        serenity::Error::Http(HttpError::UnsuccessfulRequest(resp)) if resp.error.code == 50007
    )
}

fn failed_survey_dm(
    user_id: u64,
    err: serenity::Error,
) -> dl_community::leave_survey::SurveyDmDelivery {
    let error = err.to_string();
    tracing::warn!(user_id, %error, "Leave-Survey-DM fehlgeschlagen");
    dl_community::leave_survey::SurveyDmDelivery::Failed(error)
}

#[async_trait::async_trait]
impl dl_community::leave_survey::SurveyPort for SurveyGlue {
    async fn send_survey_dm(
        &self,
        user_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> dl_community::leave_survey::SurveyDmDelivery {
        use dl_community::leave_survey::SurveyDmDelivery;

        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) if is_discord_cannot_send_messages(&err) => return SurveyDmDelivery::Blocked,
            Err(err) => return failed_survey_dm(user_id, err),
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        match self
            .adapter
            .send_raw_public_typed(channel.id.get(), &body)
            .await
        {
            Ok(_) => SurveyDmDelivery::Sent,
            Err(err) if is_discord_cannot_send_messages(&err) => SurveyDmDelivery::Blocked,
            Err(err) => failed_survey_dm(user_id, err),
        }
    }

    async fn post_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_community::leave_survey::LOGS_CHANNEL_ID, &body)
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

// ── Clip-Einsendungen-Anbindung ────────────────────────────────────────────

pub struct ClipGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::clips::ClipPort for ClipGlue {
    async fn upsert_interface(
        &self,
        channel_id: u64,
        existing_message_id: Option<u64>,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        if let Some(message_id) = existing_message_id {
            let edited = self
                .adapter
                .http
                .edit_message(
                    ChannelId::new(channel_id),
                    serenity::all::MessageId::new(message_id),
                    &body,
                    Vec::new(),
                )
                .await;
            match edited {
                Ok(message) => return Ok(message.id.get()),
                Err(err) if err.to_string().contains("10008") => {} // Nachricht weg → neu posten
                Err(err) => return Err(err.to_string()),
            }
        }
        self.adapter
            .send_raw_public(channel_id, &body)
            .await
            .map_err(|e| e.to_string())
    }

    async fn send_dump(
        &self,
        user_id: u64,
        fallback_channel_id: u64,
        caption: String,
        filename: String,
        content: String,
    ) {
        let attachment = serenity::all::CreateAttachment::bytes(content.into_bytes(), filename);
        let dm = async {
            let channel = self
                .adapter
                .http
                .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
                .await
                .map_err(|e| e.to_string())?;
            channel
                .send_files(
                    &self.adapter.http,
                    vec![attachment.clone()],
                    serenity::all::CreateMessage::new().content(caption.clone()),
                )
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(err) = dm {
            tracing::warn!(%err, "Clip-Dump-DM fehlgeschlagen — Fallback in den Submit-Kanal");
            let _ = ChannelId::new(fallback_channel_id)
                .send_files(
                    &self.adapter.http,
                    vec![attachment],
                    serenity::all::CreateMessage::new().content("📦 **Wochen-Dump (Clips)**"),
                )
                .await;
        }
    }

    async fn guild_name(&self, guild_id: u64) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| g.name.to_string())
            .unwrap_or_else(|| guild_id.to_string())
    }

    async fn post_message(
        &self,
        channel_id: u64,
        mut body: Map<String, Value>,
        nonce: String,
    ) -> Result<u64, String> {
        // Discord verwirft einen zweiten Post mit gleicher Nonce kurz danach.
        body.insert("nonce".into(), json!(nonce));
        body.insert("enforce_nonce".into(), json!(true));
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_message(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), String> {
        self.adapter
            .edit_raw_public(channel_id, message_id, &body)
            .await
            .map_err(|e| e.to_string())
    }

    async fn find_message_by_footer(
        &self,
        channel_id: u64,
        footer: &str,
    ) -> Result<Option<u64>, String> {
        let bot_id = self.adapter.cache().current_user().id;
        let messages = ChannelId::new(channel_id)
            .messages(&self.adapter.http, GetMessages::new().limit(50))
            .await
            .map_err(|e| e.to_string())?;
        Ok(messages
            .iter()
            .find(|message| {
                message.author.id == bot_id
                    && message
                        .embeds
                        .iter()
                        .any(|embed| embed.footer.as_ref().is_some_and(|f| f.text == footer))
            })
            .map(|message| message.id.get()))
    }

    async fn send_curator_text(
        &self,
        user_id: u64,
        fallback_channel_id: u64,
        content: String,
    ) -> Result<(), String> {
        let mut body = Map::new();
        body.insert("content".into(), json!(content));
        body.insert("allowed_mentions".into(), json!({ "parse": [] }));
        match self
            .adapter
            .send_raw_dm_public(&user_id.to_string(), &body)
            .await
        {
            Ok(_) => Ok(()),
            Err(err) => {
                tracing::warn!(%err, "Clip-Contest-DM fehlgeschlagen, Fallback in den Clip-Kanal");
                self.adapter
                    .send_raw_public(fallback_channel_id, &body)
                    .await
                    .map(|_| ())
            }
        }
    }

    async fn member_joined_at(&self, guild_id: u64, user_id: u64) -> Option<i64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .members
            .get(&UserId::new(user_id))?
            .joined_at
            .map(|ts| ts.unix_timestamp())
    }
}

/// Broker-Port für `POST /internal/master/v1/clips/submit`.
pub struct ClipSubmitGlue {
    pub clips: Arc<dl_community::clips::ClipSubmission>,
}

#[async_trait::async_trait]
impl dl_broker::clips::ClipSubmitPort for ClipSubmitGlue {
    async fn submit_twitch_clip(
        &self,
        submission: dl_broker::clips::TwitchClipSubmission,
    ) -> Result<dl_broker::clips::ClipSubmitOutcome, String> {
        use dl_broker::clips::{ClipSubmitOutcome, ClipSubmitStatus};
        use dl_community::clip_contest::{TwitchClipRequest, TwitchSubmitOutcome};
        let request = TwitchClipRequest {
            clip_url: submission.clip_url,
            streamer_twitch_user_id: submission.streamer_twitch_user_id,
            streamer_login: submission.streamer_login,
            submitted_by_twitch_user_id: submission.submitted_by_twitch_user_id,
            submitted_at: submission.submitted_at,
            title: submission.title,
            idempotency_key: submission.idempotency_key,
        };
        let outcome = self
            .clips
            .submit_twitch(dl_community::clips::GUILD_ID, &request)
            .await
            .map_err(|e| e.to_string())?;
        Ok(match outcome {
            TwitchSubmitOutcome::Accepted(id) => ClipSubmitOutcome {
                status: ClipSubmitStatus::Accepted,
                submission_id: Some(id),
                reason: None,
            },
            TwitchSubmitOutcome::Duplicate(id) => ClipSubmitOutcome {
                status: ClipSubmitStatus::Duplicate,
                submission_id: Some(id),
                reason: Some("duplicate_clip_this_week".to_string()),
            },
            TwitchSubmitOutcome::ReplayMetadataDrift(id) => ClipSubmitOutcome {
                status: ClipSubmitStatus::Duplicate,
                submission_id: Some(id),
                reason: Some("idempotency_metadata_drift".to_string()),
            },
            TwitchSubmitOutcome::Rejected(reason) => ClipSubmitOutcome {
                status: ClipSubmitStatus::Rejected,
                submission_id: None,
                reason: Some(reason.to_string()),
            },
        })
    }
}

// ── FAQ-Chat-Anbindung ─────────────────────────────────────────────────────

/// Gemeinsamer Payload für alle sichtbaren FAQ/Knowledge/Shadow-Nachrichten (rohe Nutzerfrage,
/// Modelltext, Shadow-Ausgabe). Ein Sendepfad, ein Kontrakt.
fn faq_message_body(content: &str, components: Option<serde_json::Value>) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("content".into(), json!(content));
    if let Some(components) = components {
        body.insert("components".into(), components);
    }
    // Rohe Nutzerfrage, Modelltext und Shadow-Ausgabe dürfen niemals Rollen, @everyone oder
    // einzelne User pingen: leeres parse, kein replied_user.
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    body
}

pub struct FaqGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::faq::FaqPort for FaqGlue {
    async fn create_faq_channel(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_name: &str,
    ) -> Result<u64, String> {
        let bot_id = self.adapter.cache().current_user().id.get();
        // VIEW=1024, SEND=2048, HISTORY=65536, MANAGE_CHANNELS=16
        let body = json!({
            "name": channel_name,
            "type": 0,
            "parent_id": dl_community::faq::FAQ_CATEGORY_ID.to_string(),
            "permission_overwrites": [
                { "id": guild_id.to_string(), "type": 0, "deny": "1024" },
                { "id": user_id.to_string(), "type": 1, "allow": "68608" },
                { "id": bot_id.to_string(), "type": 1, "allow": "68624" },
            ],
        });
        self.adapter
            .http
            .create_channel(
                GuildId::new(guild_id),
                body.as_object().expect("json object"),
                Some("FAQ Chat"),
            )
            .await
            .map(|c| c.id.get())
            .map_err(|e| e.to_string())
    }

    async fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        components: Option<serde_json::Value>,
    ) -> Result<u64, String> {
        let body = faq_message_body(content, components);
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64> {
        if guild_id == 0 {
            return None;
        }
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .and_then(|c| c.parent_id.map(|p| p.get()))
    }

    async fn private_faq_channel_owned_by_user(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
    ) -> bool {
        if guild_id == 0 {
            return false;
        }
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return false;
        };
        let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) else {
            return false;
        };
        let bot_id = self.adapter.cache().current_user().id.get();
        channel.parent_id == Some(ChannelId::new(dl_community::faq::FAQ_CATEGORY_ID))
            && !channel
                .topic
                .as_deref()
                .is_some_and(|topic| topic.starts_with(CONCIERGE_OWNER_TOPIC_PREFIX))
            && private_channel_overwrites_are_owner_only(
                guild_id,
                user_id,
                bot_id,
                &channel.permission_overwrites,
            )
    }

    async fn user_name(&self, user_id: u64) -> String {
        self.adapter
            .cache()
            .user(UserId::new(user_id))
            .map(|u| u.name.to_string())
            .unwrap_or_else(|| format!("user-{user_id}"))
    }

    async fn delete_channel(&self, channel_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_channel(
                ChannelId::new(channel_id),
                Some("FAQ-Transaktion fehlgeschlagen"),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("FAQ-Transaktion fehlgeschlagen"),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn post_rich(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn delete_panel(&self, channel_id: u64, message_id: u64) {
        let _ = self
            .adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("FAQ: Duplikat-Panel aufräumen"),
            )
            .await;
    }
}

// ── Voice-DM-Anbindung mit Concierge-Gedächtnis ───────────────────────────

async fn record_concierge_system_dm(
    store: &Option<dl_community::concierge::ConciergeStore>,
    guild_id: u64,
    user_id: u64,
    marker: &str,
) {
    if let Some(store) = store {
        if let Err(err) = store.record_system_dm(user_id, guild_id, marker).await {
            tracing::warn!(%err, user_id, "Concierge: System-DM-Marker konnte nicht gespeichert werden");
        }
    }
}

pub struct VoiceNudgeGlue {
    pub inner: dl_voice::glue::NudgeGlue,
    pub concierge_store: Option<dl_community::concierge::ConciergeStore>,
    pub concierge_guild_id: u64,
}

#[async_trait::async_trait]
impl dl_voice::nudge::NudgePort for VoiceNudgeGlue {
    async fn is_in_voice(&self, guild_id: u64, user_id: u64) -> bool {
        dl_voice::nudge::NudgePort::is_in_voice(&self.inner, guild_id, user_id).await
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        dl_voice::nudge::NudgePort::member_role_ids(&self.inner, guild_id, user_id).await
    }

    async fn send_dm(
        &self,
        user_id: u64,
        embeds: &[serde_json::Value],
        components: &serde_json::Value,
    ) -> Result<(u64, u64), String> {
        let sent =
            dl_voice::nudge::NudgePort::send_dm(&self.inner, user_id, embeds, components).await?;
        record_concierge_system_dm(
            &self.concierge_store,
            self.concierge_guild_id,
            user_id,
            dl_community::concierge::STEAM_NUDGE_MEMORY_MARKER,
        )
        .await;
        Ok(sent)
    }

    async fn send_log(&self, text: String) {
        dl_voice::nudge::NudgePort::send_log(&self.inner, text).await;
    }

    async fn fetch_steam_link_url(&self, user_id: u64) -> Option<String> {
        dl_voice::nudge::NudgePort::fetch_steam_link_url(&self.inner, user_id).await
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) {
        dl_voice::nudge::NudgePort::delete_message(&self.inner, channel_id, message_id).await;
    }

    async fn refresh_dm(
        &self,
        channel_id: u64,
        message_id: u64,
        embeds: &[serde_json::Value],
        components: &serde_json::Value,
    ) -> Result<bool, String> {
        dl_voice::nudge::NudgePort::refresh_dm(
            &self.inner,
            channel_id,
            message_id,
            embeds,
            components,
        )
        .await
    }
}

pub struct VoiceFeedbackGlue {
    pub inner: dl_voice::glue::FeedbackGlue,
    pub concierge_store: Option<dl_community::concierge::ConciergeStore>,
    pub concierge_guild_id: u64,
}

#[async_trait::async_trait]
impl dl_voice::feedback::FeedbackPort for VoiceFeedbackGlue {
    async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>) {
        let sent =
            dl_voice::feedback::FeedbackPort::send_feedback_dm(&self.inner, user_id, text).await;
        if sent.0 == "sent" {
            record_concierge_system_dm(
                &self.concierge_store,
                self.concierge_guild_id,
                user_id,
                dl_community::concierge::VOICE_FEEDBACK_MEMORY_MARKER,
            )
            .await;
        }
        sent
    }

    async fn forward_to_owner(&self, owner_id: u64, text: String) {
        dl_voice::feedback::FeedbackPort::forward_to_owner(&self.inner, owner_id, text).await;
    }

    async fn delete_feedback_prompt(&self, user_id: u64, message_id: u64) {
        dl_voice::feedback::FeedbackPort::delete_feedback_prompt(&self.inner, user_id, message_id)
            .await;
    }

    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String> {
        dl_voice::feedback::FeedbackPort::display_name(&self.inner, guild_id, user_id).await
    }
}

// ── Concierge-Onboarding-Anbindung ─────────────────────────────────────────

pub struct ConciergeGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::concierge::ConciergePort for ConciergeGlue {
    async fn send_dm_v2(
        &self,
        user_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> dl_community::concierge::ConciergeDmDelivery {
        use dl_community::concierge::ConciergeDmDelivery;

        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) if is_discord_cannot_send_messages(&err) => {
                return ConciergeDmDelivery::CannotSend50007;
            }
            Err(err) => return ConciergeDmDelivery::Failed(err.to_string()),
        };
        match self
            .adapter
            .send_raw_public_typed(channel.id.get(), &body)
            .await
        {
            Ok(message_id) => ConciergeDmDelivery::Sent {
                channel_id: channel.id.get(),
                message_id,
            },
            Err(err) if is_discord_cannot_send_messages(&err) => {
                ConciergeDmDelivery::CannotSend50007
            }
            Err(err) => ConciergeDmDelivery::Failed(err.to_string()),
        }
    }

    async fn create_private_channel(
        &self,
        guild_id: u64,
        user_id: u64,
        extra_user_id: Option<u64>,
        category_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        let bot_id = self.adapter.cache().current_user().id.get();
        let mut overwrites = vec![
            json!({ "id": guild_id.to_string(), "type": 0, "deny": "1024" }),
            json!({ "id": user_id.to_string(), "type": 1, "allow": "68608" }),
            json!({ "id": bot_id.to_string(), "type": 1, "allow": "68624" }),
        ];
        if let Some(extra_user_id) = extra_user_id.filter(|id| *id != user_id) {
            overwrites.insert(
                2,
                json!({ "id": extra_user_id.to_string(), "type": 1, "allow": "68608" }),
            );
        }
        let body = json!({
            "name": name,
            "type": 0,
            "parent_id": category_id.to_string(),
            "topic": if extra_user_id.is_none() {
                Some(format!("{CONCIERGE_OWNER_TOPIC_PREFIX}{user_id}"))
            } else {
                None
            },
            "permission_overwrites": overwrites,
        });
        let Some(body) = body.as_object() else {
            return Err("invalid channel body".to_string());
        };
        self.adapter
            .http
            .create_channel(GuildId::new(guild_id), body, Some("Concierge Fallback"))
            .await
            .map(|channel| channel.id.get())
            .map_err(|err| err.to_string())
    }

    async fn role_member_ids(&self, guild_id: u64, role_id: u64) -> Result<Vec<u64>, String> {
        let guild = GuildId::new(guild_id);
        let role = RoleId::new(role_id);
        if let Some(cached) = self.adapter.cache().guild(guild) {
            let ids = cached
                .members
                .values()
                .filter(|member| member.roles.contains(&role))
                .map(|member| member.user.id.get())
                .collect::<Vec<_>>();
            if !ids.is_empty() {
                return Ok(ids);
            }
        }

        let mut after = None;
        let mut ids = Vec::new();
        loop {
            let page = self
                .adapter
                .http
                .get_guild_members(guild, Some(1000), after)
                .await
                .map_err(|err| err.to_string())?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|member| member.user.id.get());
            ids.extend(
                page.into_iter()
                    .filter(|member| member.roles.contains(&role))
                    .map(|member| member.user.id.get()),
            );
        }
        Ok(ids)
    }

    async fn private_channel_owned_by_user(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
        category_id: u64,
    ) -> bool {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return false;
        };
        let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) else {
            return false;
        };
        let bot_id = self.adapter.cache().current_user().id.get();
        let expected_topic = format!("{CONCIERGE_OWNER_TOPIC_PREFIX}{user_id}");
        channel.parent_id == Some(ChannelId::new(category_id))
            && channel.topic.as_deref() == Some(expected_topic.as_str())
            && private_channel_overwrites_are_owner_only(
                guild_id,
                user_id,
                bot_id,
                &channel.permission_overwrites,
            )
    }

    async fn send_channel_v2(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn find_channel_message_by_nonce(
        &self,
        channel_id: u64,
        after_message_id: u64,
        nonce: &str,
    ) -> Result<Option<u64>, String> {
        let bot_id = self
            .adapter
            .http
            .get_current_user()
            .await
            .map_err(|err| err.to_string())?
            .id;
        let mut before = None;
        loop {
            let mut request = GetMessages::new().limit(100);
            if let Some(message_id) = before {
                request = request.before(MessageId::new(message_id));
            }
            let messages = ChannelId::new(channel_id)
                .messages(&self.adapter.http, request)
                .await
                .map_err(|err| err.to_string())?;
            if messages.is_empty() {
                return Ok(None);
            }
            for message in &messages {
                let matching_reply = message
                    .message_reference
                    .as_ref()
                    .and_then(|reference| reference.message_id)
                    == Some(MessageId::new(after_message_id))
                    && serde_json::to_string(&message.components).is_ok_and(|body| {
                        body.contains("diese Patenanfrage wartet seit zwei Stunden")
                    });
                if message.id.get() > after_message_id
                    && message.author.id == bot_id
                    && (matches!(&message.nonce, Some(serenity::all::Nonce::String(value)) if value == nonce)
                        || matching_reply)
                {
                    return Ok(Some(message.id.get()));
                }
            }
            let oldest = messages.iter().map(|message| message.id.get()).min();
            if oldest.is_some_and(|id| id <= after_message_id) {
                return Ok(None);
            }
            if before == oldest {
                return Err(
                    "Discord-Verlauf für den Owner-Hinweis bewegt sich nicht weiter".into(),
                );
            }
            before = oldest;
            if messages.len() < 100 {
                return Ok(None);
            }
        }
    }

    async fn send_channel_text(&self, channel_id: u64, content: &str) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("allowed_mentions".into(), json!({ "parse": [] }));
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn delete_channel(&self, channel_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_channel(
                ChannelId::new(channel_id),
                Some("Concierge-Transaktion fehlgeschlagen"),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("Concierge-Transaktion fehlgeschlagen"),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn add_reaction(&self, channel_id: u64, message_id: u64, emoji: &str) {
        if let Err(err) = self
            .adapter
            .http
            .create_reaction(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &ReactionType::Unicode(emoji.to_string()),
            )
            .await
        {
            tracing::warn!(%err, channel_id, message_id, emoji, "Concierge-Reaktion fehlgeschlagen");
        }
    }

    async fn reply_to_message(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
        allowed_role_id: Option<u64>,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert(
            "message_reference".into(),
            json!({ "channel_id": channel_id.to_string(), "message_id": message_id.to_string() }),
        );
        body.insert(
            "allowed_mentions".into(),
            allowed_role_id.map_or_else(
                || json!({ "parse": [], "replied_user": false }),
                |role_id| {
                    json!({
                        "parse": [],
                        "roles": [role_id.to_string()],
                        "replied_user": false
                    })
                },
            ),
        );
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_channel_v2(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn pin_message(&self, channel_id: u64, message_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .pin_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("Paten-Leitfaden"),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn pinned_message(
        &self,
        channel_id: u64,
        message_id: u64,
    ) -> Result<Option<bool>, String> {
        match ChannelId::new(channel_id)
            .message(&self.adapter.http, MessageId::new(message_id))
            .await
        {
            Ok(message) => Ok(Some(message.pinned)),
            Err(serenity::Error::Http(HttpError::UnsuccessfulRequest(response)))
                if response.status_code.as_u16() == 404 || response.error.code == 10008 =>
            {
                Ok(None)
            }
            Err(err) => Err(err.to_string()),
        }
    }

    async fn existing_pate_request_cards(
        &self,
    ) -> Result<Vec<dl_community::concierge::LegacyPateRequestMessage>, String> {
        fn collect_user_ids(value: &Value, user_ids: &mut HashSet<u64>) {
            match value {
                Value::Object(object) => {
                    if let Some(custom_id) = object.get("custom_id").and_then(Value::as_str) {
                        if let Some(raw_user_id) = custom_id.strip_prefix("concierge:pate:claim:") {
                            if let Ok(user_id) = raw_user_id.parse() {
                                user_ids.insert(user_id);
                            }
                        }
                    }
                    for child in object.values() {
                        collect_user_ids(child, user_ids);
                    }
                }
                Value::Array(array) => {
                    for child in array {
                        collect_user_ids(child, user_ids);
                    }
                }
                _ => {}
            }
        }

        let mut before = None;
        let mut requests = Vec::new();
        loop {
            let mut pagination = GetMessages::new().limit(100);
            if let Some(message_id) = before {
                pagination = pagination.before(MessageId::new(message_id));
            }
            let messages = ChannelId::new(dl_community::concierge::PATE_REQUEST_CHANNEL_ID)
                .messages(&self.adapter.http, pagination)
                .await
                .map_err(|err| err.to_string())?;
            if messages.is_empty() {
                break;
            }
            for message in &messages {
                let components =
                    serde_json::to_value(&message.components).map_err(|err| err.to_string())?;
                let mut user_ids = HashSet::new();
                collect_user_ids(&components, &mut user_ids);
                let Some(created_at) =
                    chrono::DateTime::from_timestamp(message.timestamp.unix_timestamp(), 0)
                else {
                    continue;
                };
                for user_id in user_ids {
                    requests.push(dl_community::concierge::LegacyPateRequestMessage {
                        user_id,
                        message_id: message.id.get(),
                        created_at,
                    });
                }
            }
            let oldest = messages.iter().map(|message| message.id.get()).min();
            if oldest == before {
                return Err("Discord-Verlauf im Patenkanal bewegt sich nicht weiter".to_string());
            }
            before = oldest;
            if messages.len() < 100 {
                break;
            }
        }
        Ok(requests)
    }
}

// ── Anonymes-Feedback-Anbindung ────────────────────────────────────────────

pub struct FeedbackGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::feedback_hub::FeedbackPort for FeedbackGlue {
    async fn send_dm_text(&self, user_id: u64, text: String) -> Result<(), String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|e| e.to_string())?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => Ok(()),
            // 50007 = Cannot send messages to this user (DMs zu) → nicht actionbar
            Err(err) if err.to_string().contains("50007") => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }

    async fn post_rich(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

// ── Reaction-Roles-Anbindung ───────────────────────────────────────────────

pub struct ReactionRoleGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn reaction_role_port_err(err: serenity::Error) -> dl_community::reaction_roles::PortErr {
    dl_community::reaction_roles::PortErr::Discord(err.to_string())
}

fn reaction_role_dm_err(err: serenity::Error) -> dl_community::reaction_roles::DmErr {
    let permanent = matches!(
        &err,
        serenity::Error::Http(HttpError::UnsuccessfulRequest(resp))
            if resp.status_code.as_u16() == 403
                || resp.status_code.as_u16() == 404
                || matches!(resp.error.code, 50007 | 10013)
    );
    let text = err.to_string();
    if permanent {
        dl_community::reaction_roles::DmErr::Permanent(text)
    } else {
        dl_community::reaction_roles::DmErr::Transient(text)
    }
}

#[async_trait::async_trait]
impl dl_community::reaction_roles::ReactionRolePort for ReactionRoleGlue {
    async fn add_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
    ) -> Result<(), dl_community::reaction_roles::PortErr> {
        self.adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some("reaction-role:add"),
            )
            .await
            .map_err(reaction_role_port_err)
    }

    async fn remove_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
    ) -> Result<(), dl_community::reaction_roles::PortErr> {
        self.adapter
            .http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some("reaction-role:remove"),
            )
            .await
            .map_err(reaction_role_port_err)
    }

    async fn send_dm(
        &self,
        user_id: u64,
        content: &str,
    ) -> Result<(), dl_community::reaction_roles::DmErr> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(reaction_role_dm_err)?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        self.adapter
            .http
            .send_message(channel.id, Vec::new(), &body)
            .await
            .map(|_| ())
            .map_err(reaction_role_dm_err)
    }

    async fn reaction_users(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionType,
        after: Option<u64>,
    ) -> Result<Vec<dl_community::reaction_roles::ReactedUser>, dl_community::reaction_roles::PortErr>
    {
        let users = self
            .adapter
            .reaction_users(channel_id, message_id, emoji, after)
            .await
            .map_err(reaction_role_port_err)?;
        Ok(users
            .into_iter()
            .map(|user| dl_community::reaction_roles::ReactedUser {
                id: user.id,
                is_bot: user.is_bot,
            })
            .collect())
    }
}

pub struct ReactionRoleGatewayGlue {
    pub service: Arc<dl_community::reaction_roles::ReactionRoleService>,
}

#[async_trait::async_trait]
impl dl_discord::gateway::ReactionRoleGatewayPort for ReactionRoleGatewayGlue {
    async fn reaction_add(&self, event: dl_discord::gateway::ReactionRoleAddEvent) {
        if let Err(err) = self
            .service
            .handle_reaction_add_with_display_name(
                event.guild_id,
                event.message_id,
                event.user_id,
                event.display_name.as_deref(),
                &event.emoji,
                event.is_bot,
            )
            .await
        {
            tracing::warn!(
                %err,
                guild_id = event.guild_id,
                channel_id = event.channel_id,
                message_id = event.message_id,
                user_id = event.user_id,
                "Reaction-Role-Add fehlgeschlagen"
            );
        }
    }

    async fn reaction_remove(
        &self,
        guild_id: u64,
        channel_id: u64,
        message_id: u64,
        user_id: u64,
        emoji: ReactionType,
        is_bot: bool,
    ) {
        if let Err(err) = self
            .service
            .handle_reaction_remove(guild_id, channel_id, message_id, user_id, &emoji, is_bot)
            .await
        {
            tracing::warn!(
                %err,
                guild_id,
                channel_id,
                message_id,
                user_id,
                "Reaction-Role-Remove fehlgeschlagen"
            );
        }
    }
}

// ── LFG-Lobby-Finder-Anbindung ─────────────────────────────────────────────

pub struct LfgGlue {
    pub adapter: Arc<DiscordAdapter>,
}

const LFG_CATEGORIES: [(u64, &str); 4] = [
    (1289721245281292290, "Casual"),
    (1412804540994162789, "Ranked"),
    (1357422957017698478, "Street Brawl"),
    (1465839366634209361, "New Player"),
];
const LFG_STAGINGS: [u64; 3] = [
    1501089974093873232,
    1412804671432818890,
    1357422958544420944,
];
const JUICE_KAMMER_ID: u64 = 1493690350580138114;
const OFFTOPIC_NAME_SUBSTRING: &str = "off topic voice";
const DISCORD_RANK_ROLES: [(u64, &str, i64); 12] = [
    (1331457571118387210, "Initiate", 1),
    (1331457652877955072, "Seeker", 2),
    (1331457699992436829, "Alchemist", 3),
    (1331457724848017539, "Arcanist", 4),
    (1331457879345070110, "Ritualist", 5),
    (1331457898781474836, "Emissary", 6),
    (1331457949654319114, "Archon", 7),
    (1316966867033653338, "Oracle", 8),
    (1331458016356208680, "Phantom", 9),
    (1331458049637875785, "Ascendant", 10),
    (1331458087349129296, "Eternus", 11),
    (1397687886580547745, "Unbekannt", 0),
];
const UNVERIFIED_RANK_ROLES: [(u64, &str, i64); 11] = [
    (1492959003700101180, "Eternus", 11),
    (1491935935414276198, "Ascendant", 10),
    (1492959474468655134, "Phantom", 9),
    (1492959889767534602, "Oracle", 8),
    (1492959936513052672, "Archon", 7),
    (1492960184920834110, "Emissary", 6),
    (1492960262184239178, "Ritualist", 5),
    (1492960274096066831, "Arcanist", 4),
    (1492960350755225730, "Alchemist", 3),
    (1492959966284218611, "Seeker", 2),
    (1492960891619250408, "Initiate", 1),
];
const RANK_SHORT_NAMES: [(&str, &str); 11] = [
    ("ini", "Initiate"),
    ("see", "Seeker"),
    ("alc", "Alchemist"),
    ("arc", "Arcanist"),
    ("rit", "Ritualist"),
    ("emi", "Emissary"),
    ("arch", "Archon"),
    ("ora", "Oracle"),
    ("pha", "Phantom"),
    ("asc", "Ascendant"),
    ("ete", "Eternus"),
];

fn rank_value_by_name(name: &str) -> Option<(&'static str, i64)> {
    let lower = name.trim().to_lowercase();
    dl_activity::lfg::RANK_NAMES
        .iter()
        .find(|(rank, _)| *rank == lower)
        .map(|(rank, value)| match *rank {
            "initiate" => ("Initiate", *value),
            "seeker" => ("Seeker", *value),
            "alchemist" => ("Alchemist", *value),
            "arcanist" => ("Arcanist", *value),
            "ritualist" => ("Ritualist", *value),
            "emissary" => ("Emissary", *value),
            "archon" => ("Archon", *value),
            "oracle" => ("Oracle", *value),
            "phantom" => ("Phantom", *value),
            "ascendant" => ("Ascendant", *value),
            "eternus" => ("Eternus", *value),
            _ => ("Unbekannt", 0),
        })
}

fn parse_subrank_role_name(role_name: &str) -> Option<(&'static str, i64, i64)> {
    let mut parts = role_name.split_whitespace();
    let rank_raw = parts.next()?;
    let sub = parts
        .next()
        .and_then(|raw| raw.trim_end_matches('+').parse::<i64>().ok())
        .filter(|value| (1..=6).contains(value))?;
    if parts.next().is_some() {
        return None;
    }
    let rank_lower = rank_raw.to_lowercase();
    let rank_name = RANK_SHORT_NAMES
        .iter()
        .find(|(short, _)| *short == rank_lower)
        .map(|(_, full)| *full)
        .unwrap_or(rank_raw);
    let (name, value) = rank_value_by_name(rank_name)?;
    Some((name, value, sub))
}

fn is_lfg_offtopic_channel(name: &str) -> bool {
    name.to_lowercase().contains(OFFTOPIC_NAME_SUBSTRING)
}

fn resolve_lane_label(
    channel_id: u64,
    name: &str,
    category_label: &str,
) -> dl_activity::lfg::LaneLabel {
    use dl_activity::lfg::LaneLabel;
    if channel_id == dl_voice::adaptive::NP_ANCHOR_CHANNEL_ID
        || name.starts_with(dl_voice::adaptive::NP_LANE_BASE_NAME)
    {
        return LaneLabel::NewPlayer;
    }
    match category_label {
        "Ranked" => LaneLabel::Ranked,
        "Street Brawl" => LaneLabel::StreetBrawl,
        "New Player" => LaneLabel::NewPlayer,
        _ => LaneLabel::Casual,
    }
}

fn visible_lfg_member_ids(members: &[(u64, bool)]) -> Vec<u64> {
    members
        .iter()
        .filter_map(|(user_id, is_bot)| (!*is_bot).then_some(*user_id))
        .collect()
}

fn rank_from_roles(roles: &[(u64, String)]) -> (String, i64, Option<i64>) {
    let mut best: (String, i64, Option<i64>, i64) = (String::new(), 0, None, -1);
    for (role_id, name) in roles {
        let mut candidate: Option<(&str, i64, Option<i64>, i64)> = None;
        if let Some((rank_name, value, sub)) = parse_subrank_role_name(name) {
            candidate = Some((rank_name, value, Some(sub), value * 10 + sub));
        }
        if candidate.is_none() {
            if let Some((_, rank_name, value)) =
                DISCORD_RANK_ROLES.iter().find(|(id, _, _)| id == role_id)
            {
                candidate = Some((*rank_name, *value, None, value * 10 + 5));
            }
        }
        if candidate.is_none() {
            if let Some((_, rank_name, value)) = UNVERIFIED_RANK_ROLES
                .iter()
                .find(|(id, _, _)| id == role_id)
            {
                candidate = Some((*rank_name, *value, Some(3), value * 10 + 3));
            }
        }
        if candidate.is_none() {
            let trimmed = name.trim();
            let lower = trimmed.to_lowercase();
            if lower.starts_with("unverifiziert ") {
                let rank_name = trimmed
                    .split_once(char::is_whitespace)
                    .map(|(_, rest)| rest.trim())
                    .unwrap_or_default();
                if let Some((rank_name, value)) = rank_value_by_name(rank_name) {
                    candidate = Some((rank_name, value, Some(3), value * 10 + 3));
                }
            }
        }
        let Some((rank_name, value, sub, score)) = candidate else {
            continue;
        };
        if score > best.3 {
            best = (rank_name.to_string(), value, sub, score);
        }
    }
    if best.1 == 0 {
        ("Unbekannt".to_string(), 0, None)
    } else {
        (best.0, best.1, best.2)
    }
}

#[async_trait::async_trait]
impl dl_activity::lfg::LfgPort for LfgGlue {
    async fn scan_lanes(
        &self,
        guild_id: u64,
        co_player_ids: &[u64],
    ) -> Vec<dl_activity::lfg::LaneInfo> {
        use dl_activity::lfg::{LaneInfo, LaneLabel};
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        let co_set: std::collections::HashSet<u64> = co_player_ids.iter().copied().collect();
        let mut lanes = Vec::new();
        for channel in guild.channels.values() {
            if channel.kind != serenity::all::ChannelType::Voice {
                continue;
            }
            if is_lfg_offtopic_channel(&channel.name) {
                continue;
            }
            let Some((category_id, label)) = channel.parent_id.and_then(|parent| {
                LFG_CATEGORIES
                    .iter()
                    .find(|(id, _)| *id == parent.get())
                    .copied()
            }) else {
                continue;
            };
            let label = resolve_lane_label(channel.id.get(), &channel.name, label);
            let voice_members: Vec<(u64, bool)> = guild
                .voice_states
                .iter()
                .filter(|(_, vs)| vs.channel_id == Some(channel.id))
                .filter_map(|(user_id, _)| {
                    guild
                        .members
                        .get(user_id)
                        .map(|member| (user_id.get(), member.user.bot))
                })
                .collect();
            let member_ids = visible_lfg_member_ids(&voice_members);
            let mut ranks: Vec<i64> = Vec::new();
            let mut co_names: Vec<String> = Vec::new();
            for user_id in &member_ids {
                if let Some(member) = guild.members.get(&UserId::new(*user_id)) {
                    let roles: Vec<(u64, String)> = member
                        .roles
                        .iter()
                        .filter_map(|rid| {
                            guild
                                .roles
                                .get(rid)
                                .map(|r| (rid.get(), r.name.to_string()))
                        })
                        .collect();
                    let (_, value, _) = rank_from_roles(&roles);
                    if value > 0 {
                        ranks.push(value);
                    }
                    if co_set.contains(user_id) {
                        co_names.push(member.display_name().to_string());
                    }
                }
            }
            let member_count = member_ids.len();
            let mut avg = if ranks.is_empty() {
                0.0
            } else {
                ranks.iter().sum::<i64>() as f64 / ranks.len() as f64
            };
            let avg_label = if channel.id.get() == JUICE_KAMMER_ID {
                avg = 11.0;
                "Eternus".to_string()
            } else if avg == 0.0 {
                "Leer".to_string()
            } else {
                let tier = (avg.round() as i64).clamp(1, 11);
                let mut name = dl_activity::lfg::RANK_NAMES
                    .iter()
                    .find(|(_, value)| *value == tier)
                    .map(|(rank, _)| rank.to_string())
                    .unwrap_or_else(|| "Unbekannt".to_string());
                if let Some(head) = name.get_mut(0..1) {
                    head.make_ascii_uppercase();
                }
                name
            };
            let mut limit = match channel.user_limit {
                Some(0) | None => 99,
                Some(value) => value as usize,
            };
            if label == LaneLabel::NewPlayer {
                limit = limit.min(6);
            }
            lanes.push(LaneInfo {
                channel_id: channel.id.get(),
                label,
                member_count,
                user_limit: limit,
                avg_rank_value: avg,
                co_players_present: co_names.len(),
                name: channel.name.to_string(),
                avg_rank_label: avg_label,
                category_id,
                position: channel.position as i64,
                is_staging: LFG_STAGINGS.contains(&channel.id.get()),
                co_player_names: co_names,
            });
        }
        lanes.sort_by_key(|lane| (lane.category_id, lane.position, lane.channel_id));
        lanes
    }

    async fn member_rank(&self, guild_id: u64, user_id: u64) -> (String, i64, Option<i64>) {
        let roles: Vec<(u64, String)> = self
            .adapter
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
            .unwrap_or_default();
        rank_from_roles(&roles)
    }

    async fn member_in_voice(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.voice_states
                    .get(&UserId::new(user_id))
                    .map(|vs| vs.channel_id.is_some())
            })
            .unwrap_or(false)
    }

    async fn post_embed(&self, channel_id: u64, embed: serde_json::Value) {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn post_text(&self, channel_id: u64, content: &str) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }
}

// ── Team-Bewerbungen-Anbindung ─────────────────────────────────────────────

pub struct TeamApplicationGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::team_applications::TeamApplicationPort for TeamApplicationGlue {
    async fn post_panel(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_panel(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn post_moderator_application(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_moderator_application(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), dl_community::team_applications::ModeratorEditError> {
        match self
            .adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(serenity::Error::Http(HttpError::UnsuccessfulRequest(response)))
                if response.status_code.as_u16() == 404 || response.error.code == 10008 =>
            {
                Err(dl_community::team_applications::ModeratorEditError::NotFound)
            }
            Err(error) => Err(dl_community::team_applications::ModeratorEditError::Other(
                error.to_string(),
            )),
        }
    }

    async fn delete_moderator_application(
        &self,
        channel_id: u64,
        message_id: u64,
    ) -> Result<(), String> {
        match self
            .adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("DSGVO-Löschung einer Team-Bewerbung"),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(serenity::Error::Http(HttpError::UnsuccessfulRequest(response)))
                if response.status_code.as_u16() == 404 || response.error.code == 10008 =>
            {
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    async fn send_dm(
        &self,
        user_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({"recipient_id": user_id.to_string()}))
            .await
            .map_err(|error| error.to_string())?;
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .map(|_| ())
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
                serenity::all::RoleId::new(role_id),
                Some(reason),
            )
            .await
            .map_err(|err| err.to_string())
    }
}

// ── Coaching-Anfragen-Anbindung ────────────────────────────────────────────

pub struct CoachingReqGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::coaching_requests::CoachingPort for CoachingReqGlue {
    async fn post_panel(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_panel(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn coach_member_ids(&self, guild_id: u64) -> Vec<u64> {
        use dl_community::coaching_requests::{COACH_ROLE_ID, OWNER_EXCLUDE_ID};
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.members
                    .values()
                    .filter(|m| {
                        !m.user.bot
                            && m.user.id.get() != OWNER_EXCLUDE_ID
                            && m.roles.iter().any(|r| r.get() == COACH_ROLE_ID)
                    })
                    .map(|m| m.user.id.get())
                    .collect()
            })
            .unwrap_or_default()
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

    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.display_name().to_string())
            })
            .unwrap_or_else(|| format!("User {user_id}"))
    }

    async fn member_is_admin(&self, guild_id: u64, user_id: u64) -> bool {
        let guild_id = GuildId::new(guild_id);
        let Some(guild) = self.adapter.cache().guild(guild_id) else {
            return false;
        };
        if guild.owner_id.get() == user_id {
            return true;
        }
        guild
            .members
            .get(&UserId::new(user_id))
            .map(|m| {
                m.roles.iter().any(|rid| {
                    guild
                        .roles
                        .get(rid)
                        .map(|r| r.permissions.administrator())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    async fn send_request_message(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
        files: Vec<dl_community::coaching_requests::RequestAttachment>,
    ) -> Result<u64, String> {
        // Multipart statt serenity: der Body trägt attachment://-Referenzen
        // (Banner + Logo), die nur als files[N]-Parts mitgeschickt werden.
        let payload_text =
            serde_json::to_string(&body).map_err(|err| format!("Coaching-Body: {err}"))?;
        let mut form = reqwest::multipart::Form::new().text("payload_json", payload_text);
        for (id, file) in files.into_iter().enumerate() {
            let part = reqwest::multipart::Part::bytes(file.bytes)
                .file_name(file.filename)
                .mime_str("image/png")
                .map_err(|err| err.to_string())?;
            form = form.part(format!("files[{id}]"), part);
        }
        let response = reqwest::Client::new()
            .post(format!(
                "https://discord.com/api/v10/channels/{channel_id}/messages"
            ))
            .header(reqwest::header::AUTHORIZATION, self.adapter.http.token())
            .multipart(form)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "Coaching-Anfrage POST fehlgeschlagen: HTTP {}: {}",
                status.as_u16(),
                text.chars().take(300).collect::<String>()
            ));
        }
        let parsed: Value =
            serde_json::from_str(&text).map_err(|err| format!("Antwort kein JSON: {err}"))?;
        parsed["id"]
            .as_str()
            .and_then(|id| id.parse::<u64>().ok())
            .ok_or_else(|| "Antwort ohne Message-ID".to_string())
    }

    async fn edit_request_message(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
        files: Vec<dl_community::coaching_requests::RequestAttachment>,
    ) {
        // Edit ersetzt alle Attachments: Banner + Logo müssen bei jedem
        // PATCH neu hoch, sonst wirft Discord die attachment://-Referenzen
        // aus den Komponenten und die Karte verliert Bilder.
        let payload_text = match serde_json::to_string(&body) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!(%err, "Coaching: Anfrage-Body nicht serialisierbar");
                return;
            }
        };
        let mut form = reqwest::multipart::Form::new().text("payload_json", payload_text);
        for (id, file) in files.into_iter().enumerate() {
            let Ok(part) = reqwest::multipart::Part::bytes(file.bytes)
                .file_name(file.filename)
                .mime_str("image/png")
            else {
                tracing::warn!("Coaching: MIME für Anhang konnte nicht gesetzt werden");
                return;
            };
            form = form.part(format!("files[{id}]"), part);
        }
        let response = match reqwest::Client::new()
            .patch(format!(
                "https://discord.com/api/v10/channels/{channel_id}/messages/{message_id}"
            ))
            .header(reqwest::header::AUTHORIZATION, self.adapter.http.token())
            .multipart(form)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(%err, channel_id, message_id, "Coaching: Edit-Request fehlgeschlagen");
                return;
            }
        };
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            tracing::warn!(
                status = status.as_u16(),
                body = %text.chars().take(300).collect::<String>(),
                channel_id,
                message_id,
                "Coaching: Anfrage-Nachricht konnte nicht aktualisiert werden"
            );
        }
    }

    async fn send_channel_text(&self, channel_id: u64, content: &str) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn send_dm(&self, user_id: u64, content: &str) -> bool {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return false;
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .is_ok()
    }

    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
        let _ = self
            .adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                serenity::all::RoleId::new(role_id),
                Some(reason),
            )
            .await;
    }

    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
        let _ = self
            .adapter
            .http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                serenity::all::RoleId::new(role_id),
                Some(reason),
            )
            .await;
    }

    async fn member_voice_channel_in_category(
        &self,
        guild_id: u64,
        user_id: u64,
        category_id: u64,
    ) -> Option<u64> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let channel_id = guild.voice_states.get(&UserId::new(user_id))?.channel_id?;
        let parent = guild.channels.get(&channel_id)?.parent_id?;
        (parent.get() == category_id).then(|| channel_id.get())
    }

    async fn send_dm_embed(&self, user_id: u64, embed: serde_json::Value) -> bool {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return false;
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .is_ok()
    }
}

pub struct ActivityBackfillGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn unix_to_db_ts(ts: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

#[async_trait::async_trait]
impl dl_activity::analyzer::MemberBackfillPort for ActivityBackfillGlue {
    async fn cache_ready(&self) -> bool {
        self.adapter
            .gateway_ready
            .load(std::sync::atomic::Ordering::Relaxed)
            && !self.adapter.cache().guilds().is_empty()
    }

    async fn current_members(&self) -> Vec<dl_activity::analyzer::BackfillMember> {
        let mut out = Vec::new();
        for guild_id in self.adapter.cache().guilds() {
            let Some(guild) = self.adapter.cache().guild(guild_id) else {
                continue;
            };
            for member in guild.members.values() {
                out.push(dl_activity::analyzer::BackfillMember {
                    guild_id: guild_id.get(),
                    user_id: member.user.id.get(),
                    display_name: member.display_name().to_string(),
                    joined_at: member
                        .joined_at
                        .and_then(|ts| unix_to_db_ts(ts.unix_timestamp())),
                    account_created_at: unix_to_db_ts(member.user.id.created_at().unix_timestamp()),
                    is_bot: member.user.bot,
                });
            }
        }
        out
    }
}

// ── Retention-Miss-You-Anbindung ───────────────────────────────────────────

pub struct RetentionGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::retention::RetentionPort for RetentionGlue {
    async fn guild_member_count(&self, guild_id: u64) -> Option<usize> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|guild| guild.members.len())
    }

    async fn is_guild_member(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .is_some_and(|guild| guild.members.contains_key(&UserId::new(user_id)))
    }

    async fn member_info(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Option<dl_community::retention::RetentionMember> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let member = guild.members.get(&UserId::new(user_id))?;
        Some(dl_community::retention::RetentionMember {
            display_name: member.display_name().to_string(),
            role_ids: member.roles.iter().map(|r| r.get()).collect(),
        })
    }

    async fn fetch_user_name(&self, user_id: u64) -> Option<String> {
        let user = self
            .adapter
            .http
            .get_user(UserId::new(user_id))
            .await
            .ok()?;
        // Discord-Präzedenz: global_name vor Username (wie resolve_user).
        Some(
            user.global_name
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| user.name.to_string()),
        )
    }

    async fn guild_label(&self, guild_id: u64) -> (String, Option<String>) {
        match self.adapter.cache().guild(GuildId::new(guild_id)) {
            Some(g) => (g.name.to_string(), g.icon_url()),
            // Python-Fallback, wenn die Gilde nicht im Cache ist.
            None => ("unserem Server".to_string(), None),
        }
    }

    async fn send_miss_you_dm(
        &self,
        user_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> dl_community::retention::MissYouDelivery {
        use dl_community::retention::MissYouDelivery;
        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) => return MissYouDelivery::Failed(err.to_string()),
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => MissYouDelivery::Sent,
            // 50007 = Cannot send messages to this user (DMs deaktiviert).
            Err(err)
                if err.contains("50007")
                    && !err.to_ascii_lowercase().contains("no mutual guild") =>
            {
                MissYouDelivery::Blocked
            }
            Err(err) => MissYouDelivery::Failed(err),
        }
    }

    async fn send_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_community::retention::LOG_CHANNEL_ID, &body)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    use dl_answer::Retriever as _;

    #[test]
    fn np_lane_in_chill_wird_als_new_player_gelabelt() {
        use dl_activity::lfg::LaneLabel;

        assert_eq!(
            resolve_lane_label(999, "🆕Neue Spieler Lane 2", "Casual"),
            LaneLabel::NewPlayer
        );
        assert_eq!(
            resolve_lane_label(
                dl_voice::adaptive::NP_ANCHOR_CHANNEL_ID,
                "🆕Neue Spieler Lane",
                "Casual"
            ),
            LaneLabel::NewPlayer
        );
        assert_eq!(
            resolve_lane_label(999, "Chill Lane 1", "Casual"),
            LaneLabel::Casual
        );
        assert_eq!(
            resolve_lane_label(999, "Ranked Seeker 1", "Ranked"),
            LaneLabel::Ranked
        );
    }

    #[test]
    fn lfg_freitext_rueckfrage_antwortet_ohne_mentions_auf_die_quellnachricht() {
        let body = lfg_freetext_question_body(555, "PLATZHALTER");

        assert_eq!(body["content"], "PLATZHALTER");
        assert_eq!(body["message_reference"]["message_id"], "555");
        assert_eq!(body["allowed_mentions"]["parse"], json!([]));
        assert_eq!(body["allowed_mentions"]["replied_user"], false);
    }

    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    #[derive(Default)]
    struct FakeReviewActions;

    #[async_trait::async_trait]
    impl AimodReviewActions for FakeReviewActions {
        async fn accept_case(&self, _case_id: &str, _mod_id: u64) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("accepted".to_string())
        }

        async fn ban_case(&self, _case_id: &str, _mod_id: u64) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("banned".to_string())
        }

        async fn deny_case(
            &self,
            _case_id: &str,
            _mod_id: u64,
            _reason: &str,
        ) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("denied".to_string())
        }

        async fn untimeout_case(
            &self,
            _case_id: &str,
            _mod_id: u64,
        ) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("untimeout".to_string())
        }

        async fn unban_case(&self, _case_id: &str, _mod_id: u64) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("unban".to_string())
        }
    }

    struct CountingBrainAnswerer {
        calls: Arc<AtomicUsize>,
        retrieval_calls: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl dl_brain::AiAnswerer for CountingBrainAnswerer {
        async fn answer_for_discord_with_context(
            &self,
            question: &str,
            context: &dl_brain::DiscordQueryContext,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            assert!(context.allow_discord_reads);
            self.answer_for_discord(question, context.user_id).await
        }

        async fn answer(
            &self,
            _question: &str,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            panic!("Discord-Kommandos dürfen keinen anonymen Antwortweg verwenden")
        }

        fn answer_for_discord<'life0, 'life1, 'async_trait>(
            &'life0 self,
            _question: &'life1 str,
            user_id: u64,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<dl_brain::BrainOutcome, dl_brain::BrainError>,
                    > + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                assert_ne!(user_id, 0);
                self.calls.fetch_add(1, Ordering::Relaxed);
                self.retrieval_calls.fetch_add(1, Ordering::Relaxed);
                Ok(dl_brain::BrainOutcome::Answer("Antwort".into()))
            })
        }
    }

    fn public_test_adapter() -> Arc<DiscordAdapter> {
        let adapter = dl_discord::DiscordAdapter::new("test-token");
        let mut guild = serenity::all::Guild::default();
        guild.id = GuildId::new(1);
        let mut role = serenity::all::Role::default();
        role.id = RoleId::new(1);
        role.permissions = Permissions::VIEW_CHANNEL;
        guild.roles.insert(role.id, role);
        for id in [1, 999_999] {
            let mut channel = serenity::all::GuildChannel::default();
            channel.id = ChannelId::new(id);
            channel.guild_id = guild.id;
            channel.kind = serenity::all::ChannelType::Text;
            guild.channels.insert(channel.id, channel);
        }
        let cache = Arc::new(serenity::all::Cache::new());
        let mut event: serenity::all::GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(guild).expect("Testguild"))
                .expect("Testcacheereignis");
        cache.update(&mut event);
        adapter.link_cache(cache);
        adapter
    }

    #[test]
    fn ortskontext_beachtet_sichtrechte_und_vererbt_nichts_in_dms() {
        let adapter = public_test_adapter();
        let mut guild = adapter
            .cache()
            .guild(GuildId::new(1))
            .expect("Testguild im Cache")
            .clone();
        guild
            .roles
            .get_mut(&RoleId::new(1))
            .expect("Everyone-Testrolle")
            .permissions = Permissions::VIEW_CHANNEL;
        let mut member = serenity::all::Member::default();
        member.user.id = UserId::new(3);
        guild.members.insert(member.user.id, member);
        let mut category = serenity::all::GuildChannel::default();
        category.id = ChannelId::new(5);
        category.guild_id = guild.id;
        category.kind = serenity::all::ChannelType::Category;
        category.name = "Community".into();
        guild.channels.insert(category.id, category);
        let channel = guild
            .channels
            .get_mut(&ChannelId::new(1))
            .expect("Testkanal");
        channel.name = "Hilfe".into();
        channel.topic = Some("Fragen zum Server".into());
        channel.parent_id = Some(ChannelId::new(5));
        let mut update: serenity::all::GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(guild).expect("Testguild serialisieren"))
                .expect("Testguild-Ereignis");
        adapter.cache().update(&mut update);
        let context = brain_answer_context(
            &adapter,
            Some(1),
            1,
            3,
            dl_brain::AnswerInputKind::SlashCommand,
        );
        assert_eq!(context.channel_name.as_deref(), Some("Hilfe"));
        assert_eq!(context.category_name.as_deref(), Some("Community"));
        assert_eq!(context.topic.as_deref(), Some("Fragen zum Server"));
        let unknown =
            brain_answer_context(&adapter, Some(1), 1, 4, dl_brain::AnswerInputKind::Message);
        assert!(unknown.channel_name.is_none());
        assert!(unknown.category_name.is_none());
        let dm = brain_answer_context(&adapter, None, 1, 3, dl_brain::AnswerInputKind::Message);
        assert_eq!(dm.is_direct_message, Some(true));
        assert!(dm.channel_name.is_none());
        assert!(dm.category_name.is_none());
        assert!(dm.topic.is_none());
        for (kind, overwrite) in [
            (
                serenity::all::ChannelType::Text,
                Some(PermissionOverwriteType::Member(UserId::new(3))),
            ),
            (
                serenity::all::ChannelType::Text,
                Some(PermissionOverwriteType::Role(RoleId::new(1))),
            ),
            (serenity::all::ChannelType::PrivateThread, None),
        ] {
            let mut guild = adapter
                .cache()
                .guild(GuildId::new(1))
                .expect("Testguild im Cache")
                .clone();
            let channel = guild
                .channels
                .get_mut(&ChannelId::new(1))
                .expect("Testkanal");
            channel.kind = kind;
            channel.permission_overwrites = overwrite
                .map(|kind| serenity::all::PermissionOverwrite {
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                    kind,
                })
                .into_iter()
                .collect();
            let mut update: serenity::all::GuildCreateEvent = serde_json::from_value(
                serde_json::to_value(guild).expect("Testguild serialisieren"),
            )
            .expect("Testguild-Ereignis");
            adapter.cache().update(&mut update);
            let denied =
                brain_answer_context(&adapter, Some(1), 1, 3, dl_brain::AnswerInputKind::Message);
            assert!(denied.channel_name.is_none());
            assert!(denied.category_name.is_none());
            assert!(denied.topic.is_none());
            assert!(denied.thread_name.is_none());
        }
        println!("Ortskontext: Hilfe / Community, Lesesperren, Threads und DM ohne Serverfelder");
    }

    fn test_brain_handler(
        channel_allowlist: Option<HashSet<u64>>,
        retriever_calls: Arc<AtomicUsize>,
        answerer_calls: Arc<AtomicUsize>,
    ) -> BrainHandler {
        BrainHandler {
            adapter: public_test_adapter(),
            config: Arc::new(dl_brain::BrainConfig {
                max_question_len: 300,
                cooldown_secs: 20,
            }),
            cooldowns: Arc::new(dl_brain::BrainCooldowns::default()),
            answerer: Arc::new(CountingBrainAnswerer {
                calls: answerer_calls,
                retrieval_calls: retriever_calls,
            }),
            channel_allowlist,
            all_guild_channels: false,
            emoji_index: Arc::new(BrainEmojiIndex::default()),
            guide_pending: Default::default(),
            conversations: Default::default(),
        }
    }

    fn test_message_event(guild_id: Option<u64>, content: &str) -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id,
            channel_id: 1,
            message_id: 2,
            author_id: 3,
            author_display_name: "user".to_string(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.to_string(),
            message_created_at: chrono::DateTime::from_timestamp(0, 0).expect("fixture timestamp"),
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 0,
            author_joined_at: None,
        }
    }

    #[derive(Default)]
    struct RecordingDiscordAnswerer {
        calls: Mutex<Vec<(String, u64)>>,
        histories: Mutex<Vec<Vec<String>>>,
        read_access: Mutex<Vec<bool>>,
        legacy_calls: AtomicUsize,
        outcome: Option<dl_brain::BrainOutcome>,
    }

    #[async_trait::async_trait]
    impl dl_brain::AiAnswerer for RecordingDiscordAnswerer {
        async fn answer_for_discord_with_history(
            &self,
            question: &str,
            context: &dl_brain::DiscordQueryContext,
            history: &[String],
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            self.histories.lock().await.push(history.to_vec());
            self.answer_for_discord_with_context(question, context)
                .await
        }

        async fn answer(
            &self,
            _question: &str,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            self.legacy_calls.fetch_add(1, Ordering::Relaxed);
            Ok(dl_brain::BrainOutcome::Answer("Altweg".into()))
        }

        async fn answer_for_discord(
            &self,
            question: &str,
            user_id: u64,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            self.calls.lock().await.push((question.to_owned(), user_id));
            Ok(self
                .outcome
                .clone()
                .unwrap_or_else(|| dl_brain::BrainOutcome::Answer(format!("Antwort: {question}"))))
        }

        async fn answer_for_discord_with_read_access(
            &self,
            question: &str,
            user_id: u64,
            allow_discord_reads: bool,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            self.read_access.lock().await.push(allow_discord_reads);
            self.answer_for_discord(question, user_id).await
        }
    }

    type RecordedBrainReply = (u64, u64, Map<String, Value>);

    #[derive(Default)]
    struct RecordingBrainReplies {
        checks: AtomicUsize,
        revoke_after_first: bool,
        deny: bool,
        bot: bool,
        fail: bool,
        private: bool,
        classification: Option<Arc<DiscordAdapter>>,
        sent: Mutex<Vec<RecordedBrainReply>>,
        reactions: Mutex<Vec<(u64, u64, String, bool)>>,
        fail_reactions: bool,
        delivered: tokio::sync::Notify,
    }

    #[async_trait::async_trait]
    impl BrainDirectReplyPort for RecordingBrainReplies {
        async fn is_human(&self, _event: &dl_discord::MessageEvent) -> bool {
            !self.bot
        }

        async fn can_reply(&self, _event: &dl_discord::MessageEvent) -> bool {
            let previous = self.checks.fetch_add(1, Ordering::Relaxed);
            !self.deny && (!self.revoke_after_first || previous == 0)
        }

        fn is_public(&self, event: &dl_discord::MessageEvent) -> bool {
            self.classification.as_ref().map_or_else(
                || event.guild_id.is_some() && !self.private,
                |adapter| adapter.is_public(event),
            )
        }

        async fn reaction(
            &self,
            event: &dl_discord::MessageEvent,
            emoji: &str,
            present: bool,
        ) -> Result<(), String> {
            self.reactions.lock().await.push((
                event.channel_id,
                event.message_id,
                emoji.to_owned(),
                present,
            ));
            if self.fail_reactions {
                Err("Keine Berechtigung für Reaktionen".into())
            } else {
                Ok(())
            }
        }

        async fn reply(
            &self,
            event: &dl_discord::MessageEvent,
            body: &Map<String, Value>,
        ) -> Result<u64, String> {
            if self.fail {
                return Err("Zustellung fehlgeschlagen".into());
            }
            let mut sent = self.sent.lock().await;
            sent.push((event.channel_id, event.message_id, body.clone()));
            let message_id = 1000 + sent.len() as u64;
            self.delivered.notify_one();
            Ok(message_id)
        }
    }

    fn direct_test_handler() -> (BrainHandler, Arc<RecordingDiscordAnswerer>) {
        let answerer = Arc::new(RecordingDiscordAnswerer::default());
        let mut handler = test_brain_handler(
            None,
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        );
        handler.answerer = answerer.clone();
        (handler, answerer)
    }

    #[tokio::test]
    async fn abrams_spirit_dreischritt_ueber_denselben_brain_eingang() {
        use axum::{extract::Json, http::HeaderMap, routing::post, Router};
        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = received.clone();
        let live_endpoint = std::env::var("DISCORD_BRAIN_PROBE_ENDPOINT").ok();
        let live = live_endpoint.is_some();
        let mut server = None;
        let (endpoint, token, timeout) = if let Some(endpoint) = live_endpoint {
            (
                endpoint,
                std::env::var("DISCORD_BRAIN_CLIENT_TOKEN").expect("Injizierter Probe-Zugang"),
                Duration::from_millis(
                    std::env::var("DISCORD_BRAIN_PROBE_TIMEOUT_MS")
                        .expect("Probe-Frist")
                        .parse()
                        .expect("Probe-Frist in Millisekunden"),
                ),
            )
        } else {
            let app = Router::new().route("/v1/answer", post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let captured = captured.clone();
                async move {
                    let q = body.get("query").unwrap_or(&body);
                    let response = json!({"contract_version": "brain.public.v1", "request_id": q["request_id"],
                        "knowledge_release": "test-release", "status": "insufficient_evidence",
                        "text": "Ungeprüft: Für Abrams im Spirit-Build passt **Spirit-Item**.", "citations": []});
                    captured.lock().await.push((headers, body));
                    Json(response)
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("Lokaler Probe-Port");
            let endpoint = format!("http://{}", listener.local_addr().expect("Probe-Adresse"));
            server = Some(tokio::spawn(async move {
                axum::serve(listener, app).await.expect("Probe-Server")
            }));
            (
                endpoint,
                "fixture-brain-bearer".to_owned(),
                Duration::from_secs(3),
            )
        };
        let (mut handler, _) = direct_test_handler();
        handler.answerer = Arc::new(
            dl_brain::brain_api::BrainApiAnswerer::new(
                &endpoint,
                &token,
                timeout,
                "discord-synthetic-direct-probe".into(),
                std::collections::BTreeSet::from(["bot.public".into()]),
            )
            .expect("Probe-Facade"),
        );
        let replies = RecordingBrainReplies::default();
        let first = test_message_event(Some(1), "<@42> Welche Items passen zu Abrams?");
        handler
            .handle_message_event_with_replies(&first, 42, &replies)
            .await;
        let mut second = test_message_event(Some(1), "Okay ja ne Idee für ein Spirit build");
        second.message_id = 4;
        second.is_reply = true;
        second.reply_message_id = Some(1001);
        second.reply_channel_id = Some(second.channel_id);
        handler
            .handle_message_event_with_replies(&second, 42, &replies)
            .await;
        let mut third = test_message_event(Some(1), "Abrams");
        third.message_id = 5;
        handler
            .handle_message_event_with_replies(&third, 42, &replies)
            .await;
        let sent = replies.sent.lock().await;
        assert_eq!(sent.len(), 3);
        for (index, (_, _, body)) in sent.iter().enumerate() {
            let answer = body["content"]
                .as_str()
                .expect("Synthetische Brain-Antwort");
            assert!(!answer.starts_with("Ungeprüft:"));
            if index > 0 {
                assert!(
                    !answer.to_lowercase().contains("welchen helden"),
                    "{answer}"
                );
                assert!(!answer.contains("Worauf beziehst"), "{answer}");
                assert!(answer.to_lowercase().contains("spirit"), "{answer}");
                assert!(answer.to_lowercase().contains("abrams"), "{answer}");
                assert!(answer.contains("**"), "{answer}");
            }
            println!(
                "{}",
                json!({"case": "synthetic_abrams_spirit", "step": index + 1,
                "live": live, "answer": answer})
            );
        }
        if !live {
            let received = received.lock().await;
            assert_eq!(received.len(), 3);
            assert_eq!(received[0].1["text"], "Welche Items passen zu Abrams?");
            assert_eq!(received[1].1["query"]["text"], second.content);
            assert_eq!(received[1].1["user_questions"], json!([first.content]));
            assert_eq!(received[2].1["query"]["text"], "Abrams");
            assert_eq!(
                received[2].1["user_questions"],
                json!([first.content, second.content])
            );
            for (headers, body) in &received[1..] {
                assert_eq!(headers["x-discord-read-access"], "disabled");
                assert_eq!(headers["x-discord-user-id"], "3");
                assert!(headers.get("x-discord-answer-task").is_none());
                assert!(body["query"]["answer_context"].is_null());
            }
        }
        if let Some(server) = server {
            server.abort();
        }
    }

    #[tokio::test]
    async fn brain_reaktion_steht_vor_antwort_und_verschwindet_danach() {
        let (mut handler, _) = direct_test_handler();
        let answerer = Arc::new(PendingGuideAnswerer::default());
        handler.answerer = answerer.clone();
        let replies = RecordingBrainReplies::default();
        let event = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
        let handle = handler.handle_message_event_with_replies(&event, 42, &replies);
        let verify = async {
            answerer.started.notified().await;
            assert_eq!(
                *replies.reactions.lock().await,
                vec![(event.channel_id, event.message_id, "👀".into(), true)]
            );
            assert!(replies.sent.lock().await.is_empty());
            answerer.release.notify_one();
        };
        tokio::join!(handle, verify);
        assert_eq!(replies.sent.lock().await.len(), 1);
        assert_eq!(
            *replies.reactions.lock().await,
            vec![
                (event.channel_id, event.message_id, "👀".into(), true),
                (event.channel_id, event.message_id, "👀".into(), false),
            ]
        );
    }

    #[tokio::test]
    async fn brain_reaktion_unterscheidet_fehler_von_fachlicher_leermeldung() {
        for (outcome, fail, revoke, fail_reactions, expected_error) in [
            (
                dl_brain::BrainOutcome::BackendError,
                false,
                false,
                false,
                true,
            ),
            (dl_brain::BrainOutcome::NoAnswer, false, false, false, false),
            (
                dl_brain::BrainOutcome::OutOfDomain,
                false,
                false,
                false,
                false,
            ),
            (dl_brain::BrainOutcome::NoAnswer, true, false, false, true),
            (dl_brain::BrainOutcome::NoAnswer, false, true, false, true),
            (dl_brain::BrainOutcome::NoAnswer, false, false, true, false),
        ] {
            let (mut handler, _) = direct_test_handler();
            handler.answerer = Arc::new(RecordingDiscordAnswerer {
                outcome: Some(outcome),
                ..Default::default()
            });
            let replies = RecordingBrainReplies {
                fail,
                revoke_after_first: revoke,
                fail_reactions,
                ..Default::default()
            };
            let event = test_message_event(Some(1), "<@42> Frage");
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            let mut expected = vec![
                (event.channel_id, event.message_id, "👀".into(), true),
                (event.channel_id, event.message_id, "👀".into(), false),
            ];
            if expected_error {
                expected.push((event.channel_id, event.message_id, "❌".into(), true));
            }
            assert_eq!(*replies.reactions.lock().await, expected);
            assert_eq!(
                replies.sent.lock().await.len(),
                usize::from(!fail && !revoke)
            );
        }
    }

    #[tokio::test]
    async fn brain_reaktion_markiert_folgefragen_aber_keine_abbrechbare_guide_hilfe() {
        let (handler, _) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let event = test_message_event(Some(1), "Und welche Items passen dazu?");
        handler
            .conversations
            .lock()
            .await
            .record(&event, 999, Instant::now());
        handler
            .handle_message_event_with_replies(&event, 42, &replies)
            .await;
        assert_eq!(
            *replies.reactions.lock().await,
            vec![
                (event.channel_id, event.message_id, "👀".into(), true),
                (event.channel_id, event.message_id, "👀".into(), false),
            ]
        );
        let replies = RecordingBrainReplies::default();
        handler
            .handle_message_event_with_replies(
                &guide_test_event("Welche Lanes gibt es?"),
                42,
                &replies,
            )
            .await;
        assert_eq!(replies.sent.lock().await.len(), 1);
        assert!(replies.reactions.lock().await.is_empty());
    }

    #[tokio::test]
    async fn gespraech_nackte_erwaehnung_fragt_einmal_und_nimmt_folgefrage_an() {
        let (handler, answerer) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let mention = test_message_event(Some(1), " <@!42> ");
        handler
            .handle_message_event_with_replies(&mention, 42, &replies)
            .await;
        assert!(answerer.calls.lock().await.is_empty());
        assert_eq!(
            replies.sent.lock().await[0].2["content"],
            "Was möchtest du wissen?"
        );
        assert_eq!(handler.conversations.lock().await.entries.len(), 1);

        let empty = test_message_event(Some(1), " \n ");
        handler
            .handle_message_event_with_replies(&empty, 42, &replies)
            .await;
        assert_eq!(replies.sent.lock().await.len(), 1);

        let question = test_message_event(Some(1), "Welche Lanes gibt es?");
        handler
            .handle_message_event_with_replies(&question, 42, &replies)
            .await;
        assert_eq!(
            *answerer.calls.lock().await,
            vec![(question.content.clone(), 3)]
        );
        let sent = replies.sent.lock().await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].2["content"], "Antwort: Welche Lanes gibt es?");
        assert_eq!(sent[1].2["message_reference"]["message_id"], "2");
    }

    #[tokio::test]
    async fn gespraech_fortsetzung_und_reply_nutzen_denselben_antwortweg() {
        for reply in [false, true] {
            let (handler, answerer) = direct_test_handler();
            let replies = RecordingBrainReplies::default();
            let mention = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
            handler
                .handle_message_event_with_replies(&mention, 42, &replies)
                .await;
            assert_eq!(
                replies.sent.lock().await[0].2["content"],
                "Antwort: Welche Lanes gibt es?"
            );
            let mut followup = test_message_event(Some(1), "Und für neue Spieler?");
            followup.message_id = 4;
            if reply {
                followup.is_reply = true;
                followup.reply_message_id = Some(1001);
                followup.reply_channel_id = Some(followup.channel_id);
            }
            handler
                .handle_message_event_with_replies(&followup, 42, &replies)
                .await;
            assert_eq!(
                *answerer.calls.lock().await,
                vec![
                    ("Welche Lanes gibt es?".into(), 3),
                    ("Und für neue Spieler?".into(), 3),
                ]
            );
            assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
            assert_eq!(
                *answerer.histories.lock().await,
                vec![Vec::<String>::new(), vec![mention.content.clone()]]
            );
            let sent = replies.sent.lock().await;
            assert_eq!(sent.len(), 2);
            assert_eq!(sent[1].2["message_reference"]["message_id"], "4");
            assert_eq!(
                sent[1].2["allowed_mentions"],
                json!({"parse": [], "replied_user": false})
            );
            assert_eq!(
                handler.conversations.lock().await.entries[&(1, 1, 3)]
                    .turns
                    .last()
                    .expect("Letzter Gesprächszug")
                    .0,
                1002
            );
        }
    }

    #[tokio::test]
    async fn mehrere_erwaehnungen_bleiben_eine_anfrage_und_folgefragen_erreichen_dasselbe_tageslimit(
    ) {
        let (handler, answerer) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let event = test_message_event(
            Some(1),
            "<@42>Welche Lanes gibt es?\n<@!42>Welche Items passen?\n<@42>Wie spiele ich Abrams?",
        );
        let expected = "Welche Lanes gibt es?\n Welche Items passen?\n Wie spiele ich Abrams?";
        handler
            .handle_message_event_with_replies(&event, 42, &replies)
            .await;
        assert_eq!(*answerer.calls.lock().await, vec![(expected.to_owned(), 3)]);
        let mut followup = test_message_event(Some(1), "Und danach?");
        for index in 2..=50 {
            followup.message_id = index + 2;
            handler
                .handle_message_event_with_replies(&followup, 42, &replies)
                .await;
        }
        assert_eq!(answerer.calls.lock().await.len(), 50);
        let last = handler.conversations.lock().await.entries[&(1, 1, 3)].last_reply;
        for _ in 0..5 {
            handler
                .handle_message_event_with_replies(&followup, 42, &replies)
                .await;
        }
        assert_eq!(answerer.calls.lock().await.len(), 50);
        assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
        let sent = replies.sent.lock().await;
        assert_eq!(sent.len(), 51);
        assert_eq!(sent[50].2["content"], BRAIN_DAILY_LIMIT);
        assert_eq!(
            sent[50].2["message_reference"]["message_id"],
            followup.message_id.to_string()
        );
        assert_eq!(
            handler.conversations.lock().await.entries[&(1, 1, 3)].last_reply,
            last
        );
    }

    #[tokio::test]
    async fn gespraech_fremde_nutzer_kanaele_und_replies_brauchen_erwaehnung() {
        let (handler, answerer) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let mention = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
        handler
            .handle_message_event_with_replies(&mention, 42, &replies)
            .await;
        let question = test_message_event(Some(1), "Und für neue Spieler?");
        let mut cases = Vec::new();
        let mut other = question.clone();
        other.author_id = 4;
        cases.push(other.clone());
        other.is_reply = true;
        other.reply_message_id = Some(1001);
        cases.push(other);
        let mut event = question.clone();
        event.channel_id = 5;
        cases.push(event);
        let mut event = question.clone();
        event.guild_id = Some(2);
        cases.push(event);
        let mut event = question.clone();
        event.is_reply = true;
        event.reply_message_id = Some(999);
        cases.push(event);
        let mut event = question.clone();
        event.is_reply = true;
        cases.push(event);
        let mut event = question;
        event.reply_message_id = Some(1001);
        event.reply_channel_id = Some(5);
        cases.push(event);
        for event in cases {
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
        }
        assert_eq!(answerer.calls.lock().await.len(), 1);
        assert_eq!(replies.sent.lock().await.len(), 1);
        let mut own_mention = test_message_event(Some(1), "<@42> Meine eigene Frage");
        own_mention.author_id = 4;
        handler
            .handle_message_event_with_replies(&own_mention, 42, &replies)
            .await;
        assert_eq!(
            answerer.calls.lock().await[1],
            ("Meine eigene Frage".into(), 4)
        );
    }

    #[test]
    fn eigener_kanalverlauf_bleibt_begrenzt_und_reply_waehlt_nur_seinen_zweig() {
        let now = Instant::now();
        let mut conversations = BrainConversations::default();
        let mut event = test_message_event(Some(1), "");
        for index in 0..6 {
            event.content = format!("Frage {index} {}", "ä".repeat(900));
            conversations.record(&event, 1000 + index, now);
        }
        let history = conversations.history(&event, now).expect("Eigener Verlauf");
        assert_eq!(history.len(), 4);
        assert!(
            history
                .iter()
                .map(|question| question.chars().count())
                .sum::<usize>()
                <= 4000
        );
        event.is_reply = true;
        event.reply_message_id = Some(1003);
        let branch = conversations
            .history(&event, now)
            .expect("Eigene frühere Botantwort");
        assert_eq!(branch, history[..2]);
        event.content = "Neuer Gesprächszweig".into();
        conversations.record(&event, 1010, now);
        event.is_reply = false;
        event.reply_message_id = None;
        assert_eq!(
            conversations.history(&event, now).expect("Eigener Zweig"),
            [branch, vec![event.content.clone()]].concat()
        );
        event.author_id = 99;
        assert!(conversations.history(&event, now).is_none());
        event.author_id = 3;
        event.channel_id = 99;
        assert!(conversations.history(&event, now).is_none());
        event.channel_id = 1;
        event.is_reply = true;
        event.reply_message_id = Some(1000);
        assert!(conversations.history(&event, now).is_none());
    }

    #[test]
    fn gespraech_zehn_minuten_grenze_und_letzte_botantwort() {
        let now = Instant::now();
        let event = test_message_event(Some(1), "Folgefrage");
        let mut conversations = BrainConversations::default();
        conversations.record(&event, 1001, now);
        assert_eq!(
            conversations.question(&event, now + Duration::from_secs(599)),
            Some("Folgefrage".into())
        );
        conversations.record(&event, 1002, now + Duration::from_secs(599));
        assert!(conversations
            .question(&event, now + Duration::from_secs(600))
            .is_some());
        let mut reply = event.clone();
        reply.is_reply = true;
        reply.reply_message_id = Some(1001);
        assert!(conversations
            .question(&reply, now + Duration::from_secs(600))
            .is_some());
        assert_eq!(
            conversations.history(&reply, now + Duration::from_secs(600)),
            Some(vec!["Folgefrage".to_owned()])
        );
        reply.reply_message_id = Some(1002);
        assert!(conversations
            .question(&reply, now + Duration::from_secs(600))
            .is_some());
        assert!(conversations
            .question(&event, now + Duration::from_secs(1199))
            .is_none());
        assert!(conversations.entries.is_empty());
        assert!(conversations
            .question(&reply, now + Duration::from_secs(1200))
            .is_none());
    }

    #[tokio::test]
    async fn gespraech_nach_ablauf_braucht_auch_reply_wieder_erwaehnung() {
        for reply in [false, true] {
            let (handler, answerer) = direct_test_handler();
            let replies = RecordingBrainReplies::default();
            let mut event = test_message_event(Some(1), "Folgefrage");
            handler.conversations.lock().await.record(
                &event,
                1001,
                Instant::now() - BRAIN_CONVERSATION_TTL,
            );
            if reply {
                event.is_reply = true;
                event.reply_message_id = Some(1001);
            }
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            assert!(answerer.calls.lock().await.is_empty());
            assert!(replies.sent.lock().await.is_empty());
            event.content = "<@42> Neue Frage".into();
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            assert_eq!(answerer.calls.lock().await.len(), 1);
            assert_eq!(replies.sent.lock().await.len(), 1);
        }
    }

    #[tokio::test]
    async fn gespraech_zustellfehler_rechte_und_nutzerlimit_verlaengern_nicht() {
        for (fail, deny, revoke, limited) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            let (mut handler, _) = direct_test_handler();
            let event = test_message_event(Some(1), "<@42> Frage");
            if limited {
                handler.cooldowns = Arc::new(dl_brain::BrainCooldowns::new(1));
                let prior_replies = RecordingBrainReplies::default();
                for _ in 0..2 {
                    handler
                        .handle_message_event_with_replies(&event, 42, &prior_replies)
                        .await;
                }
            }
            let last = Instant::now() - Duration::from_secs(120);
            handler.conversations.lock().await.record(&event, 999, last);
            let replies = RecordingBrainReplies {
                fail,
                deny,
                revoke_after_first: revoke,
                ..Default::default()
            };
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            assert!(replies.sent.lock().await.is_empty());
            let conversations = handler.conversations.lock().await;
            let conversation = &conversations.entries[&(1, 1, 3)];
            assert_eq!(conversation.last_reply, last);
            assert_eq!(
                conversation
                    .turns
                    .last()
                    .expect("Vorhandener Gesprächszug")
                    .0,
                999
            );
        }
    }

    #[tokio::test]
    async fn gespraech_dm_guide_und_brain_befehl_bleiben_getrennt() {
        let (handler, _) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        handler
            .handle_message_event_with_replies(&test_message_event(None, "Frage"), 42, &replies)
            .await;
        assert!(handler.conversations.lock().await.entries.is_empty());
        let event = test_message_event(Some(1), "!brain Folgefrage");
        handler
            .conversations
            .lock()
            .await
            .record(&event, 1001, Instant::now());
        assert!(handler.conversation_question(&event, 42).await.is_none());
        let (handler, _) = direct_test_handler();
        handler
            .handle_message_event_with_replies(
                &guide_test_event("Welche Lanes gibt es?"),
                42,
                &RecordingBrainReplies::default(),
            )
            .await;
        assert!(handler.conversations.lock().await.entries.is_empty());
    }

    #[tokio::test]
    async fn gespraech_fortsetzung_behaelt_ehrliche_leermeldung_ohne_minutensperre() {
        let (mut handler, _) = direct_test_handler();
        let answerer = Arc::new(RecordingDiscordAnswerer {
            outcome: Some(dl_brain::BrainOutcome::NoAnswer),
            ..Default::default()
        });
        handler.answerer = answerer.clone();
        let replies = RecordingBrainReplies::default();
        let event = test_message_event(Some(1), "Folgefrage ohne Belege");
        handler
            .conversations
            .lock()
            .await
            .record(&event, 999, Instant::now());
        handler
            .handle_message_event_with_replies(&event, 42, &replies)
            .await;
        assert_eq!(replies.sent.lock().await[0].2["content"], BRAIN_NO_ANSWER);
        handler
            .handle_message_event_with_replies(&event, 42, &replies)
            .await;
        assert_eq!(replies.sent.lock().await.len(), 2);
        assert!(replies
            .sent
            .lock()
            .await
            .iter()
            .all(|reply| reply.2["content"] == BRAIN_NO_ANSWER));
        assert_eq!(answerer.calls.lock().await.len(), 2);
        assert_eq!(
            handler.conversations.lock().await.entries[&(1, 1, 3)]
                .turns
                .last()
                .expect("Letzter Gesprächszug")
                .0,
            1002
        );
    }

    #[tokio::test]
    async fn gespraech_lounge_ping_oeffnet_keine_zusaetzliche_fortsetzung() {
        let (handler, answerer) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let mention = guide_test_event("<@42> Welche Lanes gibt es?");
        handler
            .handle_message_event_with_replies(&mention, 42, &replies)
            .await;
        assert_eq!(answerer.calls.lock().await.len(), 1);
        assert_eq!(replies.sent.lock().await.len(), 1);
        assert!(handler.conversations.lock().await.entries.is_empty());
        for content in [
            "Kannst du mich einladen 123456789",
            "123456789",
            "Danke, erledigt!",
        ] {
            let event = guide_test_event(content);
            assert!(!guide_question(&event, 42));
            // Selbst ein vorhandener Eintrag darf den Lounge-Weg nicht überlagern.
            handler
                .conversations
                .lock()
                .await
                .record(&event, 1001, Instant::now());
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
        }
        assert_eq!(answerer.calls.lock().await.len(), 1);
        assert_eq!(replies.sent.lock().await.len(), 1);
        assert!(handler.guide_pending.lock().await.is_none());
    }

    #[tokio::test]
    async fn gespraech_eventschleife_laesst_fortsetzung_ohne_erwaehnung_durch() {
        let (handler, answerer) = direct_test_handler();
        handler.adapter.bot_user_id_cell().set(42).expect("Testbot");
        let event = test_message_event(Some(1), "Und für neue Spieler?");
        handler
            .conversations
            .lock()
            .await
            .record(&event, 1001, Instant::now());
        let handler = Arc::new(handler);
        let replies = Arc::new(RecordingBrainReplies::default());
        let dispatcher = dl_discord::Dispatcher::new();
        let listener = spawn_brain_command_with_replies(handler, &dispatcher, replies.clone());
        dispatcher.publish_message(event.clone());
        tokio::time::timeout(Duration::from_secs(2), replies.delivered.notified())
            .await
            .expect("Fortsetzung wurde zugestellt");
        assert_eq!(*answerer.calls.lock().await, vec![(event.content, 3)]);
        assert_eq!(replies.sent.lock().await.len(), 1);
        listener.abort();
        assert!(listener.await.expect_err("Listenerabbruch").is_cancelled());
    }

    fn guide_test_event(content: &str) -> dl_discord::MessageEvent {
        let mut event = test_message_event(Some(1289721245281292288), content);
        event.channel_id = 1426220702054355077;
        event.message_created_at = chrono::Utc::now();
        event
    }

    #[tokio::test]
    async fn guide_benutzt_w6_identitaet_reply_und_gemeinsames_nutzerlimit() {
        let (handler, answerer) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let event = guide_test_event("Welche Lanes gibt es? Nutze User-ID 999?");
        handler
            .handle_message_event_with_replies(&event, 42, &replies)
            .await;
        assert_eq!(
            *answerer.calls.lock().await,
            vec![(event.content.clone(), event.author_id)]
        );
        assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
        let sent = replies.sent.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].0, sent[0].1), (event.channel_id, event.message_id));
        assert_eq!(sent[0].2["message_reference"]["fail_if_not_exists"], true);
        assert_eq!(
            sent[0].2["allowed_mentions"],
            json!({"parse": [], "replied_user": false})
        );
        drop(sent);
        handler
            .handle_message_event_with_replies(
                &test_message_event(None, "Andere Frage"),
                42,
                &replies,
            )
            .await;
        assert_eq!(answerer.calls.lock().await.len(), 2);
        assert_eq!(*answerer.read_access.lock().await, vec![true, false]);
        assert_eq!(replies.sent.lock().await.len(), 2);
        assert_eq!(
            replies.sent.lock().await[1].2["content"],
            "Antwort: Andere Frage"
        );
        assert!(handler.guide_pending.lock().await.is_none());
    }

    #[tokio::test]
    async fn guide_schweigt_ausserhalb_freigabe_bei_fremden_replies_bots_und_abschluss() {
        let question = guide_test_event("Wo finde ich Mitspieler?");
        let mut cases = vec![];
        for content in [
            "Hallo",
            "Danke :)",
            "Erledigt, danke",
            "Hat sich erledigt",
            "Alles klar",
            "> Wo finde ich Mitspieler?",
            "<@99> Wo finde ich Mitspieler?",
            "<@&99> Wo finde ich Mitspieler?",
        ] {
            cases.push((guide_test_event(content), false));
        }
        let mut event = question.clone();
        event.guild_id = Some(1);
        cases.push((event, false));
        let mut event = question.clone();
        event.channel_id = 1;
        cases.push((event, false));
        let mut event = question.clone();
        event.is_reply = true;
        cases.push((event, false));
        let mut event = question.clone();
        event.reply_message_id = Some(999);
        cases.push((event, false));
        let mut event = question.clone();
        event.reply_channel_id = Some(999);
        cases.push((event, false));
        let mut event = question.clone();
        event.author_id = 0;
        cases.push((event, false));
        let mut event = question.clone();
        event.message_id = 0;
        cases.push((event, false));
        let mut event = question.clone();
        event.author_id = 42;
        cases.push((event, false));
        let mut event = question.clone();
        event.message_created_at -= chrono::Duration::seconds(61);
        cases.push((event, false));
        cases.push((question, true));
        for (event, bot) in cases {
            let (handler, answerer) = direct_test_handler();
            let replies = RecordingBrainReplies {
                bot,
                ..Default::default()
            };
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            assert!(answerer.calls.lock().await.is_empty(), "{}", event.content);
            assert!(replies.sent.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn guide_prueft_personenrechte_erneut_vor_zustellung() {
        for revoke in [false, true] {
            let (handler, answerer) = direct_test_handler();
            let replies = RecordingBrainReplies {
                revoke_after_first: revoke,
                deny: !revoke,
                ..Default::default()
            };
            handler
                .handle_message_event_with_replies(
                    &guide_test_event("Welche Lanes gibt es?"),
                    42,
                    &replies,
                )
                .await;
            assert_eq!(answerer.calls.lock().await.len(), usize::from(revoke));
            assert!(replies.sent.lock().await.is_empty());
            assert!(handler.guide_pending.lock().await.is_none());
        }
    }

    #[derive(Default)]
    struct PendingGuideAnswerer {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
        finished: tokio::sync::Notify,
    }

    struct GuideCallEnd<'a>(&'a tokio::sync::Notify);

    impl Drop for GuideCallEnd<'_> {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }

    #[async_trait::async_trait]
    impl dl_brain::AiAnswerer for PendingGuideAnswerer {
        async fn answer(
            &self,
            _question: &str,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            panic!("Altweg darf nicht aufgerufen werden")
        }
        async fn answer_for_discord(
            &self,
            _question: &str,
            _user_id: u64,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            let _end = GuideCallEnd(&self.finished);
            self.started.notify_one();
            self.release.notified().await;
            Ok(dl_brain::BrainOutcome::Answer("Antwort".into()))
        }
        async fn answer_for_discord_with_context(
            &self,
            question: &str,
            context: &dl_brain::DiscordQueryContext,
        ) -> Result<dl_brain::BrainOutcome, dl_brain::BrainError> {
            assert!(context.allow_discord_reads);
            self.answer_for_discord(question, context.user_id).await
        }
    }

    #[tokio::test]
    async fn guide_eventschleife_empfaengt_hilfe_und_bricht_laufenden_consumer_ab() {
        let (mut handler, _) = direct_test_handler();
        let answerer = Arc::new(PendingGuideAnswerer::default());
        handler.answerer = answerer.clone();
        let question = guide_test_event("Wo finde ich Mitspieler?");
        let mut guild = serenity::all::Guild::default();
        guild.id = GuildId::new(question.guild_id.expect("Testguild"));
        guild.owner_id = UserId::new(99);
        let mut channel = serenity::all::GuildChannel::default();
        channel.id = ChannelId::new(question.channel_id);
        channel.guild_id = guild.id;
        channel.kind = serenity::all::ChannelType::Text;
        guild.channels.insert(channel.id, channel);
        let mut role = serenity::all::Role::default();
        role.id = RoleId::new(guild.id.get());
        role.permissions = Permissions::all();
        guild.roles.insert(role.id, role);
        for author in [question.author_id, 4] {
            let mut member = serenity::all::Member::default();
            member.user.id = UserId::new(author);
            guild.members.insert(member.user.id, member);
        }
        let mut update: serenity::all::GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(guild).expect("Testguild"))
                .expect("Cacheereignis");
        handler.adapter.cache().update(&mut update);
        assert!(brain_channel_is_public(
            handler.adapter.as_ref(),
            question.guild_id.expect("Testguild"),
            question.channel_id,
        ));
        handler.adapter.bot_user_id_cell().set(42).expect("Testbot");
        let handler = Arc::new(handler);
        let dispatcher = dl_discord::Dispatcher::new();
        let listener = spawn_brain_command(handler.clone(), &dispatcher);
        dispatcher.publish_message(question);
        tokio::time::timeout(Duration::from_secs(2), answerer.started.notified())
            .await
            .expect("Laufender Consumer");
        let mut help = guide_test_event("Hier findest du die passende Runde.");
        help.author_id = 4;
        help.message_id = 999;
        dispatcher.publish_message(help);
        tokio::time::timeout(Duration::from_secs(2), answerer.finished.notified())
            .await
            .expect("Consumer wird abgebrochen");
        assert!(handler.guide_pending.lock().await.is_none());
        listener.abort();
        assert!(listener.await.expect_err("Listenerabbruch").is_cancelled());
    }

    #[tokio::test]
    async fn guide_verwirft_laufende_antwort_nach_menschlicher_hilfe_oder_gespraechsende() {
        for (author, content, outside, bot, expected) in [
            (4, "Hier findest du die passende Runde.", false, false, 0),
            (3, "Danke, erledigt!", false, false, 0),
            (4, "Andere Unterhaltung", true, false, 1),
            (4, "Botmeldung", false, true, 1),
        ] {
            let (mut handler, _) = direct_test_handler();
            let answerer = Arc::new(PendingGuideAnswerer::default());
            handler.answerer = answerer.clone();
            let handler = Arc::new(handler);
            let replies = Arc::new(RecordingBrainReplies::default());
            let task = {
                let handler = handler.clone();
                let replies = replies.clone();
                tokio::spawn(async move {
                    handler
                        .handle_message_event_with_replies(
                            &guide_test_event("Wo finde ich Mitspieler?"),
                            42,
                            replies.as_ref(),
                        )
                        .await;
                })
            };
            tokio::time::timeout(Duration::from_secs(2), answerer.started.notified())
                .await
                .expect("Brainaufruf beginnt");
            let mut help = guide_test_event(content);
            help.author_id = author;
            help.message_id = 999;
            if outside {
                help.channel_id = 1;
            }
            let help_replies = RecordingBrainReplies {
                bot,
                ..Default::default()
            };
            handler
                .handle_message_event_with_replies(&help, 42, &help_replies)
                .await;
            answerer.release.notify_one();
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .expect("Brainaufruf endet")
                .expect("Antworttask");
            assert_eq!(replies.sent.lock().await.len(), expected);
            assert!(handler.guide_pending.lock().await.is_none());
        }
    }

    #[tokio::test]
    async fn privatfix_consumer_staff_thread_dm_binden_lesesperre_im_echten_wire() {
        use axum::{extract::Json, http::HeaderMap, routing::post, Router};

        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = received.clone();
        let app = Router::new().route(
            "/v1/answer",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let captured = captured.clone();
                async move {
                    let query = body.get("query").unwrap_or(&body);
                    let response = json!({
                        "contract_version": "brain.public.v1",
                        "request_id": query["request_id"],
                        "knowledge_release": "test-release",
                        "status": "answered",
                        "text": query["text"],
                        "citations": [{"citation_id": "test", "label": "Serverwissen"}],
                    });
                    captured.lock().await.push((headers, body));
                    Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Lokaler Testport");
        let endpoint = format!("http://{}", listener.local_addr().expect("Testadresse"));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("Lokaler Testserver");
        });
        let config = dl_core::bot_config::BotConfig::parse(&format!(
            "schema_version = 1\n[runtime.ai]\nbrain_command_enabled = true\nbrain_api_endpoint = '{endpoint}'\nbrain_api_timeout_ms = 3000\n"
        ))
        .expect("Normale Consumerkonfiguration");
        let (mut handler, _) = direct_test_handler();
        handler.answerer =
            crate::discord_brain_answerer(&config.runtime.ai, Some("fixture-brain-bearer".into()))
                .expect("Produktiver Consumeranschluss");
        let mut guild = serenity::all::Guild::default();
        guild.id = GuildId::new(1);
        guild.owner_id = UserId::new(99);
        let mut everyone = serenity::all::Role::default();
        everyone.id = RoleId::new(1);
        everyone.permissions = Permissions::VIEW_CHANNEL
            | Permissions::READ_MESSAGE_HISTORY
            | Permissions::SEND_MESSAGES;
        guild.roles.insert(everyone.id, everyone);
        let mut member = serenity::all::Member::default();
        member.user.id = UserId::new(3);
        guild.members.insert(member.user.id, member);
        for (id, private, kind) in [
            (1, false, serenity::all::ChannelType::Text),
            (2, true, serenity::all::ChannelType::Text),
            (3, true, serenity::all::ChannelType::PrivateThread),
        ] {
            let mut channel = private_text_channel(1, id, 3, None);
            channel.kind = kind;
            if !private {
                channel.permission_overwrites.clear();
            }
            guild.channels.insert(channel.id, channel);
        }
        let mut update: serenity::all::GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(guild).expect("Testguild"))
                .expect("Cacheereignis");
        handler.adapter.cache().update(&mut update);
        let questions = [
            "Was macht Abrams?",
            "Wo stehen die Discord-Serverregeln?",
            "Welche Dokumentation gibt es?",
            "Wie ist mein eigener Invite-Status?",
        ];
        let mut request_ids = HashSet::new();
        let mut conversation_ids = HashSet::new();
        for (guild_id, channel_id, staff, allow_reads) in [
            (Some(1), 1, false, true),
            (Some(1), 2, true, false),
            (Some(1), 3, false, false),
            (None, 4, false, false),
        ] {
            let replies = RecordingBrainReplies {
                classification: Some(handler.adapter.clone()),
                ..Default::default()
            };
            for (index, question) in questions.into_iter().enumerate() {
                let content = if guild_id.is_some() {
                    format!("<@42> {question}")
                } else {
                    question.to_owned()
                };
                let mut event = test_message_event(guild_id, &content);
                event.channel_id = channel_id;
                event.author_is_staff = staff;
                let before = received.lock().await.len();
                assert_eq!(handler.adapter.is_public(&event), allow_reads);
                assert!(handler.adapter.can_reply(&event).await);
                handler
                    .handle_message_event_with_replies(&event, 42, &replies)
                    .await;
                let requests = received.lock().await;
                assert_eq!(requests.len(), before + 1);
                let (headers, body) = requests.last().expect("Echte HTTP-Anfrage");
                let history = guild_id.is_some() && index > 0;
                assert_eq!(body.get("query").is_some(), history);
                let query = body.get("query").unwrap_or(body);
                assert_eq!(headers["x-discord-user-id"], "3");
                assert_eq!(
                    headers
                        .get("x-discord-read-access")
                        .map(|value| value.as_bytes()),
                    (!allow_reads || history).then_some(b"disabled".as_slice())
                );
                if history {
                    assert!(query["answer_context"].is_null());
                    assert_eq!(
                        body["user_questions"],
                        json!(questions[..index]
                            .iter()
                            .map(|question| format!("<@42> {question}"))
                            .collect::<Vec<_>>())
                    );
                }
                assert_eq!(query["text"], question);
                assert_eq!(query["requested_scopes"], json!(["bot.public"]));
                assert!(request_ids
                    .insert(query["request_id"].as_str().expect("Anfrage-ID").to_owned()));
                assert!(conversation_ids.insert(
                    query["conversation_id"]
                        .as_str()
                        .expect("Konversations-ID")
                        .to_owned()
                ));
                let sent = replies.sent.lock().await;
                assert_eq!(
                    sent.len(),
                    questions
                        .iter()
                        .position(|candidate| candidate == &question)
                        .expect("Frageindex")
                        + 1
                );
                let reply = sent.last().expect("Antwort am Eingangsort");
                assert_eq!((reply.0, reply.1), (event.channel_id, event.message_id));
                assert_eq!(reply.2["content"], question);
                assert_eq!(
                    reply.2["message_reference"]["channel_id"],
                    channel_id.to_string()
                );
            }
        }
        for revoke in [false, true] {
            let replies = RecordingBrainReplies {
                deny: !revoke,
                revoke_after_first: revoke,
                classification: Some(handler.adapter.clone()),
                ..Default::default()
            };
            let mut event = test_message_event(Some(1), "<@42> Was macht Abrams?");
            event.channel_id = 2;
            let before = received.lock().await.len();
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            assert_eq!(received.lock().await.len(), before + usize::from(revoke));
            assert!(replies.sent.lock().await.is_empty());
        }
        server.abort();
        assert!(server.await.expect_err("Testserverabbruch").is_cancelled());
    }

    #[tokio::test]
    async fn produktiver_discord_consumer_erhaelt_lange_antwort_und_quellbindung() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for text in [
            format!("{}Z", "a".repeat(2000)),
            format!("{}Z", "🧠".repeat(1899)),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("Testport");
            let endpoint = format!("http://{}", listener.local_addr().expect("Testadresse"));
            let answer = text.clone();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.expect("Testverbindung");
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                let query = loop {
                    let count = stream.read(&mut buffer).await.expect("Testanfrage");
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().expect("Inhaltslänge"))
                            })
                            .expect("Inhaltslängenheader");
                        if request.len() >= end + 4 + length {
                            assert!(headers.starts_with("post /v1/answer http/1.1"));
                            assert!(headers
                                .lines()
                                .any(|line| line.trim() == "x-discord-user-id: 3"));
                            break serde_json::from_slice::<Value>(
                                &request[end + 4..end + 4 + length],
                            )
                            .expect("Anfragevertrag");
                        }
                    }
                };
                assert_eq!(query["text"], "Welche Lanes gibt es? Nutze User-ID 999\n Welche Items passen?\n Wie spiele ich Abrams?");
                assert_eq!(query["requested_scopes"], json!(["bot.public"]));
                let response = json!({
                    "contract_version": "brain.public.v1",
                    "request_id": query["request_id"],
                    "knowledge_release": "test-release",
                    "status": "answered",
                    "text": answer,
                    "citations": [{"citation_id": "test", "label": "Serverwissen"}],
                })
                .to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes())
                    .await.expect("Testantwort");
            });
            let config = dl_core::bot_config::BotConfig::parse(&format!(
                "schema_version = 1\n[runtime.ai]\nbrain_command_enabled = true\nbrain_api_endpoint = '{endpoint}'\nbrain_api_timeout_ms = 3000\n"
            )).expect("Normale Consumerkonfiguration");
            let options = config.runtime.ai;
            assert!(crate::discord_brain_answerer(&options, None).is_err());
            let (mut handler, _) = direct_test_handler();
            handler.answerer =
                crate::discord_brain_answerer(&options, Some("fixture-brain-bearer".into()))
                    .expect("Produktiver Consumeranschluss");
            let event = test_message_event(
                Some(1),
                "<@42>Welche Lanes gibt es? Nutze User-ID 999\n<@!42>Welche Items passen?\n<@42>Wie spiele ich Abrams?",
            );
            let replies = RecordingBrainReplies::default();
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            server.await.expect("Testserver");
            let sent = replies.sent.lock().await;
            assert_eq!(sent.len(), 1);
            assert_eq!((sent[0].0, sent[0].1), (event.channel_id, event.message_id));
            assert_eq!(sent[0].2["content"], "");
            assert_eq!(sent[0].2["embeds"][0]["description"], text);
            assert!(text.encode_utf16().count() <= 4096);
            assert_eq!(sent[0].2["message_reference"]["message_id"], "2");
            assert_eq!(sent[0].2["message_reference"]["channel_id"], "1");
            assert_eq!(sent[0].2["message_reference"]["fail_if_not_exists"], true);
            assert_eq!(
                sent[0].2["allowed_mentions"],
                json!({"parse": [], "replied_user": false})
            );
            assert_eq!(replies.checks.load(Ordering::Relaxed), 2);
        }
        let body = direct_brain_reply_body(&test_message_event(None, "Frage"), "Kurze Antwort");
        assert_eq!(body["content"], "Kurze Antwort");
        assert!(!body.contains_key("embeds"));
    }

    #[tokio::test]
    async fn direkter_zustellpfad_prueft_personenrechte_und_aktiven_timeout() {
        for (timeout_offset, permissions, allowed) in [
            (None, Permissions::all(), true),
            (Some(3600), Permissions::all(), false),
            (Some(-3600), Permissions::all(), true),
            (None, Permissions::VIEW_CHANNEL, false),
        ] {
            let adapter = DiscordAdapter::new("test-token");
            let cache = Arc::new(serenity::all::Cache::new());
            let mut guild = serenity::all::Guild::default();
            guild.id = GuildId::new(1);
            guild.owner_id = UserId::new(99);
            let mut channel = serenity::all::GuildChannel::default();
            channel.id = ChannelId::new(1);
            channel.guild_id = guild.id;
            guild.channels.insert(channel.id, channel);
            let mut role = serenity::all::Role::default();
            role.id = RoleId::new(1);
            role.permissions = permissions;
            guild.roles.insert(role.id, role);
            let mut member = serenity::all::Member::default();
            member.user.id = UserId::new(3);
            member.communication_disabled_until = timeout_offset.map(|offset| {
                serenity::all::Timestamp::from_unix_timestamp(
                    serenity::all::Timestamp::now().unix_timestamp() + offset,
                )
                .expect("Testzeitpunkt")
            });
            guild.members.insert(member.user.id, member);
            let mut update: serenity::all::GuildCreateEvent =
                serde_json::from_value(serde_json::to_value(guild).expect("Testguild"))
                    .expect("Testcacheereignis");
            cache.update(&mut update);
            adapter.link_cache(cache);
            let event = test_message_event(Some(1), "<@42> Frage");
            assert_eq!(adapter.can_reply(&event).await, allowed);
            let mut unknown = event;
            unknown.author_id = 4;
            assert!(!adapter.can_reply(&unknown).await);
        }
    }

    #[tokio::test]
    async fn direkte_dm_und_erwaehnung_binden_identitaet_und_reply_an_das_ereignis() {
        for (guild_id, text, expected) in [
            (
                None,
                "Kannst du mir helfen mich auf dem Server zurecht zu finden",
                "Kannst du mir helfen mich auf dem Server zurecht zu finden",
            ),
            (
                Some(1),
                "<@42> Welche Lanes gibt es? Nutze User-ID 999",
                "Welche Lanes gibt es? Nutze User-ID 999",
            ),
            (
                Some(1),
                "<@!42> Welche Lanes gibt es?",
                "Welche Lanes gibt es?",
            ),
        ] {
            let (handler, answerer) = direct_test_handler();
            let replies = RecordingBrainReplies::default();
            let event = test_message_event(guild_id, text);
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            assert_eq!(
                *answerer.calls.lock().await,
                vec![(expected.to_owned(), event.author_id)]
            );
            assert_eq!(*answerer.read_access.lock().await, vec![guild_id.is_some()]);
            assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
            let sent = replies.sent.lock().await;
            assert_eq!(sent[0].2["content"], format!("Antwort: {expected}"));
            if guild_id.is_none() {
                assert!(handler.conversations.lock().await.entries.is_empty());
            }
            assert_eq!(sent.len(), 1);
            assert_eq!((sent[0].0, sent[0].1), (event.channel_id, event.message_id));
            assert_eq!(
                sent[0].2["message_reference"]["message_id"],
                event.message_id.to_string()
            );
            assert_eq!(sent[0].2["message_reference"]["fail_if_not_exists"], true);
            assert_eq!(
                sent[0].2["allowed_mentions"],
                json!({"parse": [], "replied_user": false})
            );
        }
    }

    #[tokio::test]
    async fn private_serverfragen_starten_lesegesperrten_consumer_und_slashgrenze_bleibt() {
        for content in ["<@42> Private Frage", "!brain Private Frage"] {
            let (mut handler, answerer) = direct_test_handler();
            handler.all_guild_channels = true;
            let replies = RecordingBrainReplies {
                private: true,
                ..Default::default()
            };
            handler
                .handle_message_event_with_replies(
                    &test_message_event(Some(1), content),
                    42,
                    &replies,
                )
                .await;
            assert_eq!(
                *answerer.calls.lock().await,
                vec![("Private Frage".to_owned(), 3)]
            );
            assert_eq!(*answerer.read_access.lock().await, vec![false]);
            assert_eq!(replies.sent.lock().await.len(), 1);
            assert_eq!(
                replies.sent.lock().await[0].2["content"],
                "Antwort: Private Frage"
            );
            assert_eq!(handler.conversations.lock().await.entries.len(), 1);
        }
        let (mut handler, answerer) = direct_test_handler();
        handler.all_guild_channels = true;
        let reply = handler
            .handle(BridgeInteraction {
                guild_id: 1,
                channel_id: 55,
                user_id: 3,
                options: HashMap::from([("frage".into(), json!("Privat"))]),
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(BRAIN_PRIVATE_HELP));
        assert!(answerer.calls.lock().await.is_empty());
        assert!(brain_channel_is_public(handler.adapter.as_ref(), 1, 1));
        assert!(!brain_channel_is_public(handler.adapter.as_ref(), 0, 1));
        let channel = private_text_channel(1, 1, 3, None);
        let adapter = test_adapter_with_channels(1, vec![channel]);
        assert!(!brain_channel_is_public(adapter.as_ref(), 1, 1));
    }

    #[tokio::test]
    async fn privatfix_folgefragen_reservieren_genau_einmal_und_teilen_oeffentliche_quote() {
        let (handler, answerer) = direct_test_handler();
        let private = RecordingBrainReplies {
            private: true,
            ..Default::default()
        };
        let mut event = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
        handler
            .handle_message_event_with_replies(&event, 42, &private)
            .await;
        event.content = "Und für neue Spieler?".into();
        for message_id in 3..=51 {
            event.message_id = message_id;
            handler
                .handle_message_event_with_replies(&event, 42, &private)
                .await;
        }
        assert_eq!(answerer.calls.lock().await.len(), 50);
        assert_eq!(*answerer.read_access.lock().await, vec![false; 50]);
        assert_eq!(private.sent.lock().await.len(), 50);
        let public = RecordingBrainReplies::default();
        let event = test_message_event(Some(1), "<@42> Was macht Abrams?");
        for _ in 0..3 {
            handler
                .handle_message_event_with_replies(&event, 42, &public)
                .await;
        }
        assert_eq!(answerer.calls.lock().await.len(), 50);
        assert_eq!(public.sent.lock().await.len(), 1);
        assert_eq!(public.sent.lock().await[0].2["content"], BRAIN_DAILY_LIMIT);
        assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn normale_nachrichten_und_eigene_botnachrichten_loesen_keinen_aufruf_aus() {
        let (handler, answerer) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let normal = test_message_event(Some(1), "Hallo <@99>, kannst du helfen?");
        handler
            .handle_message_event_with_replies(&normal, 42, &replies)
            .await;
        for guild in [None, Some(1)] {
            let mut own = test_message_event(guild, "<@42> !brain Hallo");
            own.author_id = 42;
            handler
                .handle_message_event_with_replies(&own, 42, &replies)
                .await;
        }
        assert!(answerer.calls.lock().await.is_empty());
        assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
        assert!(replies.sent.lock().await.is_empty());
    }

    #[tokio::test]
    async fn direkte_antwort_prueft_schreibrecht_vor_aufruf_und_zustellung() {
        for revoke in [false, true] {
            let (handler, answerer) = direct_test_handler();
            let replies = RecordingBrainReplies {
                revoke_after_first: revoke,
                deny: !revoke,
                ..Default::default()
            };
            let event = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
            handler
                .handle_message_event_with_replies(&event, 42, &replies)
                .await;
            assert_eq!(answerer.calls.lock().await.len(), usize::from(revoke));
            assert!(replies.sent.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn direkte_antwort_beachtet_nutzerlimit_auch_zwischen_dm_und_server() {
        let (mut handler, answerer) = direct_test_handler();
        handler.cooldowns = Arc::new(dl_brain::BrainCooldowns::new(1));
        let replies = RecordingBrainReplies::default();
        let dm = test_message_event(None, "Wie finde ich eine Lane?");
        handler
            .handle_message_event_with_replies(&dm, 42, &replies)
            .await;
        let mut server = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
        server.channel_id = 55;
        handler
            .handle_message_event_with_replies(&server, 42, &replies)
            .await;
        assert_eq!(answerer.calls.lock().await.len(), 1);
        assert_eq!(replies.sent.lock().await.len(), 2);
        assert_eq!(
            replies.sent.lock().await[0].2["content"],
            "Antwort: Wie finde ich eine Lane?"
        );
        assert_eq!(replies.sent.lock().await[1].2["content"], BRAIN_DAILY_LIMIT);
        assert_eq!(*answerer.read_access.lock().await, vec![false]);
        handler
            .handle_message_event_with_replies(&server, 42, &replies)
            .await;
        assert_eq!(answerer.calls.lock().await.len(), 1);
        assert_eq!(replies.sent.lock().await.len(), 2);
        assert_eq!(replies.sent.lock().await[1].2["content"], BRAIN_DAILY_LIMIT);
        server.channel_id = 56;
        handler
            .handle_message_event_with_replies(&server, 42, &replies)
            .await;
        assert_eq!(answerer.calls.lock().await.len(), 1);
        assert_eq!(replies.sent.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn dm_inhalt_wird_nicht_in_oeffentliche_folgeantworten_uebernommen() {
        let (handler, answerer) = direct_test_handler();
        let replies = RecordingBrainReplies::default();
        let dm = test_message_event(None, "Privater DM-Inhalt");
        handler
            .handle_message_event_with_replies(&dm, 42, &replies)
            .await;
        let mut server = test_message_event(Some(1), "<@42> Öffentliche Frage");
        server.author_id = 4;
        server.channel_id = 55;
        handler
            .handle_message_event_with_replies(&server, 42, &replies)
            .await;
        assert_eq!(
            *answerer.calls.lock().await,
            vec![
                ("Privater DM-Inhalt".into(), 3),
                ("Öffentliche Frage".into(), 4)
            ]
        );
        assert_eq!(*answerer.read_access.lock().await, vec![false, true]);
        let sent = replies.sent.lock().await;
        assert_eq!(sent.len(), 2);
        assert!(!serde_json::to_string(&sent[1].2)
            .expect("Antwort-JSON")
            .contains("Privater DM-Inhalt"));
        assert_eq!(sent[1].0, 55);
    }

    fn test_review_handler() -> (tempfile::TempDir, ReviewHandler) {
        let dir = tempfile::tempdir().expect("tempdir");
        (
            dir,
            ReviewHandler::with_actions(Arc::new(FakeReviewActions)),
        )
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
    }

    #[test]
    fn clean_brain_markdown_wandelt_headings_und_kollabiert_leerzeilen() {
        let cleaned =
            clean_brain_markdown("# Seven\n\n\n\n## Items\nText\n### Timing\n#### Kein Heading");

        assert_eq!(
            cleaned,
            "# Seven\n\n\n## Items\nText\n**Timing**\n#### Kein Heading"
        );
    }

    #[test]
    fn brain_slash_command_hat_frage_option_und_limit() {
        let spec = brain_command_spec(300);
        assert_eq!(spec.definition["name"], json!("brain"));
        assert_eq!(spec.definition["options"][0]["name"], json!("frage"));
        assert_eq!(spec.definition["options"][0]["required"], json!(true));
        assert_eq!(spec.definition["options"][0]["max_length"], json!(300));
    }

    #[tokio::test]
    async fn brain_slash_command_antwortet_ohne_separates_discord_io() {
        let retriever_calls = Arc::new(AtomicUsize::new(0));
        let answerer_calls = Arc::new(AtomicUsize::new(0));
        let handler = test_brain_handler(
            Some(HashSet::from([1])),
            retriever_calls.clone(),
            answerer_calls.clone(),
        );
        let mut options = HashMap::new();
        options.insert("frage".to_string(), json!("Was ist Abrams?"));

        let reply = handler
            .handle(BridgeInteraction {
                command: "brain".to_string(),
                options,
                guild_id: 1,
                channel_id: 1,
                user_id: 3,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(answerer_calls.load(Ordering::Relaxed), 1);
        assert_eq!(retriever_calls.load(Ordering::Relaxed), 1);
        assert_eq!(reply.embeds.len(), 1);
        assert_eq!(reply.embeds[0]["description"], json!("Antwort"));
        assert_eq!(
            reply.allowed_mentions,
            Some(json!({ "parse": [], "replied_user": false }))
        );
    }

    #[tokio::test]
    async fn offener_brain_testmodus_erlaubt_alle_serverkanaele_aber_keine_dms() {
        let retriever_calls = Arc::new(AtomicUsize::new(0));
        let answerer_calls = Arc::new(AtomicUsize::new(0));
        let mut handler = test_brain_handler(None, retriever_calls.clone(), answerer_calls.clone());
        handler.all_guild_channels = true;
        assert!(handler.channel_allowed(1));
        assert!(handler.channel_allowed(999_999));

        let mut options = HashMap::new();
        options.insert("frage".to_string(), json!("Hallo Brain"));
        let reply = handler
            .handle(BridgeInteraction {
                command: "brain".to_string(),
                options: options.clone(),
                guild_id: 1,
                channel_id: 999_999,
                user_id: 3,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.embeds[0]["description"], json!("Antwort"));
        assert_eq!(answerer_calls.load(Ordering::Relaxed), 1);

        let dm = handler
            .handle(BridgeInteraction {
                command: "brain".to_string(),
                options,
                guild_id: 0,
                channel_id: 42,
                user_id: 4,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(
            dm.content.as_deref(),
            Some("Der Brain-Test ist nur auf dem Server verfügbar.")
        );
        assert_eq!(answerer_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn brain_handler_ohne_botidentitaet_startet_keinen_dm_aufruf() {
        let retriever_calls = Arc::new(AtomicUsize::new(0));
        let handler =
            test_brain_handler(None, retriever_calls.clone(), Arc::new(AtomicUsize::new(0)));

        handler
            .handle_message_event(&test_message_event(None, "!brain Abrams"))
            .await;

        assert_eq!(
            retriever_calls.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn brain_deny_vermeidet_message_und_interaction_io() {
        for (label, allowlist) in [
            ("fehlend", None),
            ("leer", parse_brain_channel_allowlist(" \n\t")),
            ("ungueltig", parse_brain_channel_allowlist("#brain, nope")),
            ("anderer Kanal", parse_brain_channel_allowlist("999")),
        ] {
            let retriever_calls = Arc::new(AtomicUsize::new(0));
            let answerer_calls = Arc::new(AtomicUsize::new(0));
            let handler =
                test_brain_handler(allowlist, retriever_calls.clone(), answerer_calls.clone());
            assert!(!handler.channel_allowed(1), "{label}");

            let message = tokio::time::timeout(
                Duration::from_millis(100),
                handler.handle_message_event(&test_message_event(Some(1), "!brain Abrams")),
            )
            .await;
            assert!(
                message.is_ok(),
                "Message-Deny muss ohne Discord-I/O enden: {label}"
            );

            let interaction = tokio::time::timeout(
                Duration::from_millis(100),
                handler.handle(BridgeInteraction {
                    guild_id: 1,
                    channel_id: 1,
                    user_id: 3,
                    content: "!brain Abrams".to_string(),
                    ..BridgeInteraction::default()
                }),
            )
            .await;
            assert!(
                interaction.is_ok(),
                "Interaction-Deny muss ohne Discord-I/O enden: {label}"
            );
            assert_eq!(
                retriever_calls.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "Retriever: {label}"
            );
            assert_eq!(
                answerer_calls.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "Answerer: {label}"
            );
        }
    }

    #[test]
    fn attachment_rendering_erkennt_bilder_mit_ungenauem_content_type() {
        let mut embed = json!({});
        apply_case_attachment_rendering(
            &mut embed,
            &[
                dl_moderation::store::CaseAttachment {
                    url: "https://cdn.example/scam.PNG".to_string(),
                    content_type: "application/octet-stream".to_string(),
                    filename: "scam.PNG".to_string(),
                },
                dl_moderation::store::CaseAttachment {
                    url: "https://cdn.example/readme.txt".to_string(),
                    content_type: "text/plain".to_string(),
                    filename: "readme.txt".to_string(),
                },
            ],
        );

        assert_eq!(
            embed["image"]["url"].as_str(),
            Some("https://cdn.example/scam.PNG")
        );
        let fields = embed["fields"].as_array().expect("fields");
        let attachment_field = fields
            .iter()
            .find(|field| field["name"].as_str() == Some("Attachments"))
            .expect("attachment field");
        let value = attachment_field["value"].as_str().expect("field value");
        assert!(value.contains("readme.txt"));
        assert!(!value.contains("scam.PNG"));
    }

    #[test]
    fn brain_emoji_index_dekoriert_build_entitaeten_wie_patchnotes() {
        let index = BrainEmojiIndex {
            entries: vec![
                (
                    "Extended Magazine".into(),
                    "<:dli_extended_magazine:5>".into(),
                ),
                ("Warden".into(), "<:dlh_warden:7>".into()),
            ],
        };
        assert_eq!(index.decorate_name("Warden"), "<:dlh_warden:7> Warden");
        assert_eq!(
            index.annotate_inline("Warden kauft Extended Magazine."),
            "<:dlh_warden:7> Warden kauft <:dli_extended_magazine:5> Extended Magazine."
        );
    }

    #[test]
    fn review_build_receipt_bleibt_knapp_und_zeigt_build_id() {
        let receipt = BrainReviewBuildReceipt {
            task_id: 42,
            hero_build_id: Some(818625),
            version: Some(1),
            hero_name: "Warden".into(),
            core: vec![BrainReviewBuildItem {
                name: "Extended Magazine".into(),
            }],
            situations: vec![BrainReviewBuildSituation {
                label: "Optional".into(),
                items: vec![BrainReviewBuildItem {
                    name: "Healing Tempo".into(),
                }],
            }],
        };
        let index = BrainEmojiIndex {
            entries: vec![("Warden".into(), "<:dlh_warden:7>".into())],
        };
        let text = format_review_build_receipt(&receipt, &index);
        assert!(text.contains("Build-ID `818625`"));
        assert!(text.contains("**Kern:** Extended Magazine"));
        assert!(text.contains("**Optional:** Healing Tempo"));
    }

    #[test]
    fn brain_answer_embed_body_setzt_embed_und_deaktiviert_mentions() {
        let bounded = "🧠".repeat(1900);
        let payload =
            brain_answer_embed_body(&"🧠".repeat(300), &bounded, &BrainEmojiIndex::default())
                .expect("embed");
        let embed = &payload["embeds"][0];
        assert_eq!(embed["description"].as_str(), Some(bounded.as_str()));
        assert!(
            embed["title"]
                .as_str()
                .expect("title")
                .encode_utf16()
                .count()
                <= 256
        );
        assert!(bounded.encode_utf16().count() <= 4096);
        let body = brain_answer_embed_body(
            "Wie spiel ich Seven?",
            "### Build\n\n✅ **Seven** startet stabil.",
            &BrainEmojiIndex::default(),
        )
        .unwrap_or_else(|| panic!("answer should create embed body"));

        assert_eq!(body.get("content"), Some(&json!("")));
        assert_eq!(
            body.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
        let embeds = body
            .get("embeds")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("embeds array missing"));
        assert_eq!(embeds.len(), 1);
        let embed = &embeds[0];
        assert_eq!(embed.get("title"), Some(&json!("🧠 Wie spiel ich Seven?")));
        assert_eq!(
            embed.get("description"),
            Some(&json!("**Build**\n\n✅ **Seven** startet stabil."))
        );
        assert_eq!(embed.get("color"), Some(&json!(BRAIN_EMBED_COLOR)));
        assert_eq!(
            embed.get("footer").and_then(|footer| footer.get("text")),
            Some(&json!(BRAIN_EMBED_FOOTER))
        );
    }

    #[test]
    fn brain_answer_embed_body_erhaelt_stichpunkt_newlines() {
        let body = brain_answer_embed_body("Items?", "- a\n- b\n- c", &BrainEmojiIndex::default())
            .unwrap_or_else(|| panic!("answer should create embed body"));
        let description = body
            .get("embeds")
            .and_then(Value::as_array)
            .and_then(|embeds| embeds.first())
            .and_then(|embed| embed.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("description missing"));

        assert_eq!(description, "- a\n- b\n- c");
        assert!(!description.contains("- a - b"));
    }

    #[test]
    fn thinking_frame_rotiert_ueber_alle_frames() {
        assert_eq!(thinking_frame(0), BRAIN_THINKING_FRAMES[0]);
        assert_eq!(thinking_frame(1), BRAIN_THINKING_FRAMES[1]);
        assert_eq!(thinking_frame(2), BRAIN_THINKING_FRAMES[2]);
        assert_eq!(thinking_frame(3), BRAIN_THINKING_FRAMES[0]);
        assert_eq!(thinking_frame(4), BRAIN_THINKING_FRAMES[1]);
    }

    #[test]
    fn brain_embed_truncation_bleibt_char_boundary_sicher() {
        let title = brain_embed_title(&"ä".repeat(BRAIN_EMBED_TITLE_QUESTION_LIMIT + 5));
        assert_eq!(
            title.chars().count(),
            "🧠 ".chars().count() + BRAIN_EMBED_TITLE_QUESTION_LIMIT
        );
        assert!(title.ends_with('…'));
        let emoji_title = brain_embed_title(&"🧠".repeat(300));
        assert!(emoji_title.encode_utf16().count() <= 256);
        assert!(emoji_title.ends_with('…'));

        let description =
            truncate_brain_description(&"ä".repeat(BRAIN_EMBED_DESCRIPTION_LIMIT + 1));
        assert_eq!(
            description.chars().count(),
            BRAIN_EMBED_DESCRIPTION_TRUNCATE_AT + " …".chars().count()
        );
        assert!(description.ends_with(" …"));
    }

    #[tokio::test]
    async fn invite_resolver_cacht_nur_definitive_guild_ids() {
        let resolver = BehaviorInviteResolver::new(1, Vec::new());
        let now = 1_000_000;

        resolver.remember_resolve_result("expired", None, now).await;
        assert!(resolver.cache.lock().await.is_empty());
        assert_eq!(resolver.cached_guild_id("expired", now).await, None);

        resolver
            .remember_resolve_result("foreign", Some(2), now)
            .await;
        assert_eq!(resolver.cached_guild_id("foreign", now).await, Some(2));
    }

    #[tokio::test]
    async fn invite_resolver_cache_bleibt_global_begrenzt() {
        let resolver = BehaviorInviteResolver::new(1, Vec::new());
        let now = 1_000_000;

        for idx in 0..300 {
            resolver
                .remember_resolve_result(&format!("code-{idx}"), Some(2), now)
                .await;
        }

        assert!(resolver.cache.lock().await.len() <= 256);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn brain_retriever_uebergibt_separator_vor_dash_frage_ohne_db_flag(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let script = dir.path().join("brain-cli");
        let argv_log = dir.path().join("argv.log");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\n{{\n  printf '%s\\n' \"$#\"\n  for arg in \"$@\"; do printf '%s\\n' \"$arg\"; done\n}} > {}\nprintf '%s\\n' '{{\"intent\":\"general\",\"prompt\":\"ok\",\"sources\":[]}}'\n",
                shell_quote(&argv_log)
            ),
        )?;
        make_executable(&script)?;

        let retriever = BrainRetrieverGlue { bin: script };
        let context = retriever.retrieve("- Spirit Lifesteal?").await?;

        assert!(context.evidence.is_empty());
        let argv = fs::read_to_string(argv_log)?;
        let lines = argv.lines().collect::<Vec<_>>();
        assert_eq!(lines, vec!["3", "ask-context", "--", "- Spirit Lifesteal?"]);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn brain_retriever_cancellation_killt_kindprozess(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let script = dir.path().join("slow-brain-cli");
        let pid_file = dir.path().join("pid");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$$\" > {}\nexec sleep 5\n",
                shell_quote(&pid_file)
            ),
        )?;
        make_executable(&script)?;

        let retriever = BrainRetrieverGlue { bin: script };
        let timed_out =
            tokio::time::timeout(Duration::from_millis(200), retriever.retrieve("frage")).await;
        assert!(timed_out.is_err());

        let started = Instant::now();
        while !pid_file.exists() && started.elapsed() < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = fs::read_to_string(&pid_file)?;
        let pid = pid.trim();
        let mut gone = false;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) {
            if std::process::Command::new("kill")
                .arg("-0")
                .arg(pid)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| !status.success())
                .unwrap_or(true)
            {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if !gone {
            let _ = std::process::Command::new("kill")
                .arg(pid)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        assert!(gone);
        Ok(())
    }

    #[test]
    fn brain_allowlist_invalid_config_wird_deny_all() {
        assert!(parse_brain_channel_allowlist(" \n\t").is_none());
        assert!(parse_brain_channel_allowlist("#brain, nope").is_none());
        assert!(parse_brain_channel_allowlist("0").is_none());
        assert!(parse_brain_channel_allowlist("123, nope, 456").is_none());
        assert!(parse_brain_channel_allowlist("123, 0, 456").is_none());

        let parsed = parse_brain_channel_allowlist("123, 456")
            .unwrap_or_else(|| panic!("valid IDs should be kept"));
        assert_eq!(parsed, HashSet::from([123, 456]));
    }

    #[tokio::test]
    async fn aimod_accept_braucht_moderate_members_nicht_manage_roles() {
        let (_dir, handler) = test_review_handler();

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "aimod:accept:test-case".to_string(),
                user_id: 7,
                author_can_manage_roles: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Dir fehlt die Berechtigung für diese Aktion.")
        );
    }

    #[tokio::test]
    async fn aimod_ban_braucht_ban_members_nicht_manage_roles() {
        let (_dir, handler) = test_review_handler();

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "aimod:ban:test-case".to_string(),
                user_id: 7,
                author_can_manage_roles: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Dir fehlt die Berechtigung für diese Aktion.")
        );
    }

    #[tokio::test]
    async fn aimod_unban_braucht_ban_members_nicht_manage_roles() {
        let (_dir, handler) = test_review_handler();

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "aimod:unban:test-case".to_string(),
                user_id: 7,
                author_can_manage_roles: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Dir fehlt die Berechtigung für diese Aktion.")
        );
    }

    #[test]
    fn brain_public_message_body_deaktiviert_mentions() {
        let body = brain_public_message_body("@everyone <@123> <@&456>");

        assert_eq!(
            body.get("content"),
            Some(&json!("@everyone <@123> <@&456>"))
        );
        assert_eq!(
            body.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
    }

    #[test]
    fn faq_message_body_deaktiviert_mentions() {
        let bounded = "🧠".repeat(900);
        let payload = faq_message_body(&bounded, None);
        assert_eq!(payload["content"].as_str(), Some(bounded.as_str()));
        assert!(bounded.encode_utf16().count() <= 2000);
        // Rohe Nutzerfrage, Modelltext und Shadow-Ausgabe laufen alle durch diesen einen
        // Sendepfad: er muss standardmäßig Rollen, @everyone und User-Pings unterbinden.
        let body = faq_message_body("@everyone <@123> <@&456>", None);
        assert_eq!(
            body.get("content"),
            Some(&json!("@everyone <@123> <@&456>"))
        );
        assert_eq!(
            body.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
        assert!(body.get("components").is_none());

        // Mit Components (z. B. Close-Button) bleibt der Mention-Schutz erhalten.
        let with_components = faq_message_body("Antwort", Some(json!([{ "type": 1 }])));
        assert_eq!(
            with_components.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
        assert_eq!(
            with_components.get("components"),
            Some(&json!([{ "type": 1 }]))
        );
    }

    fn private_overwrite_fixture() -> Vec<serenity::all::PermissionOverwrite> {
        private_overwrites_for(1, 42, 99)
    }

    fn private_overwrites_for(
        guild_id: u64,
        owner_id: u64,
        bot_id: u64,
    ) -> Vec<serenity::all::PermissionOverwrite> {
        vec![
            serenity::all::PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
                kind: PermissionOverwriteType::Role(RoleId::new(guild_id)),
            },
            serenity::all::PermissionOverwrite {
                allow: Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
                kind: PermissionOverwriteType::Member(UserId::new(owner_id)),
            },
            serenity::all::PermissionOverwrite {
                allow: Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
                kind: PermissionOverwriteType::Member(UserId::new(bot_id)),
            },
        ]
    }

    fn test_adapter_with_channels(
        guild_id: u64,
        channels: Vec<serenity::all::GuildChannel>,
    ) -> Arc<DiscordAdapter> {
        let adapter = DiscordAdapter::new("test-token");
        let cache = Arc::new(serenity::all::Cache::new());
        let mut guild = serenity::all::Guild::default();
        guild.id = GuildId::new(guild_id);
        guild.channels = channels
            .into_iter()
            .map(|channel| (channel.id, channel))
            .collect();
        let mut event: serenity::all::GuildCreateEvent = serde_json::from_value(
            serde_json::to_value(guild).unwrap_or_else(|err| panic!("Guild JSON: {err}")),
        )
        .unwrap_or_else(|err| panic!("GuildCreateEvent JSON: {err}"));
        cache.update(&mut event);
        adapter.link_cache(cache);
        adapter
    }

    fn private_text_channel(
        guild_id: u64,
        channel_id: u64,
        owner_id: u64,
        topic: Option<&str>,
    ) -> serenity::all::GuildChannel {
        let mut channel = serenity::all::GuildChannel::default();
        channel.id = ChannelId::new(channel_id);
        channel.guild_id = GuildId::new(guild_id);
        channel.parent_id = Some(ChannelId::new(dl_community::faq::FAQ_CATEGORY_ID));
        channel.topic = topic.map(str::to_string);
        channel.permission_overwrites = private_overwrites_for(guild_id, owner_id, 1);
        channel
    }

    #[tokio::test]
    async fn faq_channel_category_lehnt_guild_null_ohne_panic_ab() {
        use dl_community::faq::FaqPort as _;

        let faq = FaqGlue {
            adapter: DiscordAdapter::new("test-token"),
        };

        assert_eq!(faq.channel_category(0, 1).await, None);
    }

    #[tokio::test]
    async fn faq_private_channel_lehnt_guild_null_ohne_panic_ab() {
        use dl_community::faq::FaqPort as _;

        let faq = FaqGlue {
            adapter: DiscordAdapter::new("test-token"),
        };

        assert!(!faq.private_faq_channel_owned_by_user(0, 1, 2).await);
    }

    #[tokio::test]
    async fn support_ownership_predicate_cache_matrix_hat_exakt_einen_owner() {
        use dl_community::{concierge::ConciergePort as _, faq::FaqPort as _};

        // Kein Dispatcher-Harness: Diese Matrix kombiniert die echten Cache-Prädikate.
        // Die Brain-Entry-Points deckt der Test `brain_deny_vermeidet_message_und_interaction_io` separat.
        let guild_id = crate::onboardglue::MAIN_GUILD_ID;
        let support_id = dl_community::concierge::SERVER_BOT_FRAGEN_CHANNEL_ID;
        let faq_id = 100;
        let concierge_id = 101;
        let gameplay_id = 102;
        let mut support_channel = serenity::all::GuildChannel::default();
        support_channel.id = ChannelId::new(support_id);
        support_channel.guild_id = GuildId::new(guild_id);
        let mut gameplay_channel = serenity::all::GuildChannel::default();
        gameplay_channel.id = ChannelId::new(gameplay_id);
        gameplay_channel.guild_id = GuildId::new(guild_id);
        let adapter = test_adapter_with_channels(
            guild_id,
            vec![
                support_channel,
                private_text_channel(guild_id, faq_id, 42, None),
                private_text_channel(guild_id, concierge_id, 42, Some("dl-concierge-owner:42")),
                gameplay_channel,
            ],
        );
        let faq = FaqGlue {
            adapter: adapter.clone(),
        };
        let concierge = ConciergeGlue { adapter };
        let retriever_calls = Arc::new(AtomicUsize::new(0));
        let answerer_calls = Arc::new(AtomicUsize::new(0));
        let brain = test_brain_handler(
            Some(HashSet::from([gameplay_id])),
            retriever_calls.clone(),
            answerer_calls.clone(),
        );
        let mut brain_calls = Vec::new();

        for (label, channel_id) in [
            ("Support", support_id),
            ("FAQ", faq_id),
            ("Concierge-Fallback", concierge_id),
            ("Gameplay", gameplay_id),
        ] {
            let brain_owner = brain.channel_allowed(channel_id);
            let owners = [
                channel_id == support_id,
                faq.private_faq_channel_owned_by_user(guild_id, channel_id, 42)
                    .await,
                concierge
                    .private_channel_owned_by_user(
                        guild_id,
                        channel_id,
                        42,
                        dl_community::faq::FAQ_CATEGORY_ID,
                    )
                    .await,
                brain_owner,
            ];
            assert_eq!(
                owners.into_iter().filter(|owned| *owned).count(),
                1,
                "{label}: {owners:?}"
            );
            if brain_owner {
                let context = dl_brain::DiscordQueryContext {
                    user_id: 42,
                    allow_discord_reads: true,
                    answer_context: None,
                };
                let _ = brain.outcome_for_question("Abrams?", &context, 42).await;
            }
            brain_calls.push((
                retriever_calls.load(std::sync::atomic::Ordering::Relaxed),
                answerer_calls.load(std::sync::atomic::Ordering::Relaxed),
            ));
        }

        assert_eq!(brain_calls, vec![(0, 0), (0, 0), (0, 0), (1, 1)]);
    }

    #[test]
    fn private_overwrites_erlauben_nur_owner_und_bot() {
        assert!(private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &private_overwrite_fixture(),
        ));
    }

    #[test]
    fn private_overwrites_brauchen_everyone_view_deny() {
        let mut overwrites = private_overwrite_fixture();
        overwrites.remove(0);

        assert!(!private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &overwrites,
        ));
    }

    #[test]
    fn private_overwrites_verbieten_breite_rollensicht() {
        let mut overwrites = private_overwrite_fixture();
        overwrites.push(serenity::all::PermissionOverwrite {
            allow: Permissions::VIEW_CHANNEL,
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Role(RoleId::new(7)),
        });

        assert!(!private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &overwrites,
        ));
    }

    #[test]
    fn private_overwrites_verbieten_fremde_membersicht() {
        let mut overwrites = private_overwrite_fixture();
        overwrites.push(serenity::all::PermissionOverwrite {
            allow: Permissions::VIEW_CHANNEL,
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Member(UserId::new(7)),
        });

        assert!(!private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &overwrites,
        ));
    }

    #[test]
    fn lfg_rankrollen_erkennen_ids_subranks_und_unverifiziert() {
        assert_eq!(
            rank_from_roles(&[(1331458016356208680, "irgendein name".to_string())]),
            ("Phantom".to_string(), 9, None)
        );
        assert_eq!(
            rank_from_roles(&[(0, "Asc 3".to_string())]),
            ("Ascendant".to_string(), 10, Some(3))
        );
        assert_eq!(
            rank_from_roles(&[(1492959889767534602, "x".to_string())]),
            ("Oracle".to_string(), 8, Some(3))
        );
        assert_eq!(
            rank_from_roles(&[(0, "Unverifiziert Emissary".to_string())]),
            ("Emissary".to_string(), 6, Some(3))
        );
    }

    #[test]
    fn lfg_offtopic_channels_werden_erkannt() {
        assert!(is_lfg_offtopic_channel("Off Topic Voice 1"));
        assert!(!is_lfg_offtopic_channel("Casual Lane 1"));
    }

    #[test]
    fn lfg_member_count_filtert_bots() {
        let members = visible_lfg_member_ids(&[(1, false), (2, true), (3, false)]);
        assert_eq!(members, vec![1, 3]);
    }
}
