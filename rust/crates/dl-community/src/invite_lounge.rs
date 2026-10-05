//! Invite-Lounge: Hinweise, sofortige Bot-Bitten und verzögerte Raumbitten.
//! Der Versand nutzt ausschließlich den vorhandenen Slash-Event `invite`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use dl_central_db::kv;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::PgPool;

/// Stabile ID von `#frag-die-community`.
pub const INVITE_LOUNGE_CHANNEL_ID: u64 = 1_426_220_702_054_355_077;
pub const INVITE_LOUNGE_HINT_TEXT: &str = "Kleiner Tipp: pack noch deinen Steam-Freundescode dazu, sonst kann dich niemand einladen :) Du findest ihn in Steam unter Freunde → „Freund hinzufügen\". Einfach hier posten — wer Zeit hat, lädt dich ein.";

const COOLDOWN_SECONDS: i64 = 24 * 60 * 60;
const COOLDOWN_KV_NS: &str = "invite_lounge:cooldown";
const STATE_KV_NS: &str = "invite_lounge:requests";
const BACKFILL_KV_NS: &str = "invite_lounge:history";
const ROOM_WAIT_SECONDS: i64 = 60 * 60;
pub const NEWCOMER_MAX_JOIN_SECONDS: i64 = 7 * 24 * 60 * 60;

static FRIEND_CODE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\b\d{6,12}\b").ok());
static STEAM_LINK_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:https?://)?(?:www\.)?(?:s\.team/p/|steamcommunity\.com)").ok()
});
static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:https?://|www\.|steamcommunity\.com/|s\.team/)\S+")
        .expect("gültiger URL-Ausdruck")
});
static INVITE_TERM_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"invite|playtest|beta|\bein\w{0,3}lad|\blad(?:e|et|t)?\s+(?:\w+\s+){0,3}\bein\b")
        .ok()
});
static QUESTION_SIGNAL_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?iu)\b(?:wer|mag|kann|könnte|koennte|jemand|würde|wuerde|hätte|haette)\b").ok()
});

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

// async_trait markiert Futures zusätzlich zu dem bereits markierten Result.
#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
pub trait InviteLoungeReplyPort: Send + Sync {
    fn bot_id(&self) -> Option<u64>;
    async fn history(&self, before_id: Option<u64>) -> Result<Vec<LoungeMessage>, String>;
    async fn reply_text(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<u64, String>;
}

#[async_trait::async_trait]
impl InviteLoungeReplyPort for dl_discord::DiscordAdapter {
    fn bot_id(&self) -> Option<u64> {
        self.bot_user_id_cell().get().copied().filter(|id| *id > 0)
    }

