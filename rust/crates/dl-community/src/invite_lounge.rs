use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, MutexGuard};
use std::time::Duration;

use dl_brain::brain_api::bounded_turn;
use dl_brain::{
    ConversationReply, InviteChatLine, InviteMemberState, InviteRequestPhase, InviteSpeaker,
    InviteTurn, InviteTurnAction, InviteTurnDecision,
};
use dl_bridges::steam::STEAM_BOT_FRIEND_CODE;
use dl_central_db::kv;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[path = "invite_lounge_status.rs"]
mod status;
pub use status::CHECK_IN_SECONDS;
use status::{
    after_invite, after_progress, check_in_age, check_in_step, classify_invite_reply,
    steam_outcome, CheckIn, Claim, ManualInvite, Notice, OutboxEntry, RequestStatus, SteamProgress,
    SteamWait, Step,
};

pub const CHECK_IN_CHANNEL_ID: u64 = 1_464_736_918_951_432_222;

const COOLDOWN_KV_NS: &str = "invite_lounge:cooldown";
const STATE_KV_NS: &str = "invite_lounge:requests";
const BACKFILL_KV_NS: &str = "invite_lounge:history";
const ACTIONS_SINCE_KEY: &str = "actions_since_v1";
const CATCH_UP_MARK_KEY: &str = "catch_up_mark_v1";
const TURN_FLOW_SINCE_KEY: &str = "turn_flow_since_v1";
const USER_KEY_PREFIX: &str = "user:";
const REQUEST_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;
const DISCORD_EPOCH_MILLIS: i64 = 1_420_070_400_000;
const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;
const DISPATCH_STALE_MILLIS: i64 =
    (dl_bridges::steam::INVITE_FORWARD_TIMEOUT_SECS as i64 + status::FOLLOW_UP_SECONDS) * 1000;
const LIVE_QUEUE: usize = 256;
const TICK: Duration = Duration::from_secs(15);
const MAX_CATCH_UP_BACKOFF: Duration = Duration::from_secs(16 * 60);
const CLOCK_SKEW_SECONDS: i64 = 5 * 60;

pub(crate) const PRIVACY_KV_NAMESPACES: [&str; 2] = [STATE_KV_NS, COOLDOWN_KV_NS];

pub(crate) fn privacy_kv_key(user_id: i64) -> String {
    user_key(user_id)
}

fn user_key(user_id: impl std::fmt::Display) -> String {
    format!("{USER_KEY_PREFIX}{user_id}")
}

static FRIEND_CODE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)[0-9]+(?-u:\b)").ok());
const MIN_FRIEND_CODE: u64 = 100_000;
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
    pub system: bool,
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
            system: false,
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
                system: !message.author.bot && !dl_discord::gateway::is_user_message(&message),
                guild_id: message.guild_id.map(|id| id.get()),
                channel_id: message.channel_id.get(),
                message_id: message.id.get(),
                author_id: message.author.id.get(),
                content: message.content,
                created_at: message.timestamp.unix_timestamp(),
                created_at_millis: snowflake_millis(message.id.get()),
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

impl Phase {
    const OPEN: [Self; 3] = [Self::WaitingForCode, Self::Pending, Self::Dispatching];

    fn open(self) -> bool {
        Self::OPEN.contains(&self)
    }

