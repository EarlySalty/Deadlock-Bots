use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use dl_brain::{
    ConversationReply, InviteChatLine, InviteMemberState, InviteRequestPhase, InviteSpeaker,
    InviteTurn, InviteTurnAction, InviteTurnDecision, MAX_CHAT_LINE_CHARS, MAX_TASK_TEXT_CHARS,
};
use dl_central_db::kv;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[path = "invite_lounge_status.rs"]
mod status;
use status::{
    after_invite, after_progress, check_in_age, check_in_step, classify_invite_reply,
    steam_outcome, CheckIn, Notice, OutboxEntry, RequestStatus, SteamProgress, SteamWait, Step,
};

pub const CHECK_IN_CHANNEL_ID: u64 = 1_464_736_918_951_432_222;

const COOLDOWN_KV_NS: &str = "invite_lounge:cooldown";
const STATE_KV_NS: &str = "invite_lounge:requests";
const BACKFILL_KV_NS: &str = "invite_lounge:history";
const ACTIONS_SINCE_KEY: &str = "actions_since_v1";
const REQUEST_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;
const CATCH_UP_SECONDS: i64 = 3 * 24 * 60 * 60;
const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

pub(crate) const PRIVACY_KV_NAMESPACES: [&str; 2] = [STATE_KV_NS, COOLDOWN_KV_NS];

pub(crate) fn privacy_kv_key(user_id: i64) -> String {
    format!("user:{user_id}")
}

static FRIEND_CODE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\b\d{6,12}\b").ok());
static URL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)(?:https?://|www\.|steamcommunity\.com/|s\.team/)\S+").ok());
static MENTION_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"<(@!?|@&|#)(\d+)>").ok());

#[derive(Debug, Clone)]
pub struct LoungeMessage {
    pub guild_id: Option<u64>,
    pub channel_id: u64,
    pub message_id: u64,
    pub author_id: u64,
    pub content: String,
    pub created_at: i64,
    pub created_at_millis: i64,
    pub is_bot: bool,
    pub reply_message_id: Option<u64>,
}

impl From<dl_discord::MessageEvent> for LoungeMessage {
    fn from(event: dl_discord::MessageEvent) -> Self {
        Self {
            guild_id: event.guild_id,
            channel_id: event.channel_id,
            message_id: event.message_id,
            author_id: event.author_id,
            content: event.content,
            created_at: event.message_created_at.timestamp(),
            created_at_millis: event.message_created_at.timestamp_millis(),
            is_bot: false,
            reply_message_id: event.reply_message_id,
        }
    }
}

#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
pub trait InviteLoungeReplyPort: Send + Sync {
    fn bot_id(&self) -> Option<u64>;
    async fn history(
        &self,
        channel_id: u64,
        before_id: Option<u64>,
    ) -> Result<Vec<LoungeMessage>, String>;
}

#[async_trait::async_trait]
impl InviteLoungeReplyPort for dl_discord::DiscordAdapter {
    fn bot_id(&self) -> Option<u64> {
        self.bot_user_id_cell().get().copied().filter(|id| *id > 0)
    }

    async fn history(
        &self,
        channel_id: u64,
        before_id: Option<u64>,
    ) -> Result<Vec<LoungeMessage>, String> {
        Ok(self
            .channel_message_history(channel_id, before_id)
            .await?
            .into_iter()
            .map(|message| LoungeMessage {
                guild_id: message.guild_id.map(|id| id.get()),
                channel_id: message.channel_id.get(),
                message_id: message.id.get(),
                author_id: message.author.id.get(),
                content: message.content,
                created_at: message.timestamp.unix_timestamp(),
                created_at_millis: (message.id.get() >> 22) as i64 + 1_420_070_400_000,
                is_bot: message.author.bot,
                reply_message_id: message
                    .message_reference
                    .and_then(|reference| reference.message_id)
                    .map(|id| id.get()),
            })
            .collect())
    }
}

#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
trait InviteEventPort: Send + Sync {
    async fn invite(
        &self,
        bot_id: u64,
        guild_id: u64,
        code: &str,
        user_id: u64,
    ) -> Result<String, String>;
}

