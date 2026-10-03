//! Discord-Adapter für den gemeinsamen Guide-Kern. Keine Persona und keine Profile.

use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, DiscordAdapter, Dispatcher, InteractionHandler,
    InteractionRouter, MessageEvent,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OwnedMutexGuard, Semaphore};

fn community_question(content: &str) -> bool {
    let text = content.trim().to_lowercase();
    if text.is_empty() || text.contains("<@") || text.starts_with('>')
        || text.starts_with("danke") || text.starts_with("erledigt")
        || text.starts_with("hat sich") || text.starts_with("alles klar") {
        return false;
    }
    (text.ends_with('?') && ["wie ","wo ","wer ","was ","warum ","wann ","kann ","könnte ","gibt ","hat ","ist ","sind ","welche ","welcher "]
        .iter().any(|start|text.starts_with(start))) || ["wie kann ich ", "wo finde ich ", "kann mir jemand ",
        "ich brauche hilfe", "ich möchte deadlock spielen", "kann mich jemand einladen"]
        .iter().any(|start| text.starts_with(start))
}

const UNAVAILABLE: &str = "Der Serverguide ist gerade nicht erreichbar. Du kannst deine Frage in <#1491953161747955853> stellen.";
const PILOT_CLOSED: &str = "Der Serverguide ist hier noch nicht freigeschaltet. Für Hilfe erreichst du die Community in <#1426220702054355077>.";
const LEGACY_DISABLED: &str = "Diese frühere Aktion ist ausgeschaltet. Hilfe bekommst du in <#1426220702054355077>, Mitspieler findest du in <#1376335502919335936>.";

#[derive(Clone)]
pub struct GuideConfig {
    pub enabled: bool,
    pub guild_id: u64,
    pub test_users: HashSet<u64>,
    pub public_channels: HashSet<u64>,
    pub proactive_channels: HashSet<u64>,
    pub base_url: String,
    pub timeout: Duration,
    pub idle_timeout: Duration,
    pub moderator_channel_id: u64,
    pub display_name: String,
    pub source_sync_enabled: bool,
    pub source_channels: HashSet<u64>,
    pub rule_channels: HashSet<u64>,
    pub public_roles: HashSet<u64>,
    pub source_interval: Duration,
}

impl GuideConfig {
    pub fn from_lookup(guild_id: u64, lookup: impl Fn(&str) -> Option<String>) -> Self {
        let ids = |key| {
            lookup(key)
                .unwrap_or_default()
                .split(',')
                .filter_map(|v| v.trim().parse::<u64>().ok())
                .filter(|v| *v > 0)
                .collect()
        };
        let seconds = |key, default| {
            lookup(key)
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0 && *v <= 3600)
                .unwrap_or(default)
        };
        Self {
            enabled: lookup("DL_GUIDE_ENABLED").is_some_and(|v| v == "true" || v == "1"),
            guild_id,
            test_users: ids("DL_GUIDE_TEST_USERS"),
            public_channels: ids("DL_GUIDE_PUBLIC_CHANNELS"),
            proactive_channels: ids("DL_GUIDE_PROACTIVE_CHANNELS"),
            base_url: lookup("DL_GUIDE_URL").unwrap_or_else(|| "http://127.0.0.1:8788".into()),
            timeout: Duration::from_secs(seconds("DL_GUIDE_TIMEOUT_SECONDS", 100)),
            idle_timeout: Duration::from_secs(seconds("DL_GUIDE_IDLE_SECONDS", 300)),
            moderator_channel_id: lookup("DL_GUIDE_MODERATOR_CHANNEL_ID")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1315684135175716978),
            display_name: lookup("DL_GUIDE_DISPLAY_NAME")
                .filter(|value| {
                    !value.trim().is_empty()
                        && value.len() <= 80
                        && !value.to_lowercase().contains("brain")
                })
                .unwrap_or_else(|| "Serverguide".into()),
            source_sync_enabled: lookup("DL_GUIDE_SOURCE_SYNC_ENABLED")
                .is_some_and(|v| v == "true" || v == "1"),
            source_channels: ids("DL_GUIDE_SOURCE_CHANNELS"),
            rule_channels: ids("DL_GUIDE_RULE_CHANNELS"),
            public_roles: ids("DL_GUIDE_PUBLIC_ROLES"),
            source_interval: Duration::from_secs(
                seconds("DL_GUIDE_SOURCE_INTERVAL_SECONDS", 60).max(60),
            ),
        }
    }

    fn proactive_question(&self, event: &MessageEvent) -> bool {
        let age = chrono::Utc::now().signed_duration_since(event.message_created_at).num_seconds();
        self.guild_id == 1289721245281292288
            && event.guild_id == Some(self.guild_id)
            && event.channel_id == 1426220702054355077
            && self.proactive_channels.contains(&event.channel_id)
            && self.public_channels.contains(&event.channel_id)
            && event.reply_message_id.is_none()
            && !event.content.contains("<@")
            && community_question(&event.content)
            && (0..=60).contains(&age)
    }

    fn allowed(&self, user_id: u64) -> bool {
        self.enabled && !self.test_users.is_empty() && self.test_users.contains(&user_id)
    }
}

#[derive(Serialize)]
pub struct GuideTurn {
    request_id: String,
    guild_id: String,
    user_id: String,
    channel_id: String,
    message_id: String,
    thread_id: Option<String>,
    reply_to_message_id: Option<String>,
    conversation_id: Option<String>,
    bot_user_id: Option<String>,
    surface: &'static str,
    addressed: &'static str,
    content: String,
    event: &'static str,
    control: Option<Value>,
    human_helped: bool,
}