    fn open_names() -> Result<Vec<String>, String> {
        Self::OPEN
            .iter()
            .map(|phase| match serde_json::to_value(phase) {
                Ok(serde_json::Value::String(name)) => Ok(name),
                _ => Err(format!("Phase {phase:?} hat keinen Textnamen")),
            })
            .collect()
    }
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
    #[serde(default)]
    hint_sent: bool,
    #[serde(default)]
    last_hint_at: Option<i64>,
    last_result: Option<String>,
    dispatch_started_at_millis: Option<i64>,
    #[serde(default)]
    last_dispatch_finished_at_millis: Option<i64>,
    #[serde(default, skip_serializing_if = "RequestStatus::is_empty")]
    status: RequestStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    outbox: Vec<OutboxEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeResult {
    Delivered,
    Dropped,
    Retired,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cas {
    Written,
    Stale,
    OptedOut,
}

impl Cas {
    fn written(self, user_id: u64, step: &'static str) -> bool {
        match self {
            Self::Written => true,
            Self::Stale => {
                tracing::info!(
                    user_id,
                    step,
                    "Invite-Lounge: Stand hat sich parallel geändert, der Schritt wird beim nächsten Durchlauf neu bewertet"
                );
                false
            }
            Self::OptedOut => {
                tracing::info!(
                    user_id,
                    step,
                    "Invite-Lounge verwirft den Schritt nach Datenschutz-Opt-out"
                );
                false
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Admission {
    Accept(String),
    Ignore,
    Defer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    Stored(u64),
    Skipped,
    Failed,
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
    ) -> Result<Cas, String>;
    async fn pending_users(&self) -> Result<Vec<u64>, String>;
    async fn outbox_users(&self) -> Result<Vec<u64>, String>;
    async fn known_code(&self, user_id: u64) -> Result<Option<String>, String>;
    async fn opted_out(&self, user_id: u64) -> Result<bool, String>;
    async fn hidden_authors(&self, user_ids: &[u64]) -> Result<HashSet<u64>, String>;
    async fn actions_since(&self) -> Result<Option<i64>, String>;
    async fn mark_actions_since(&self, now: i64) -> Result<(), String>;
    async fn turn_flow_since(&self) -> Result<Option<i64>, String>;
    async fn mark_turn_flow_since(&self, now: i64) -> Result<(), String>;
    async fn catch_up_mark(&self) -> Result<Option<u64>, String>;
    async fn set_catch_up_mark(&self, message_id: u64) -> Result<(), String>;
    async fn steam_progress(
        &self,
        code: &str,
        since: i64,
        requested_at: i64,
    ) -> Result<SteamProgress, String>;
    async fn invited(&self, code: &str) -> Result<bool, String>;
    async fn manual_invite(&self, code: &str, since: i64) -> Result<ManualInvite, String>;
}

fn stored_matches(raw: Option<&str>, previous: Option<&LoungeState>) -> bool {
    match (raw, previous) {
        (None, None) => true,
        (Some(raw), Some(previous)) => {
            serde_json::from_str::<LoungeState>(raw).is_ok_and(|current| current == *previous)
        }
        _ => false,
    }
}

struct PgLoungeStore {
    pool: PgPool,
}

#[async_trait::async_trait]
impl LoungeStore for PgLoungeStore {
    async fn load(&self, user_id: u64) -> Result<Option<LoungeState>, String> {
        let state = kv::get(&self.pool, STATE_KV_NS, &user_key(user_id))
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
    ) -> Result<Cas, String> {
        let privacy_id = i64::try_from(user_id).map_err(|err| err.to_string())?;
        let key = user_key(user_id);
        let value = serde_json::to_string(next).map_err(|err| err.to_string())?;
        let mut tx = self.pool.begin().await.map_err(|err| err.to_string())?;
        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, privacy_id)
            .await
            .map_err(|err| err.to_string())?
        {
            return Ok(Cas::OptedOut);
        }
        let current: Option<String> =
            sqlx::query_scalar("SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2 FOR UPDATE")
                .bind(STATE_KV_NS)
                .bind(&key)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|err| err.to_string())?;
        if !stored_matches(current.as_deref(), previous) {
            return Ok(Cas::Stale);
        }
        sqlx::query(
            "INSERT INTO bot.kv_store (ns, k, v) VALUES ($1, $2, $3)
             ON CONFLICT (ns, k) DO UPDATE SET v = EXCLUDED.v",
        )
        .bind(STATE_KV_NS)
        .bind(&key)
        .bind(value)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        tx.commit().await.map_err(|err| err.to_string())?;
        Ok(Cas::Written)
    }

    async fn pending_users(&self) -> Result<Vec<u64>, String> {
        let keys: Vec<String> =
            sqlx::query_scalar(&visible_rows("kv.v::jsonb #>> '{request,phase}' = ANY($3)"))
                .bind(STATE_KV_NS)
                .bind(USER_KEY_PREFIX)
                .bind(Phase::open_names()?)
                .fetch_all(&self.pool)
                .await
                .map_err(|err| err.to_string())?;
        Ok(user_ids(keys))
    }

    async fn outbox_users(&self) -> Result<Vec<u64>, String> {
        let keys: Vec<String> = sqlx::query_scalar(&visible_rows(
            "jsonb_array_length(COALESCE(kv.v::jsonb -> 'outbox', '[]'::jsonb)) > 0",
        ))
        .bind(STATE_KV_NS)
        .bind(USER_KEY_PREFIX)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(user_ids(keys))
    }

    async fn known_code(&self, user_id: u64) -> Result<Option<String>, String> {
        let discord_id = i64::try_from(user_id).map_err(|err| err.to_string())?;
        let steam_id: Option<String> = sqlx::query_scalar("SELECT steam_id FROM core.steam_links WHERE discord_id = $1 AND verified ORDER BY primary_account DESC, steam_id LIMIT 1")
            .bind(discord_id).fetch_optional(&self.pool).await.map_err(|err| err.to_string())?;
        Ok(steam_id
            .and_then(|id| id.parse::<u64>().ok())
            .and_then(|id| id.checked_sub(STEAM_ID64_BASE))
            .and_then(|id| friend_code_id(&id.to_string()))
            .map(|id| id.to_string()))
    }

    async fn opted_out(&self, user_id: u64) -> Result<bool, String> {
        Ok(!self.hidden_authors(&[user_id]).await?.is_empty())
    }

    async fn hidden_authors(&self, user_ids: &[u64]) -> Result<HashSet<u64>, String> {
        let mut hidden: HashSet<u64> = user_ids
            .iter()
            .copied()
            .filter(|id| i64::try_from(*id).is_err())
            .collect();
        let ids: Vec<i64> = user_ids
            .iter()
            .filter_map(|id| i64::try_from(*id).ok())
            .collect();
        let rows = dl_central_db::hidden_privacy_users(&self.pool, &ids)
            .await
            .map_err(|err| err.to_string())?;
        hidden.extend(rows.into_iter().filter_map(|id| u64::try_from(id).ok()));
        Ok(hidden)
    }

    async fn catch_up_mark(&self) -> Result<Option<u64>, String> {
        let raw = kv::get(&self.pool, BACKFILL_KV_NS, CATCH_UP_MARK_KEY)
            .await
            .map_err(|err| err.to_string())?;
        raw.map(|value| {
            value
                .parse()
                .map_err(|_| "Ungültige Rückblick-Marke".to_string())
        })
        .transpose()
    }

    async fn set_catch_up_mark(&self, message_id: u64) -> Result<(), String> {
        kv::set(
            &self.pool,
            BACKFILL_KV_NS,
            CATCH_UP_MARK_KEY,
            &message_id.to_string(),
        )
        .await
        .map_err(|err| err.to_string())
    }

    async fn invited(&self, code: &str) -> Result<bool, String> {
        let Some(steam_id) = steam_id64(code) else {
            return Ok(false);
        };
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM steam.beta_invite_audit WHERE steam_id64 = $1)",
        )
        .bind(steam_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| err.to_string())
    }

    async fn actions_since(&self) -> Result<Option<i64>, String> {
        kv::get(&self.pool, BACKFILL_KV_NS, ACTIONS_SINCE_KEY)
            .await
            .map_err(|err| err.to_string())?
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| "Ungültige Startzeit der Invite-Lounge".to_string())
            })
            .transpose()
    }

    async fn mark_actions_since(&self, now: i64) -> Result<(), String> {
        let stored = kv::get(&self.pool, BACKFILL_KV_NS, ACTIONS_SINCE_KEY)
            .await
            .map_err(|err| err.to_string())?;
        match stored {
            Some(value) if value.parse::<i64>().is_ok() => Ok(()),
            Some(value) => {
                tracing::warn!(%value, now, "Invite-Lounge: ungültige Startzeit wird durch den aktuellen Start ersetzt");
                kv::set(
                    &self.pool,
                    BACKFILL_KV_NS,
                    ACTIONS_SINCE_KEY,
                    &now.to_string(),
                )
                .await
                .map_err(|err| err.to_string())
            }
            None => kv::set_if_absent(
                &self.pool,
                BACKFILL_KV_NS,
                ACTIONS_SINCE_KEY,
                &now.to_string(),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string()),
        }
    }

    async fn turn_flow_since(&self) -> Result<Option<i64>, String> {
        kv::get(&self.pool, BACKFILL_KV_NS, TURN_FLOW_SINCE_KEY)
            .await
            .map_err(|err| err.to_string())?
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| "Ungültige Startzeit des Brain-Ablaufs".to_string())
            })
            .transpose()
    }

    async fn mark_turn_flow_since(&self, now: i64) -> Result<(), String> {
        kv::set_if_absent(
            &self.pool,
            BACKFILL_KV_NS,
            TURN_FLOW_SINCE_KEY,
            &now.to_string(),
        )
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
    }

    async fn steam_progress(
        &self,
        code: &str,
        since: i64,
        requested_at: i64,
    ) -> Result<SteamProgress, String> {
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
            "SELECT EXISTS(SELECT 1 FROM steam.beta_invite_audit
                            WHERE steam_id64 = $1 AND invited_at >= to_timestamp($2)),
                    EXISTS(SELECT 1 FROM steam.invite_requests WHERE steam_id64 = $1)",
        )
        .bind(steam_id)
        .bind(requested_at as f64)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(SteamProgress {
            outcome: outcome.and_then(|(decision, code)| steam_outcome(&decision, code)),
            invited,
            open,
        })
    }

    async fn manual_invite(&self, code: &str, since: i64) -> Result<ManualInvite, String> {
        let Some(steam_id) = steam_id64(code) else {
            return Ok(ManualInvite::None);
        };
        let (invited, in_flight): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM steam.bot_event_log
                            WHERE event_type = 'playtest_invite' AND steam_id = $1
                              AND decision = 'sent' AND occurred_at >= to_timestamp($2))
                 OR EXISTS(SELECT 1 FROM steam.beta_invite_audit
                            WHERE steam_id64 = $1 AND invited_at >= to_timestamp($2)),
                    EXISTS(SELECT 1 FROM steam.invite_requests WHERE steam_id64 = $1)",
        )
        .bind(steam_id)
        .bind(since as f64)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(ManualInvite::from_evidence(invited, in_flight))
    }
}