#[async_trait::async_trait]
impl InviteEventPort for dl_bridges::steam::SteamBotClient {
    async fn invite(
        &self,
        bot_id: u64,
        guild_id: u64,
        code: &str,
        user_id: u64,
    ) -> Result<String, String> {
        self.invite_from_bot(bot_id, guild_id, code, user_id).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    WaitingForCode,
    Pending,
    Dispatching,
    Attempted,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Request {
    message_id: u64,
    created_at: i64,
    direct: bool,
    phase: Phase,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LoungeState {
    last_seen: u64,
    friend_code: Option<String>,
    request: Option<Request>,
    hint_sent: bool,
    last_hint_at: Option<i64>,
    last_result: Option<String>,
    dispatch_started_at_millis: Option<i64>,
    last_dispatch_finished_at_millis: Option<i64>,
    #[serde(default, skip_serializing_if = "RequestStatus::is_empty")]
    status: RequestStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    outbox: Vec<OutboxEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeResult {
    Delivered(Option<u64>),
    Dropped,
    Retry,
}

#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
trait LoungeStore: Send + Sync {
    async fn load(&self, user_id: u64) -> Result<Option<LoungeState>, String>;
    async fn compare_exchange(
        &self,
        user_id: u64,
        previous: Option<&LoungeState>,
        next: &LoungeState,
    ) -> Result<bool, String>;
    async fn pending_users(&self) -> Result<Vec<u64>, String>;
    async fn outbox_users(&self) -> Result<Vec<u64>, String>;
    async fn known_code(&self, user_id: u64) -> Result<Option<String>, String>;
    async fn opted_out(&self, user_id: u64) -> bool;
    async fn actions_since(&self) -> Result<Option<i64>, String>;
    async fn mark_actions_since(&self, now: i64) -> Result<(), String>;
    async fn steam_progress(&self, code: &str, since: i64) -> Result<SteamProgress, String>;
    async fn handled_since(&self, code: &str, since: i64) -> Result<bool, String>;
}

struct PgLoungeStore {
    pool: PgPool,
}

#[async_trait::async_trait]
impl LoungeStore for PgLoungeStore {
    async fn load(&self, user_id: u64) -> Result<Option<LoungeState>, String> {
        let state = kv::get(&self.pool, STATE_KV_NS, &cooldown_key(user_id))
            .await
            .map_err(|err| err.to_string())?;
        state
            .map(|value| serde_json::from_str(&value).map_err(|err| err.to_string()))
            .transpose()
    }

    async fn compare_exchange(
        &self,
        user_id: u64,
        previous: Option<&LoungeState>,
        next: &LoungeState,
    ) -> Result<bool, String> {
        let key = cooldown_key(user_id);
        let value = serde_json::to_string(next).map_err(|err| err.to_string())?;
        let Some(previous) = previous else {
            return kv::set_if_absent(&self.pool, STATE_KV_NS, &key, &value)
                .await
                .map_err(|err| err.to_string());
        };
        let previous = serde_json::to_string(previous).map_err(|err| err.to_string())?;
        let result =
            sqlx::query("UPDATE bot.kv_store SET v = $1 WHERE ns = $2 AND k = $3 AND v = $4")
                .bind(value)
                .bind(STATE_KV_NS)
                .bind(key)
                .bind(previous)
                .execute(&self.pool)
                .await
                .map_err(|err| err.to_string())?;
        Ok(result.rows_affected() == 1)
    }

    async fn pending_users(&self) -> Result<Vec<u64>, String> {
        let keys: Vec<String> = sqlx::query_scalar("SELECT k FROM bot.kv_store WHERE ns = $1 AND v::jsonb #>> '{request,phase}' = 'Pending'")
            .bind(STATE_KV_NS).fetch_all(&self.pool).await.map_err(|err| err.to_string())?;
        user_ids(keys)
    }

    async fn outbox_users(&self) -> Result<Vec<u64>, String> {
        let keys: Vec<String> = sqlx::query_scalar("SELECT k FROM bot.kv_store WHERE ns = $1 AND jsonb_array_length(COALESCE(v::jsonb -> 'outbox', '[]'::jsonb)) > 0")
            .bind(STATE_KV_NS).fetch_all(&self.pool).await.map_err(|err| err.to_string())?;
        user_ids(keys)
    }

    async fn known_code(&self, user_id: u64) -> Result<Option<String>, String> {
        let steam_id: Option<String> = sqlx::query_scalar("SELECT steam_id FROM core.steam_links WHERE discord_id = $1 ORDER BY primary_account DESC, verified DESC, steam_id LIMIT 1")
            .bind(user_id as i64).fetch_optional(&self.pool).await.map_err(|err| err.to_string())?;
        Ok(steam_id
            .and_then(|id| id.parse::<u64>().ok())
            .and_then(|id| id.checked_sub(STEAM_ID64_BASE))
            .filter(|id| *id > 0 && *id <= u32::MAX as u64)
            .map(|id| id.to_string()))
    }

    async fn opted_out(&self, user_id: u64) -> bool {
        match i64::try_from(user_id) {
            Ok(user_id) => crate::privacy::is_opted_out(&self.pool, user_id).await,
            Err(_) => true,
        }
    }

    async fn actions_since(&self) -> Result<Option<i64>, String> {
        Ok(kv::get(&self.pool, BACKFILL_KV_NS, ACTIONS_SINCE_KEY)
            .await
            .map_err(|err| err.to_string())?
            .and_then(|value| value.parse().ok()))
    }

    async fn mark_actions_since(&self, now: i64) -> Result<(), String> {
        kv::set_if_absent(
            &self.pool,
            BACKFILL_KV_NS,
            ACTIONS_SINCE_KEY,
            &now.to_string(),
        )
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
    }

    async fn steam_progress(&self, code: &str, since: i64) -> Result<SteamProgress, String> {
        let Some(steam_id) = steam_id64(code) else {
            return Ok(SteamProgress::default());
        };
        let outcome: Option<(String, Option<i64>)> = sqlx::query_as(
            "SELECT decision, CASE WHEN jsonb_typeof(detail->'code') = 'number' THEN (detail->>'code')::bigint END
             FROM steam.bot_event_log
             WHERE event_type = 'playtest_invite' AND steam_id = $1
               AND decision IN ('sent', 'rejected', 'timeout') AND occurred_at >= to_timestamp($2)
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
        )
        .bind(steam_id)
        .bind(since as f64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        let (invited, open): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM steam.beta_invite_audit WHERE steam_id64 = $1),
                    EXISTS(SELECT 1 FROM steam.invite_requests WHERE steam_id64 = $1)",
        )
        .bind(steam_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(SteamProgress {
            outcome: outcome.and_then(|(decision, code)| steam_outcome(&decision, code)),
            invited,
            open,
        })
    }

    async fn handled_since(&self, code: &str, since: i64) -> Result<bool, String> {
        let Some(steam_id) = steam_id64(code) else {
            return Ok(false);
        };
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM steam.bot_event_log
                            WHERE event_type = 'playtest_invite' AND steam_id = $1
                              AND occurred_at >= to_timestamp($2))
                 OR EXISTS(SELECT 1 FROM steam.beta_invite_audit
                            WHERE steam_id64 = $1 AND invited_at >= to_timestamp($2))
                 OR EXISTS(SELECT 1 FROM steam.invite_requests
                            WHERE steam_id64 = $1 AND created_at >= $2)",
        )
        .bind(steam_id)
        .bind(since as f64)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| err.to_string())
    }
}

fn user_ids(keys: Vec<String>) -> Result<Vec<u64>, String> {
    keys.into_iter()
        .map(|key| {
            key.strip_prefix("user:")
                .and_then(|id| id.parse().ok())
                .ok_or_else(|| "Ungültiger Nutzer-Schlüssel im Lounge-Zustand".to_string())
        })
        .collect()
}

fn steam_id64(code: &str) -> Option<i64> {
    code.parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && *id <= u64::from(u32::MAX))
        .and_then(|id| i64::try_from(id + STEAM_ID64_BASE).ok())
}

type UserLocks = std::sync::Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>>;

