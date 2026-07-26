use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use dl_central_db::kv;
use dl_discord::{
    BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler, InteractionRouter, ModalField,
    ModalSpec, VoiceEvent,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

pub const MAIN_GUILD_ID: u64 = 1289721245281292288;
pub const NEW_PLAYER_CATEGORY_ID: u64 = 1465839366634209361;
pub const LFG_CHANNEL_ID: u64 = 1376335502919335936;
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const SOLO_DELAY: Duration = Duration::seconds(120);
pub const TICK_INTERVAL: StdDuration = StdDuration::from_secs(5);
pub const NEVER_ASK_NS: &str = "solo_lfg_never_ask";
pub const LAST_PROMPT_NS: &str = "solo_lfg_last_prompt";
pub const ACTIVE_POST_NS: &str = "solo_lfg_active_post";
// ponytail: bewusste Obergrenze gegen verpasste Voice-Events; erst bei echtem Bedarf konfigurierbar machen.
pub const MAX_POST_AGE: Duration = Duration::hours(3);
pub const ENTER_CUSTOM_ID: &str = "solo_lfg:enter";
pub const LATER_CUSTOM_ID: &str = "solo_lfg:later";
pub const NEVER_CUSTOM_ID: &str = "solo_lfg:never";
pub const MODAL_CUSTOM_ID: &str = "solo_lfg:modal";

pub const DM_TITLE: &str = "Du sitzt gerade allein";
pub const MODAL_TITLE: &str = "Eintrag in die Mitspieler-Suche";
pub const CONFIRMATION_REPLY: &str =
    "Steht drin. Wenn jemand joint, lösch ich den Post wieder — du musst dich um nichts kümmern.";
pub const LATER_REPLY: &str =
    "Alles gut. Wenn du magst, klick einfach nochmal, wenn du länger sitzt.";
pub const NEVER_REPLY: &str = "Erledigt, ich frag dich nicht mehr. Eintragen kannst du dich trotzdem jederzeit selbst in <#1376335502919335936>.";
pub const INVALID_REPLY: &str =
    "Das ist schon abgelaufen. Wenn du noch allein sitzt, meld ich mich gleich nochmal.";
pub const POST_ERROR_REPLY: &str = "Da ist beim Posten was schiefgelaufen, tut mir leid. Probier es gleich nochmal, oder schreib direkt in <#1376335502919335936>.";
pub const DAILY_SUMMARY_PREFIX: &str = "🎙️ Solo-LFG — Tagesbilanz";

pub const HARD_EXCLUDED_CATEGORY_IDS: [u64; 8] = [
    1326983313549820035,
    1318329776695676960,
    1304416153728450600,
    1459526231686119600,
    1523225910034305074,
    1412800850580996256,
    1407787097217040547,
    1522391122863849554,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSnapshot {
    pub category_id: u64,
    pub name: String,
    pub non_bot_members: Vec<u64>,
    pub user_limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDecision {
    Prompt,
    TooManySessions(usize),
    OptedOut,
    NeverAsk,
    Cooldown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPost {
    pub user_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub created_at: i64,
}

#[async_trait::async_trait]
pub trait SoloWatchPort: Send + Sync {
    async fn lane_snapshot(&self, guild_id: u64, channel_id: u64) -> Option<LaneSnapshot>;
    async fn claim_prompt(
        &self,
        guild_id: u64,
        user_id: u64,
        now: DateTime<Utc>,
    ) -> Result<PromptDecision, String>;
    async fn verified_rank(&self, user_id: u64) -> Result<Option<String>, String>;
    async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String>;
    async fn set_never_ask(&self, user_id: u64) -> Result<(), String>;
    async fn post_lfg(&self, channel_id: u64, content: String) -> Result<u64, String>;
    async fn load_active_posts(&self) -> Result<Vec<PersistedPost>, String>;
    async fn save_active_post(&self, post: &PersistedPost) -> Result<(), String>;
    async fn remove_active_post(&self, user_id: u64) -> Result<(), String>;
    async fn delete_post(
        &self,
        channel_id: u64,
        message_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    async fn send_log(&self, text: String) -> Result<(), String>;
    fn log_decision(
        &self,
        user_id: u64,
        decision: &'static str,
        reason: &'static str,
        error: Option<&str>,
    );
}

#[derive(Debug, Clone)]
struct PendingSolo {
    guild_id: u64,
    channel_id: u64,
    since: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ActivePost {
    channel_id: u64,
    message_id: u64,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct DailyCounters {
    checked: u64,
    prompted: u64,
    registered: u64,
    rejected: u64,
    dm_errors: u64,
}

pub struct SoloWatch {
    port: Arc<dyn SoloWatchPort>,
    pending: tokio::sync::Mutex<HashMap<u64, PendingSolo>>,
    posts: tokio::sync::Mutex<HashMap<u64, ActivePost>>,
    daily: tokio::sync::Mutex<DailyCounters>,
}

impl SoloWatch {
    pub fn new(port: Arc<dyn SoloWatchPort>) -> Arc<Self> {
        Arc::new(Self {
            port,
            pending: tokio::sync::Mutex::new(HashMap::new()),
            posts: tokio::sync::Mutex::new(HashMap::new()),
            daily: tokio::sync::Mutex::new(DailyCounters::default()),
        })
    }

    fn decision(
        &self,
        user_id: u64,
        decision: &'static str,
        reason: &'static str,
        error: Option<&str>,
    ) {
        self.port.log_decision(user_id, decision, reason, error);
    }

    async fn count_checked(&self) {
        self.daily.lock().await.checked += 1;
    }

    pub async fn handle_event_at(self: &Arc<Self>, event: VoiceEvent, now: DateTime<Utc>) {
        match event {
            VoiceEvent::Join {
                guild_id,
                user_id,
                channel_id,
            } => {
                self.reconcile_channel(guild_id, channel_id, user_id, now)
                    .await;
            }
            VoiceEvent::Leave {
                guild_id,
                user_id,
                channel_id,
            } => {
                self.reconcile_channel(guild_id, channel_id, user_id, now)
                    .await;
            }
            VoiceEvent::Move {
                guild_id,
                user_id,
                from_channel_id,
                to_channel_id,
            } => {
                self.reconcile_channel(guild_id, from_channel_id, user_id, now)
                    .await;
                self.reconcile_channel(guild_id, to_channel_id, user_id, now)
                    .await;
            }
            VoiceEvent::Update { user_id, .. } => {
                self.decision(user_id, "verworfen", "voice_update_ohne_kanalwechsel", None);
            }
        }
    }

    async fn reconcile_channel(
        &self,
        guild_id: u64,
        channel_id: u64,
        actor_id: u64,
        now: DateTime<Utc>,
    ) {
        self.count_checked().await;
        let snapshot = self.port.lane_snapshot(guild_id, channel_id).await;
        self.reconcile_posts(channel_id, snapshot.as_ref()).await;

        let Some(lane) = snapshot else {
            self.log_cleared_pending(channel_id, "kanal_nicht_im_cache")
                .await;
            self.decision(actor_id, "verworfen", "kanal_nicht_im_cache", None);
            return;
        };
        if guild_id != MAIN_GUILD_ID {
            self.log_cleared_pending(channel_id, "falscher_server")
                .await;
            self.decision(actor_id, "verworfen", "falscher_server", None);
            return;
        }
        if HARD_EXCLUDED_CATEGORY_IDS.contains(&lane.category_id) {
            self.log_cleared_pending(channel_id, "ausgeschlossene_kategorie")
                .await;
            self.decision(actor_id, "verworfen", "ausgeschlossene_kategorie", None);
            return;
        }
        if !allowed_category(lane.category_id) {
            self.log_cleared_pending(channel_id, "falsche_kategorie")
                .await;
            self.decision(actor_id, "verworfen", "falsche_kategorie", None);
            return;
        }
        let [user_id] = lane.non_bot_members.as_slice() else {
            self.log_cleared_pending(channel_id, "nicht_mehr_allein")
                .await;
            self.decision(actor_id, "verworfen", "nicht_allein", None);
            return;
        };

        let user_id = *user_id;
        let mut pending = self.pending.lock().await;
        let replaced: Vec<u64> = pending
            .iter()
            .filter(|(pending_user, state)| {
                **pending_user != user_id && state.channel_id == channel_id
            })
            .map(|(pending_user, _)| *pending_user)
            .collect();
        pending.retain(|pending_user, state| {
            *pending_user == user_id || state.channel_id != channel_id
        });
        let unchanged = pending
            .get(&user_id)
            .is_some_and(|state| state.guild_id == guild_id && state.channel_id == channel_id);
        if !unchanged {
            pending.insert(
                user_id,
                PendingSolo {
                    guild_id,
                    channel_id,
                    since: now,
                },
            );
        }
        drop(pending);
        for pending_user in replaced {
            self.decision(pending_user, "verworfen", "solo_status_beendet", None);
        }
        if !unchanged {
            self.decision(user_id, "wartet", "unter_2_minuten_allein", None);
        }
    }

    async fn log_cleared_pending(&self, channel_id: u64, reason: &'static str) {
        let cleared: Vec<u64> = {
            let mut pending = self.pending.lock().await;
            let users = pending
                .iter()
                .filter(|(_, state)| state.channel_id == channel_id)
                .map(|(user_id, _)| *user_id)
                .collect::<Vec<_>>();
            pending.retain(|_, state| state.channel_id != channel_id);
            users
        };
        for user_id in cleared {
            self.decision(user_id, "verworfen", reason, None);
        }
    }

    pub async fn restore_at(&self, now: DateTime<Utc>) {
        let persisted = match self.port.load_active_posts().await {
            Ok(posts) => posts,
            Err(error) => {
                tracing::warn!(%error, "Solo-LFG: persistierte Posts konnten nicht geladen werden");
                return;
            }
        };

        for persisted in persisted {
            let Some(created_at) = DateTime::from_timestamp(persisted.created_at, 0) else {
                let post = ActivePost {
                    channel_id: persisted.channel_id,
                    message_id: persisted.message_id,
                    created_at: now,
                };
                self.posts
                    .lock()
                    .await
                    .insert(persisted.user_id, post.clone());
                self.delete_active_post(persisted.user_id, post, "rehydriert_verwaist")
                    .await;
                continue;
            };
            let post = ActivePost {
                channel_id: persisted.channel_id,
                message_id: persisted.message_id,
                created_at,
            };
            self.posts
                .lock()
                .await
                .insert(persisted.user_id, post.clone());

            if now - created_at >= MAX_POST_AGE {
                self.delete_active_post(persisted.user_id, post, "altersgrenze")
                    .await;
                continue;
            }
            let Some(lane) = self
                .port
                .lane_snapshot(MAIN_GUILD_ID, persisted.channel_id)
                .await
            else {
                self.delete_active_post(persisted.user_id, post, "rehydriert_verwaist")
                    .await;
                continue;
            };
            if let Some(reason) = post_cleanup_reason(persisted.user_id, &lane) {
                self.delete_active_post(persisted.user_id, post, reason)
                    .await;
            }
        }
    }

    async fn reconcile_posts(&self, channel_id: u64, lane: Option<&LaneSnapshot>) {
        let stale: Vec<(u64, ActivePost, &'static str)> = {
            let posts = self.posts.lock().await;
            posts
                .iter()
                .filter(|(_, post)| post.channel_id == channel_id)
                .filter_map(|(user_id, post)| {
                    let reason = match lane {
                        Some(lane) => post_cleanup_reason(*user_id, lane)?,
                        None => "sitzer_hat_lane_verlassen",
                    };
                    Some((*user_id, post.clone(), reason))
                })
                .collect()
        };
        for (user_id, post, reason) in stale {
            self.delete_active_post(user_id, post, reason).await;
        }
    }

    async fn delete_active_post(&self, user_id: u64, post: ActivePost, reason: &'static str) {
        match self
            .port
            .delete_post(LFG_CHANNEL_ID, post.message_id, reason)
            .await
        {
            Ok(()) => {
                if let Err(error) = self.port.remove_active_post(user_id).await {
                    self.decision(
                        user_id,
                        "post_loeschen_fehlgeschlagen",
                        reason,
                        Some(&error),
                    );
                    return;
                }
                let mut posts = self.posts.lock().await;
                if posts
                    .get(&user_id)
                    .is_some_and(|current| current.message_id == post.message_id)
                {
                    posts.remove(&user_id);
                }
                self.decision(user_id, "post_geloescht", reason, None);
            }
            Err(error) => {
                self.decision(
                    user_id,
                    "post_loeschen_fehlgeschlagen",
                    reason,
                    Some(&error),
                );
            }
        }
    }

    pub async fn tick_at(&self, now: DateTime<Utc>) {
        let expired: Vec<(u64, ActivePost)> = {
            let posts = self.posts.lock().await;
            posts
                .iter()
                .filter(|(_, post)| now - post.created_at >= MAX_POST_AGE)
                .map(|(user_id, post)| (*user_id, post.clone()))
                .collect()
        };
        for (user_id, post) in expired {
            self.delete_active_post(user_id, post, "altersgrenze").await;
        }

        let due: Vec<(u64, PendingSolo)> = {
            let mut pending = self.pending.lock().await;
            let due_users: Vec<u64> = pending
                .iter()
                .filter(|(_, state)| now - state.since >= SOLO_DELAY)
                .map(|(user_id, _)| *user_id)
                .collect();
            due_users
                .into_iter()
                .filter_map(|user_id| pending.remove(&user_id).map(|state| (user_id, state)))
                .collect()
        };
        for (user_id, pending) in due {
            self.check_due(user_id, pending, now).await;
        }
    }

    async fn check_due(&self, user_id: u64, pending: PendingSolo, now: DateTime<Utc>) {
        let Some(lane) = self
            .port
            .lane_snapshot(pending.guild_id, pending.channel_id)
            .await
        else {
            self.decision(user_id, "verworfen", "kanal_nicht_im_cache", None);
            return;
        };
        if !allowed_category(lane.category_id) || lane.non_bot_members.as_slice() != [user_id] {
            self.decision(user_id, "verworfen", "nicht_mehr_allein", None);
            return;
        }

        match self.port.claim_prompt(pending.guild_id, user_id, now).await {
            Err(error) => {
                self.decision(user_id, "verworfen", "eligibility_fehler", Some(&error));
            }
            Ok(PromptDecision::TooManySessions(_)) => {
                self.decision(user_id, "verworfen", "zu_viele_sessions", None);
            }
            Ok(PromptDecision::OptedOut) => {
                self.decision(user_id, "verworfen", "Opt-out", None);
            }
            Ok(PromptDecision::NeverAsk) => {
                self.decision(user_id, "verworfen", "nie_fragen", None);
            }
            Ok(PromptDecision::Cooldown) => {
                self.decision(user_id, "verworfen", "cooldown", None);
            }
            Ok(PromptDecision::Prompt) => {
                let rank = match self.port.verified_rank(user_id).await {
                    Ok(rank) => rank,
                    Err(error) => {
                        self.decision(user_id, "rang_fehlt", "rangabfrage_fehler", Some(&error));
                        None
                    }
                };
                let body = dm_body(pending.guild_id, pending.channel_id, &lane, rank.as_deref());
                match self.port.send_dm(user_id, body).await {
                    Ok(()) => {
                        self.daily.lock().await.prompted += 1;
                        self.decision(user_id, "dm_zugestellt", "solo_2_minuten", None);
                    }
                    Err(error) => {
                        self.daily.lock().await.dm_errors += 1;
                        self.decision(user_id, "dm_fehlgeschlagen", "discord_fehler", Some(&error));
                    }
                }
            }
        }
    }

    pub async fn handle_interaction(&self, interaction: BridgeInteraction) -> BridgeReply {
        if let Some((guild_id, channel_id)) =
            custom_id_context(&interaction.custom_id, ENTER_CUSTOM_ID)
        {
            return self
                .handle_enter(interaction.user_id, guild_id, channel_id)
                .await;
        }
        if custom_id_context(&interaction.custom_id, LATER_CUSTOM_ID).is_some() {
            self.daily.lock().await.rejected += 1;
            self.decision(
                interaction.user_id,
                "abgelehnt",
                "nicht_jetzt_gewaehlt",
                None,
            );
            return BridgeReply::ephemeral_text(LATER_REPLY);
        }
        if custom_id_context(&interaction.custom_id, NEVER_CUSTOM_ID).is_some() {
            return self.handle_never(interaction.user_id).await;
        }
        if let Some((guild_id, channel_id)) =
            custom_id_context(&interaction.custom_id, MODAL_CUSTOM_ID)
        {
            return self.handle_modal(interaction, guild_id, channel_id).await;
        }
        self.decision(
            interaction.user_id,
            "verworfen",
            "ungueltige_interaktion",
            None,
        );
        BridgeReply::ephemeral_text(INVALID_REPLY)
    }

    async fn handle_enter(&self, user_id: u64, guild_id: u64, channel_id: u64) -> BridgeReply {
        self.decision(user_id, "eintragen_geklickt", "dm_button", None);
        let Some(lane) = self.port.lane_snapshot(guild_id, channel_id).await else {
            self.decision(user_id, "verworfen", "kanal_nicht_im_cache", None);
            return BridgeReply::ephemeral_text(INVALID_REPLY);
        };
        if lane.non_bot_members.as_slice() != [user_id] {
            self.decision(user_id, "verworfen", "nicht_mehr_allein", None);
            return BridgeReply::ephemeral_text(INVALID_REPLY);
        }
        let rank = match self.port.verified_rank(user_id).await {
            Ok(rank) => rank,
            Err(error) => {
                self.decision(user_id, "rang_fehlt", "rangabfrage_fehler", Some(&error));
                None
            }
        };
        BridgeReply {
            modal: Some(solo_modal(guild_id, channel_id, rank)),
            ..BridgeReply::default()
        }
    }

    async fn handle_never(&self, user_id: u64) -> BridgeReply {
        match self.port.set_never_ask(user_id).await {
            Ok(()) => {
                self.daily.lock().await.rejected += 1;
                self.decision(user_id, "abgelehnt", "nie_fragen_gewaehlt", None);
                BridgeReply::ephemeral_text(NEVER_REPLY)
            }
            Err(error) => {
                self.decision(
                    user_id,
                    "nie_fragen_fehlgeschlagen",
                    "kv_fehler",
                    Some(&error),
                );
                BridgeReply::ephemeral_text(POST_ERROR_REPLY)
            }
        }
    }

    async fn handle_modal(
        &self,
        interaction: BridgeInteraction,
        guild_id: u64,
        channel_id: u64,
    ) -> BridgeReply {
        let user_id = interaction.user_id;
        let Some(lane) = self.port.lane_snapshot(guild_id, channel_id).await else {
            self.decision(user_id, "verworfen", "kanal_nicht_im_cache", None);
            return BridgeReply::ephemeral_text(INVALID_REPLY);
        };
        if lane.non_bot_members.as_slice() != [user_id] {
            self.decision(user_id, "verworfen", "nicht_mehr_allein", None);
            return BridgeReply::ephemeral_text(INVALID_REPLY);
        }
        let value = |key: &str| {
            interaction
                .options
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
        };
        let content = lfg_post(
            user_id,
            guild_id,
            channel_id,
            &lane,
            value("time"),
            value("rank"),
            value("note"),
        );
        let message_id = match self.port.post_lfg(LFG_CHANNEL_ID, content).await {
            Ok(message_id) => message_id,
            Err(error) => {
                self.decision(
                    user_id,
                    "post_fehlgeschlagen",
                    "discord_fehler",
                    Some(&error),
                );
                return BridgeReply::ephemeral_text(POST_ERROR_REPLY);
            }
        };
        let post = ActivePost {
            channel_id,
            message_id,
            created_at: Utc::now(),
        };
        let persisted = PersistedPost {
            user_id,
            channel_id,
            message_id,
            created_at: post.created_at.timestamp(),
        };
        if let Err(error) = self.port.save_active_post(&persisted).await {
            self.decision(
                user_id,
                "post_fehlgeschlagen",
                "persistenz_fehler",
                Some(&error),
            );
            if let Err(delete_error) = self
                .port
                .delete_post(LFG_CHANNEL_ID, message_id, "persistenz_rollback")
                .await
            {
                self.decision(
                    user_id,
                    "post_loeschen_fehlgeschlagen",
                    "persistenz_rollback",
                    Some(&delete_error),
                );
            }
            return BridgeReply::ephemeral_text(POST_ERROR_REPLY);
        }
        let previous = self.posts.lock().await.insert(user_id, post);
        if let Some(previous) = previous {
            match self
                .port
                .delete_post(LFG_CHANNEL_ID, previous.message_id, "post_erneuert")
                .await
            {
                Ok(()) => self.decision(user_id, "post_geloescht", "post_erneuert", None),
                Err(error) => self.decision(
                    user_id,
                    "post_loeschen_fehlgeschlagen",
                    "post_erneuert",
                    Some(&error),
                ),
            }
        }
        self.daily.lock().await.registered += 1;
        self.decision(user_id, "post_erstellt", "modal_abgesendet", None);

        let current = self.port.lane_snapshot(guild_id, channel_id).await;
        self.reconcile_posts(channel_id, current.as_ref()).await;
        BridgeReply::ephemeral_text(CONFIRMATION_REPLY)
    }

    pub async fn flush_daily_summary(&self) {
        let counters = std::mem::take(&mut *self.daily.lock().await);
        let text = format!(
            "{DAILY_SUMMARY_PREFIX}\ngeprüft: {} · angesprochen: {} · eingetragen: {} · abgelehnt: {} · DM-Fehler: {}",
            counters.checked,
            counters.prompted,
            counters.registered,
            counters.rejected,
            counters.dm_errors
        );
        if let Err(error) = self.port.send_log(text).await {
            tracing::warn!(%error, "Solo-LFG: tägliche Zusammenfassung fehlgeschlagen");
        }
    }
}

fn allowed_category(category_id: u64) -> bool {
    crate::status::TARGET_CATEGORY_IDS.contains(&category_id)
        || category_id == NEW_PLAYER_CATEGORY_ID
}

fn post_cleanup_reason(user_id: u64, lane: &LaneSnapshot) -> Option<&'static str> {
    if lane.non_bot_members.as_slice() == [user_id] {
        None
    } else if lane.non_bot_members.contains(&user_id) {
        Some("zweite_person_beigetreten")
    } else {
        Some("sitzer_hat_lane_verlassen")
    }
}

fn mode_for_category(category_id: u64) -> &'static str {
    match category_id {
        1412804540994162789 => "Ranked",
        1357422957017698478 => "Street Brawl",
        NEW_PLAYER_CATEGORY_ID => "New Player",
        _ => "Casual",
    }
}

fn custom_id_context(custom_id: &str, prefix: &str) -> Option<(u64, u64)> {
    let suffix = custom_id.strip_prefix(prefix)?.strip_prefix(':')?;
    let (guild_id, channel_id) = suffix.split_once(':')?;
    Some((guild_id.parse().ok()?, channel_id.parse().ok()?))
}

fn dm_body(guild_id: u64, channel_id: u64, lane: &LaneSnapshot, rank: Option<&str>) -> Value {
    let rank_suffix = rank.map(|rank| format!(" — {rank}")).unwrap_or_default();
    let mut preview = vec![format!(
        "> **{}{rank_suffix}**",
        mode_for_category(lane.category_id)
    )];
    if let Some(free) = free_slots(lane) {
        preview.push(format!(
            "> Sitzt gerade in {}, {free} Plätze frei",
            lane.name
        ));
    }
    preview.push("> Zeit: _sagst du gleich selbst_".to_string());
    let description = format!(
        "Hey, du sitzt seit ein paar Minuten allein in **{}**. Das fällt hier keinem auf, solange es niemand weiß — die meisten schauen erst in die Mitspieler-Suche, bevor sie selbst eine Lane aufmachen.\n\nSoll ich dich da eintragen? Sowas in der Art kommt dann rein:\n\n{}\n\nDauert zehn Sekunden, und du musst nicht warten, bis dich jemand zufällig sieht.",
        lane.name,
        preview.join("\n")
    );
    json!({
        "embeds": [{
            "title": DM_TITLE,
            "description": description,
        }],
        "components": [{
            "type": 1,
            "components": [
                {
                    "type": 2,
                    "style": 1,
                    "label": "Eintragen",
                    "custom_id": format!("{ENTER_CUSTOM_ID}:{guild_id}:{channel_id}"),
                },
                {
                    "type": 2,
                    "style": 2,
                    "label": "Nicht jetzt",
                    "custom_id": format!("{LATER_CUSTOM_ID}:{guild_id}:{channel_id}"),
                },
                {
                    "type": 2,
                    "style": 2,
                    "label": "Nie fragen",
                    "custom_id": format!("{NEVER_CUSTOM_ID}:{guild_id}:{channel_id}"),
                }
            ]
        }]
    })
}

fn solo_modal(guild_id: u64, channel_id: u64, rank: Option<String>) -> ModalSpec {
    ModalSpec {
        custom_id: format!("{MODAL_CUSTOM_ID}:{guild_id}:{channel_id}"),
        title: MODAL_TITLE.to_string(),
        fields: vec![
            ModalField {
                custom_id: "time".to_string(),
                label: "Wie lange hast du Zeit?".to_string(),
                placeholder: "z.B. \"noch 2 Stunden\" oder \"eine Runde\"".to_string(),
                value: None,
                required: true,
                min_length: 1,
                max_length: 100,
                paragraph: false,
            },
            ModalField {
                custom_id: "rank".to_string(),
                label: "Dein Rang".to_string(),
                placeholder: "z.B. \"Ascendant 2\" — leer lassen, wenn egal".to_string(),
                value: rank,
                required: false,
                min_length: 0,
                max_length: 100,
                paragraph: false,
            },
            ModalField {
                custom_id: "note".to_string(),
                label: "Noch was dazu?".to_string(),
                placeholder: "z.B. \"chill, kein Sweat\" oder \"will ranked grinden\"".to_string(),
                value: None,
                required: false,
                min_length: 0,
                max_length: 500,
                paragraph: true,
            },
        ],
    }
}

fn lfg_post(
    user_id: u64,
    guild_id: u64,
    channel_id: u64,
    lane: &LaneSnapshot,
    time: &str,
    rank: &str,
    note: &str,
) -> String {
    let mut lines = vec![format!("<@{user_id}>")];
    let mode = mode_for_category(lane.category_id);
    if rank.is_empty() {
        lines.push(format!("**{mode}**"));
    } else {
        lines.push(format!("**{mode}** — {rank}"));
    }
    if let Some(free) = free_slots(lane) {
        lines.push(format!("Sitzt gerade in {}, {free} Plätze frei", lane.name));
    }
    lines.push(format!("⏱️ {time}"));
    if !note.is_empty() {
        lines.push(format!("\"{note}\""));
    }
    lines.push(String::new());
    lines.push(format!(
        "→ Beitreten: https://discord.com/channels/{guild_id}/{channel_id}"
    ));
    lines.join("\n")
}

fn free_slots(lane: &LaneSnapshot) -> Option<usize> {
    lane.user_limit
        .filter(|limit| *limit > 0)
        .map(|limit| limit.saturating_sub(lane.non_bot_members.len()))
}

struct SoloInteractionHandler {
    watch: Arc<SoloWatch>,
}

#[async_trait::async_trait]
impl InteractionHandler for SoloInteractionHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        self.watch.handle_interaction(interaction).await
    }
}

pub fn register(router: &mut InteractionRouter, watch: Arc<SoloWatch>) {
    router.on_prefix("solo_lfg:", Arc::new(SoloInteractionHandler { watch }));
}

pub fn spawn(watch: Arc<SoloWatch>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_voice();
    tokio::spawn(async move {
        watch.restore_at(Utc::now()).await;
        let mut tick = tokio::time::interval(TICK_INTERVAL);
        let mut daily = tokio::time::interval(StdDuration::from_secs(24 * 60 * 60));
        daily.tick().await;
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Ok(event) => watch.handle_event_at(event, Utc::now()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "Solo-LFG: Voice-Events übersprungen");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => watch.tick_at(Utc::now()).await,
                _ = daily.tick() => watch.flush_daily_summary().await,
            }
        }
    })
}

pub(crate) async fn claim_prompt_db(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
    now: DateTime<Utc>,
) -> Result<PromptDecision, String> {
    let guild_id = i64::try_from(guild_id).map_err(|error| error.to_string())?;
    let user_id_i64 = i64::try_from(user_id).map_err(|error| error.to_string())?;
    let key = user_id.to_string();
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id_i64)
        .await
        .map_err(|error| error.to_string())?
    {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(PromptDecision::OptedOut);
    }

    let sessions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint
           FROM activity.voice_session_log
          WHERE guild_id = $1
            AND user_id = $2
            AND started_at >= $3",
    )
    .bind(guild_id)
    .bind(user_id_i64)
    .bind(now - Duration::days(90))
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    if sessions >= 4 {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(PromptDecision::TooManySessions(sessions as usize));
    }

    let never: Option<String> =
        sqlx::query_scalar("SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2")
            .bind(NEVER_ASK_NS)
            .bind(&key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
    if never.is_some() {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(PromptDecision::NeverAsk);
    }

    let last_prompt: Option<String> =
        sqlx::query_scalar("SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2 FOR UPDATE")
            .bind(LAST_PROMPT_NS)
            .bind(&key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
    if let Some(last_prompt) = last_prompt {
        let timestamp = last_prompt
            .parse::<i64>()
            .map_err(|error| format!("ungültiger Solo-LFG-Cooldown: {error}"))?;
        let last_prompt = DateTime::from_timestamp(timestamp, 0)
            .ok_or_else(|| "ungültiger Solo-LFG-Cooldown-Zeitstempel".to_string())?;
        if now - last_prompt < Duration::hours(24) {
            tx.commit().await.map_err(|error| error.to_string())?;
            return Ok(PromptDecision::Cooldown);
        }
    }

    sqlx::query(
        "INSERT INTO bot.kv_store (ns, k, v)
         VALUES ($1, $2, $3)
         ON CONFLICT (ns, k) DO UPDATE SET v = EXCLUDED.v",
    )
    .bind(LAST_PROMPT_NS)
    .bind(&key)
    .bind(now.timestamp().to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(PromptDecision::Prompt)
}

pub(crate) async fn set_never_ask_db(pool: &PgPool, user_id: u64) -> Result<(), String> {
    kv::set(pool, NEVER_ASK_NS, &user_id.to_string(), "true")
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn load_active_posts_db(pool: &PgPool) -> Result<Vec<PersistedPost>, String> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT k, v FROM bot.kv_store WHERE ns = $1")
        .bind(ACTIVE_POST_NS)
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|(key, value)| {
            let post: PersistedPost =
                serde_json::from_str(&value).map_err(|error| error.to_string())?;
            if key != post.user_id.to_string() {
                return Err(format!(
                    "Solo-LFG-Post hat widersprüchliche User-ID: key={key}"
                ));
            }
            Ok(post)
        })
        .collect()
}

pub(crate) async fn save_active_post_db(pool: &PgPool, post: &PersistedPost) -> Result<(), String> {
    let value = serde_json::to_string(post).map_err(|error| error.to_string())?;
    kv::set(pool, ACTIVE_POST_NS, &post.user_id.to_string(), &value)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn remove_active_post_db(pool: &PgPool, user_id: u64) -> Result<(), String> {
    kv::delete(pool, ACTIVE_POST_NS, &user_id.to_string())
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn verified_rank_db(
    pool: &PgPool,
    user_id: u64,
) -> Result<Option<String>, String> {
    let user_id = i64::try_from(user_id).map_err(|error| error.to_string())?;
    let row = sqlx::query_as::<_, (String, Option<i32>)>(
        "SELECT deadlock_rank_name, deadlock_subrank
           FROM core.steam_links
          WHERE discord_id = $1
            AND verified = TRUE
            AND deadlock_rank_name IS NOT NULL
          ORDER BY primary_account DESC, deadlock_rank_updated_at DESC NULLS LAST
          LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(row.map(
        |(name, subrank)| match subrank.filter(|subrank| *subrank > 0) {
            Some(subrank) => format!("{} {subrank}", title_case_rank(&name)),
            None => title_case_rank(&name),
        },
    ))
}

fn title_case_rank(rank: &str) -> String {
    let mut chars = rank.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use chrono::{DateTime, Duration, TimeZone, Utc};
    use dl_discord::{BridgeInteraction, VoiceEvent};
    use serde_json::Value;

    #[derive(Default)]
    struct MockState {
        lane: Option<LaneSnapshot>,
        sessions: usize,
        opted_out: bool,
        never: bool,
        last_prompt_at: Option<DateTime<Utc>>,
        persisted_posts: HashMap<u64, (u64, u64, DateTime<Utc>)>,
        dms: Vec<(u64, Value)>,
        posts: Vec<(u64, String)>,
        deletes: Vec<(u64, u64, String)>,
        logs: Vec<String>,
    }

    #[derive(Clone, Default)]
    struct MockPort {
        state: Arc<Mutex<MockState>>,
    }

    impl MockPort {
        fn set_lane(&self, category_id: u64, members: &[u64]) {
            self.state.lock().expect("lock").lane = Some(LaneSnapshot {
                category_id,
                name: "Lobby 1".to_string(),
                non_bot_members: members.to_vec(),
                user_limit: Some(6),
            });
        }

        fn dms(&self) -> usize {
            self.state.lock().expect("lock").dms.len()
        }
    }

    #[async_trait::async_trait]
    impl SoloWatchPort for MockPort {
        async fn lane_snapshot(&self, _guild_id: u64, _channel_id: u64) -> Option<LaneSnapshot> {
            self.state.lock().expect("lock").lane.clone()
        }

        async fn claim_prompt(
            &self,
            _guild_id: u64,
            _user_id: u64,
            now: DateTime<Utc>,
        ) -> Result<PromptDecision, String> {
            let mut state = self.state.lock().expect("lock");
            let decision = if state.opted_out {
                PromptDecision::OptedOut
            } else if state.never {
                PromptDecision::NeverAsk
            } else if state.sessions >= 4 {
                PromptDecision::TooManySessions(state.sessions)
            } else if state
                .last_prompt_at
                .is_some_and(|last| now - last < Duration::hours(24))
            {
                PromptDecision::Cooldown
            } else {
                state.last_prompt_at = Some(now);
                PromptDecision::Prompt
            };
            Ok(decision)
        }

        async fn verified_rank(&self, _user_id: u64) -> Result<Option<String>, String> {
            Ok(Some("Ascendant 2".to_string()))
        }

        async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String> {
            self.state.lock().expect("lock").dms.push((user_id, body));
            Ok(())
        }

        async fn set_never_ask(&self, _user_id: u64) -> Result<(), String> {
            self.state.lock().expect("lock").never = true;
            Ok(())
        }

        async fn post_lfg(&self, channel_id: u64, content: String) -> Result<u64, String> {
            let mut state = self.state.lock().expect("lock");
            state.posts.push((channel_id, content));
            Ok(700 + state.posts.len() as u64)
        }

        async fn load_active_posts(&self) -> Result<Vec<PersistedPost>, String> {
            Ok(self
                .state
                .lock()
                .expect("lock")
                .persisted_posts
                .iter()
                .map(
                    |(user_id, (channel_id, message_id, created_at))| PersistedPost {
                        user_id: *user_id,
                        channel_id: *channel_id,
                        message_id: *message_id,
                        created_at: created_at.timestamp(),
                    },
                )
                .collect())
        }

        async fn save_active_post(&self, post: &PersistedPost) -> Result<(), String> {
            let created_at = DateTime::from_timestamp(post.created_at, 0)
                .ok_or_else(|| "invalid timestamp".to_string())?;
            self.state
                .lock()
                .expect("lock")
                .persisted_posts
                .insert(post.user_id, (post.channel_id, post.message_id, created_at));
            Ok(())
        }

        async fn remove_active_post(&self, user_id: u64) -> Result<(), String> {
            self.state
                .lock()
                .expect("lock")
                .persisted_posts
                .remove(&user_id);
            Ok(())
        }

        async fn delete_post(
            &self,
            channel_id: u64,
            message_id: u64,
            reason: &str,
        ) -> Result<(), String> {
            self.state.lock().expect("lock").deletes.push((
                channel_id,
                message_id,
                reason.to_string(),
            ));
            Ok(())
        }

        async fn send_log(&self, text: String) -> Result<(), String> {
            self.state.lock().expect("lock").logs.push(text);
            Ok(())
        }

        fn log_decision(
            &self,
            user_id: u64,
            decision: &'static str,
            reason: &'static str,
            error: Option<&str>,
        ) {
            self.state.lock().expect("lock").logs.push(format!(
                "user_id={user_id} entscheidung={decision} grund={reason} fehler={}",
                error.unwrap_or_default()
            ));
        }
    }

    const GUILD: u64 = MAIN_GUILD_ID;
    const USER: u64 = 42;
    const CHANNEL: u64 = 99;
    const CHILL: u64 = 1289721245281292290;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 26, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    async fn start_solo(watch: &Arc<SoloWatch>, port: &MockPort, category_id: u64) {
        port.set_lane(category_id, &[USER]);
        watch
            .handle_event_at(
                VoiceEvent::Join {
                    guild_id: GUILD,
                    user_id: USER,
                    channel_id: CHANNEL,
                },
                now(),
            )
            .await;
    }

    async fn prompt(watch: &Arc<SoloWatch>, port: &MockPort) {
        start_solo(watch, port, CHILL).await;
        watch.tick_at(now() + SOLO_DELAY).await;
    }

    #[tokio::test]
    async fn bestandsuser_mit_vier_sessions_wird_nicht_angesprochen() {
        let port = Arc::new(MockPort::default());
        port.state.lock().expect("lock").sessions = 4;
        let watch = SoloWatch::new(port.clone());

        prompt(&watch, &port).await;

        assert_eq!(port.dms(), 0);
    }

    #[tokio::test]
    async fn unbekannter_wird_nach_zwei_minuten_angesprochen() {
        let port = Arc::new(MockPort::default());
        port.state.lock().expect("lock").sessions = 3;
        let watch = SoloWatch::new(port.clone());

        prompt(&watch, &port).await;

        let state = port.state.lock().expect("lock");
        assert_eq!(state.dms.len(), 1);
        assert_eq!(state.dms[0].1["embeds"][0]["title"], DM_TITLE);
        assert!(state.dms[0].1["embeds"][0]["description"]
            .as_str()
            .expect("description")
            .contains("Dauert zehn Sekunden"));
        assert_eq!(
            state.dms[0].1["components"][0]["components"][0]["label"],
            "Eintragen"
        );
        assert_eq!(
            state.dms[0].1["components"][0]["components"][1]["label"],
            "Nicht jetzt"
        );
        assert_eq!(
            state.dms[0].1["components"][0]["components"][2]["label"],
            "Nie fragen"
        );
    }

    #[tokio::test]
    async fn unter_zwei_minuten_allein_keine_ansprache() {
        let port = Arc::new(MockPort::default());
        let watch = SoloWatch::new(port.clone());
        start_solo(&watch, &port, CHILL).await;

        watch
            .tick_at(now() + SOLO_DELAY - Duration::seconds(1))
            .await;

        assert_eq!(port.dms(), 0);
    }

    #[tokio::test]
    async fn zweite_person_vor_zwei_minuten_verhindert_ansprache() {
        let port = Arc::new(MockPort::default());
        let watch = SoloWatch::new(port.clone());
        start_solo(&watch, &port, CHILL).await;
        port.set_lane(CHILL, &[USER, 43]);
        watch
            .handle_event_at(
                VoiceEvent::Join {
                    guild_id: GUILD,
                    user_id: 43,
                    channel_id: CHANNEL,
                },
                now() + Duration::minutes(1),
            )
            .await;

        watch.tick_at(now() + SOLO_DELAY).await;

        assert_eq!(port.dms(), 0);
    }

    #[tokio::test]
    async fn scrims_und_afk_sind_ausgeschlossen() {
        for category_id in [1523225910034305074, 1407787097217040547] {
            let port = Arc::new(MockPort::default());
            let watch = SoloWatch::new(port.clone());

            start_solo(&watch, &port, category_id).await;
            watch.tick_at(now() + SOLO_DELAY).await;

            assert_eq!(port.dms(), 0, "Kategorie {category_id}");
        }
    }

    #[tokio::test]
    async fn opt_out_verhindert_ansprache_und_wird_geloggt() {
        let port = Arc::new(MockPort::default());
        port.state.lock().expect("lock").opted_out = true;
        let watch = SoloWatch::new(port.clone());

        prompt(&watch, &port).await;

        let state = port.state.lock().expect("lock");
        assert!(state.dms.is_empty());
        assert!(state.logs.iter().any(|line| line.contains("Opt-out")));
    }

    #[tokio::test]
    async fn nie_fragen_bleibt_ueber_neustart_wirksam() {
        let port = Arc::new(MockPort::default());
        let watch = SoloWatch::new(port.clone());
        let reply = watch
            .handle_interaction(BridgeInteraction {
                custom_id: format!("{NEVER_CUSTOM_ID}:{GUILD}:{CHANNEL}"),
                user_id: USER,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(NEVER_REPLY));

        let restarted = SoloWatch::new(port.clone());
        prompt(&restarted, &port).await;

        assert_eq!(port.dms(), 0);
    }

    #[tokio::test]
    async fn cooldown_von_24_stunden_greift() {
        let port = Arc::new(MockPort::default());
        port.state.lock().expect("lock").last_prompt_at = Some(now() - Duration::hours(23));
        let watch = SoloWatch::new(port.clone());

        prompt(&watch, &port).await;

        assert_eq!(port.dms(), 0);
    }

    #[tokio::test]
    async fn modal_ist_mit_verifiziertem_steam_rang_vorbelegt() {
        let port = Arc::new(MockPort::default());
        port.set_lane(CHILL, &[USER]);
        let watch = SoloWatch::new(port);

        let reply = watch
            .handle_interaction(BridgeInteraction {
                custom_id: format!("{ENTER_CUSTOM_ID}:{GUILD}:{CHANNEL}"),
                user_id: USER,
                ..BridgeInteraction::default()
            })
            .await;

        let modal = reply.modal.expect("modal");
        assert_eq!(modal.title, MODAL_TITLE);
        assert_eq!(modal.fields.len(), 3);
        assert_eq!(modal.fields[0].label, "Wie lange hast du Zeit?");
        assert_eq!(
            modal.fields[0].placeholder,
            "z.B. \"noch 2 Stunden\" oder \"eine Runde\""
        );
        assert_eq!(modal.fields[1].value.as_deref(), Some("Ascendant 2"));
        assert_eq!(modal.fields[2].label, "Noch was dazu?");
    }

    async fn create_post(watch: &Arc<SoloWatch>, port: &MockPort) {
        port.set_lane(CHILL, &[USER]);
        let reply = watch
            .handle_interaction(BridgeInteraction {
                custom_id: format!("{MODAL_CUSTOM_ID}:{GUILD}:{CHANNEL}"),
                user_id: USER,
                options: [
                    ("time".to_string(), Value::String("eine Runde".to_string())),
                    ("rank".to_string(), Value::String("Ascendant 2".to_string())),
                    (
                        "note".to_string(),
                        Value::String("chill, kein Sweat".to_string()),
                    ),
                ]
                .into_iter()
                .collect(),
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(CONFIRMATION_REPLY));
        let state = port.state.lock().expect("lock");
        assert_eq!(state.posts.len(), 1);
        assert_eq!(state.posts[0].0, LFG_CHANNEL_ID);
        assert_eq!(
            state.posts[0].1,
            "<@42>\n**Casual** — Ascendant 2\nSitzt gerade in Lobby 1, 5 Plätze frei\n⏱️ eine Runde\n\"chill, kein Sweat\"\n\n→ Beitreten: https://discord.com/channels/1289721245281292288/99"
        );
    }

    #[tokio::test]
    async fn post_wird_geloescht_wenn_jemand_joint() {
        let port = Arc::new(MockPort::default());
        let watch = SoloWatch::new(port.clone());
        create_post(&watch, &port).await;
        port.set_lane(CHILL, &[USER, 43]);

        watch
            .handle_event_at(
                VoiceEvent::Join {
                    guild_id: GUILD,
                    user_id: 43,
                    channel_id: CHANNEL,
                },
                now(),
            )
            .await;

        assert_eq!(port.state.lock().expect("lock").deletes.len(), 1);
    }

    #[tokio::test]
    async fn post_wird_geloescht_wenn_sitzer_geht() {
        let port = Arc::new(MockPort::default());
        let watch = SoloWatch::new(port.clone());
        create_post(&watch, &port).await;
        port.set_lane(CHILL, &[]);

        watch
            .handle_event_at(
                VoiceEvent::Leave {
                    guild_id: GUILD,
                    user_id: USER,
                    channel_id: CHANNEL,
                },
                now(),
            )
            .await;

        assert_eq!(port.state.lock().expect("lock").deletes.len(), 1);
    }

    #[tokio::test]
    async fn persistierter_post_wird_nach_neustart_beim_join_geloescht() {
        let port = Arc::new(MockPort::default());
        create_post(&SoloWatch::new(port.clone()), &port).await;
        assert_eq!(port.state.lock().expect("lock").persisted_posts.len(), 1);

        let restarted = SoloWatch::new(port.clone());
        restarted.restore_at(now()).await;
        port.set_lane(CHILL, &[USER, 43]);
        restarted
            .handle_event_at(
                VoiceEvent::Join {
                    guild_id: GUILD,
                    user_id: 43,
                    channel_id: CHANNEL,
                },
                now(),
            )
            .await;

        let state = port.state.lock().expect("lock");
        assert_eq!(
            state.deletes,
            vec![(LFG_CHANNEL_ID, 701, "zweite_person_beigetreten".to_string())]
        );
    }

    #[tokio::test]
    async fn persistierter_post_wird_nach_neustart_beim_leave_geloescht() {
        let port = Arc::new(MockPort::default());
        create_post(&SoloWatch::new(port.clone()), &port).await;

        let restarted = SoloWatch::new(port.clone());
        restarted.restore_at(now()).await;
        port.set_lane(CHILL, &[]);
        restarted
            .handle_event_at(
                VoiceEvent::Leave {
                    guild_id: GUILD,
                    user_id: USER,
                    channel_id: CHANNEL,
                },
                now(),
            )
            .await;

        assert_eq!(
            port.state.lock().expect("lock").deletes,
            vec![(LFG_CHANNEL_ID, 701, "sitzer_hat_lane_verlassen".to_string())]
        );
    }

    #[tokio::test]
    async fn post_aelter_als_drei_stunden_wird_trotz_sitzer_geloescht() {
        let port = Arc::new(MockPort::default());
        create_post(&SoloWatch::new(port.clone()), &port).await;
        port.state
            .lock()
            .expect("lock")
            .persisted_posts
            .get_mut(&USER)
            .expect("persisted post")
            .2 = now() - Duration::hours(3) - Duration::seconds(1);

        let restarted = SoloWatch::new(port.clone());
        restarted.restore_at(now() - Duration::hours(2)).await;
        assert!(port.state.lock().expect("lock").deletes.is_empty());
        restarted.tick_at(now()).await;

        assert_eq!(
            port.state.lock().expect("lock").deletes,
            vec![(LFG_CHANNEL_ID, 701, "altersgrenze".to_string())]
        );
    }

    #[tokio::test]
    async fn geloeschter_post_steht_nach_weiterem_neustart_nicht_wieder_auf() {
        let port = Arc::new(MockPort::default());
        create_post(&SoloWatch::new(port.clone()), &port).await;

        let restarted = SoloWatch::new(port.clone());
        restarted.restore_at(now()).await;
        port.set_lane(CHILL, &[USER, 43]);
        restarted
            .handle_event_at(
                VoiceEvent::Join {
                    guild_id: GUILD,
                    user_id: 43,
                    channel_id: CHANNEL,
                },
                now(),
            )
            .await;
        assert!(port.state.lock().expect("lock").persisted_posts.is_empty());

        SoloWatch::new(port.clone()).restore_at(now()).await;

        assert_eq!(port.state.lock().expect("lock").deletes.len(), 1);
    }

    #[tokio::test]
    async fn taegliche_zusammenfassung_enthaelt_positive_und_negative_zaehler() {
        let port = Arc::new(MockPort::default());
        let watch = SoloWatch::new(port.clone());
        prompt(&watch, &port).await;
        create_post(&watch, &port).await;
        watch
            .handle_interaction(BridgeInteraction {
                custom_id: format!("{LATER_CUSTOM_ID}:{GUILD}:{CHANNEL}"),
                user_id: USER,
                ..BridgeInteraction::default()
            })
            .await;

        watch.flush_daily_summary().await;

        let state = port.state.lock().expect("lock");
        let summary = state
            .logs
            .iter()
            .find(|line| line.starts_with(DAILY_SUMMARY_PREFIX))
            .expect("daily summary");
        assert!(summary.contains("geprüft: 1"));
        assert!(summary.contains("angesprochen: 1"));
        assert!(summary.contains("eingetragen: 1"));
        assert!(summary.contains("abgelehnt: 1"));
        assert!(summary.contains("DM-Fehler: 0"));
    }
}