fn visible_rows(filter: &str) -> String {
    format!(
        "SELECT kv.k FROM bot.kv_store kv
          WHERE kv.ns = $1 AND {filter}
            AND NOT EXISTS (SELECT 1 FROM core.user_privacy p
                             WHERE $2 || p.user_id::text = kv.k AND {})",
        dl_central_db::PRIVACY_HIDDEN_PREDICATE
    )
}

fn user_ids(keys: Vec<String>) -> Vec<u64> {
    keys.into_iter()
        .filter_map(|key| {
            let id = key
                .strip_prefix(USER_KEY_PREFIX)
                .and_then(|id| id.parse().ok());
            if id.is_none() {
                tracing::warn!(%key, "Invite-Lounge: ungültiger Nutzer-Schlüssel im Lounge-Zustand wird übersprungen");
            }
            id
        })
        .collect()
}

fn friend_code_id(code: &str) -> Option<u64> {
    if code.is_empty() || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    code.parse::<u64>().ok().filter(|id| {
        (MIN_FRIEND_CODE..=u64::from(u32::MAX)).contains(id)
            && *id != u64::from(STEAM_BOT_FRIEND_CODE)
    })
}

fn invite_code(
    message: Option<&str>,
    stored: Option<&str>,
    linked: Option<&str>,
) -> Option<String> {
    [message, stored, linked]
        .into_iter()
        .flatten()
        .find_map(friend_code_id)
        .map(|id| id.to_string())
}

fn steam_id64(code: &str) -> Option<i64> {
    friend_code_id(code).and_then(|id| i64::try_from(id + STEAM_ID64_BASE).ok())
}

type UserLocks = std::sync::Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>>;

#[derive(Debug, Default)]
struct LiveOrder {
    started: u64,
    everyone: Option<u64>,
    authors: HashMap<u64, u64>,
    missed: bool,
}

impl LiveOrder {
    fn at_start() -> Self {
        Self {
            everyone: Some(1),
            missed: true,
            ..Self::default()
        }
    }

    fn hold(&mut self, author_id: u64) {
        let needed = self.started + 1;
        let held = self.authors.entry(author_id).or_insert(needed);
        *held = (*held).max(needed);
        self.missed = true;
    }

    fn hold_everyone(&mut self) {
        let needed = self.started + 1;
        self.everyone = Some(self.everyone.map_or(needed, |held| held.max(needed)));
        self.missed = true;
    }

    fn holds(&mut self, author_id: u64) -> bool {
        let held = self.everyone.is_some() || self.authors.contains_key(&author_id);
        if held {
            self.hold(author_id);
        }
        held
    }

    fn begin(&mut self) -> u64 {
        self.started += 1;
        self.started
    }

    fn finish(&mut self, generation: u64, failed: &HashSet<u64>) {
        if self.everyone.is_some_and(|needed| needed <= generation) {
            self.everyone = None;
        }
        self.authors.retain(|_, needed| *needed > generation);
        for author_id in failed {
            self.hold(*author_id);
        }
    }

    fn take_missed(&mut self) -> bool {
        std::mem::take(&mut self.missed)
    }
}
type FollowUps = std::sync::Mutex<HashMap<u64, bool>>;

struct InFlight<'a> {
    running: &'a FollowUps,
    user_id: u64,
    released: bool,
}

impl<'a> InFlight<'a> {
    fn enter(running: &'a FollowUps, user_id: u64) -> Option<Self> {
        let mut map = running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(rerun) = map.get_mut(&user_id) {
            *rerun = true;
            return None;
        }
        map.insert(user_id, false);
        Some(Self {
            running,
            user_id,
            released: false,
        })
    }

    fn again(&mut self) -> bool {
        let mut map = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(rerun) = map.get_mut(&self.user_id).filter(|rerun| **rerun) {
            *rerun = false;
            return true;
        }
        map.remove(&self.user_id);
        self.released = true;
        false
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if !self.released {
            self.running
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&self.user_id);
        }
    }
}

pub struct InviteLoungeWatcher {
    store: Arc<dyn LoungeStore>,
    port: Arc<dyn InviteLoungeReplyPort>,
    invite: Arc<dyn InviteEventPort>,
    brain: Arc<dyn dl_brain::AiAnswerer>,
    guild_id: u64,
    clock_millis: Arc<dyn Fn() -> i64 + Send + Sync>,
    locks: UserLocks,
    follow_ups: FollowUps,
    order: std::sync::Mutex<LiveOrder>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MemberFacts {
    message_code: Option<String>,
    linked_code: Option<String>,
    invited: bool,
}

impl MemberFacts {
    fn code(&self, state: Option<&LoungeState>) -> Option<String> {
        invite_code(
            self.message_code.as_deref(),
            stored_code(state),
            self.linked_code.as_deref(),
        )
    }

    fn known_code(&self, state: Option<&LoungeState>) -> Option<String> {
        invite_code(None, stored_code(state), self.linked_code.as_deref())
    }
}

fn stored_code(state: Option<&LoungeState>) -> Option<&str> {
    state.and_then(|state| state.friend_code.as_deref())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnOutcome {
    Failed,
    Silent,
    Decided(InviteTurnDecision),
}

enum Context<'a> {
    Fetch,
    Given(&'a [LoungeMessage]),
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
            follow_ups: FollowUps::default(),
            order: std::sync::Mutex::new(LiveOrder::at_start()),
        }
    }

    fn order(&self) -> MutexGuard<'_, LiveOrder> {
        self.order
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn watches(&self, event: &LoungeMessage) -> bool {
        !event.is_bot
            && !event.system
            && event.guild_id == Some(self.guild_id)
            && event.channel_id == CHECK_IN_CHANNEL_ID
    }