pub struct InviteLoungeWatcher {
    store: Arc<dyn LoungeStore>,
    port: Arc<dyn InviteLoungeReplyPort>,
    invite: Arc<dyn InviteEventPort>,
    brain: Arc<dyn dl_brain::AiAnswerer>,
    guild_id: u64,
    clock_millis: Arc<dyn Fn() -> i64 + Send + Sync>,
    locks: UserLocks,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MemberFacts {
    message_code: Option<String>,
    known_code: Option<String>,
    invited: bool,
}

impl MemberFacts {
    fn code(&self) -> Option<String> {
        self.message_code
            .clone()
            .or_else(|| self.known_code.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnOutcome {
    Failed,
    Silent,
    Decided(InviteTurnDecision),
}

impl InviteLoungeWatcher {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn InviteLoungeReplyPort>,
        invite: Arc<dl_bridges::steam::SteamBotClient>,
        brain: Arc<dyn dl_brain::AiAnswerer>,
        guild_id: u64,
    ) -> Self {
        Self {
            store: Arc::new(PgLoungeStore { pool }),
            port,
            invite,
            brain,
            guild_id,
            clock_millis: Arc::new(|| chrono::Utc::now().timestamp_millis()),
            locks: UserLocks::default(),
        }
    }

    fn watches(&self, event: &LoungeMessage) -> bool {
        !event.is_bot
            && event.guild_id == Some(self.guild_id)
            && event.channel_id == CHECK_IN_CHANNEL_ID
    }

    async fn serve(&self, event: LoungeMessage) {
        let context = match self
            .port
            .history(event.channel_id, Some(event.message_id))
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, message_id = event.message_id, "Invite-Lounge: Verlauf für den Kontext nicht lesbar");
                Vec::new()
            }
        };
        let now = (self.clock_millis)() / 1000;
        match self.handle_turn(&event, &context, now).await {
            Ok(Some(user_id)) => self.follow_up(user_id, now).await,
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, message_id = event.message_id, "Invite-Lounge-Nachricht konnte nicht verarbeitet werden");
            }
        }
    }

    async fn follow_up(&self, user_id: u64, now: i64) {
        if let Err(error) = self.dispatch_due(user_id, now).await {
            tracing::warn!(%error, user_id, "Invite-Lounge-Auftrag konnte nicht abgeschlossen werden");
        }
        if let Err(error) = self.deliver_outbox(user_id).await {
            tracing::warn!(%error, user_id, "Invite-Lounge-Meldung konnte nicht zugestellt werden");
        }
    }

    async fn handle_turn(
        &self,
        event: &LoungeMessage,
        context: &[LoungeMessage],
        now: i64,
    ) -> Result<Option<u64>, String> {
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        if !self.accepts(event, bot_id, now).await? {
            return Ok(None);
        }
        let user_id = event.author_id;
        let lock = self.user_lock(user_id);
        let _guard = lock.lock().await;
        let previous = self.store.load(user_id).await?;
        if previous
            .as_ref()
            .is_some_and(|state| event.message_id <= state.last_seen)
        {
            return Ok(None);
        }
        if self.store.opted_out(user_id).await {
            tracing::info!(
                message_id = event.message_id,
                "Invite-Lounge überspringt Nachricht nach Datenschutz-Opt-out"
            );
            return Ok(None);
        }
        let facts = self.member_facts(event, previous.as_ref()).await;
        let member = member_state(previous.as_ref(), &facts);
        let Some(turn) = build_turn(event, context, bot_id, member) else {
            return Ok(None);
        };
        let decision = match self.decide(event, turn).await {
            TurnOutcome::Failed => return Ok(None),
            TurnOutcome::Silent => None,
            TurnOutcome::Decided(decision) => Some(decision),
        };
        tracing::info!(
            user_id,
            message_id = event.message_id,
            intent = decision
                .as_ref()
                .map_or("none", |decision| decision.intent.as_str()),
            action = decision
                .as_ref()
                .map_or("silent", |decision| decision.action.as_str()),
            "Invite-Lounge-Entscheidung"
        );
        self.store_turn(event, decision.as_ref(), &facts, now)
            .await
            .map(|stored| stored.then_some(user_id))
    }

    async fn store_turn(
        &self,
        event: &LoungeMessage,
        decision: Option<&InviteTurnDecision>,
        facts: &MemberFacts,
        now: i64,
    ) -> Result<bool, String> {
        loop {
            let previous = self.store.load(event.author_id).await?;
            if previous
                .as_ref()
                .is_some_and(|state| event.message_id <= state.last_seen)
            {
                return Ok(false);
            }
            let mut next = previous.clone().unwrap_or_default();
            let applied = apply_turn(&mut next, event, decision, facts, now);
            if !self
                .store
                .compare_exchange(event.author_id, previous.as_ref(), &next)
                .await?
            {
                continue;
            }
            if let (Some(decision), false) = (decision, applied) {
                tracing::info!(
                    user_id = event.author_id,
                    message_id = event.message_id,
                    action = decision.action.as_str(),
                    "Invite-Lounge: Entscheidung passt nicht mehr zum Zustand, bleibt still"
                );
            }
            return Ok(true);
        }
    }

    async fn accepts(&self, event: &LoungeMessage, bot_id: u64, now: i64) -> Result<bool, String> {
        if !self.watches(event) || event.author_id == bot_id || event.content.trim().is_empty() {
            return Ok(false);
        }
        if !is_fresh(event.created_at, now, REQUEST_WINDOW_SECONDS) {
            tracing::info!(
                message_id = event.message_id,
                "Invite-Lounge überspringt verspätete Nachricht"
            );
            return Ok(false);
        }
        self.allows_actions(event.created_at).await
    }

    fn user_lock(&self, user_id: u64) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|id, lock| *id == user_id || Arc::strong_count(lock) > 1);
        locks.entry(user_id).or_default().clone()
    }

    async fn member_facts(
        &self,
        event: &LoungeMessage,
        state: Option<&LoungeState>,
    ) -> MemberFacts {
        let stored = state.and_then(|state| state.friend_code.clone());
        let known_code = match stored {
            Some(code) => Some(code),
            None => self
                .store
                .known_code(event.author_id)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, user_id = event.author_id, "Invite-Lounge: verknüpfter Steam-Account nicht lesbar");
                    None
                }),
        };
        let mut facts = MemberFacts {
            message_code: friend_code(&event.content),
            known_code,
            invited: false,
        };
        facts.invited = self.invited(facts.code().as_deref()).await;
        facts
    }

    async fn invited(&self, code: Option<&str>) -> bool {
        let Some(code) = code else {
            return false;
        };
        match self.store.steam_progress(code, 0).await {
            Ok(progress) => progress.invited,
            Err(error) => {
                tracing::warn!(%error, "Invite-Lounge: Einladungsstand nicht lesbar");
                false
            }
        }
    }

    async fn decide(&self, event: &LoungeMessage, turn: InviteTurn) -> TurnOutcome {
        let task = dl_brain::DiscordConversation::InviteTurn { turn: turn.clone() };
        match self.converse(event.author_id, event.channel_id, task).await {
            Some(ConversationReply::Decided { decision }) if decision.fits(&turn) => {
                TurnOutcome::Decided(decision)
            }
            Some(ConversationReply::Decided { decision }) => {
                tracing::warn!(
                    message_id = event.message_id,
                    action = decision.action.as_str(),
                    "Invite-Lounge: Entscheidung passt nicht zum Stand"
                );
                TurnOutcome::Silent
            }
            Some(ConversationReply::Silent) => TurnOutcome::Silent,
            Some(_) => {
                tracing::warn!(
                    message_id = event.message_id,
                    "Invite-Lounge: unerwartete Brain-Antwort auf die Nachricht"
                );
                TurnOutcome::Failed
            }
            None => TurnOutcome::Failed,
        }
    }

    async fn allows_actions(&self, created_at: i64) -> Result<bool, String> {
        Ok(self
            .store
            .actions_since()
            .await?
            .is_some_and(|since| created_at >= since))
    }

    async fn queue_step_notice(&self, state: &mut LoungeState, notice: Option<Notice>) {
        if notice.is_some() {
            let invited = self.invited(state.friend_code.as_deref()).await;
            queue_notice(state, notice, invited);
        }
    }

    async fn converse(
        &self,
        user_id: u64,
        channel_id: u64,
        task: dl_brain::DiscordConversation,
    ) -> Option<ConversationReply> {
        let conversation = dl_brain::DiscordConversationRequest {
            user_id,
            channel_id,
            guild_id: Some(self.guild_id),
            task,
            images: dl_brain::AttachedImages::default(),
        };
        match self.brain.converse(&conversation).await {
            Ok(reply) => Some(reply),
            Err(error) => {
                tracing::warn!(%error, user_id, "Invite-Lounge: Brain-Antwort fehlgeschlagen");
                None
            }
        }
    }

    async fn catch_up(&self, now: i64) -> Result<(), String> {
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        let mut history = self
            .recent_history(CHECK_IN_CHANNEL_ID, now - CATCH_UP_SECONDS)
            .await?;
        for message in &mut history {
            message.guild_id = Some(self.guild_id);
        }
        history.sort_by_key(|message| message.message_id);
        for message in unanswered(&history, bot_id) {
            if let Some(user_id) = self.handle_turn(&message, &history, now).await? {
                self.follow_up(user_id, now).await;
            }
        }
        Ok(())
    }

    async fn recent_history(
        &self,
        channel_id: u64,
        since: i64,
    ) -> Result<Vec<LoungeMessage>, String> {
        let mut before = None;
        let mut history = Vec::new();
        loop {
            let page = self.port.history(channel_id, before).await?;
            if page.is_empty() {
                return Ok(history);
            }
            let oldest = page
                .iter()
                .map(|message| message.message_id)
                .min()
                .ok_or("Leere Verlaufsseite")?;
            if before.is_some_and(|id| oldest >= id) {
                return Err("Der Verlauf liefert keine ältere Seite".into());
            }
            let reached_start = page.iter().any(|message| message.created_at < since);
            history.extend(
                page.into_iter()
                    .filter(|message| message.created_at >= since),
            );
            if reached_start {
                return Ok(history);
            }
            before = Some(oldest);
        }
    }

    async fn deliver_outbox(&self, user_id: u64) -> Result<(), String> {
        loop {
            let now_millis = (self.clock_millis)();
            let Some(previous) = self.store.load(user_id).await? else {
                return Ok(());
            };
            let mut claimed = previous.clone();
            let Some(head) = claimed
                .outbox
                .first_mut()
                .filter(|entry| entry.ready(now_millis))
            else {
                return Ok(());
            };
            let retrying = head.first_attempt_at_millis.is_some();
            head.claim(now_millis);
            let entry = head.clone();
            if !self
                .store
                .compare_exchange(user_id, Some(&previous), &claimed)
                .await?
            {
                continue;
            }
            let result = self.send_notice(user_id, &entry, retrying).await;
            self.finish_notice(user_id, &entry, result).await?;
            if result == NoticeResult::Retry {
                return Ok(());
            }
        }
    }

    async fn send_notice(&self, user_id: u64, entry: &OutboxEntry, retrying: bool) -> NoticeResult {
        if let (true, Some(channel_id)) = (retrying, entry.channel_id) {
            let since = entry.first_attempt_at_millis.unwrap_or(0);
            match self
                .bot_replied_since(channel_id, entry.request_message_id, user_id, since)
                .await
            {
                Ok(Some(message_id)) => return NoticeResult::Delivered(Some(message_id)),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, user_id, key = %entry.key, "Invite-Lounge: Zustellung nicht prüfbar");
                    return NoticeResult::Retry;
                }
            }
        }
        let Some(update) = entry.update.current() else {
            tracing::warn!(user_id, key = %entry.key, "Invite-Lounge: veralteter Ausgangseintrag verworfen");
            return NoticeResult::Dropped;
        };
        let target = entry
            .channel_id
            .map_or(dl_brain::NoticeTarget::DirectMessage, |channel_id| {
                dl_brain::NoticeTarget::Reply {
                    channel_id,
                    message_id: entry.request_message_id,
                }
            });
        let task = dl_brain::DiscordConversation::InviteNotice {
            notice: dl_brain::InviteNotice {
                idempotency_key: entry.key.clone(),
                update,
                target,
                already_invited: entry.already_invited,
                answer: entry.answer.clone(),
            },
        };
        let channel_id = entry.channel_id.unwrap_or(CHECK_IN_CHANNEL_ID);
        match self.converse(user_id, channel_id, task).await {
            Some(ConversationReply::Delivered { message_id, .. }) => {
                NoticeResult::Delivered(message_id)
            }
            Some(ConversationReply::Rejected) => NoticeResult::Dropped,
            _ => NoticeResult::Retry,
        }
    }

    async fn finish_notice(
        &self,
        user_id: u64,
        entry: &OutboxEntry,
        result: NoticeResult,
    ) -> Result<(), String> {
        loop {
            let Some(previous) = self.store.load(user_id).await? else {
                return Ok(());
            };
            let mut next = previous.clone();
            let Some(position) = next
                .outbox
                .iter()
                .position(|queued| queued.key == entry.key)
            else {
                return Ok(());
            };
            settle_notice(&mut next, position, result, (self.clock_millis)());
            if self
                .store
                .compare_exchange(user_id, Some(&previous), &next)
                .await?
            {
                return Ok(());
            }
        }
    }

    async fn bot_replied_since(
        &self,
        channel_id: u64,
        request_id: u64,
        user_id: u64,
        since_millis: i64,
    ) -> Result<Option<u64>, String> {
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        let mut before = None;
        loop {
            let page = self.port.history(channel_id, before).await?;
            if page.is_empty() {
                return Ok(None);
            }
            if let Some(reply) = page.iter().find(|message| {
                message.author_id == bot_id
                    && message.reply_message_id == Some(request_id)
                    && mentions_user(&message.content, user_id)
                    && message.created_at_millis >= since_millis
            }) {
                return Ok(Some(reply.message_id));
            }
            let oldest = page
                .iter()
                .map(|message| message.message_id)
                .min()
                .ok_or("Leere Verlaufsseite")?;
            if oldest <= request_id {
                return Ok(None);
            }
            if before.is_some_and(|id| oldest >= id) {
                return Err("Der Verlauf liefert keine ältere Seite".into());
            }
            before = Some(oldest);
        }
    }

    async fn dispatch_due(&self, user_id: u64, now: i64) -> Result<(), String> {
        let Some(previous) = self.store.load(user_id).await? else {
            return Ok(());
        };
        let Some(request) = previous
            .request
            .as_ref()
            .filter(|request| request.phase == Phase::Pending)
        else {
            return Ok(());
        };
        let now = now.max((self.clock_millis)() / 1000);
        if !is_fresh(request.created_at, now, REQUEST_WINDOW_SECONDS) {
            self.expire(user_id, &previous).await?;
            return Ok(());
        }
        if !self.allows_actions(request.created_at).await? {
            return Ok(());
        }
        if let Some(wait) = previous.status.wait {
            if previous.status.check_at.is_some_and(|at| now < at) {
                return Ok(());
            }
            return self.follow_steam(user_id, &previous, wait, now).await;
        }
        let check_in_now = check_in_due(request, &previous.status, now);
        if !check_in_now && !is_due(request, &previous.status) {
            return Ok(());
        }
        if self.someone_on_it(user_id, &previous, request).await? {
            return self.close_handled(user_id, &previous).await;
        }
        if check_in_now {
            return self.check_in(user_id, &previous, now).await;
        }
        self.send_invite(user_id, &previous, now).await
    }

    async fn someone_on_it(
        &self,
        user_id: u64,
        state: &LoungeState,
        request: &Request,
    ) -> Result<bool, String> {
        let dispatched_here = state
            .dispatch_started_at_millis
            .is_some_and(|started| started / 1000 >= request.created_at);
        if request.direct || state.status.invite_now || dispatched_here {
            return Ok(false);
        }
        if let Some(code) = state.friend_code.as_deref() {
            match self.store.handled_since(code, request.created_at).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(user_id, error = %err, "Invite-Lounge: Einladungsprüfung fehlgeschlagen, Bitte läuft weiter");
                }
            }
        }
        let Some(channel_id) = state.status.channel_id else {
            return Ok(false);
        };
        match self
            .request_answered(channel_id, request.message_id, user_id)
            .await
        {
            Ok(answered) => Ok(answered),
            Err(err) => {
                tracing::warn!(user_id, channel_id, error = %err, "Invite-Lounge: Verlauf nicht lesbar, Bitte läuft weiter");
                Ok(false)
            }
        }
    }

    async fn close_handled(&self, user_id: u64, previous: &LoungeState) -> Result<(), String> {
        let mut next = previous.clone();
        if let Some(request) = previous.request.as_ref() {
            next.outbox
                .retain(|entry| entry.request_message_id != request.message_id);
        }
        apply_step(&mut next, Step::Finish(None));
        next.last_result = Some("Schon von Hand erledigt; keine Meldung".into());
        if self
            .store
            .compare_exchange(user_id, Some(previous), &next)
            .await?
        {
            tracing::info!(
                user_id,
                "Invite-Lounge schließt von Hand erledigte Bitte ohne Meldung"
            );
        }
        Ok(())
    }

    async fn expire(&self, user_id: u64, previous: &LoungeState) -> Result<(), String> {
        let mut expired = previous.clone();
        let Some(request) = expired.request.as_mut() else {
            return Ok(());
        };
        request.phase = Phase::Expired;
        let message_id = request.message_id;
        expired.last_result = Some("Verspätete Bitte ohne Versand beendet".into());
        if self
            .store
            .compare_exchange(user_id, Some(previous), &expired)
            .await?
        {
            tracing::info!(
                user_id,
                message_id,
                "Invite-Lounge beendet verspäteten Auftrag"
            );
        }
        Ok(())
    }

    async fn follow_steam(
        &self,
        user_id: u64,
        previous: &LoungeState,
        wait: SteamWait,
        now: i64,
    ) -> Result<(), String> {
        let Some(code) = previous.friend_code.as_deref() else {
            return Ok(());
        };
        let since = previous.dispatch_started_at_millis.unwrap_or(0) / 1000;
        let progress = self.store.steam_progress(code, since).await?;
        match after_progress(wait, progress, now) {
            Step::Dispatch => self.send_invite(user_id, previous, now).await,
            step => self.settle(user_id, previous, step).await,
        }
    }

    async fn settle(&self, user_id: u64, previous: &LoungeState, step: Step) -> Result<(), String> {
        let mut next = previous.clone();
        let notice = apply_step(&mut next, step);
        self.queue_step_notice(&mut next, notice).await;
        self.store
            .compare_exchange(user_id, Some(previous), &next)
            .await
            .map(|_| ())
    }

    async fn check_in(&self, user_id: u64, previous: &LoungeState, now: i64) -> Result<(), String> {
        let (Some(request), Some(code)) =
            (previous.request.as_ref(), previous.friend_code.as_deref())
        else {
            return Ok(());
        };
        let Some(channel_id) = previous.status.channel_id else {
            return Ok(());
        };
        let answered = match self
            .request_answered(channel_id, request.message_id, user_id)
            .await
        {
            Ok(answered) => answered,
            Err(err) => {
                tracing::warn!(user_id, channel_id, error = %err, "Invite-Lounge: Verlauf für die Nachfrage nicht lesbar");
                false
            }
        };
        let invited = self.store.steam_progress(code, 0).await?.invited;
        let mut next = previous.clone();
        next.status.check_in_at = Some(now);
        let notice = match check_in_step(invited, answered) {
            CheckIn::AlreadyInvited => {
                apply_step(&mut next, Step::Finish(Some(Notice::AlreadyInvited)))
            }
            CheckIn::Skip => None,
            CheckIn::Ask => next.status.claim(Some(Notice::CheckIn)),
        };
        queue_notice(&mut next, notice, invited);
        self.store
            .compare_exchange(user_id, Some(previous), &next)
            .await
            .map(|_| ())
    }

    async fn request_answered(
        &self,
        channel_id: u64,
        message_id: u64,
        user_id: u64,
    ) -> Result<bool, String> {
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        let mut before = None;
        loop {
            let page = self.port.history(channel_id, before).await?;
            if page.is_empty() {
                return Ok(false);
            }
            if page
                .iter()
                .any(|message| answers_request(message, message_id, user_id, bot_id))
            {
                return Ok(true);
            }
            let oldest = page
                .iter()
                .map(|message| message.message_id)
                .min()
                .ok_or("Leere Verlaufsseite")?;
            if oldest <= message_id {
                return Ok(false);
            }
            if before.is_some_and(|id| oldest >= id) {
                return Err("Der Verlauf liefert keine ältere Seite".into());
            }
            before = Some(oldest);
        }
    }

    async fn send_invite(
        &self,
        user_id: u64,
        previous: &LoungeState,
        now: i64,
    ) -> Result<(), String> {
        let Some(request) = previous.request.as_ref() else {
            return Ok(());
        };
        let Some(code) = previous.friend_code.as_deref() else {
            return Ok(());
        };
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        let message_id = request.message_id;
        let created_at = request.created_at;
        let mut claimed = previous.clone();
        if let Some(request) = claimed.request.as_mut() {
            request.phase = Phase::Dispatching;
        }
        claimed.status.invite_now = false;
        claimed.dispatch_started_at_millis = Some((self.clock_millis)());
        claimed.last_result = Some("Versuch begonnen; Ausgang noch unbekannt".into());
        if !self
            .store
            .compare_exchange(user_id, Some(previous), &claimed)
            .await?
        {
            return Ok(());
        }
        let dispatch_at = (self.clock_millis)();
        if !is_fresh(
            created_at,
            now.max(dispatch_at / 1000),
            REQUEST_WINDOW_SECONDS,
        ) {
            self.finish_dispatch(
                user_id,
                message_id,
                Step::Expire,
                "Verspätete Bitte ohne Versand beendet",
                dispatch_at,
            )
            .await?;
            tracing::info!(
                user_id,
                message_id,
                "Invite-Lounge beendet abgelaufenen Versand-Claim"
            );
            return Ok(());
        }
        let result = self
            .invite
            .invite(bot_id, self.guild_id, code, user_id)
            .await;
        let outcome = match &result {
            Ok(text) => text.clone(),
            Err(err) => format!("Fehlgeschlagen: {err}"),
        };
        let finished_at = (self.clock_millis)();
        let step = after_invite(classify_invite_reply(&result), finished_at / 1000);
        self.finish_dispatch(user_id, message_id, step, &outcome, finished_at)
            .await?;
        tracing::info!(user_id, message_id, result = %outcome, "Invite-Lounge-Versuch abgeschlossen");
        Ok(())
    }

    async fn finish_dispatch(
        &self,
        user_id: u64,
        message_id: u64,
        step: Step,
        outcome: &str,
        finished_at: i64,
    ) -> Result<(), String> {
        loop {
            let Some(current) = self.store.load(user_id).await? else {
                return Err("Der gespeicherte Lounge-Versuch fehlt".into());
            };
            if !current.request.as_ref().is_some_and(|request| {
                request.message_id == message_id && request.phase == Phase::Dispatching
            }) {
                return Ok(());
            }
            let mut finished = current.clone();
            let notice = apply_step(&mut finished, step);
            self.queue_step_notice(&mut finished, notice).await;
            finished.last_result = Some(outcome.to_string());
            finished.last_dispatch_finished_at_millis = Some(finished_at);
            if self
                .store
                .compare_exchange(user_id, Some(&current), &finished)
                .await?
            {
                return Ok(());
            }
        }
    }

    #[cfg(test)]
    async fn poll_due(&self, now: i64) -> Result<(), String> {
        for user_id in self.due_users().await? {
            self.follow_up(user_id, now).await;
        }
        Ok(())
    }

    async fn due_users(&self) -> Result<Vec<u64>, String> {
        let mut users = self.store.pending_users().await?;
        users.extend(self.store.outbox_users().await?);
        users.sort_unstable();
        users.dedup();
        Ok(users)
    }
}