#[derive(Default, Deserialize)]
struct GuideReply {
    #[serde(default)]
    contract_version: String,
    #[serde(default)]
    request_id: String,
    status: String,
    reply: Option<String>,
    conversation_id: Option<String>,
    #[serde(default)]
    actions: Vec<GuideAction>,
    memory_notice: Option<String>,
    #[serde(default)]
    contact_proactive: bool,
    profile: Option<Value>,
    control_result: Option<String>,
    privacy_epoch: Option<i64>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GuideAction {
    Feedback {
        delivery_id: String,
        destination_channel_id: String,
        text: String,
    },
}

struct Conversation {
    id: Option<String>,
    bot_messages: HashSet<u64>,
    last_user_message: u64,
    updated: Instant,
    interrupted: bool,
    epoch: u64,
}

#[derive(Default)]
struct Routes {
    conversations: HashMap<(u64, u64, u64), Conversation>,
    sequence: u64,
    channel_activity: HashMap<(u64, u64), ChannelActivity>,
}

struct ChannelActivity {
    author: u64,
    sequence: u64,
    previous_other: u64,
}

/// Hält die vorhandene Datenschutzsperre bis zur tatsächlichen Interaction-Zustellung.
struct PrivacySendHook(Mutex<Option<sqlx::Transaction<'static, sqlx::Postgres>>>);

#[async_trait::async_trait]
impl dl_discord::ResponseMessageHook for PrivacySendHook {
    async fn on_response_message(&self, _message_id: u64) {
        if let Some(tx) = self.0.lock().await.take() {
            if tx.commit().await.is_err() {
                tracing::warn!("Datenschutzsperre nach Guide-Zustellung konnte nicht bestätigt freigegeben werden");
            }
        }
    }
}

impl Routes {
    fn observe_human(&mut self, event: &MessageEvent) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        let sequence = self.sequence;
        if let Some(guild) = event.guild_id {
            self.channel_activity
                .entry((guild, event.channel_id))
                .and_modify(|activity| {
                    if activity.author != event.author_id {
                        activity.previous_other = activity.sequence;
                    }
                    activity.author = event.author_id;
                    activity.sequence = sequence;
                })
                .or_insert(ChannelActivity {
                    author: event.author_id,
                    sequence,
                    previous_other: 0,
                });
            for ((channel_guild, channel, user), conversation) in &mut self.conversations {
                if *channel_guild == guild
                    && *channel == event.channel_id
                    && *user != event.author_id
                {
                    conversation.interrupted = true;
                }
            }
        }
        self.sequence
    }
    fn human_after(&self, event: &MessageEvent, received_sequence: u64) -> bool {
        event.guild_id.is_some_and(|guild| {
            self.channel_activity
                .get(&(guild, event.channel_id))
                .is_some_and(|activity| {
                    let other = if activity.author == event.author_id {
                        activity.previous_other
                    } else {
                        activity.sequence
                    };
                    other > received_sequence
                })
        })
    }
    fn addressed(
        &mut self,
        event: &MessageEvent,
        bot: Option<u64>,
        idle: Duration,
    ) -> Option<(&'static str, String, Option<String>)> {
        self.conversations
            .retain(|_, value| value.updated.elapsed() < idle);
        let guild = event.guild_id.unwrap_or(0);
        let key = (guild, event.channel_id, event.author_id);
        if event.guild_id.is_none() {
            return Some(("dm", event.content.clone(), None));
        }
        if let Some(bot) = bot {
            for mention in [format!("<@{bot}>"), format!("<@!{bot}>")] {
                if event.content.contains(&mention) {
                    return Some((
                        "mention",
                        event.content.replace(&mention, "").trim().into(),
                        self.conversations.get(&key).and_then(|v| v.id.clone()),
                    ));
                }
            }
        }
        if let Some(content) = event
            .content
            .strip_prefix("!brain ")
            .or_else(|| event.content.strip_prefix("!serverguide "))
        {
            return Some((
                "command",
                content.trim().into(),
                self.conversations.get(&key).and_then(|v| v.id.clone()),
            ));
        }
        if let Some(conversation) = self.conversations.get(&key) {
            let reply_here = event
                .reply_channel_id
                .is_none_or(|channel| channel == event.channel_id);
            if reply_here
                && event
                    .reply_message_id
                    .is_some_and(|id| conversation.bot_messages.contains(&id))
            {
                return Some(("reply", event.content.clone(), conversation.id.clone()));
            }
            let reply_other = event
                .reply_message_id
                .is_some_and(|id| id != conversation.last_user_message);
            if !conversation.interrupted && !reply_other {
                return Some(("followup", event.content.clone(), conversation.id.clone()));
            }
        }
        for ((channel_guild, channel, user), conversation) in &mut self.conversations {
            if *channel_guild == guild && *channel == event.channel_id && *user != event.author_id {
                // Sobald ein Mensch eingreift, braucht der Guide eine neue ausdrückliche Ansprache.
                conversation.interrupted = true;
            }
        }
        None
    }
}

pub struct GuideAdapter {
    config: GuideConfig,
    http: reqwest::Client,
    adapter: Arc<DiscordAdapter>,
    pool: PgPool,
    feedback: Arc<dl_community::feedback_hub::FeedbackHub>,
    auth_token: Option<String>,
    privacy_epochs: std::sync::Mutex<HashMap<u64, u64>>,
    routes: Mutex<Routes>,
    turn_locks: Mutex<HashMap<u64, Arc<Mutex<()>>>>,
    turn_capacity: Arc<Semaphore>,
    privacy_capacity: Arc<Semaphore>,
}

impl GuideAdapter {
    pub fn clear_user_runtime(&self, user_id: u64) {
        if let Ok(mut epochs) = self.privacy_epochs.lock() {
            let epoch = epochs.entry(user_id).or_default();
            *epoch = epoch.saturating_add(1);
        }
        if let Ok(mut routes) = self.routes.try_lock() {
            routes
                .conversations
                .retain(|(_, _, user), _| *user != user_id);
        }
    }