    async fn serve_live(&self, event: LoungeMessage) -> Turn {
        if self.order().holds(event.author_id) {
            tracing::info!(
                user_id = event.author_id,
                message_id = event.message_id,
                "Invite-Lounge: eine frühere Nachricht des Mitglieds wartet auf den Rückblick, diese folgt dort in Reihenfolge"
            );
            return Turn::Failed;
        }
        let now = (self.clock_millis)() / 1000;
        let turn = self
            .handle_turn(&event, Context::Fetch, now)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, message_id = event.message_id, "Invite-Lounge-Nachricht konnte nicht verarbeitet werden, der Rückblick holt sie nach");
                Turn::Failed
            });
        if turn == Turn::Failed {
            self.order().hold(event.author_id);
        }
        turn
    }

    async fn run_follow_up(self: Arc<Self>, user_id: u64) {
        let Some(mut in_flight) = InFlight::enter(&self.follow_ups, user_id) else {
            return;
        };
        loop {
            let now = (self.clock_millis)() / 1000;
            self.follow_up(user_id, now).await;
            if !in_flight.again() {
                return;
            }
        }
    }

    async fn context_page(&self, event: &LoungeMessage) -> Result<Vec<LoungeMessage>, String> {
        self.port
            .history(event.channel_id, Some(event.message_id))
            .await
            .map_err(|error| format!("Verlauf für den Kontext nicht lesbar: {error}"))
    }

    async fn visible(
        &self,
        context: Vec<LoungeMessage>,
        member_id: u64,
    ) -> Result<Vec<LoungeMessage>, String> {
        let mut others: Vec<u64> = context
            .iter()
            .filter(|message| !message.is_bot && message.author_id != member_id)
            .map(|message| message.author_id)
            .collect();
        others.sort_unstable();
        others.dedup();
        if others.is_empty() {
            return Ok(context);
        }
        let hidden = self.store.hidden_authors(&others).await.map_err(|error| {
            format!("Datenschutzstand anderer Mitglieder nicht lesbar: {error}")
        })?;
        Ok(context
            .into_iter()
            .filter(|message| !hidden.contains(&message.author_id))
            .collect())
    }

    async fn follow_up(&self, user_id: u64, now: i64) {
        match self.store.opted_out(user_id).await {
            Ok(false) => {}
            Ok(true) => {
                tracing::info!(
                    user_id,
                    "Invite-Lounge handelt nach Datenschutz-Opt-out nicht mehr"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(%error, user_id, "Invite-Lounge: Datenschutzstand nicht lesbar, der Folgeschritt wartet auf den nächsten Takt");
                return;
            }
        }
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
        context: Context<'_>,
        now: i64,
    ) -> Result<Turn, String> {
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        let message = match self.accepts(event, bot_id, now).await? {
            Admission::Accept(message) => message,
            Admission::Ignore => return Ok(Turn::Skipped),
            Admission::Defer => return Ok(Turn::Failed),
        };
        let user_id = event.author_id;
        let lock = self.user_lock(user_id);
        let _guard = lock.lock().await;
        let previous = self.store.load(user_id).await?;
        if previous
            .as_ref()
            .is_some_and(|state| event.message_id <= state.last_seen)
        {
            return Ok(Turn::Skipped);
        }
        if self.store.opted_out(user_id).await? {
            tracing::info!(
                message_id = event.message_id,
                "Invite-Lounge überspringt Nachricht nach Datenschutz-Opt-out"
            );
            return Ok(Turn::Skipped);
        }
        let context = match context {
            Context::Fetch => self.context_page(event).await?,
            Context::Given(context) => context.to_vec(),
        };
        let context = self.visible(context, user_id).await?;
        let last_seen = previous.as_ref().map_or(0, |state| state.last_seen);
        let facts = self
            .member_facts(event, &context, last_seen, previous.as_ref())
            .await?;
        let member = member_state(previous.as_ref(), &facts);
        let turn = build_turn(event, message, &context, bot_id, member);
        let decision = match self.decide(event, &turn).await {
            TurnOutcome::Failed => return Ok(Turn::Failed),
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
        self.store_turn(event, &turn, decision.as_ref(), &facts, now)
            .await
            .map(|stored| {
                if stored {
                    Turn::Stored(user_id)
                } else {
                    Turn::Skipped
                }
            })
    }

    async fn store_turn(
        &self,
        event: &LoungeMessage,
        turn: &InviteTurn,
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
            let current = member_state(previous.as_ref(), facts);
            let fitting = decision.filter(|decision| still_fits(decision, turn, &current));
            let mut next = previous.clone().unwrap_or_default();
            apply_turn(&mut next, event, fitting, facts, now);
            match self
                .store
                .compare_exchange(event.author_id, previous.as_ref(), &next)
                .await?
            {
                Cas::Written => {}
                Cas::Stale => continue,
                Cas::OptedOut => {
                    tracing::info!(
                        message_id = event.message_id,
                        "Invite-Lounge verwirft die Entscheidung nach Datenschutz-Opt-out"
                    );
                    return Ok(false);
                }
            }
            if let (Some(decision), None) = (decision, fitting) {
                tracing::info!(
                    user_id = event.author_id,
                    message_id = event.message_id,
                    action = decision.action.as_str(),
                    "Invite-Lounge: Stand hat sich während der Entscheidung geändert, Entscheidung verworfen"
                );
            }
            return Ok(true);
        }
    }

    async fn accepts(
        &self,
        event: &LoungeMessage,
        bot_id: u64,
        now: i64,
    ) -> Result<Admission, String> {
        if !self.watches(event) {
            return Ok(Admission::Ignore);
        }
        let Some(message) = turn_text(&event.content, event.author_id, bot_id) else {
            tracing::info!(
                message_id = event.message_id,
                "Invite-Lounge: Nachricht ohne Text (Bild, Sticker oder Anhang) geht nicht an den Brain"
            );
            return Ok(Admission::Ignore);
        };
        if now.saturating_sub(event.created_at) < -CLOCK_SKEW_SECONDS {
            tracing::warn!(
                message_id = event.message_id,
                created_at = event.created_at,
                now,
                "Invite-Lounge: Nachricht liegt in der Zukunft, der Rückblick versucht sie erneut"
            );
            return Ok(Admission::Defer);
        }
        if !is_fresh(event.created_at, now, REQUEST_WINDOW_SECONDS) {
            tracing::info!(
                message_id = event.message_id,
                "Invite-Lounge überspringt verspätete Nachricht"
            );
            return Ok(Admission::Ignore);
        }
        match self.store.actions_since().await? {
            None => {
                tracing::warn!(
                    message_id = event.message_id,
                    "Invite-Lounge: Startzeit noch nicht gespeichert, der Rückblick holt die Nachricht nach"
                );
                Ok(Admission::Defer)
            }
            Some(since) if event.created_at < since => {
                tracing::info!(
                    message_id = event.message_id,
                    "Invite-Lounge überspringt Nachricht von vor dem ersten Start"
                );
                Ok(Admission::Ignore)
            }
            Some(_) => Ok(Admission::Accept(message)),
        }
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
        context: &[LoungeMessage],
        last_seen: u64,
        state: Option<&LoungeState>,
    ) -> Result<MemberFacts, String> {
        let message_code = unseen_code(event, context, last_seen);
        let linked_code = match message_code {
            Some(_) => None,
            None => self
                .store
                .known_code(event.author_id)
                .await
                .map_err(|error| format!("Verknüpfter Steam-Account nicht lesbar: {error}"))?,
        };
        let mut facts = MemberFacts {
            message_code,
            linked_code,
            invited: false,
        };
        if let Some(code) = facts.code(state) {
            facts.invited = self
                .store
                .invited(&code)
                .await
                .map_err(|error| format!("Einladungsstand nicht lesbar: {error}"))?;
        }
        Ok(facts)
    }

    async fn invited(&self, code: Option<&str>) -> bool {
        let Some(code) = code else {
            return false;
        };
        match self.store.invited(code).await {
            Ok(invited) => invited,
            Err(error) => {
                tracing::warn!(%error, "Invite-Lounge: Einladungsstand nicht lesbar");
                false
            }
        }
    }

    async fn decide(&self, event: &LoungeMessage, turn: &InviteTurn) -> TurnOutcome {
        let task = dl_brain::DiscordConversation::InviteTurn { turn: turn.clone() };
        match self.converse(event.author_id, event.channel_id, task).await {
            Some(ConversationReply::Decided { decision }) if decision.fits(turn) => {
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

    async fn skipped_by_old_flow(&self, status: &RequestStatus) -> Result<bool, String> {
        let Some(check_in_at) = status.check_in_at.filter(|_| !status.asked()) else {
            return Ok(false);
        };
        Ok(self
            .store
            .turn_flow_since()
            .await?
            .is_some_and(|since| check_in_at < since))
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

    async fn catch_up(self: &Arc<Self>, now: i64, start_mark: u64) -> Result<HashSet<u64>, String> {
        let mark = match self.store.catch_up_mark().await? {
            Some(mark) => mark,
            None => {
                self.store.set_catch_up_mark(start_mark).await?;
                start_mark
            }
        };
        let mut history = self
            .recent_history(CHECK_IN_CHANNEL_ID, now - REQUEST_WINDOW_SECONDS, mark)
            .await?;
        for message in &mut history {
            message.guild_id = Some(self.guild_id);
        }
        history.sort_by_key(|message| message.message_id);
        history.dedup_by_key(|message| message.message_id);
        let mut failed: HashSet<u64> = HashSet::new();
        let mut effects = tokio::task::JoinSet::new();
        let mut reached = mark;
        for message in history.iter().filter(|message| message.message_id > mark) {
            if failed.contains(&message.author_id) {
                continue;
            }
            match self
                .handle_turn(message, Context::Given(&history), now)
                .await
            {
                Ok(Turn::Stored(user_id)) => {
                    effects.spawn(self.clone().run_follow_up(user_id));
                }
                Ok(Turn::Skipped) => {}
                Ok(Turn::Failed) => {
                    failed.insert(message.author_id);
                }
                Err(error) => {
                    tracing::warn!(%error, message_id = message.message_id, "Invite-Lounge-Rückblick: Nachricht nicht verarbeitet");
                    failed.insert(message.author_id);
                }
            }
            if failed.is_empty() {
                reached = message.message_id;
            }
        }
        while let Some(result) = effects.join_next().await {
            log_follow_up(result);
        }
        if reached > mark {
            self.store.set_catch_up_mark(reached).await?;
        }
        Ok(failed)
    }

    async fn recent_history(
        &self,
        channel_id: u64,
        since: i64,
        mark: u64,
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
            let reached_window = page.iter().any(|message| message.created_at < since);
            history.extend(page);
            if oldest <= mark {
                return Ok(history);
            }
            if reached_window {
                tracing::info!(
                    mark,
                    oldest,
                    "Invite-Lounge-Rückblick endet am Anfragefenster, ältere ungelesene Nachrichten bleiben unbewertet"
                );
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
            let cas = self
                .store
                .compare_exchange(user_id, Some(&previous), &claimed)
                .await?;
            if cas == Cas::Stale {
                continue;
            }
            if !cas.written(user_id, "deliver_outbox") {
                return Ok(());
            }
            let result = self.send_notice(user_id, &entry, retrying).await;
            self.finish_notice(user_id, &entry, result).await?;
            if result == NoticeResult::Retry {
                return Ok(());
            }
        }
    }

    async fn send_notice(&self, user_id: u64, entry: &OutboxEntry, retrying: bool) -> NoticeResult {
        let Some(update) = entry.update.current() else {
            return NoticeResult::Retired;
        };
        if let (true, Some(channel_id)) = (retrying, entry.channel_id) {
            let since = entry.first_attempt_at_millis.unwrap_or(0);
            match self
                .bot_replied_since(channel_id, entry.request_message_id, user_id, since)
                .await
            {
                Ok(true) => return NoticeResult::Delivered,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(%error, user_id, key = %entry.key, "Invite-Lounge: Zustellung nicht prüfbar");
                    return NoticeResult::Retry;
                }
            }
        }
        let target =
            entry
                .channel_id
                .map_or(dl_brain::NoticeTarget::DirectMessage {}, |channel_id| {
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
            Some(ConversationReply::Delivered { .. }) => NoticeResult::Delivered,
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
            let cas = self
                .store
                .compare_exchange(user_id, Some(&previous), &next)
                .await?;
            if cas != Cas::Stale {
                cas.written(user_id, "finish_notice");
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
            if page.iter().any(|message| {
                message.author_id == bot_id
                    && message.reply_message_id == Some(request_id)
                    && mentions_user(&message.content, user_id)
                    && message.created_at_millis >= since_millis
            }) {
                return Ok(true);
            }
            let oldest = page
                .iter()
                .map(|message| message.message_id)
                .min()
                .ok_or("Leere Verlaufsseite")?;
            if oldest <= request_id {
                return Ok(false);
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
        let Some(request) = open_request(&previous) else {
            return Ok(());
        };
        let now_millis = (self.clock_millis)();
        if request.phase == Phase::Dispatching {
            return self.recover_dispatch(user_id, &previous, now_millis).await;
        }
        let now = now.max(now_millis / 1000);
        if !is_fresh(request.created_at, now, REQUEST_WINDOW_SECONDS) {
            return self
                .expire(user_id, &previous, "Verspätete Bitte ohne Versand beendet")
                .await;
        }
        if unwatched(request, &previous.status) {
            return self
                .expire(
                    user_id,
                    &previous,
                    "Bitte aus nicht mehr beobachtetem Kanal beendet",
                )
                .await;
        }
        if request.phase == Phase::WaitingForCode {
            return Ok(());
        }
        let Some(code) = previous
            .friend_code
            .as_deref()
            .and_then(friend_code_id)
            .map(|id| id.to_string())
        else {
            return self
                .expire(
                    user_id,
                    &previous,
                    "Bitte ohne gültigen Freundescode beendet",
                )
                .await;
        };
        if !self.allows_actions(request.created_at).await? {
            return Ok(());
        }
        if let Some(wait) = previous.status.wait {
            if previous.status.check_at.is_some_and(|at| now < at) {
                return Ok(());
            }
            return self
                .follow_steam(user_id, &previous, &code, wait, now)
                .await;
        }
        let skipped_before = self.skipped_by_old_flow(&previous.status).await?;
        let check_in_now = check_in_due(request, &previous.status, now, skipped_before);
        if !check_in_now && !is_due(request, &previous.status) {
            return Ok(());
        }
        match self.manual_state(&previous, request, &code).await? {
            ManualInvite::Invited => return self.close_handled(user_id, &previous).await,
            ManualInvite::InFlight => {
                tracing::debug!(
                    user_id,
                    "Invite-Lounge: Einladung von Hand läuft noch, die Nachfrage wartet"
                );
                return Ok(());
            }
            ManualInvite::None => {}
        }
        if check_in_now {
            if skipped_before {
                tracing::info!(
                    user_id,
                    "Invite-Lounge: die vorige Version hat die Nachfrage übersprungen, sie wird jetzt gestellt"
                );
            }
            return self.check_in(user_id, &previous, &code, now).await;
        }
        self.send_invite(user_id, &previous, &code, now).await
    }

    async fn manual_state(
        &self,
        state: &LoungeState,
        request: &Request,
        code: &str,
    ) -> Result<ManualInvite, String> {
        let dispatched_here = state
            .dispatch_started_at_millis
            .is_some_and(|started| started / 1000 >= request.created_at);
        if request.direct || state.status.invite_now || dispatched_here {
            return Ok(ManualInvite::None);
        }
        self.store.manual_invite(code, request.created_at).await
    }

    async fn recover_dispatch(
        &self,
        user_id: u64,
        previous: &LoungeState,
        now_millis: i64,
    ) -> Result<(), String> {
        let stale = previous
            .dispatch_started_at_millis
            .is_none_or(|started| now_millis.saturating_sub(started) >= DISPATCH_STALE_MILLIS);
        if !stale {
            return Ok(());
        }
        let mut next = previous.clone();
        apply_step(
            &mut next,
            Step::Wait {
                wait: SteamWait::Delivery,
                at: now_millis / 1000,
                notice: None,
            },
        );
        next.last_result = Some("Versuch ohne Abschluss; Steam-Stand wird geprüft".into());
        if self
            .store
            .compare_exchange(user_id, Some(previous), &next)
            .await?
            .written(user_id, "recover_dispatch")
        {
            tracing::warn!(
                user_id,
                "Invite-Lounge: Versand ohne Abschluss, der Steam-Stand entscheidet"
            );
        }
        Ok(())
    }

    async fn close_handled(&self, user_id: u64, previous: &LoungeState) -> Result<(), String> {
        let mut next = previous.clone();
        if let Some(request) = previous.request.as_ref() {
            next.outbox
                .retain(|entry| !entry.belongs_to(request.message_id));
        }
        apply_step(&mut next, Step::Finish(None));
        next.last_result = Some("Schon von Hand erledigt; keine Meldung".into());
        if self
            .store
            .compare_exchange(user_id, Some(previous), &next)
            .await?
            .written(user_id, "close_handled")
        {
            tracing::info!(
                user_id,
                "Invite-Lounge schließt von Hand erledigte Bitte ohne Meldung"
            );
        }
        Ok(())
    }

    async fn expire(
        &self,
        user_id: u64,
        previous: &LoungeState,
        reason: &str,
    ) -> Result<(), String> {
        let mut expired = previous.clone();
        let Some(request) = expired.request.as_mut() else {
            return Ok(());
        };
        request.phase = Phase::Expired;
        let message_id = request.message_id;
        expired.last_result = Some(reason.into());
        if self
            .store
            .compare_exchange(user_id, Some(previous), &expired)
            .await?
            .written(user_id, "expire")
        {
            tracing::info!(
                user_id,
                message_id,
                reason,
                "Invite-Lounge beendet Auftrag ohne Versand"
            );
        }
        Ok(())
    }

    async fn follow_steam(
        &self,
        user_id: u64,
        previous: &LoungeState,
        code: &str,
        wait: SteamWait,
        now: i64,
    ) -> Result<(), String> {
        let Some(request) = previous.request.as_ref() else {
            return Ok(());
        };
        let since = steam_since(previous, request);
        let progress = self
            .store
            .steam_progress(code, since, request.created_at)
            .await?;
        match after_progress(wait, progress, now) {
            Step::Dispatch => self.send_invite(user_id, previous, code, now).await,
            step => self.settle(user_id, previous, step).await,
        }
    }

    async fn settle(&self, user_id: u64, previous: &LoungeState, step: Step) -> Result<(), String> {
        let mut next = previous.clone();
        let notice = apply_step(&mut next, step);
        self.queue_step_notice(&mut next, notice).await;
        self.store
            .compare_exchange(user_id, Some(previous), &next)
            .await?
            .written(user_id, "settle");
        Ok(())
    }

    async fn check_in(
        &self,
        user_id: u64,
        previous: &LoungeState,
        code: &str,
        now: i64,
    ) -> Result<(), String> {
        let invited = self.store.invited(code).await?;
        let mut next = previous.clone();
        next.status.check_in_at = Some(now);
        let notice = match check_in_step(invited) {
            CheckIn::AlreadyInvited => {
                apply_step(&mut next, Step::Finish(Some(Notice::AlreadyInvited)))
            }
            CheckIn::Ask => next.status.claim(Some(Notice::CheckIn)),
        };
        queue_notice(&mut next, notice, invited);
        self.store
            .compare_exchange(user_id, Some(previous), &next)
            .await?
            .written(user_id, "check_in");
        Ok(())
    }

    async fn send_invite(
        &self,
        user_id: u64,
        previous: &LoungeState,
        code: &str,
        now: i64,
    ) -> Result<(), String> {
        let Some(request) = previous.request.as_ref() else {
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
            .written(user_id, "send_invite")
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
        self.finish_dispatch(user_id, message_id, step, &outcome)
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
    ) -> Result<(), String> {
        loop {
            let Some(current) = self.store.load(user_id).await? else {
                return Err("Der gespeicherte Lounge-Versuch fehlt".into());
            };
            if !current.request.as_ref().is_some_and(|request| {
                request.message_id == message_id && request.phase == Phase::Dispatching
            }) {
                tracing::info!(
                    user_id,
                    message_id,
                    result = outcome,
                    "Invite-Lounge: Bitte wurde während des Versands geschlossen, das Ergebnis bleibt ohne Meldung"
                );
                return Ok(());
            }
            let mut finished = current.clone();
            let notice = apply_step(&mut finished, step);
            self.queue_step_notice(&mut finished, notice).await;
            finished.last_result = Some(outcome.to_string());
            let cas = self
                .store
                .compare_exchange(user_id, Some(&current), &finished)
                .await?;
            if cas != Cas::Stale {
                cas.written(user_id, "finish_dispatch");
                return Ok(());
            }
        }
    }

    async fn poll_due(
        self: &Arc<Self>,
        follow_ups: &mut tokio::task::JoinSet<()>,
    ) -> Result<(), String> {
        for user_id in self.due_users().await? {
            follow_ups.spawn(self.clone().run_follow_up(user_id));
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
    let messages = dispatcher.subscribe_messages();
    let started_at_millis = (watcher.clock_millis)();
    let (queue, inbox) = tokio::sync::mpsc::channel(LIVE_QUEUE);
    let reader = tokio::spawn(read_live(watcher.clone(), inbox));
    tokio::spawn(async move {
        watch(watcher, messages, queue, started_at_millis).await;
        reader.abort();
    })
}

async fn read_live(
    watcher: Arc<InviteLoungeWatcher>,
    mut inbox: tokio::sync::mpsc::Receiver<LoungeMessage>,
) {
    let mut follow_ups = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            message = inbox.recv() => {
                let Some(message) = message else {
                    break;
                };
                if let Turn::Stored(user_id) = watcher.serve_live(message).await {
                    follow_ups.spawn(watcher.clone().run_follow_up(user_id));
                }
            }
            Some(result) = follow_ups.join_next(), if !follow_ups.is_empty() => log_follow_up(result),
        }
    }
    while let Some(result) = follow_ups.join_next().await {
        log_follow_up(result);
    }
}

fn log_follow_up(result: Result<(), tokio::task::JoinError>) {
    if let Err(err) = result {
        tracing::error!(%err, "Invite-Lounge-Folgeschritt ist abgebrochen");
    }
}

async fn watch(
    watcher: Arc<InviteLoungeWatcher>,
    mut messages: tokio::sync::broadcast::Receiver<dl_discord::MessageEvent>,
    queue: tokio::sync::mpsc::Sender<LoungeMessage>,
    started_at_millis: i64,
) {
    let start_mark = snowflake_at(started_at_millis);
    let mut tick = tokio::time::interval(TICK);
    let mut caught_up = false;
    let mut flow_marked = false;
    let mut backoff = TICK;
    let mut next_catch_up = tokio::time::Instant::now();
    let mut catch_ups = tokio::task::JoinSet::new();
    let mut dispatches = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            event = messages.recv() => match event {
                Ok(event) => {
                    let message = LoungeMessage::from(event);
                    if !watcher.watches(&message) {
                        continue;
                    }
                    match queue.try_send(message) {
                        Ok(()) => {}
                        Err(tokio::sync::mpsc::error::TrySendError::Full(message)) => {
                            tracing::warn!(message_id = message.message_id, "Invite-Lounge-Warteschlange voll, der Rückblick holt die Nachricht nach");
                            watcher.order().hold(message.author_id);
                        }
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "Invite-Lounge hat Gateway-Nachrichten verpasst, der Rückblick holt sie nach");
                    watcher.order().hold_everyone();
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = tick.tick() => {
                let now = (watcher.clock_millis)() / 1000;
                if catch_ups.is_empty() && watcher.order().take_missed() {
                    caught_up = false;
                }
                if !caught_up && catch_ups.is_empty() && tokio::time::Instant::now() >= next_catch_up {
                    match watcher.store.mark_actions_since(started_at_millis / 1000).await {
                        Ok(()) => {
                            let reader = watcher.clone();
                            let generation = watcher.order().begin();
                            catch_ups.spawn(async move { (generation, reader.catch_up(now, start_mark).await) });
                        }
                        Err(err) => tracing::warn!(%err, "Invite-Lounge-Startzeit konnte nicht gespeichert werden, der Rückblick wartet"),
                    }
                }
                if !flow_marked {
                    match watcher.store.mark_turn_flow_since(now).await {
                        Ok(()) => flow_marked = true,
                        Err(err) => tracing::warn!(%err, "Invite-Lounge: Startzeit des Brain-Ablaufs konnte nicht gespeichert werden, Warteaufträge warten"),
                    }
                }
                if flow_marked {
                    if let Err(err) = watcher.poll_due(&mut dispatches).await {
                        tracing::warn!(%err, "Invite-Lounge-Warteaufträge konnten nicht geprüft werden");
                    }
                }
            }
            Some(result) = catch_ups.join_next(), if !catch_ups.is_empty() => {
                match result {
                    Ok((generation, Ok(failed))) => {
                        watcher.order().finish(generation, &failed);
                        if failed.is_empty() {
                            caught_up = true;
                            backoff = TICK;
                        } else {
                            backoff = (backoff * 2).min(MAX_CATCH_UP_BACKOFF);
                            next_catch_up = tokio::time::Instant::now() + backoff;
                            tracing::warn!(members = failed.len(), retry_secs = backoff.as_secs(), "Invite-Lounge-Rückblick: Mitglieder warten auf die nächste Runde");
                        }
                    }
                    Ok((_, Err(err))) => {
                        backoff = (backoff * 2).min(MAX_CATCH_UP_BACKOFF);
                        next_catch_up = tokio::time::Instant::now() + backoff;
                        tracing::warn!(%err, retry_secs = backoff.as_secs(), "Invite-Lounge-Rückblick unvollständig, neuer Versuch folgt");
                    }
                    Err(err) => {
                        backoff = (backoff * 2).min(MAX_CATCH_UP_BACKOFF);
                        next_catch_up = tokio::time::Instant::now() + backoff;
                        tracing::error!(%err, "Invite-Lounge-Rückblick ist abgebrochen");
                    }
                }
            }
            Some(result) = dispatches.join_next(), if !dispatches.is_empty() => log_follow_up(result),
        }
    }
}

fn snowflake_at(millis: i64) -> u64 {
    u64::try_from(millis.saturating_sub(DISCORD_EPOCH_MILLIS)).map_or(0, |since| since << 22)
}

fn snowflake_millis(id: u64) -> i64 {
    i64::try_from(id >> 22).map_or(i64::MAX, |since| since.saturating_add(DISCORD_EPOCH_MILLIS))
}

fn build_turn(
    event: &LoungeMessage,
    message: String,
    context: &[LoungeMessage],
    bot_id: u64,
    member: InviteMemberState,
) -> InviteTurn {
    let line = |message: &LoungeMessage| {
        turn_text(&message.content, event.author_id, bot_id).map(|text| InviteChatLine {
            speaker: speaker(message, event.author_id, bot_id),
            text,
        })
    };
    let mut earlier: Vec<&LoungeMessage> = context
        .iter()
        .filter(|message| !message.system && message.message_id < event.message_id)
        .collect();
    earlier.sort_by_key(|message| message.message_id);
    earlier.dedup_by_key(|message| message.message_id);
    let recent: Vec<InviteChatLine> = earlier.into_iter().filter_map(line).collect();
    let reply_to = event
        .reply_message_id
        .and_then(|id| {
            context
                .iter()
                .find(|message| !message.system && message.message_id == id)
        })
        .and_then(line);
    bounded_turn(&InviteTurn {
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

fn turn_text(content: &str, member_id: u64, bot_id: u64) -> Option<String> {
    let text = match MENTION_RE.as_ref() {
        Some(regex) => regex
            .replace_all(content, |caps: &Captures| {
                mention_label(&caps[1], &caps[2], member_id, bot_id)
            })
            .into_owned(),
        None => content.to_string(),
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
        has_friend_code: facts.known_code(state).is_some(),
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

fn still_fits(
    decision: &InviteTurnDecision,
    judged: &InviteTurn,
    current: &InviteMemberState,
) -> bool {
    let acts = matches!(
        decision.action,
        InviteTurnAction::WaitForCommunity
            | InviteTurnAction::AskAlreadyInvited
            | InviteTurnAction::Invite
    );
    (!acts || current.phase == judged.member.phase)
        && decision.fits(&InviteTurn {
            member: current.clone(),
            ..judged.clone()
        })
}

fn apply_turn(
    state: &mut LoungeState,
    event: &LoungeMessage,
    decision: Option<&InviteTurnDecision>,
    facts: &MemberFacts,
    now: i64,
) {
    state.last_seen = event.message_id;
    if let Some(code) = facts.message_code.clone() {
        state.friend_code = Some(code);
    }
    let Some(decision) = decision else {
        return;
    };
    let code = facts.code(Some(state));
    let invited = member_state(Some(state), facts).invited();
    let mut claim = None;
    match decision.action {
        InviteTurnAction::Silent | InviteTurnAction::Help => {}
        InviteTurnAction::AskFriendCode => ask_friend_code(state, event),
        InviteTurnAction::WaitForCommunity => wait_for_community(state, event, code),
        InviteTurnAction::AskAlreadyInvited => {
            claim = ask_already_invited(state, event, code.filter(|_| !invited), now);
        }
        InviteTurnAction::Invite => invite_now(state, event, code),
        InviteTurnAction::Close => close_request(state, event, decision.text.is_none(), invited),
    }
    if let Some(text) = decision.text.clone() {
        state.outbox.push(OutboxEntry::reply(
            event.message_id,
            event.channel_id,
            text,
            facts.invited,
            claim,
        ));
    }
}

fn open_request(state: &LoungeState) -> Option<&Request> {
    state
        .request
        .as_ref()
        .filter(|request| request.phase.open())
}

fn coded_request(state: &LoungeState) -> bool {
    open_request(state).is_some_and(|request| request.phase != Phase::WaitingForCode)
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

fn ask_friend_code(state: &mut LoungeState, event: &LoungeMessage) {
    if open_request(state).is_none() {
        start_request(state, event, Phase::WaitingForCode, false);
    }
}

fn wait_for_community(state: &mut LoungeState, event: &LoungeMessage, code: Option<String>) {
    let Some(code) = code else {
        return;
    };
    state.friend_code = Some(code);
    if !coded_request(state) {
        start_request(state, event, Phase::Pending, false);
    }
}

fn ask_already_invited(
    state: &mut LoungeState,
    event: &LoungeMessage,
    code: Option<String>,
    now: i64,
) -> Option<Claim> {
    if !coded_request(state) {
        state.friend_code = Some(code?);
        start_request(state, event, Phase::Pending, false);
    }
    let request_message_id = state.request.as_ref()?.message_id;
    let notice = state.status.claim(Some(Notice::CheckIn))?;
    state.status.check_in_at = Some(now);
    Some(Claim {
        notice,
        request_message_id,
    })
}

fn invite_now(state: &mut LoungeState, event: &LoungeMessage, code: Option<String>) {
    let Some(code) = code else {
        return;
    };
    match open_request(state).map(|request| request.phase) {
        Some(Phase::Dispatching) => {}
        Some(Phase::Pending) => {
            state.friend_code = Some(code);
            state.status.invite_now = true;
        }
        _ => {
            state.friend_code = Some(code);
            start_request(state, event, Phase::Pending, true);
        }
    }
}

fn close_request(state: &mut LoungeState, event: &LoungeMessage, notify: bool, invited: bool) {
    let Some(message_id) = open_request(state).map(|request| request.message_id) else {
        if notify {
            tracing::info!(
                user_id = event.author_id,
                message_id = event.message_id,
                "Invite-Lounge: Bestätigung ohne offene Bitte und ohne Text bleibt still"
            );
        }
        return;
    };
    state.outbox.retain(|entry| !entry.belongs_to(message_id));
    let notice = apply_step(state, Step::Finish(Some(Notice::Confirmed)));
    if notify {
        queue_notice(state, notice, invited);
    }
}

fn unseen_code(event: &LoungeMessage, context: &[LoungeMessage], last_seen: u64) -> Option<String> {
    friend_code(&event.content).or_else(|| {
        context
            .iter()
            .filter(|message| {
                message.author_id == event.author_id
                    && !message.is_bot
                    && !message.system
                    && message.message_id > last_seen
                    && message.message_id < event.message_id
            })
            .filter_map(|message| {
                friend_code(&message.content).map(|code| (message.message_id, code))
            })
            .max_by_key(|(message_id, _)| *message_id)
            .map(|(_, code)| code)
    })
}

fn unwatched(request: &Request, status: &RequestStatus) -> bool {
    !request.direct
        && !status.invite_now
        && status.wait.is_none()
        && status.channel_id != Some(CHECK_IN_CHANNEL_ID)
}

fn is_fresh(created_at: i64, now: i64, max_age: i64) -> bool {
    (-CLOCK_SKEW_SECONDS..=max_age).contains(&now.saturating_sub(created_at))
}

fn is_due(request: &Request, status: &RequestStatus) -> bool {
    request.phase == Phase::Pending
        && status.wait.is_none()
        && (request.direct || status.invite_now)
}

fn check_in_due(
    request: &Request,
    status: &RequestStatus,
    now: i64,
    skipped_by_old_flow: bool,
) -> bool {
    status.channel_id == Some(CHECK_IN_CHANNEL_ID)
        && request.phase == Phase::Pending
        && !request.direct
        && status.wait.is_none()
        && !status.invite_now
        && (status.check_in_at.is_none() || skipped_by_old_flow)
        && check_in_age(now.saturating_sub(request.created_at))
}

fn apply_step(state: &mut LoungeState, step: Step) -> Option<Notice> {
    if step == Step::Finish(Some(Notice::InvalidCode)) {
        state.friend_code = None;
    }
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
    if result != NoticeResult::Delivered {
        release_claim(state, &removed);
    }
    match result {
        NoticeResult::Delivered => {}
        NoticeResult::Retired => {
            tracing::info!(key = %removed.key, "Invite-Lounge: veralteter Ausgangseintrag ohne Zustellung entfernt");
        }
        NoticeResult::Dropped => {
            tracing::warn!(key = %removed.key, "Invite-Lounge: Meldung vom Broker abgelehnt");
        }
        NoticeResult::Retry => {
            tracing::warn!(key = %removed.key, attempts = removed.attempts, "Invite-Lounge: Meldung nach sieben Tagen ohne Zustellung aufgegeben");
        }
    }
}

fn release_claim(state: &mut LoungeState, entry: &OutboxEntry) {
    let Some(claim) = entry.held_claim() else {
        return;
    };
    if !state
        .request
        .as_ref()
        .is_some_and(|request| request.message_id == claim.request_message_id)
    {
        return;
    }
    state.status.forget(claim.notice);
    if entry.rearms_check_in() {
        state.status.check_in_at = None;
    }
}

fn steam_since(state: &LoungeState, request: &Request) -> i64 {
    state
        .dispatch_started_at_millis
        .map_or(request.created_at, |started| {
            (started / 1000).max(request.created_at)
        })
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
        .find_iter(&without_urls)
        .find_map(|found| friend_code_id(found.as_str()))
        .map(|id| id.to_string())
}

#[cfg(test)]
#[path = "invite_lounge_tests.rs"]
mod decision_tests;