pub fn spawn(
    pool: PgPool,
    port: Arc<dyn InviteLoungeReplyPort>,
    invite: Arc<dl_bridges::steam::SteamBotClient>,
    brain: Arc<dyn dl_brain::AiAnswerer>,
    guild_id: u64,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let watcher = Arc::new(InviteLoungeWatcher::new(
        pool, port, invite, brain, guild_id,
    ));
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        let mut actions_since_ready = false;
        let mut caught_up = false;
        let mut catch_ups = tokio::task::JoinSet::new();
        let mut turns = tokio::task::JoinSet::new();
        let mut dispatches = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                event = messages.recv() => match event {
                    Ok(event) => {
                        let message = LoungeMessage::from(event);
                        if watcher.watches(&message) {
                            let reader = watcher.clone();
                            turns.spawn(async move { reader.serve(message).await });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "Invite-Lounge hat Gateway-Nachrichten verpasst");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    let now = chrono::Utc::now().timestamp();
                    if !actions_since_ready {
                        match watcher.store.mark_actions_since(now).await {
                            Ok(()) => actions_since_ready = true,
                            Err(err) => tracing::warn!(%err, "Invite-Lounge-Startzeit konnte nicht gespeichert werden"),
                        }
                    }
                    if actions_since_ready && !caught_up && catch_ups.is_empty() {
                        let reader = watcher.clone();
                        catch_ups.spawn(async move { reader.catch_up(now).await });
                    }
                    match watcher.due_users().await {
                        Ok(users) => for user_id in users { spawn_dispatch(&mut dispatches, watcher.clone(), user_id); },
                        Err(err) => tracing::warn!(%err, "Invite-Lounge-Warteaufträge konnten nicht geprüft werden"),
                    }
                }
                Some(result) = catch_ups.join_next(), if !catch_ups.is_empty() => {
                    match result {
                        Ok(Ok(())) => caught_up = true,
                        Ok(Err(err)) => tracing::warn!(%err, "Invite-Lounge-Rückblick konnte nicht verarbeitet werden"),
                        Err(err) => tracing::error!(%err, "Invite-Lounge-Rückblick ist abgebrochen"),
                    }
                }
                Some(result) = turns.join_next(), if !turns.is_empty() => {
                    if let Err(err) = result { tracing::error!(%err, "Invite-Lounge-Nachrichtentask ist abgebrochen"); }
                }
                Some(result) = dispatches.join_next(), if !dispatches.is_empty() => {
                    if let Err(err) = result { tracing::error!(%err, "Invite-Lounge-Versandtask ist abgebrochen"); }
                }
            }
        }
    })
}