    fn public_text(&self, text: &str) -> String {
        text.replace("Serverguide", &self.config.display_name)
    }
    pub fn new(
        config: GuideConfig,
        adapter: Arc<DiscordAdapter>,
        pool: PgPool,
        feedback: Arc<dl_community::feedback_hub::FeedbackHub>,
        auth_token: Option<String>,
    ) -> anyhow::Result<Arc<Self>> {
        let url = reqwest::Url::parse(&config.base_url)?;
        anyhow::ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1") | Some("[::1]"))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none(),
            "Guide-Adresse muss lokal sein"
        );
        Ok(Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(config.timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            config,
            adapter,
            pool,
            feedback,
            auth_token,
            privacy_epochs: std::sync::Mutex::new(HashMap::new()),
            routes: Mutex::new(Routes::default()),
            turn_locks: Mutex::new(HashMap::new()),
            turn_capacity: Arc::new(Semaphore::new(64)),
            privacy_capacity: Arc::new(Semaphore::new(4)),
        }))
    }

    async fn claim(&self, request_id: &str) -> Result<bool, sqlx::Error> {
        let key = hex::encode(Sha256::digest(request_id.as_bytes()));
        sqlx::query("INSERT INTO bot.serverguide_discord_events(event_key) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(key).execute(&self.pool).await.map(|result| result.rows_affected() == 1)
    }

    async fn sync_server_sources(&self) -> anyhow::Result<()> {
        use serenity::all::{
            ChannelId, ChannelType, GuildId, MessageId, PermissionOverwriteType, Permissions,
        };
        anyhow::ensure!(
            !self.config.source_channels.is_empty(),
            "Quellenliste ist leer"
        );
        let token = self
            .auth_token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Guide-Zugang fehlt"))?;
        let guild = GuildId::new(self.config.guild_id);
        let channels = self.adapter.http.get_channels(guild).await?;
        let roles = self.adapter.http.get_guild_roles(guild).await?;
        let everyone = roles
            .iter()
            .find(|role| role.id.get() == self.config.guild_id)
            .map(|role| role.permissions)
            .unwrap_or_default();
        let mut channel_rows = Vec::new();
        let mut rule_rows = Vec::new();
        for id in &self.config.source_channels {
            let Some(channel) = channels.iter().find(|channel| channel.id.get() == *id) else {
                channel_rows.push(json!({"id":id.to_string(),"name":"","kind":"text","public_readable":false,"deleted":true}));
                continue;
            };
            let mut permissions = everyone;
            for overwrite in &channel.permission_overwrites {
                if overwrite.kind
                    == PermissionOverwriteType::Role(serenity::all::RoleId::new(
                        self.config.guild_id,
                    ))
                {
                    permissions.remove(overwrite.deny);
                    permissions.insert(overwrite.allow);
                }
            }
            let safe = *id != self.config.moderator_channel_id
                && *id != 1411409828126920785
                && !channel.name.to_lowercase().contains("ticket")
                && !channel.name.starts_with("closed-")
                && channel.parent_id.map(|id| id.get()) != Some(1459628097145147645)
                && permissions.contains(Permissions::VIEW_CHANNEL)
                && matches!(
                    channel.kind,
                    ChannelType::Text
                        | ChannelType::News
                        | ChannelType::Voice
                        | ChannelType::Category
                        | ChannelType::Forum
                );
            let mut readable = safe;
            if safe && self.config.rule_channels.contains(id) {
                // Ausschließlich zuvor freigegebene öffentliche Regeln lesen. Keine freie Suche.
                let mut before = None;
                let mut complete = false;
                let mut fetched = Vec::new();
                for _ in 0..5 {
                    let pagination = before
                        .map(|id| serenity::http::MessagePagination::Before(MessageId::new(id)));
                    match self
                        .adapter
                        .http
                        .get_messages(ChannelId::new(*id), pagination, Some(100))
                        .await
                    {
                        Ok(messages) => {
                            let count = messages.len();
                            before = messages.iter().map(|message| message.id.get()).min();
                            fetched.extend(messages);
                            if count < 100 {
                                complete = true;
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                if complete {
                    for message in fetched {
                        if !message.content.trim().is_empty() {
                            rule_rows.push(json!({"channel_id":id.to_string(),"message_id":message.id.to_string(),"text":message.content,"updated_at":message.edited_timestamp.unwrap_or(message.timestamp).unix_timestamp(),"deleted":false}));
                        }
                    }
                } else {
                    // Rechteverlust oder unvollständiger Abruf entzieht auch abgeleiteten Regeln den Zugriff.
                    readable = false;
                }
            }
            channel_rows.push(json!({"id":id.to_string(),"name":if readable {channel.name.as_str()} else {""},"kind":format!("{:?}",channel.kind).to_lowercase(),"public_readable":readable,"deleted":!readable}));
        }
        let public_roles: Vec<Value> = roles
            .iter()
            .filter(|role| self.config.public_roles.contains(&role.id.get()))
            .map(|role| json!({"id":role.id.to_string(),"name":role.name,"public":true}))
            .collect();
        let now = chrono::Utc::now();
        self.http.post(format!("{}/v1/guide/server-snapshot",self.config.base_url.trim_end_matches('/')))
            .bearer_auth(token).json(&json!({"guild_id":self.config.guild_id.to_string(),"revision":now.timestamp_millis() as u64,"observed_at":now.timestamp(),"channels":channel_rows,"roles":public_roles,"rules":rule_rows}))
            .send().await?.error_for_status()?;
        Ok(())
    }

    fn public_allowed(&self, channel_id: u64) -> bool {
        if channel_id == self.config.moderator_channel_id {
            return false;
        }
        let Some(guild) = self.adapter.cache().guild(self.config.guild_id) else {
            return false;
        };
        let Some(channel) = guild
            .channels
            .get(&serenity::all::ChannelId::new(channel_id))
            .or_else(|| {
                guild
                    .threads
                    .iter()
                    .find(|thread| thread.id.get() == channel_id)
            })
        else {
            return false;
        };
        let everyone = guild
            .roles
            .get(&serenity::all::RoleId::new(self.config.guild_id))
            .map(|role| role.permissions);
        if !public_view_allowed(self.config.guild_id, everyone, channel) {
            return false;
        }
        let parent_allowed = matches!(
            channel.kind,
            serenity::all::ChannelType::PublicThread | serenity::all::ChannelType::NewsThread
        ) && channel.parent_id.is_some_and(|parent| {
            self.config.public_channels.contains(&parent.get())
                && guild.channels.get(&parent).is_some_and(|parent| {
                    public_view_allowed(self.config.guild_id, everyone, parent)
                        && !parent.name.starts_with("ticket-")
                        && !parent.name.starts_with("closed-")
                        && parent.id.get() != self.config.moderator_channel_id
                        && parent.parent_id.map(|id| id.get()) != Some(1459628097145147645)
                })
        });
        if matches!(
            channel.kind,
            serenity::all::ChannelType::PublicThread | serenity::all::ChannelType::NewsThread
        ) && !parent_allowed
        {
            return false;
        }
        if !self.config.public_channels.contains(&channel_id) && !parent_allowed {
            return false;
        }
        !channel.name.starts_with("ticket-")
            && !channel.name.starts_with("closed-")
            && (channel.parent_id.map(|v| v.get()) != Some(1459628097145147645)
                || channel_id == 1491953161747955853)
    }

    async fn request(&self, turn: &GuideTurn) -> anyhow::Result<GuideReply> {
        let token = self
            .auth_token
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Guide-Zugang fehlt"))?;
        let reply: GuideReply = self
            .http
            .post(format!(
                "{}/v1/guide/turn",
                self.config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(token)
            .json(turn)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::ensure!(
            reply.contract_version == "guide.v1"
                && reply.request_id == turn.request_id
                && matches!(reply.status.as_str(), "reply" | "silent" | "unavailable")
                && !reply.contact_proactive
                && (turn.surface == "dm" || reply.profile.is_none()),
            "Guide-Vertrag wurde verletzt"
        );
        Ok(reply)
    }

    async fn complete_control(
        &self,
        turn: &GuideTurn,
        reply: &mut GuideReply,
    ) -> anyhow::Result<()> {
        if reply.control_result.as_deref() != Some("forget") {
            return Ok(());
        }
        anyhow::ensure!(
            turn.surface == "dm",
            "Löschen braucht einen privaten Kontrollweg"
        );
        let user_id = turn.user_id.parse::<i64>()?;
        dl_community::privacy::delete_user_data(
            &self.pool,
            user_id,
            "Eigener Wunsch beim Serverguide".into(),
            chrono::Utc::now().timestamp(),
        )
        .await?;
        self.clear_user_runtime(user_id as u64);
        reply.privacy_epoch = sqlx::query_scalar(
            "SELECT epoch FROM brain.guide_subjects WHERE guild_id=$1 AND user_id=$2",
        )
        .bind(&turn.guild_id)
        .bind(&turn.user_id)
        .fetch_optional(&self.pool)
        .await?;
        reply.reply = Some("Deine gespeicherten Angaben und Verläufe wurden gelöscht. Bereits versendete Discord-Nachrichten werden dadurch nicht entfernt.".into());
        Ok(())
    }

    async fn final_send_lock(
        &self,
        turn: &GuideTurn,
        reply: &GuideReply,
    ) -> anyhow::Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        let mut tx = self.pool.begin().await?;
        dl_central_db::lock_user_privacy(&mut tx, turn.user_id.parse::<i64>()?).await?;
        if let Some(expected) = reply.privacy_epoch {
            let current: Option<i64> = sqlx::query_scalar(
                "SELECT epoch FROM brain.guide_subjects WHERE guild_id=$1 AND user_id=$2",
            )
            .bind(&turn.guild_id)
            .bind(&turn.user_id)
            .fetch_optional(&mut *tx)
            .await?;
            anyhow::ensure!(
                current == Some(expected),
                "Datenschutzstand der Antwort ist überholt"
            );
        } else {
            anyhow::ensure!(
                reply.conversation_id.is_none()
                    && reply.profile.is_none()
                    && reply.control_result.is_none()
                    && reply.status == "unavailable",
                "Guide-Antwort ohne Datenschutzstand"
            );
        }
        Ok(tx)
    }

    async fn process_actions(
        &self,
        turn: &GuideTurn,
        reply: &mut GuideReply,
    ) -> anyhow::Result<()> {
        for action in std::mem::take(&mut reply.actions) {
            match action {
                GuideAction::Feedback {
                    delivery_id,
                    destination_channel_id,
                    text,
                } => {
                    anyhow::ensure!(
                        destination_channel_id == self.config.moderator_channel_id.to_string(),
                        "Feedbackziel nicht freigegeben"
                    );
                    let sent = self
                        .feedback
                        .deliver_guide_feedback(
                            &delivery_id,
                            &turn.guild_id,
                            &turn.user_id,
                            self.config.moderator_channel_id,
                            &text,
                        )
                        .await;
                    let token = self
                        .auth_token
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("Guide-Zugang fehlt"))?;
                    let result: GuideReply = self.http.post(format!("{}/v1/guide/action-result", self.config.base_url.trim_end_matches('/'))).bearer_auth(token)
                        .json(&json!({"request_id": format!("{}:delivery", turn.request_id), "guild_id": turn.guild_id, "user_id": turn.user_id, "delivery_id": delivery_id, "success": sent.is_ok(), "sent_message_id": sent.as_ref().ok().map(ToString::to_string), "reply_message_id": null}))
                        .send().await?.error_for_status()?.json().await?;
                    anyhow::ensure!(
                        result.contract_version == "guide.v1"
                            && result.request_id == format!("{}:delivery", turn.request_id)
                            && !result.contact_proactive
                            && result.actions.is_empty()
                            && matches!(result.status.as_str(), "reply" | "silent" | "unavailable")
                            && (turn.surface == "dm" || result.profile.is_none()),
                        "Guide-Zustellvertrag wurde verletzt"
                    );
                    reply.reply = result.reply;
                    reply.status = result.status;
                    reply.privacy_epoch = result.privacy_epoch;
                }
            }
        }
        Ok(())
    }

    async fn try_user_turn(&self, user_id: u64) -> Option<OwnedMutexGuard<()>> {
        self.turn_locks
            .lock()
            .await
            .entry(user_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
            .try_lock_owned()
            .ok()
    }

    async fn handle_message(&self, event: MessageEvent, received_sequence: u64) {
        let bot = self.adapter.bot_user_id_cell().get().copied();
        if bot == Some(event.author_id) {
            return;
        }
        if !self.config.allowed(event.author_id) {
            return;
        }
        let Some(_user_turn) = self.try_user_turn(event.author_id).await else {
            return;
        };
        let mut routes = self.routes.lock().await;
        if routes.human_after(&event, received_sequence) {
            return;
        }
        let epoch = match self.privacy_epochs.lock() {
            Ok(epochs) => epochs.get(&event.author_id).copied().unwrap_or(0),
            Err(_) => return,
        };
        routes
            .conversations
            .retain(|(_, _, user), value| *user != event.author_id || value.epoch == epoch);
        if !self.config.allowed(event.author_id)
            || event
                .guild_id
                .is_some_and(|guild| guild != self.config.guild_id)
            || event.content.trim().is_empty()
        {
            return;
        }
        if event.guild_id.is_some() && !self.public_allowed(event.channel_id) {
            return;
        }
        // Die Zuordnung wird kurz gesperrt; menschliche Hilfe bleibt während des Kernaufrufs sichtbar.
        let Some((addressed, content, conversation_id)) =
            routes.addressed(&event, bot, self.config.idle_timeout).or_else(|| {
                self.config.proactive_question(&event).then(||
                    ("community_question", event.content.clone(), None))
            })
        else {
            return;
        };
        let request_id = format!("discord:message:{}", event.message_id);
        match self.claim(&request_id).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(_) => {
                tracing::warn!("Guide-Ereignis konnte nicht sicher reserviert werden");
                return;
            }
        }
        let route_key = (
            event.guild_id.unwrap_or(0),
            event.channel_id,
            event.author_id,
        );
        routes
            .conversations
            .entry(route_key)
            .and_modify(|conversation| {
                conversation.interrupted = false;
                conversation.last_user_message = event.message_id;
                conversation.updated = Instant::now();
            })
            .or_insert_with(|| Conversation {
                id: conversation_id.clone(),
                bot_messages: HashSet::new(),
                last_user_message: event.message_id,
                updated: Instant::now(),
                interrupted: false,
                epoch,
            });
        drop(routes);
        let thread_id = event.guild_id.and_then(|guild| {
            self.adapter.cache().guild(guild).and_then(|guild| {
                guild
                    .threads
                    .iter()
                    .find(|channel| channel.id.get() == event.channel_id)
                    .map(|channel| channel.id.to_string())
            })
        });
        let turn = GuideTurn {
            request_id,
            guild_id: self.config.guild_id.to_string(),
            user_id: event.author_id.to_string(),
            channel_id: event.channel_id.to_string(),
            message_id: event.message_id.to_string(),
            thread_id,
            reply_to_message_id: event.reply_message_id.map(|v| v.to_string()),
            conversation_id,
            bot_user_id: bot.map(|v| v.to_string()),
            surface: if event.guild_id.is_none() {
                "dm"
            } else {
                "public"
            },
            addressed,
            content,
            event: "message",
            control: None,
            human_helped: false,
        };
        let mut reply = match self.request(&turn).await {
            Ok(reply) => reply,
            Err(_) => GuideReply {
                status: "unavailable".into(),
                reply: Some(self.public_text(UNAVAILABLE)),
                ..Default::default()
            },
        };
        if event.guild_id.is_some()
            && reply.actions.is_empty()
            && self
                .routes
                .lock()
                .await
                .conversations
                .get(&route_key)
                .is_none_or(|conversation| conversation.interrupted)
        {
            return;
        }
        if self
            .privacy_epochs
            .lock()
            .map(|epochs| epochs.get(&event.author_id).copied().unwrap_or(0) != epoch)
            .unwrap_or(true)
        {
            return;
        }
        let forget_requested = reply.control_result.as_deref() == Some("forget");
        if self.complete_control(&turn, &mut reply).await.is_err() {
            reply.status = "unavailable".into();
            reply.reply = Some("Die Löschung konnte nicht vollständig bestätigt werden. Bitte versuch es später nochmal.".into());
        }
        if self.process_actions(&turn, &mut reply).await.is_err() {
            reply.status = "unavailable".into();
            reply.reply = Some("Die Weiterleitung lässt sich gerade nicht sicher bestätigen. Bei Bedarf erreichst du das Team über die vorhandenen Serverwege.".into());
        }
        if reply.status == "silent" {
            return;
        }
        if !forget_requested
            && self
                .privacy_epochs
                .lock()
                .map(|epochs| epochs.get(&event.author_id).copied().unwrap_or(0) != epoch)
                .unwrap_or(true)
        {
            return;
        }
        let final_lock = match self.final_send_lock(&turn, &reply).await {
            Ok(lock) => lock,
            Err(_) => return,
        };
        let Some(text) = visible_reply(&reply) else {
            tracing::warn!("Guide-Antwort verletzt sichtbare Textgrenzen");
            return;
        };
        let body = json!({"content": text, "allowed_mentions": {"parse": []}, "message_reference": {"message_id": event.message_id.to_string(), "fail_if_not_exists": false}, "nonce": event.message_id.to_string(), "enforce_nonce": true});
        let Some(body) = body.as_object() else {
            return;
        };
        // Die letzte Prüfung steht unmittelbar vor dem Discordversand, ohne Kern-HTTP unter der Routensperre.
        let mut routes = self.routes.lock().await;
        if event.guild_id.is_some()
            && (routes.human_after(&event, received_sequence)
                || routes
                    .conversations
                    .get(&route_key)
                    .is_none_or(|conversation| conversation.interrupted))
        {
            return;
        }
        if let Ok(message_id) = self.adapter.send_raw_public(event.channel_id, body).await {
            if final_lock.commit().await.is_err() {
                return;
            }
            let key = (
                event.guild_id.unwrap_or(0),
                event.channel_id,
                event.author_id,
            );
            if is_end(&event.content) {
                routes.conversations.remove(&key);
            } else {
                let conversation_id = reply.conversation_id.clone();
                routes.conversations.insert(
                    key,
                    Conversation {
                        id: reply.conversation_id,
                        bot_messages: HashSet::from([message_id]),
                        last_user_message: event.message_id,
                        updated: Instant::now(),
                        interrupted: false,
                        epoch,
                    },
                );
                drop(routes);
                if let Some(conversation_id) = &conversation_id {
                    if let Some(token) = &self.auth_token {
                        let ack = self.http.post(format!("{}/v1/guide/action-result", self.config.base_url.trim_end_matches('/'))).bearer_auth(token)
                            .json(&json!({"request_id": format!("{}:reply", turn.request_id), "guild_id": turn.guild_id, "user_id": turn.user_id, "delivery_id": format!("reply:{conversation_id}"), "success": true, "sent_message_id": null, "reply_message_id": message_id.to_string()})).send().await;
                        if !ack.is_ok_and(|response| response.status().is_success()) {
                            tracing::warn!("Guide-Unterhaltungszuordnung wurde nicht bestätigt");
                            return;
                        }
                    }
                }
            }
        } else {
            tracing::warn!("Guide-Antwort konnte nicht bestätigt zugestellt werden");
        }
    }
}

fn public_view_allowed(
    guild_id: u64,
    everyone: Option<serenity::all::Permissions>,
    channel: &serenity::all::GuildChannel,
) -> bool {
    use serenity::all::{ChannelType, PermissionOverwriteType, Permissions, RoleId};
    if !matches!(
        channel.kind,
        ChannelType::Text
            | ChannelType::News
            | ChannelType::Voice
            | ChannelType::Stage
            | ChannelType::Forum
            | ChannelType::PublicThread
            | ChannelType::NewsThread
    ) {
        return false;
    }
    let Some(mut permissions) = everyone else {
        return false;
    };
    if permissions.contains(Permissions::ADMINISTRATOR) {
        return true;
    }
    for overwrite in &channel.permission_overwrites {
        if overwrite.kind == PermissionOverwriteType::Role(RoleId::new(guild_id)) {
            permissions.remove(overwrite.deny);
            permissions.insert(overwrite.allow);
        }
    }
    permissions.contains(Permissions::VIEW_CHANNEL)
}

fn is_end(content: &str) -> bool {
    matches!(
        content
            .trim()
            .to_lowercase()
            .trim_end_matches(['.', '!', ':', ')'])
            .trim(),
        "danke" | "dankeschön" | "alles klar" | "passt" | "bis später" | "tschüss" | "das war's"
    )
}

fn visible_reply(reply: &GuideReply) -> Option<String> {
    let mut text = reply.reply.clone()?;
    if let Some(notice) = &reply.memory_notice {
        text.push_str("\n\n");
        text.push_str(notice);
    }
    if text.encode_utf16().count() > 2000
        || text.to_lowercase().contains("deadlock brain")
        || text.to_lowercase().contains("deadlock-brain")
        || text.chars().any(|c| matches!(c, '\u{2014}' | '\u{2013}'))
    {
        return None;
    }
    Some(text)
}

#[async_trait::async_trait]
impl InteractionHandler for GuideAdapter {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let epoch = match self.privacy_epochs.lock() {
            Ok(epochs) => epochs.get(&interaction.user_id).copied().unwrap_or(0),
            Err(_) => return BridgeReply::ephemeral_text(self.public_text(UNAVAILABLE)),
        };
        if interaction.custom_id.starts_with("concierge:pate:")
            || interaction.custom_id.starts_with("concierge:steckbrief:")
        {
            return BridgeReply::ephemeral_text(self.public_text(LEGACY_DISABLED));
        }
        if !self.config.allowed(interaction.user_id) {
            return BridgeReply::ephemeral_text(self.public_text(PILOT_CLOSED));
        }
        if interaction.guild_id != 0 && interaction.guild_id != self.config.guild_id {
            return BridgeReply::ephemeral_text(self.public_text(PILOT_CLOSED));
        }
        if interaction.guild_id != 0 && !self.public_allowed(interaction.channel_id) {
            return BridgeReply::ephemeral_text(
                "Hier kann der Guide keine Inhalte verwenden. Schreib ihm bitte direkt.",
            );
        }
        if interaction.custom_id == "feedback_hub:open_modal" {
            return dl_community::feedback_hub::feedback_modal();
        }
        let (control, control_error) = profile_control(&interaction);
        if let Some(error) = control_error {
            return BridgeReply::ephemeral_text(error);
        }
        // Löschen und Erinnerung ausschalten dürfen laufende Antwortturns durch die Epoche verwerfen.
        let privacy_interrupt = control.as_ref().is_some_and(|control| {
            control.get("type").and_then(Value::as_str) == Some("forget")
                || (control.get("type").and_then(Value::as_str) == Some("memory")
                    && control.get("enabled").and_then(Value::as_bool) == Some(false))
        });
        let capacity = if privacy_interrupt {
            self.privacy_capacity.clone()
        } else {
            self.turn_capacity.clone()
        };
        let Ok(_permit) = capacity.try_acquire_owned() else {
            return BridgeReply::ephemeral_text(
                "Ich bin gerade ausgelastet. Bitte versuch es gleich nochmal.",
            );
        };
        let _user_turn = if privacy_interrupt {
            None
        } else {
            let Some(lock) = self.try_user_turn(interaction.user_id).await else {
                return BridgeReply::ephemeral_text(
                    "Deine vorige Anfrage läuft noch. Bitte warte kurz.",
                );
            };
            Some(lock)
        };
        let request_id = format!("discord:interaction:{}", interaction.interaction_id);
        match self.claim(&request_id).await {
            Ok(true) => {}
            Ok(false) => {
                return BridgeReply::ephemeral_text("Diese Aktion wurde bereits verarbeitet.")
            }
            Err(_) => return BridgeReply::ephemeral_text(UNAVAILABLE),
        }
        let event = if control.is_some() {
            "profile_control"
        } else if interaction.custom_id == "feedback_hub:submit" {
            "feedback_submit"
        } else if interaction.custom_id.starts_with("tour:v1:next:") {
            "tour_step"
        } else if interaction.custom_id.contains("tour")
            || interaction.custom_id == "faq_chat:start"
            || interaction.command == "faq"
        {
            "tour_start"
        } else {
            "message"
        };
        let content = if interaction.command == "serverguide-profil" {
            "Eigene gespeicherte Angaben verwalten".into()
        } else if interaction.command == "brain" || interaction.command == "serverguide" {
            interaction
                .options
                .get("frage")
                .or_else(|| interaction.options.get("question"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        } else if event == "feedback_submit" {
            interaction
                .options
                .iter()
                .map(|(key, value)| format!("{key}: {}", value.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n")
        } else if event == "tour_start" {
            "Server-Tour starten".into()
        } else if interaction.custom_id == "concierge:play" {
            "Ich suche Mitspieler und möchte gemeinsam spielen".into()
        } else if interaction.custom_id == "concierge:later"
            || interaction.custom_id.starts_with("faq_chat:close")
        {
            "Gespräch beenden".into()
        } else {
            interaction.custom_id.clone()
        };
        let turn = GuideTurn {
            request_id,
            guild_id: self.config.guild_id.to_string(),
            user_id: interaction.user_id.to_string(),
            channel_id: interaction.channel_id.to_string(),
            message_id: interaction.interaction_id.to_string(),
            thread_id: None,
            reply_to_message_id: interaction.message_id.map(|v| v.to_string()),
            conversation_id: None,
            bot_user_id: self
                .adapter
                .bot_user_id_cell()
                .get()
                .map(ToString::to_string),
            surface: if interaction.guild_id == 0 || control.is_some() {
                "dm"
            } else {
                "public"
            },
            addressed: if control.is_some() {
                "control"
            } else if event == "tour_start" || event == "tour_step" {
                "tour_button"
            } else {
                "command"
            },
            content,
            event,
            control,
            human_helped: false,
        };
        let mut reply = match self.request(&turn).await {
            Ok(reply) => reply,
            Err(_) => return BridgeReply::ephemeral_text(UNAVAILABLE),
        };
        if self
            .privacy_epochs
            .lock()
            .map(|epochs| epochs.get(&interaction.user_id).copied().unwrap_or(0) != epoch)
            .unwrap_or(true)
        {
            return BridgeReply::ephemeral_text(
                "Diese Antwort wurde nach einer Datenschutzänderung verworfen.",
            );
        }
        if self.complete_control(&turn, &mut reply).await.is_err() {
            return BridgeReply::ephemeral_text("Die Löschung konnte nicht vollständig bestätigt werden. Bitte versuch es später nochmal.");
        }
        let forget_completed = reply.control_result.as_deref() == Some("forget");
        if self.process_actions(&turn, &mut reply).await.is_err() {
            return BridgeReply::ephemeral_text("Die Weiterleitung lässt sich gerade nicht sicher bestätigen. Bei Bedarf erreichst du das Team über die vorhandenen Serverwege.");
        }
        if !forget_completed
            && self
                .privacy_epochs
                .lock()
                .map(|epochs| epochs.get(&interaction.user_id).copied().unwrap_or(0) != epoch)
                .unwrap_or(true)
        {
            return BridgeReply::ephemeral_text(
                "Diese Antwort wurde nach einer Datenschutzänderung verworfen.",
            );
        }
        let lock = match self.final_send_lock(&turn, &reply).await {
            Ok(lock) => lock,
            Err(_) => {
                return BridgeReply::ephemeral_text(
                    "Diese Antwort wurde nach einer Datenschutzänderung verworfen.",
                )
            }
        };
        let Some(text) = visible_reply(&reply) else {
            return BridgeReply::ephemeral_text(self.public_text(UNAVAILABLE));
        };
        let mut response = BridgeReply::ephemeral_text(text);
        response.response_message_hook = Some(Arc::new(PrivacySendHook(Mutex::new(Some(lock)))));
        response
    }
}

pub fn register(router: &mut InteractionRouter, guide: Arc<GuideAdapter>) {
    for prefix in ["concierge:", "faq_chat:", "tour:v1:", "serverguide:"] {
        router.on_prefix(prefix, guide.clone());
    }
    router.on_custom_id("feedback_hub:open_modal", guide.clone());
    router.on_custom_id("feedback_hub:submit", guide.clone());
    for command in ["brain", "serverguide", "faq"] {
        router.on_command(command, CommandSpec { definition: json!({"name": command, "description": "Eine Frage an den Serverguide stellen", "type": 1, "options": [{"name": "frage", "description": "Deine Frage", "type": 3, "required": command != "faq"}]}) }, guide.clone());
    }
    router.on_command("serverguide-profil", CommandSpec { definition: json!({"name": "serverguide-profil", "description": "Deine gespeicherten Angaben ansehen, ändern oder vergessen", "type": 1, "options": [
        {"name": "aktion", "description": "Was möchtest du tun?", "type": 3, "required": true, "choices": [{"name":"Ansehen", "value":"view"}, {"name":"Erinnerung ausschalten", "value":"memory_off"}, {"name":"Erinnerung einschalten", "value":"memory_on"}, {"name":"Alles vergessen", "value":"forget"}, {"name":"Angabe korrigieren", "value":"correct"}, {"name":"Angabe entfernen", "value":"remove"}]},
        {"name": "feld", "description": "Die betroffene Angabe", "type": 3, "choices": [{"name":"Spielinteressen", "value":"game_interests"}, {"name":"Aktuelle Ziele", "value":"current_goals"}, {"name":"Spielzeiten", "value":"play_times"}, {"name":"Chat oder Voice", "value":"communication"}, {"name":"Antwortlänge", "value":"answer_length"}, {"name":"Bereits erklärte Abläufe", "value":"explained_flows"}, {"name":"Offenes Anliegen", "value":"open_concern"}]},
        {"name": "wert", "description": "Deine korrigierte Angabe", "type": 3, "max_length": 500}
    ]}) }, guide);
}

fn profile_control(interaction: &BridgeInteraction) -> (Option<Value>, Option<&'static str>) {
    if interaction.command != "serverguide-profil" {
        return (None, None);
    }
    let value = |key| {
        interaction
            .options
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    let control = match value("aktion") {
        "view" => json!({"type":"view"}),
        "forget" => json!({"type":"forget"}),
        "memory_off" => json!({"type":"memory", "enabled":false}),
        "memory_on" => json!({"type":"memory", "enabled":true}),
        "correct" | "remove"
            if matches!(
                value("feld"),
                "game_interests"
                    | "current_goals"
                    | "play_times"
                    | "communication"
                    | "answer_length"
                    | "explained_flows"
                    | "open_concern"
            ) =>
        {
            if value("aktion") == "correct" {
                if value("wert").trim().is_empty()
                    || value("wert").len() > 500
                    || value("wert").chars().any(char::is_control)
                {
                    return (None, Some("Gib bitte eine kurze korrigierte Angabe ein."));
                }
                json!({"type":"correct", "field":value("feld"), "value":value("wert")})
            } else {
                json!({"type":"remove", "field":value("feld")})
            }
        }
        _ => {
            return (
                None,
                Some("Wähle bitte die Aktion und bei Bedarf das Feld aus."),
            )
        }
    };
    (Some(control), None)
}

pub fn spawn(guide: Arc<GuideAdapter>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    if guide.config.source_sync_enabled {
        let source_guide = guide.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(source_guide.config.source_interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                if source_guide.sync_server_sources().await.is_err() {
                    tracing::warn!("Freigegebene Serverquellen konnten nicht aktualisiert werden");
                }
            }
        });
    }
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    if guide.adapter.bot_user_id_cell().get().copied() == Some(event.author_id)
                        || event.guild_id.is_some_and(|guild| {
                            guild != guide.config.guild_id
                                || !guide.public_allowed(event.channel_id)
                        })
                    {
                        continue;
                    }
                    // Metadaten anderer Menschen werden sofort berücksichtigt, ohne Kern- oder Profilzugriff.
                    let received_sequence = guide.routes.lock().await.observe_human(&event);
                    if guide.config.allowed(event.author_id) {
                        let Ok(permit) = guide.turn_capacity.clone().try_acquire_owned() else {
                            continue;
                        };
                        let turn_guide = guide.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            turn_guide.handle_message(event, received_sequence).await;
                        });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    tracing::warn!("Guide-Ereignisse wurden übersprungen")
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proaktive_frage_braucht_exakten_kanal_frisches_event_und_keinen_reply() {
        let config = GuideConfig::from_lookup(1289721245281292288, |key| match key {
            "DL_GUIDE_PROACTIVE_CHANNELS" | "DL_GUIDE_PUBLIC_CHANNELS" => Some("1426220702054355077".into()),
            _ => None,
        });
        let mut event = message(20,1426220702054355077,"Wo finde ich Mitspieler?");
        event.guild_id=Some(1289721245281292288);
        assert!(config.proactive_question(&event));
        event.channel_id=1426220702054355078;
        assert!(!config.proactive_question(&event));
        event.channel_id=1426220702054355077;
        event.reply_message_id=Some(1);
        assert!(!config.proactive_question(&event));
        event.reply_message_id=None;
        event.message_created_at=chrono::Utc::now()-chrono::Duration::seconds(61);
        assert!(!config.proactive_question(&event));
        for text in ["Danke, alles klar?", "<@123> kannst du helfen?", "> Wo finde ich Hilfe?", "Erledigt?", "Nani, kannst du helfen?"] {
            assert!(!community_question(text));
        }
    }
    #[test]
    fn oeffentliche_leserechte_werden_nach_entzug_gesperrt() {
        use serenity::all::{
            ChannelType, PermissionOverwrite, PermissionOverwriteType, Permissions, RoleId,
        };
        let mut channel = serenity::all::GuildChannel::default();
        channel.kind = ChannelType::Text;
        assert!(!public_view_allowed(1, None, &channel));
        assert!(public_view_allowed(
            1,
            Some(Permissions::VIEW_CHANNEL),
            &channel
        ));
        channel.permission_overwrites.push(PermissionOverwrite {
            kind: PermissionOverwriteType::Role(RoleId::new(1)),
            allow: Permissions::empty(),
            deny: Permissions::VIEW_CHANNEL,
        });
        assert!(!public_view_allowed(
            1,
            Some(Permissions::VIEW_CHANNEL),
            &channel
        ));
        channel.permission_overwrites.clear();
        channel.kind = ChannelType::PrivateThread;
        assert!(!public_view_allowed(
            1,
            Some(Permissions::VIEW_CHANNEL),
            &channel
        ));
        channel.kind = ChannelType::PublicThread;
        assert!(public_view_allowed(
            1,
            Some(Permissions::VIEW_CHANNEL),
            &channel
        ));
        channel.kind = ChannelType::Unknown(255);
        assert!(!public_view_allowed(
            1,
            Some(Permissions::VIEW_CHANNEL),
            &channel
        ));
    }
    #[test]
    fn hilfe_vor_pending_eintrag_bleibt_sichtbar() {
        let mut routes = Routes::default();
        let question = message(20, 10, "<@99> Wo finde ich Mitspieler?");
        let received = routes.observe_human(&question);
        routes.observe_human(&message(21, 10, "Hier gibt es eine passende Runde."));
        assert!(routes.conversations.is_empty());
        assert!(routes.human_after(&question, received));
        assert!(!routes.human_after(&message(20, 11, "Andere Frage"), received));
    }

    #[test]
    fn weitere_eigene_nachricht_verdeckt_keine_zwischenzeitliche_hilfe() {
        let mut routes = Routes::default();
        let question = message(20, 10, "<@99> Kannst du helfen?");
        let received = routes.observe_human(&question);
        routes.observe_human(&message(21, 10, "Hier findest du die Antwort."));
        let new_question = message(20, 10, "<@99> Kannst du das ergänzen?");
        let new_received = routes.observe_human(&new_question);
        assert!(routes.human_after(&question, received));
        assert!(!routes.human_after(&new_question, new_received));
        assert!(!routes.human_after(
            &MessageEvent {
                guild_id: None,
                ..question
            },
            received
        ));
    }
    #[test]
    fn sichtbarer_text_beachtet_hinweis_und_discord_utf16_limit() {
        let reply = GuideReply {
            reply: Some("Gern.".into()),
            memory_notice: Some("Deine Erinnerung bleibt ausgeschaltet.".into()),
            ..Default::default()
        };
        assert_eq!(
            visible_reply(&reply).as_deref(),
            Some("Gern.\n\nDeine Erinnerung bleibt ausgeschaltet.")
        );
        let reply = GuideReply {
            reply: Some("🙂".repeat(1001)),
            ..Default::default()
        };
        assert!(visible_reply(&reply).is_none());
        let reply = GuideReply {
            reply: Some("Deadlock Brain hilft dir.".into()),
            ..Default::default()
        };
        assert!(visible_reply(&reply).is_none());
    }
    fn message(user: u64, channel: u64, content: &str) -> MessageEvent {
        MessageEvent {
            guild_id: Some(1),
            channel_id: channel,
            message_id: 50,
            author_id: user,
            author_display_name: String::new(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.into(),
            message_created_at: chrono::Utc::now(),
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
    fn ongoing() -> Routes {
        Routes {
            conversations: HashMap::from([(
                (1, 10, 20),
                Conversation {
                    id: Some("conversation".into()),
                    bot_messages: HashSet::from([40]),
                    last_user_message: 30,
                    updated: Instant::now(),
                    interrupted: false,
                    epoch: 0,
                },
            )]),
            ..Default::default()
        }
    }
    #[test]
    fn öffentliche_folgefragen_bleiben_beim_richtigen_nutzer_und_kanal() {
        let mut routes = ongoing();
        assert_eq!(
            routes
                .addressed(
                    &message(20, 10, "Und dann?"),
                    Some(99),
                    Duration::from_secs(300)
                )
                .unwrap()
                .0,
            "followup"
        );
        assert!(routes
            .addressed(
                &message(21, 10, "Und dann?"),
                Some(99),
                Duration::from_secs(300)
            )
            .is_none());
        assert!(routes
            .addressed(
                &message(20, 11, "Und dann?"),
                Some(99),
                Duration::from_secs(300)
            )
            .is_none());
        assert!(routes
            .addressed(
                &message(20, 10, "Und dann?"),
                Some(99),
                Duration::from_secs(300)
            )
            .is_none());
    }
    #[test]
    fn menschliche_hilfe_beendet_unadressierte_folgefragen_aber_reply_bleibt_möglich() {
        let mut routes = ongoing();
        routes.observe_human(&message(21, 10, "Hier findest du die Antwort."));
        assert!(routes
            .addressed(
                &message(20, 10, "Alles klar"),
                Some(99),
                Duration::from_secs(300)
            )
            .is_none());
        let mut reply = message(20, 10, "Kannst du ergänzen?");
        reply.reply_message_id = Some(40);
        assert_eq!(
            routes
                .addressed(&reply, Some(99), Duration::from_secs(300))
                .unwrap()
                .0,
            "reply"
        );
        reply.reply_message_id = Some(41);
        assert!(routes
            .addressed(&reply, Some(99), Duration::from_secs(300))
            .is_none());
    }
    #[test]
    fn abgelaufene_unterhaltungen_antworten_nicht_weiter() {
        let mut routes = ongoing();
        routes.conversations.get_mut(&(1, 10, 20)).unwrap().updated =
            Instant::now() - Duration::from_secs(301);
        assert!(routes
            .addressed(
                &message(20, 10, "Und dann?"),
                Some(99),
                Duration::from_secs(300)
            )
            .is_none());
    }
    #[test]
    fn standard_ist_geschlossen_auch_mit_leerer_testliste() {
        let config = GuideConfig::from_lookup(1, |_| None);
        assert!(!config.allowed(2));
        let config =
            GuideConfig::from_lookup(1, |key| (key == "DL_GUIDE_ENABLED").then(|| "true".into()));
        assert!(!config.allowed(2));
    }
    #[test]
    fn gespräch_endet_bei_natürlichem_abschluss() {
        assert!(is_end("Danke :)"));
        assert!(!is_end("Danke, wie finde ich Mitspieler?"));
    }
}