    async fn history(&self, before_id: Option<u64>) -> Result<Vec<LoungeMessage>, String> {
        Ok(self
            .channel_message_history(INVITE_LOUNGE_CHANNEL_ID, before_id)
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

    async fn reply_text(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<u64, String> {
        let mut body = Map::new();
        body.insert("content".into(), Value::String(content.to_string()));
        body.insert(
            "message_reference".into(),
            json!({
                "channel_id": channel_id.to_string(), "message_id": message_id.to_string(),
                "fail_if_not_exists": false,
            }),
        );
        body.insert(
            "allowed_mentions".into(),
            json!({ "parse": [], "replied_user": false }),
        );
        self.send_raw_public(channel_id, &body).await
    }
}

#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
pub trait InviteEventPort: Send + Sync {
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
    async fn known_code(&self, user_id: u64) -> Result<Option<String>, String>;
    async fn last_hint_at(&self, user_id: u64) -> Result<Option<i64>, String>;
    async fn backfill_done(&self) -> Result<bool, String>;
    async fn mark_backfill_done(&self) -> Result<(), String>;
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
        keys.into_iter()
            .map(|key| {
                key.strip_prefix("user:")
                    .and_then(|id| id.parse().ok())
                    .ok_or_else(|| "Ungültiger Nutzer-Schlüssel im Lounge-Zustand".to_string())
            })
            .collect()
    }

    async fn known_code(&self, user_id: u64) -> Result<Option<String>, String> {
        let steam_id: Option<String> = sqlx::query_scalar("SELECT steam_id FROM core.steam_links WHERE discord_id = $1 ORDER BY primary_account DESC, verified DESC, steam_id LIMIT 1")
            .bind(user_id as i64).fetch_optional(&self.pool).await.map_err(|err| err.to_string())?;
        Ok(steam_id
            .and_then(|id| id.parse::<u64>().ok())
            .and_then(|id| id.checked_sub(76_561_197_960_265_728))
            .filter(|id| *id > 0 && *id <= u32::MAX as u64)
            .map(|id| id.to_string()))
    }

    async fn last_hint_at(&self, user_id: u64) -> Result<Option<i64>, String> {
        Ok(kv::get(&self.pool, COOLDOWN_KV_NS, &cooldown_key(user_id))
            .await
            .map_err(|err| err.to_string())?
            .and_then(|value| value.parse().ok()))
    }

    async fn backfill_done(&self) -> Result<bool, String> {
        Ok(kv::get(&self.pool, BACKFILL_KV_NS, "seven_days_v1")
            .await
            .map_err(|err| err.to_string())?
            .is_some())
    }

    async fn mark_backfill_done(&self) -> Result<(), String> {
        kv::set(
            &self.pool,
            BACKFILL_KV_NS,
            "seven_days_v1",
            &chrono::Utc::now().timestamp().to_string(),
        )
        .await
        .map_err(|err| err.to_string())
    }
}

pub struct InviteLoungeWatcher {
    store: Arc<dyn LoungeStore>,
    port: Arc<dyn InviteLoungeReplyPort>,
    invite: Arc<dyn InviteEventPort>,
    guild_id: u64,
    clock_millis: fn() -> i64,
}

impl InviteLoungeWatcher {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn InviteLoungeReplyPort>,
        invite: Arc<dl_bridges::steam::SteamBotClient>,
        guild_id: u64,
    ) -> Self {
        Self {
            store: Arc::new(PgLoungeStore { pool }),
            port,
            invite,
            guild_id,
            clock_millis: || chrono::Utc::now().timestamp_millis(),
        }
    }

    #[cfg(test)]
    async fn handle_message(
        &self,
        event: LoungeMessage,
        now: i64,
        existing_hint: Option<bool>,
    ) -> Result<(), String> {
        if let Some(user_id) = self.record_message(event, now, existing_hint).await? {
            self.dispatch_due(user_id, now).await?;
        }
        Ok(())
    }

    async fn record_message(
        &self,
        event: LoungeMessage,
        now: i64,
        existing_hint: Option<bool>,
    ) -> Result<Option<u64>, String> {
        if event.is_bot
            || event.channel_id != INVITE_LOUNGE_CHANNEL_ID
            || event.guild_id != Some(self.guild_id)
        {
            return Ok(None);
        }
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        let kind = request_kind(&event.content, bot_id);
        if kind == Some(RequestKind::Information) {
            return Ok(None);
        }
        let previous = self.store.load(event.author_id).await?;
        let mut next = previous.clone().unwrap_or_default();
        if event.message_id <= next.last_seen {
            return Ok(None);
        }
        let code = friend_code(&event.content);
        if is_offer(&event.content) || (kind.is_none() && code.is_none()) {
            return Ok(None);
        }
        if previous.is_none() {
            next.last_hint_at = self.store.last_hint_at(event.author_id).await?;
        }
        if code.is_none() && next.friend_code.is_none() && kind == Some(RequestKind::Direct) {
            next.friend_code = self.store.known_code(event.author_id).await?;
        }
        let mut wants_hint = update_state(
            &mut next,
            &event,
            kind,
            code,
            existing_hint.unwrap_or(false),
            now,
            (self.clock_millis)(),
        );
        if wants_hint && existing_hint.is_none() && self.has_hint(event.message_id, bot_id).await? {
            wants_hint = false;
        }
        if !self
            .store
            .compare_exchange(event.author_id, previous.as_ref(), &next)
            .await?
        {
            return Ok(None);
        }
        if wants_hint {
            // Die Reservierung steht vor dem Discord-Aufruf. Auch ein unklarer
            // HTTP-Ausgang darf keinen zweiten Hinweis unter derselben Bitte erzeugen.
            self.port
                .reply_text(event.channel_id, event.message_id, INVITE_LOUNGE_HINT_TEXT)
                .await?;
        }
        Ok(Some(event.author_id))
    }

    async fn has_hint(&self, message_id: u64, bot_id: u64) -> Result<bool, String> {
        let mut before = None;
        loop {
            let page = self.port.history(before).await?;
            if page.is_empty() {
                return Ok(false);
            }
            if hint_references(&page, bot_id).contains(&message_id) {
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

    async fn dispatch_due(&self, user_id: u64, now: i64) -> Result<(), String> {
        let Some(previous) = self.store.load(user_id).await? else {
            return Ok(());
        };
        let Some(request) = previous
            .request
            .as_ref()
            .filter(|request| is_due(request, now))
        else {
            return Ok(());
        };
        let Some(code) = previous.friend_code.as_deref() else {
            return Ok(());
        };
        let Some(bot_id) = self.port.bot_id() else {
            return Err("Die Discord-ID des Bots fehlt".into());
        };
        let message_id = request.message_id;
        let mut claimed = previous.clone();
        if let Some(request) = claimed.request.as_mut() {
            request.phase = Phase::Dispatching;
        }
        claimed.dispatch_started_at_millis = Some((self.clock_millis)());
        claimed.last_result = Some("Versuch begonnen; Ausgang noch unbekannt".into());
        if !self
            .store
            .compare_exchange(user_id, Some(&previous), &claimed)
            .await?
        {
            return Ok(());
        }
        // Ausschließlich dieser Event ruft handle_invite_command auf. Dort
        // entscheidet betainvite_lookup_audit_by_steam und der Dispatch-Claim.
        let result = self
            .invite
            .invite(bot_id, self.guild_id, code, user_id)
            .await;
        let outcome = match &result {
            Ok(text) => text.clone(),
            Err(err) => format!("Fehlgeschlagen: {err}"),
        };
        let finished_at = (self.clock_millis)();
        self.finish_attempt(user_id, message_id, &outcome, finished_at)
            .await?;
        let text = match result {
            Ok(text) => lounge_response(&text),
            Err(err) => {
                tracing::warn!(%err, user_id, "Invite-Lounge-Versuch fehlgeschlagen");
                "Die Einladung hat gerade nicht geklappt. Bitte frag mich erneut, wenn ich es noch einmal versuchen soll.".into()
            }
        };
        self.port
            .reply_text(INVITE_LOUNGE_CHANNEL_ID, message_id, &text)
            .await?;
        Ok(())
    }

    async fn finish_attempt(
        &self,
        user_id: u64,
        message_id: u64,
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
            if let Some(request) = finished.request.as_mut() {
                request.phase = Phase::Attempted;
            }
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
        for user_id in self.store.pending_users().await? {
            if let Err(err) = self.dispatch_due(user_id, now).await {
                tracing::warn!(%err, user_id, "Invite-Lounge-Auftrag konnte nicht abgeschlossen werden");
            }
        }
        Ok(())
    }

    #[cfg(test)]
    async fn backfill(&self, now: i64) -> Result<(), String> {
        self.record_history(now).await?;
        self.poll_due(now).await
    }

    async fn record_history(&self, now: i64) -> Result<(), String> {
        if self.store.backfill_done().await? {
            return Ok(());
        }
        if self.port.bot_id().is_none() {
            return Err("Die Discord-ID des Bots fehlt".into());
        }
        let since = now - NEWCOMER_MAX_JOIN_SECONDS;
        let mut before = None;
        let mut history = Vec::new();
        loop {
            let page = self.port.history(before).await?;
            if page.is_empty() {
                break;
            }
            let oldest_id = page
                .iter()
                .map(|message| message.message_id)
                .min()
                .ok_or("Leere Verlaufsseite")?;
            if before.is_some_and(|id| oldest_id >= id) {
                return Err("Der Verlauf liefert keine ältere Seite".into());
            }
            let reached_start = page.iter().any(|message| message.created_at < since);
            history.extend(
                page.into_iter()
                    .filter(|message| message.created_at >= since),
            );
            if reached_start {
                break;
            }
            before = Some(oldest_id);
        }
        let hints = hint_references(
            &history,
            self.port.bot_id().ok_or("Die Discord-ID des Bots fehlt")?,
        );
        history.sort_by_key(|message| message.message_id);
        for mut message in history {
            // Discord liefert guild_id bei REST-Nachrichten nicht immer mit.
            if message.guild_id.is_none() {
                message.guild_id = Some(self.guild_id);
            }
            let hinted = hints.contains(&message.message_id);
            self.record_message(message, now, Some(hinted)).await?;
        }
        self.store.mark_backfill_done().await
    }
}

pub fn spawn(
    pool: PgPool,
    port: Arc<dyn InviteLoungeReplyPort>,
    invite: Arc<dl_bridges::steam::SteamBotClient>,
    guild_id: u64,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let watcher = Arc::new(InviteLoungeWatcher::new(pool, port, invite, guild_id));
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        let mut history_ready = false;
        let mut buffered = Vec::new();
        let mut dispatches = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                event = messages.recv() => match event {
                    Ok(event) => {
                        let message = LoungeMessage::from(event);
                        if message.channel_id != INVITE_LOUNGE_CHANNEL_ID { continue; }
                        if !history_ready { buffered.push(message); continue; }
                        match watcher.record_message(message, chrono::Utc::now().timestamp(), None).await {
                            Ok(Some(user_id)) => spawn_dispatch(&mut dispatches, watcher.clone(), user_id),
                            Ok(None) => {},
                            Err(err) => tracing::warn!(%err, "Invite-Lounge-Nachricht konnte nicht verarbeitet werden"),
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "Invite-Lounge hat Gateway-Nachrichten verpasst");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    let now = chrono::Utc::now().timestamp();
                    if !history_ready {
                        if let Err(err) = watcher.record_history(now).await {
                            tracing::warn!(%err, "Invite-Lounge-Verlauf konnte nicht verarbeitet werden");
                            continue;
                        }
                        history_ready = true;
                        buffered.sort_by_key(|message| message.message_id);
                        for message in buffered.drain(..) {
                            if let Err(err) = watcher.record_message(message, now, None).await {
                                tracing::warn!(%err, "Invite-Lounge-Nachricht konnte nicht verarbeitet werden");
                            }
                        }
                    }
                    match watcher.store.pending_users().await {
                        Ok(users) => for user_id in users { spawn_dispatch(&mut dispatches, watcher.clone(), user_id); },
                        Err(err) => tracing::warn!(%err, "Invite-Lounge-Warteaufträge konnten nicht geprüft werden"),
                    }
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
        if let Err(err) = watcher.dispatch_due(user_id, chrono::Utc::now().timestamp()).await {
            tracing::warn!(%err, user_id, "Invite-Lounge-Auftrag konnte nicht abgeschlossen werden");
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Direct,
    Room,
    Information,
}

fn update_state(
    state: &mut LoungeState,
    event: &LoungeMessage,
    kind: Option<RequestKind>,
    code: Option<String>,
    existing_hint: bool,
    now: i64,
    now_millis: i64,
) -> bool {
    if kind == Some(RequestKind::Information) {
        return false;
    }
    state.last_seen = event.message_id;
    let has_message_code = code.is_some();
    let code_changed = code
        .as_ref()
        .is_some_and(|code| state.friend_code.as_ref() != Some(code));
    if let Some(code) = code {
        state.friend_code = Some(code);
    }
    if let Some(kind) = kind {
        let direct = kind == RequestKind::Direct;
        let arrived_during_dispatch = state
            .dispatch_started_at_millis
            .is_some_and(|started| event.created_at_millis >= started)
            && state
                .last_dispatch_finished_at_millis
                .is_some_and(|finished| event.created_at_millis <= finished);
        let replace = state.request.as_ref().is_none_or(|request| {
            let active_dispatch = request.phase == Phase::Dispatching
                && state
                    .dispatch_started_at_millis
                    .is_some_and(|started| now_millis.saturating_sub(started) < 120_000);
            !active_dispatch
                && !arrived_during_dispatch
                && (direct || request.phase == Phase::WaitingForCode)
        });
        if replace {
            let continuing_direct = state
                .request
                .as_ref()
                .is_some_and(|request| request.direct && request.phase == Phase::WaitingForCode);
            state.request = Some(Request {
                message_id: event.message_id,
                created_at: event.created_at,
                direct: direct || continuing_direct,
                phase: if has_message_code
                    || ((direct || continuing_direct) && state.friend_code.is_some())
                {
                    Phase::Pending
                } else {
                    Phase::WaitingForCode
                },
            });
            // Eine wiederholte Bitte ohne Code erhält keinen zweiten Hinweis.
            if !continuing_direct && state.friend_code.is_some() {
                state.hint_sent = false;
            }
        }
    } else if let Some(request) = state
        .request
        .as_mut()
        .filter(|request| request.phase == Phase::WaitingForCode)
    {
        if state.friend_code.is_some() {
            request.phase = Phase::Pending;
            request.message_id = event.message_id;
            request.created_at = event.created_at;
        }
    }
    if code_changed {
        if let Some(request) = state
            .request
            .as_mut()
            .filter(|request| request.phase == Phase::Pending && !request.direct)
        {
            request.message_id = event.message_id;
            request.created_at = event.created_at;
        }
    }
    if existing_hint {
        state.hint_sent = true;
    }
    let needs_hint = state
        .request
        .as_ref()
        .is_some_and(|request| request.phase == Phase::WaitingForCode)
        && !state.hint_sent
        && state
            .last_hint_at
            .is_none_or(|sent| now.saturating_sub(sent) >= COOLDOWN_SECONDS);
    if needs_hint {
        state.hint_sent = true;
        state.last_hint_at = Some(now);
    }
    needs_hint
}

fn is_due(request: &Request, now: i64) -> bool {
    request.phase == Phase::Pending
        && (request.direct || now.saturating_sub(request.created_at) >= ROOM_WAIT_SECONDS)
}

pub fn friend_code(content: &str) -> Option<String> {
    let without_urls = URL_RE.replace_all(content, " ");
    FRIEND_CODE_RE
        .as_ref()?
        .find(&without_urls)
        .map(|found| found.as_str().to_string())
}

pub fn is_offer(content: &str) -> bool {
    let folded = fold_german_umlauts(&content.to_lowercase());
    let words: Vec<&str> = folded
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.contains(&"ich")
        && words
            .iter()
            .any(|word| matches!(*word, "dich" | "euch" | "dir"))
        && (words.iter().any(|word| {
            word.starts_with("lad") || word.starts_with("einlad") || word.starts_with("invit")
        }))
}

pub fn request_kind(content: &str, bot_id: u64) -> Option<RequestKind> {
    if is_offer(content) {
        return None;
    }
    let bot_mention =
        content.contains(&format!("<@{bot_id}>")) || content.contains(&format!("<@!{bot_id}>"));
    let mut kind = None;
    let mut sentence_start = 0;
    for (position, punctuation) in content
        .char_indices()
        .filter(|(_, ch)| matches!(ch, '.' | '!' | '?' | ';'))
        .chain(std::iter::once((content.len(), '\0')))
    {
        // Das Ausrufezeichen einer Nickname-Mention ist keine Satzgrenze.
        if punctuation == '!'
            && content[..position].ends_with("<@")
            && content[position + 1..]
                .split_once('>')
                .is_some_and(|(id, _)| {
                    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
                })
        {
            continue;
        }
        if punctuation == '.' {
            let word = content[sentence_start..position]
                .split_whitespace()
                .next_back()
                .unwrap_or_default();
            // Punkte in Abkürzungen wie „z.B.“ oder „z. B.“ beenden den Satz nicht.
            let abbreviation = word.chars().all(|ch| ch.is_alphabetic() || ch == '.')
                && (word.contains('.') || word.chars().count() == 1);
            let inside_word = word.chars().next_back().is_some_and(char::is_alphabetic)
                && content[position + 1..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphabetic);
            if abbreviation || inside_word {
                continue;
            }
        }
        let sentence_end = position + usize::from(punctuation != '\0');
        let sentence = &content[sentence_start..sentence_end];
        sentence_start = sentence_end;
        let mut sentence_kind = sentence_request_kind(sentence, bot_mention);
        if sentence_kind == Some(RequestKind::Information) {
            let folded = fold_german_umlauts(&sentence.to_lowercase());
            // Nach „und“ kann eine eigene Bitte folgen. Indirekte Nebensätze
            // bleiben bei der Informationsfrage des gesamten Satzes.
            for clause in folded.split(" und ").skip(1) {
                let words: Vec<&str> = clause
                    .split(|ch: char| !ch.is_alphanumeric())
                    .filter(|word| !word.is_empty())
                    .take(3)
                    .collect();
                let opening = words.strip_prefix(&["bitte"]).unwrap_or(&words);
                let own_request = matches!(
                    opening,
                    [verb, "du" | "mich" | "mcih" | "mir" | "uns" | "me", ..]
                        if is_direct_request_verb(verb)
                ) || matches!(
                    opening,
                    [
                        "lad" | "lade" | "ladet" | "ladt",
                        "mich" | "mcih" | "uns" | "me",
                        ..
                    ]
                );
                if own_request
                    && sentence_request_kind(clause, bot_mention) == Some(RequestKind::Direct)
                {
                    sentence_kind = Some(RequestKind::Direct);
                    break;
                }
            }
        }
        match sentence_kind {
            Some(RequestKind::Direct) => return sentence_kind,
            Some(RequestKind::Room) => kind = sentence_kind,
            Some(RequestKind::Information) if kind.is_none() => kind = sentence_kind,
            _ => {}
        }
    }
    kind
}

fn sentence_request_kind(content: &str, bot_mention: bool) -> Option<RequestKind> {
    let folded = fold_german_umlauts(&content.to_lowercase());
    let words: Vec<&str> = folded
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    // Das angefragte Verb entscheidet. Substantive wie „Erklärung“ sind keine Bitte.
    let explanation_verb = words.iter().position(|word| {
        ["erklar", "erklaer", "erlauter", "erlaeuter", "informier"]
            .iter()
            .any(|stem| {
                word.strip_prefix(stem)
                    .is_some_and(|ending| matches!(ending, "" | "e" | "en" | "n" | "st" | "t"))
            })
    });
    let explanation_noun = words.iter().any(|word| {
        matches!(
            *word,
            "erklarung"
                | "erklarungen"
                | "erklaerung"
                | "erklaerungen"
                | "erlauterung"
                | "erlauterungen"
                | "erlaeuterung"
                | "erlaeuterungen"
        )
    });
    let requested_explanation = explanation_noun
        .then(|| {
            words
                .iter()
                .position(|word| matches!(*word, "gib" | "gibt" | "geben" | "gebt" | "gebe"))
        })
        .flatten();
    let explanation = explanation_verb
        .into_iter()
        .chain(requested_explanation)
        .min();
    let information = words.iter().enumerate().find_map(|(index, word)| {
        (matches!(*word, "sag" | "sage" | "sagen" | "sagst" | "sagt")
            && words[index + 1..].iter().any(|word| {
                matches!(
                    *word,
                    "warum" | "wie" | "wieso" | "weshalb" | "ob" | "wann" | "wo" | "was" | "wer"
                )
            }))
        .then_some(index)
    });
    // Der Frageanfang endet am ersten Komma. „Wie besprochen, du ...“ enthält
    // dadurch keine Prozessfrage. Mention-IDs gehören nicht zum Frageanfang.
    let opening_words: Vec<&str> = folded
        .split(',')
        .next()
        .unwrap_or_default()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let question_start = opening_words
        .iter()
        .position(|word| !word.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(opening_words.len());
    let question_opening = &opening_words[question_start..];
    let process_question = matches!(
        question_opening,
        ["warum" | "wieso" | "weshalb", ..]
            | [
                "kann"
                    | "konnen"
                    | "koennen"
                    | "konnte"
                    | "koennte"
                    | "konnten"
                    | "koennten"
                    | "muss"
                    | "mussen"
                    | "muessen"
                    | "musste"
                    | "mussten"
                    | "darf"
                    | "durfen"
                    | "duerfen"
                    | "durfte"
                    | "durften"
                    | "soll"
                    | "sollen"
                    | "sollte"
                    | "sollten",
                "ich" | "wir" | "man",
                ..
            ]
    ) || (matches!(question_opening, ["wie" | "wo" | "wann" | "was", ..])
        // Kurze Einleitungen wie „Wie besprochen,“ und Vorschläge bleiben Bitten.
        && !matches!(question_opening, ["wie", _] if folded.contains(','))
        && !matches!(question_opening, ["wie", "ware", "es", ..]));
    let invite_positions: Vec<usize> = INVITE_TERM_RE
        .as_ref()
        .into_iter()
        .flat_map(|regex| regex.find_iter(&folded))
        .map(|found| {
            let before = &folded[..found.start()];
            let starts_inside_word = before
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
            before
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .count()
                .saturating_sub(usize::from(starts_inside_word))
        })
        .chain(words.iter().enumerate().filter_map(|(position, word)| {
            (close_word(word, "einladen", 2) || close_word(word, "inviten", 1)).then_some(position)
        }))
        .collect();
    let invite_action_position = invite_positions
        .iter()
        .copied()
        .filter(|position| {
            let word = words[*position];
            let action_word = matches!(word, "lad" | "lade" | "ladet" | "ladt")
                || (!word.starts_with("einladung")
                    && !word.ends_with("invite")
                    && !word.ends_with("invites")
                    && (close_word(word, "einladen", 2) || close_word(word, "inviten", 1)));
            action_word
                && !position.checked_sub(1).is_some_and(|previous| {
                    matches!(words[previous], "das" | "beim" | "zum" | "vom" | "durchs")
                })
        })
        .min();
    let self_request_position = words.iter().position(|word| {
        matches!(*word, "mich" | "mcih" | "mir" | "uns" | "me")
            || (word.starts_with('m') && word.len() >= 3 && close_word(word, "mich", 1))
    });
    // Eine anschließende Auskunft hebt eine bereits formulierte Bitte nicht auf.
    if process_question
        || explanation.into_iter().chain(information).any(|position| {
            invite_action_position.is_none_or(|invite| position < invite)
                || self_request_position.is_none_or(|request| position < request)
        })
    {
        return Some(RequestKind::Information);
    }
    if invite_positions.is_empty() || self_request_position.is_none() {
        return None;
    }
    let room_address = words.iter().any(|word| {
        matches!(
            *word,
            "jemand" | "wer" | "irgendwer" | "irgendjemand" | "einer" | "jmd"
        )
    });
    let direct_verb = words.iter().any(|word| is_direct_request_verb(word));
    if bot_mention || (!room_address && (direct_verb || words.contains(&"du"))) {
        Some(RequestKind::Direct)
    } else if has_question_signal(&content.to_lowercase()) || room_address {
        Some(RequestKind::Room)
    } else {
        None
    }
}

fn is_direct_request_verb(word: &str) -> bool {
    ["kannst", "kannste", "konntest", "wurdest", "magst"]
        .iter()
        .any(|candidate| close_word(word, candidate, 1))
}

/// Kleine Tippfehler in Anrede und Einladungsverb werden ohne KI erkannt.
fn close_word(word: &str, expected: &str, max_edits: usize) -> bool {
    if word.len().abs_diff(expected.len()) > max_edits {
        return false;
    }
    let expected: Vec<char> = expected.chars().collect();
    let mut row: Vec<usize> = (0..=expected.len()).collect();
    for (index, ch) in word.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = index + 1;
        for (column, target) in expected.iter().enumerate() {
            let old = row[column + 1];
            row[column + 1] = (row[column] + 1)
                .min(old + 1)
                .min(diagonal + usize::from(ch != *target));
            diagonal = old;
        }
    }
    row[expected.len()] <= max_edits
}

fn hint_references(history: &[LoungeMessage], bot_id: u64) -> HashSet<u64> {
    history
        .iter()
        .filter(|message| message.author_id == bot_id && message.content == INVITE_LOUNGE_HINT_TEXT)
        .filter_map(|message| message.reply_message_id)
        .collect()
}

pub fn lounge_response(handler_text: &str) -> String {
    if handler_text.starts_with("🎮 Ihr seid schon Steam-Freunde,") {
        "Die Steam-Einladung ist raus.".into()
    } else if handler_text.starts_with("📨 Freundschaftsanfrage ist raus.") {
        "Die Steam-Freundschaftsanfrage ist raus. Nimm sie an, dann folgt die Einladung.".into()
    } else {
        // Insbesondere Audit- und Dispatch-Antworten kommen unverändert vom Handler.
        handler_text.to_string()
    }
}

// Bisherige Hinweisentscheidung für bestehende Aufrufer und Regressionstests.
// Versand und Zahlencode-Nachfrage nutzen die Regeln oben.
pub fn should_reply_to_content(content: &str) -> bool {
    is_invite_request(content) && !has_friend_code(content)
}
pub fn should_reply(content: &str, author_joined_at: Option<i64>, now: i64) -> bool {
    is_newcomer(author_joined_at, now) && should_reply_to_content(content)
}
fn is_newcomer(joined_at: Option<i64>, now: i64) -> bool {
    joined_at.is_some_and(|joined| now.saturating_sub(joined) < NEWCOMER_MAX_JOIN_SECONDS)
}
fn is_invite_request(content: &str) -> bool {
    let lower = content.to_lowercase();
    let folded = fold_german_umlauts(&lower);
    INVITE_TERM_RE
        .as_ref()
        .is_some_and(|regex| regex.is_match(&folded))
        && has_question_signal(&lower)
}
fn has_question_signal(lower_content: &str) -> bool {
    lower_content.contains('?')
        || QUESTION_SIGNAL_RE
            .as_ref()
            .is_some_and(|regex| regex.is_match(lower_content))
}
fn has_friend_code(content: &str) -> bool {
    FRIEND_CODE_RE
        .as_ref()
        .is_some_and(|regex| regex.is_match(content))
        || STEAM_LINK_RE
            .as_ref()
            .is_some_and(|regex| regex.is_match(content))
}
fn fold_german_umlauts(value: &str) -> String {
    value
        .replace('ä', "a")
        .replace('ö', "o")
        .replace('ü', "u")
        .replace('ß', "ss")
}
fn cooldown_key(user_id: u64) -> String {
    format!("user:{user_id}")
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nette_frage_ohne_code_triggert() {
        assert!(should_reply_to_content(
            "Hey, kann mich jemand bitte zu Deadlock inviten?"
        ));
    }

    #[test]
    fn frage_mit_neunstelligem_code_triggert_nicht() {
        assert!(!should_reply_to_content(
            "Mag mich jemand zum Playtest einladen? Mein Code ist 123456789"
        ));
    }

    #[test]
    fn trennbares_verb_einzuladen_triggert() {
        assert!(should_reply_to_content(
            "Hallöchen zusammen, bin neu hier und suche zum einen jemand der so freundlich wäre mich in Deadlock einzuladen und zum anderen bissl Anschluss zum Game"
        ));
    }

    #[test]
    fn getrenntes_laedt_mich_ein_triggert() {
        assert!(should_reply_to_content("Wer lädt mich ein?"));
        assert!(should_reply_to_content("Kann mich jemand mal einladen"));
    }

    #[test]
    fn antwort_ohne_fragesignal_triggert_nicht() {
        assert!(!should_reply_to_content("ich lad dich ein"));
    }

    #[test]
    fn steam_link_zaehlt_als_code() {
        assert!(!should_reply_to_content(
            "Kann mich jemand inviten https://steamcommunity.com/id/example"
        ));
        assert!(!should_reply_to_content(
            "Wer hat einen Invite fuer mich? s.team/p/abc-def"
        ));
    }

    #[test]
    fn veteran_mit_invite_frage_triggert_nicht() {
        let now = 1_700_000_000;
        let content = "Hast du schon eine Einladung bekommen? Ich kann dich gerne im Laufe des Tages einladen";

        assert!(!should_reply(content, Some(now - 400 * 24 * 60 * 60), now));
    }

    #[test]
    fn neuling_mit_invite_frage_triggert() {
        let now = 1_700_000_000;
        let content = "Hast du schon eine Einladung bekommen? Ich kann dich gerne im Laufe des Tages einladen";

        assert!(should_reply(content, Some(now - 2 * 24 * 60 * 60), now));
    }

    #[test]
    fn unbekanntes_beitrittsdatum_triggert_nicht() {
        let now = 1_700_000_000;

        assert!(!should_reply(
            "Hey, kann mich jemand bitte zu Deadlock inviten?",
            None,
            now,
        ));
    }

    #[test]
    fn grenzfall_genau_sieben_tage_triggert_nicht() {
        let now = 1_700_000_000;

        assert!(!should_reply(
            "Hey, kann mich jemand bitte zu Deadlock inviten?",
            Some(now - NEWCOMER_MAX_JOIN_SECONDS),
            now,
        ));
    }

    #[test]
    fn neuling_mit_freundescode_triggert_nicht() {
        let now = 1_700_000_000;

        assert!(!should_reply(
            "Kann mich jemand inviten? Mein Code ist 123456789",
            Some(now - 60),
            now,
        ));
    }
}

#[cfg(test)]
#[path = "invite_lounge_tests.rs"]
mod decision_tests;