fn spawn_dispatch(
    dispatches: &mut tokio::task::JoinSet<()>,
    watcher: Arc<InviteLoungeWatcher>,
    user_id: u64,
) {
    dispatches.spawn(async move {
        let now = (watcher.clock_millis)() / 1000;
        watcher.follow_up(user_id, now).await;
    });
}

fn build_turn(
    event: &LoungeMessage,
    context: &[LoungeMessage],
    bot_id: u64,
    member: InviteMemberState,
) -> Option<InviteTurn> {
    let message = turn_text(&event.content, event.author_id, bot_id, MAX_TASK_TEXT_CHARS)?;
    let line = |message: &LoungeMessage| {
        turn_text(
            &message.content,
            event.author_id,
            bot_id,
            MAX_CHAT_LINE_CHARS,
        )
        .map(|text| InviteChatLine {
            speaker: speaker(message, event.author_id, bot_id),
            text,
        })
    };
    let mut earlier: Vec<&LoungeMessage> = context
        .iter()
        .filter(|message| message.message_id < event.message_id)
        .collect();
    earlier.sort_by_key(|message| message.message_id);
    earlier.dedup_by_key(|message| message.message_id);
    let mut recent: Vec<InviteChatLine> = earlier.into_iter().filter_map(line).collect();
    recent.drain(..recent.len().saturating_sub(dl_brain::MAX_TURN_RECENT_LINES));
    let reply_to = event
        .reply_message_id
        .and_then(|id| context.iter().find(|message| message.message_id == id))
        .and_then(line);
    Some(InviteTurn {
        message,
        reply_to,
        recent,
        member,
    })
}

fn speaker(message: &LoungeMessage, member_id: u64, bot_id: u64) -> InviteSpeaker {
    if message.author_id == member_id {
        InviteSpeaker::Member
    } else if message.author_id == bot_id {
        InviteSpeaker::Bot
    } else {
        InviteSpeaker::Other
    }
}

fn turn_text(content: &str, member_id: u64, bot_id: u64, max_chars: usize) -> Option<String> {
    let text = match MENTION_RE.as_ref() {
        Some(regex) => regex
            .replace_all(content, |caps: &Captures| {
                mention_label(&caps[1], &caps[2], member_id, bot_id)
            })
            .into_owned(),
        None => content.to_string(),
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(max_chars).collect())
}

fn mention_label(kind: &str, id: &str, member_id: u64, bot_id: u64) -> &'static str {
    let id = id.parse::<u64>().ok();
    match kind {
        "@&" => "@Rolle",
        "#" => "#kanal",
        _ if id == Some(bot_id) => "@Bot",
        _ if id == Some(member_id) => "@Mitglied",
        _ => "@jemand",
    }
}

fn member_state(state: Option<&LoungeState>, facts: &MemberFacts) -> InviteMemberState {
    InviteMemberState {
        phase: state.map_or(InviteRequestPhase::None, request_phase),
        has_friend_code: facts.known_code.is_some(),
        code_in_message: facts.message_code.is_some(),
        already_invited: facts.invited,
        last_update: state
            .and_then(|state| state.status.notices.last())
            .map(|notice| notice.update()),
    }
}

fn request_phase(state: &LoungeState) -> InviteRequestPhase {
    let Some(request) = state.request.as_ref() else {
        return InviteRequestPhase::None;
    };
    match request.phase {
        Phase::WaitingForCode => InviteRequestPhase::WaitingForCode,
        Phase::Dispatching => InviteRequestPhase::InProgress,
        Phase::Attempted | Phase::Expired => InviteRequestPhase::Closed,
        Phase::Pending
            if request.direct || state.status.invite_now || state.status.wait.is_some() =>
        {
            InviteRequestPhase::InProgress
        }
        Phase::Pending if state.status.asked() => InviteRequestPhase::CheckInAsked,
        Phase::Pending => InviteRequestPhase::WaitingForCommunity,
    }
}

fn apply_turn(
    state: &mut LoungeState,
    event: &LoungeMessage,
    decision: Option<&InviteTurnDecision>,
    facts: &MemberFacts,
    now: i64,
) -> bool {
    state.last_seen = event.message_id;
    let Some(decision) = decision else {
        return false;
    };
    let code = facts.code();
    let ready = !busy(state);
    let invited = member_state(Some(state), facts).invited();
    let applied = match decision.action {
        InviteTurnAction::Silent => false,
        InviteTurnAction::AskFriendCode => ask_friend_code(state, event, code.is_none()),
        InviteTurnAction::WaitForCommunity => {
            ready && !invited && wait_for_community(state, event, code)
        }
        InviteTurnAction::AskAlreadyInvited => {
            ready && ask_already_invited(state, event, code.filter(|_| !invited), now)
        }
        InviteTurnAction::Invite => ready && !invited && invite_now(state, event, code),
        InviteTurnAction::Close => ready && close_request(state, decision.text.is_none(), invited),
        InviteTurnAction::Help => true,
    };
    if let (true, Some(text)) = (applied, decision.text.clone()) {
        state.outbox.push(OutboxEntry::reply(
            event.message_id,
            event.channel_id,
            text,
            facts.invited,
        ));
    }
    applied
}

fn busy(state: &LoungeState) -> bool {
    state.request.as_ref().is_some_and(|request| {
        request.phase == Phase::Dispatching
            || (request.phase == Phase::Pending && state.status.wait.is_some())
    })
}

fn pending(state: &LoungeState) -> bool {
    state
        .request
        .as_ref()
        .is_some_and(|request| request.phase == Phase::Pending)
}

fn start_request(state: &mut LoungeState, event: &LoungeMessage, phase: Phase, direct: bool) {
    state.request = Some(Request {
        message_id: event.message_id,
        created_at: event.created_at,
        direct,
        phase,
    });
    state.status = RequestStatus::for_channel(event.channel_id);
}

fn ask_friend_code(state: &mut LoungeState, event: &LoungeMessage, without_code: bool) -> bool {
    if !without_code {
        return false;
    }
    let closed = state
        .request
        .as_ref()
        .is_none_or(|request| matches!(request.phase, Phase::Attempted | Phase::Expired));
    if closed {
        start_request(state, event, Phase::WaitingForCode, false);
    }
    true
}

fn wait_for_community(
    state: &mut LoungeState,
    event: &LoungeMessage,
    code: Option<String>,
) -> bool {
    let Some(code) = code else {
        return false;
    };
    state.friend_code = Some(code);
    if !pending(state) {
        start_request(state, event, Phase::Pending, false);
    }
    true
}

fn ask_already_invited(
    state: &mut LoungeState,
    event: &LoungeMessage,
    code: Option<String>,
    now: i64,
) -> bool {
    if !pending(state) {
        let Some(code) = code else {
            return true;
        };
        state.friend_code = Some(code);
        start_request(state, event, Phase::Pending, false);
    }
    state.status.check_in_at = Some(now);
    state.status.claim(Some(Notice::CheckIn));
    true
}

fn invite_now(state: &mut LoungeState, event: &LoungeMessage, code: Option<String>) -> bool {
    let Some(code) = code else {
        return false;
    };
    state.friend_code = Some(code);
    if pending(state) {
        state.status.invite_now = true;
    } else {
        start_request(state, event, Phase::Pending, true);
    }
    true
}

fn close_request(state: &mut LoungeState, notify: bool, invited: bool) -> bool {
    let open = state
        .request
        .as_ref()
        .is_some_and(|request| matches!(request.phase, Phase::Pending | Phase::WaitingForCode));
    let Some(message_id) = state
        .request
        .as_ref()
        .filter(|_| open)
        .map(|request| request.message_id)
    else {
        return true;
    };
    state
        .outbox
        .retain(|entry| entry.request_message_id != message_id);
    let notice = apply_step(state, Step::Finish(Some(Notice::Confirmed)));
    if notify {
        queue_notice(state, notice, invited);
    }
    true
}

fn unanswered(history: &[LoungeMessage], bot_id: u64) -> Vec<LoungeMessage> {
    let mut latest: BTreeMap<u64, &LoungeMessage> = BTreeMap::new();
    for message in history
        .iter()
        .filter(|message| !message.is_bot && message.author_id != bot_id)
    {
        latest.insert(message.author_id, message);
    }
    let mut open: Vec<LoungeMessage> = latest
        .into_values()
        .filter(|message| !history.iter().any(|later| answers(later, message)))
        .cloned()
        .collect();
    open.sort_by_key(|message| message.message_id);
    open
}

fn answers(later: &LoungeMessage, message: &LoungeMessage) -> bool {
    later.message_id > message.message_id
        && later.author_id != message.author_id
        && (later.reply_message_id == Some(message.message_id)
            || mentions_user(&later.content, message.author_id))
}

fn is_fresh(created_at: i64, now: i64, max_age: i64) -> bool {
    (0..=max_age).contains(&now.saturating_sub(created_at))
}

fn is_due(request: &Request, status: &RequestStatus) -> bool {
    request.phase == Phase::Pending
        && status.wait.is_none()
        && (request.direct || status.invite_now)
}

fn check_in_due(request: &Request, status: &RequestStatus, now: i64) -> bool {
    status.channel_id == Some(CHECK_IN_CHANNEL_ID)
        && request.phase == Phase::Pending
        && !request.direct
        && status.wait.is_none()
        && !status.invite_now
        && status.check_in_at.is_none()
        && check_in_age(now.saturating_sub(request.created_at))
}

fn apply_step(state: &mut LoungeState, step: Step) -> Option<Notice> {
    let (phase, wait, check_at, notice) = match step {
        Step::Finish(notice) => (Phase::Attempted, None, None, notice),
        Step::Wait { wait, at, notice } => (Phase::Pending, Some(wait), Some(at), notice),
        Step::Expire => (Phase::Expired, None, None, None),
        Step::Dispatch => return None,
    };
    let request = state.request.as_mut()?;
    request.phase = phase;
    state.status.wait = wait;
    state.status.check_at = check_at;
    state.status.invite_now = false;
    state.status.claim(notice)
}

fn queue_notice(state: &mut LoungeState, notice: Option<Notice>, invited: bool) {
    let (Some(notice), Some(request)) = (notice, state.request.as_ref()) else {
        return;
    };
    let invited = invited || notice == Notice::AlreadyInvited;
    let entry = OutboxEntry::notice(request.message_id, state.status.channel_id, notice, invited);
    state.outbox.push(entry);
}

fn settle_notice(state: &mut LoungeState, position: usize, result: NoticeResult, now_millis: i64) {
    let keep = match (result, state.outbox.get_mut(position)) {
        (NoticeResult::Retry, Some(entry)) => entry.reschedule(now_millis),
        _ => false,
    };
    if keep {
        return;
    }
    let removed = state.outbox.remove(position);
    match result {
        NoticeResult::Delivered(Some(message_id))
            if removed.remembers_check_in()
                && state
                    .request
                    .as_ref()
                    .is_some_and(|request| request.message_id == removed.request_message_id) =>
        {
            state.status.check_in_message_id = Some(message_id);
        }
        NoticeResult::Delivered(_) => {}
        NoticeResult::Dropped => {
            tracing::warn!(key = %removed.key, "Invite-Lounge: Meldung vom Broker abgelehnt");
        }
        NoticeResult::Retry => {
            tracing::warn!(key = %removed.key, attempts = removed.attempts, "Invite-Lounge: Meldung nach sieben Tagen ohne Zustellung aufgegeben");
        }
    }
}

fn answers_request(message: &LoungeMessage, request_id: u64, user_id: u64, bot_id: u64) -> bool {
    message.message_id > request_id
        && message.author_id != user_id
        && message.author_id != bot_id
        && !message.is_bot
        && (message.reply_message_id == Some(request_id)
            || mentions_user(&message.content, user_id))
}

fn mentions_user(content: &str, user_id: u64) -> bool {
    content.contains(&format!("<@{user_id}>")) || content.contains(&format!("<@!{user_id}>"))
}

fn friend_code(content: &str) -> Option<String> {
    let without_urls = match URL_RE.as_ref() {
        Some(regex) => regex.replace_all(content, " ").into_owned(),
        None => content.to_string(),
    };
    FRIEND_CODE_RE
        .as_ref()?
        .find(&without_urls)
        .map(|found| found.as_str().to_string())
}

fn cooldown_key(user_id: u64) -> String {
    format!("user:{user_id}")
}

#[cfg(test)]
#[path = "invite_lounge_tests.rs"]
mod decision_tests;
