//! Stateful behavior detection for the unified moderation pipeline.
//!
//! This module keeps the non-LLM SecurityGuard heuristics: account takeover
//! image spread, burst/rate windows, young accounts, image multichannel,
//! foreign invites and keyword hits. It returns a signal only; content
//! classification and enforcement happen in `ModerationSystem`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, OnceLock};

use regex::Regex;

pub const WINDOW_SECONDS: i64 = 3600;
pub const BURST_WINDOW_SECONDS: i64 = 60;
pub const CHANNEL_THRESHOLD: usize = 3;
pub const MESSAGE_THRESHOLD: usize = 3;
pub const NEW_ACCOUNT_MAX_AGE_HOURS: i64 = 720;
pub const NEW_MEMBER_MAX_JOIN_HOURS: i64 = 168;
pub const IMAGE_CHANNEL_THRESHOLD: usize = 2;
pub const IMAGE_MULTICHANNEL_WINDOW_SECONDS: i64 = 300;
pub const TAKEOVER_WINDOW_SECONDS: i64 = 30;
pub const TAKEOVER_IMAGE_CHANNELS: usize = 2;
/// Fenster, aus dem beim Takeover-Treffer alle Wellennachrichten als Loesch- und
/// Beweisziel eingesammelt werden. Der 30-s-Trigger bleibt das Ausloesekriterium; die
/// Welle selbst laeuft laenger, deshalb ein breiteres Loeschfenster.
pub const TAKEOVER_WAVE_SECONDS: i64 = 120;
/// Nach einem Takeover-Treffer noch offene Wellennachrichten desselben Kontos, die
/// bereits in der Broadcast-Queue lagen, werden in diesem Fenster nur geloescht, ohne
/// neuen Case und ohne neue Sanktion.
pub const TAKEOVER_CLEANUP_SECONDS: i64 = 120;
pub const TIMEOUT_MINUTES: i64 = 1440;
pub const PROPOSAL_TIMEOUT_MINUTES: i64 = 60;
pub const CASE_COOLDOWN_SECONDS: i64 = 600;
pub const HISTORY_MAX: usize = 20;
pub const HISTORY_USER_MAX: usize = 128;
pub const SUPPRESSED_USER_MAX: usize = 512;
pub const EVIDENCE_IMAGE_LIMIT: usize = 10;

pub const KEYWORDS: [&str; 15] = [
    "telegram",
    "dm me",
    "pm me",
    "friend request",
    "how to start earning",
    "100k",
    "usdt",
    "withdrawal",
    "payout",
    "profit",
    "woamax",
    "promo code",
    "bonus",
    "first 10 people",
    "earning $",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentMessage {
    pub channel_id: u64,
    pub message_id: u64,
    pub created_at: i64,
    pub content: String,
    pub attachment_count: u32,
    pub image_count: u32,
    pub image_urls: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviorTriggerType {
    AccountTakeover,
    BurstRate,
    YoungAccountBurst,
    ImageMultichannel,
    ForeignInvite,
    Keyword,
}

impl BehaviorTriggerType {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::AccountTakeover => "account_takeover",
            Self::BurstRate => "burst_rate",
            Self::YoungAccountBurst => "young_account_burst",
            Self::ImageMultichannel => "image_multichannel",
            Self::ForeignInvite => "foreign_invite",
            Self::Keyword => "keyword",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviorSeverity {
    Suspicious,
    Critical,
}

impl BehaviorSeverity {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Suspicious => "suspicious",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviorActionHint {
    Ban,
    Timeout,
    Proposal,
}

impl BehaviorActionHint {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Ban => "ban",
            Self::Timeout => "timeout",
            Self::Proposal => "proposal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorEvidence {
    pub window_seconds: i64,
    pub channel_ids: Vec<u64>,
    pub message_ids: Vec<u64>,
    pub message_count: usize,
    pub attachment_count: u32,
    pub image_count: u32,
    pub keyword_hit: bool,
    pub account_age_hours: i64,
    pub join_age_hours: Option<i64>,
    pub invite_code: Option<String>,
    pub image_urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorSignal {
    pub trigger_type: BehaviorTriggerType,
    pub severity: BehaviorSeverity,
    pub action_hint: BehaviorActionHint,
    pub reason_code: String,
    pub account_is_new: bool,
    pub evidence: BehaviorEvidence,
    pub messages: Vec<RecentMessage>,
}

impl BehaviorSignal {
    pub fn source_label(&self) -> &'static str {
        "behavior"
    }

    pub fn trigger_label(&self) -> &'static str {
        self.trigger_type.as_label()
    }
}

/// Ergebnis einer Detektor-Pruefung. `Cleanup` markiert eine Nachricht, die zu einer
/// bereits geahndeten Welle gehoert und nur noch geloescht werden soll: kein Case, keine
/// Sanktion, kein Spiegeln.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectOutcome {
    Ignore,
    Signal(Box<BehaviorSignal>),
    Cleanup,
}

impl DetectOutcome {
    pub fn into_signal(self) -> Option<BehaviorSignal> {
        match self {
            DetectOutcome::Signal(signal) => Some(*signal),
            DetectOutcome::Ignore | DetectOutcome::Cleanup => None,
        }
    }
}

#[async_trait::async_trait]
pub trait BehaviorDetectorPort: Send + Sync {
    async fn resolve_invite_guild(&self, code: &str) -> Option<u64>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorDetectorConfig {
    pub case_cooldown_seconds: i64,
}

impl Default for BehaviorDetectorConfig {
    fn default() -> Self {
        Self {
            case_cooldown_seconds: CASE_COOLDOWN_SECONDS,
        }
    }
}

pub struct BehaviorDetector {
    port: Arc<dyn BehaviorDetectorPort>,
    config: BehaviorDetectorConfig,
    history: tokio::sync::Mutex<HashMap<u64, VecDeque<RecentMessage>>>,
    active: tokio::sync::Mutex<HashSet<u64>>,
    suppressed_until: tokio::sync::Mutex<HashMap<u64, i64>>,
    cleanup_until: tokio::sync::Mutex<HashMap<u64, i64>>,
    cleanup_deleted: tokio::sync::Mutex<HashMap<u64, u32>>,
}

impl BehaviorDetector {
    pub fn new(port: Arc<dyn BehaviorDetectorPort>) -> Arc<Self> {
        Self::new_with_config(port, BehaviorDetectorConfig::default())
    }

    pub fn new_with_config(
        port: Arc<dyn BehaviorDetectorPort>,
        config: BehaviorDetectorConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            port,
            config,
            history: tokio::sync::Mutex::new(HashMap::new()),
            active: tokio::sync::Mutex::new(HashSet::new()),
            suppressed_until: tokio::sync::Mutex::new(HashMap::new()),
            cleanup_until: tokio::sync::Mutex::new(HashMap::new()),
            cleanup_deleted: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub async fn detect(
        self: &Arc<Self>,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
    ) -> DetectOutcome {
        if !event.author_staff_status_known || event.author_is_staff {
            return DetectOutcome::Ignore;
        }
        let now = chrono::Utc::now().timestamp();
        // Restwelle: nach einem Takeover-Treffer bereits geposteten Nachrichten desselben
        // Kontos werden nur noch geloescht, ohne neuen Case oder neue Sanktion. Diese
        // Pruefung liegt vor is_suppressed, weil der Treffer beide Fenster setzt und das
        // Cleanup-Fenster das kuerzere ist.
        if self.in_cleanup(event.author_id, now).await {
            return DetectOutcome::Cleanup;
        }
        if self.is_suppressed(event.author_id, now).await {
            return DetectOutcome::Ignore;
        }

        let recent = self.record_recent(event, now).await;
        if !self.try_claim(event.author_id).await {
            return DetectOutcome::Ignore;
        }
        let signal = self.run_detection(guild_id, event, &recent, now).await;
        self.release(event.author_id).await;
        match signal {
            Some(signal) => {
                // Das Cleanup-Fenster wird bewusst NICHT hier beim Erkennen armiert.
                // moderation_system armiert es erst nach erfolgreicher Durchsetzung
                // (Case persistiert, enforce aktiv, Sanktion/Delete gelaufen) ueber
                // arm_takeover_cleanup. So verschwindet im Shadow-Modus oder bei
                // gescheitertem Case keine Folgenachricht spurlos.
                self.history.lock().await.remove(&event.author_id);
                self.suppress_user(event.author_id, now).await;
                DetectOutcome::Signal(Box::new(signal))
            }
            None => DetectOutcome::Ignore,
        }
    }

    async fn record_recent(
        &self,
        event: &dl_discord::MessageEvent,
        now: i64,
    ) -> Vec<RecentMessage> {
        let mut history = self.history.lock().await;
        prune_history(&mut history, now, event.author_id);
        let recent = {
            let entry = history.entry(event.author_id).or_default();
            entry.push_back(RecentMessage {
                channel_id: event.channel_id,
                message_id: event.message_id,
                created_at: now,
                content: event.content.clone(),
                attachment_count: event.attachment_count,
                image_count: event.image_attachment_count,
                image_urls: event.image_attachment_urls.clone(),
            });
            while entry.len() > HISTORY_MAX {
                entry.pop_front();
            }
            let cutoff = now - WINDOW_SECONDS;
            entry.retain(|message| message.created_at >= cutoff);
            entry.iter().cloned().collect()
        };
        while history.len() > HISTORY_USER_MAX {
            if !evict_oldest_history(&mut history, event.author_id) {
                break;
            }
        }
        recent
    }

    async fn try_claim(&self, user_id: u64) -> bool {
        self.active.lock().await.insert(user_id)
    }

    async fn release(&self, user_id: u64) {
        self.active.lock().await.remove(&user_id);
    }

    async fn is_suppressed(&self, user_id: u64, now: i64) -> bool {
        let mut suppressed = self.suppressed_until.lock().await;
        suppressed.retain(|_, until| *until > now);
        suppressed.get(&user_id).is_some_and(|until| *until > now)
    }

    async fn suppress_user(&self, user_id: u64, now: i64) {
        let mut suppressed = self.suppressed_until.lock().await;
        suppressed.retain(|_, until| *until > now);
        suppressed.insert(user_id, now + self.config.case_cooldown_seconds);
        while suppressed.len() > SUPPRESSED_USER_MAX {
            let Some(victim) = suppressed
                .iter()
                .min_by_key(|(user_id, until)| (*until, *user_id))
                .map(|(user_id, _)| *user_id)
            else {
                break;
            };
            suppressed.remove(&victim);
        }
    }

    async fn in_cleanup(&self, user_id: u64, now: i64) -> bool {
        let mut cleanup = self.cleanup_until.lock().await;
        cleanup.retain(|_, until| *until > now);
        cleanup.get(&user_id).is_some_and(|until| *until > now)
    }

    /// Armiert das Cleanup-Fenster fuer ein Konto, dessen Takeover gerade durchgesetzt
    /// wurde. Wird bewusst NICHT vom Detektor beim Erkennen aufgerufen, sondern erst von
    /// moderation_system nach erfolgreicher Durchsetzung (Case persistiert, enforce aktiv,
    /// Sanktion/Delete gelaufen). So verschwinden bei gescheitertem Case oder im
    /// Shadow-Modus keine Folgenachrichten spurlos.
    pub async fn arm_takeover_cleanup(&self, user_id: u64) {
        let now = chrono::Utc::now().timestamp();
        let mut cleanup = self.cleanup_until.lock().await;
        cleanup.retain(|_, until| *until > now);
        cleanup.insert(user_id, now + TAKEOVER_CLEANUP_SECONDS);
        while cleanup.len() > SUPPRESSED_USER_MAX {
            let Some(victim) = cleanup
                .iter()
                .min_by_key(|(user_id, until)| (*until, *user_id))
                .map(|(user_id, _)| *user_id)
            else {
                break;
            };
            cleanup.remove(&victim);
        }
        // Zaehler der Welle zuruecksetzen und auf die noch aktiven Konten eindampfen.
        let mut deleted = self.cleanup_deleted.lock().await;
        deleted.retain(|user, _| cleanup.contains_key(user));
        deleted.insert(user_id, 0);
    }

    /// Zaehlt eine erfolgte Cleanup-Loeschung dieser Welle und liefert den laufenden Stand.
    /// Nur erfolgreiche Loeschungen erhoehen den Zaehler, damit die Zahl die real
    /// entfernten Nachrichten der Welle widerspiegelt.
    pub async fn note_cleanup_delete(&self, user_id: u64, deleted_ok: bool) -> u32 {
        let mut deleted = self.cleanup_deleted.lock().await;
        let entry = deleted.entry(user_id).or_insert(0);
        if deleted_ok {
            *entry = entry.saturating_add(1);
        }
        *entry
    }

    pub async fn clear_user_after_moderator_reversal(&self, user_id: u64) {
        self.history.lock().await.remove(&user_id);
        self.active.lock().await.remove(&user_id);
        self.suppressed_until.lock().await.remove(&user_id);
        self.cleanup_until.lock().await.remove(&user_id);
        self.cleanup_deleted.lock().await.remove(&user_id);
    }

    #[cfg(test)]
    pub async fn cleanup_armed(&self, user_id: u64) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.cleanup_until
            .lock()
            .await
            .get(&user_id)
            .is_some_and(|until| *until > now)
    }

    async fn foreign_invite_code(&self, guild_id: u64, content: &str) -> Option<String> {
        for code in extract_invite_codes(content) {
            match self.port.resolve_invite_guild(&code).await {
                Some(invite_guild_id) if invite_guild_id == guild_id => {}
                Some(_) => return Some(code),
                None => {
                    tracing::debug!(
                        guild_id,
                        %code,
                        "BehaviorDetector: invite unresolved; not treated as foreign"
                    );
                }
            }
        }
        None
    }

    async fn run_detection(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        recent: &[RecentMessage],
        now: i64,
    ) -> Option<BehaviorSignal> {
        if let Some((_reason, burst)) = detect_takeover(recent, now) {
            return Some(build_signal(
                event,
                now,
                BehaviorTriggerType::AccountTakeover,
                BehaviorSeverity::Critical,
                takeover_action_hint(event.author_created_at, event.author_joined_at, now),
                "behavior:account_takeover",
                TAKEOVER_WINDOW_SECONDS,
                burst,
                None,
            ));
        }

        if let Some(code) = self.foreign_invite_code(guild_id, &event.content).await {
            return Some(build_signal(
                event,
                now,
                BehaviorTriggerType::ForeignInvite,
                BehaviorSeverity::Suspicious,
                BehaviorActionHint::Proposal,
                "behavior:foreign_invite",
                WINDOW_SECONDS,
                vec![event_to_recent(event, now)],
                Some(code),
            ));
        }

        if should_trigger(recent, now).is_some() {
            // A burst is only spam when it happens fast: many channels within a
            // short window. should_trigger already restricts to BURST_WINDOW_SECONDS,
            // so any hit here is a genuine fast multi-channel burst. Established
            // and possibly compromised accounts count too, not just new ones.
            let burst_cutoff = now - BURST_WINDOW_SECONDS;
            let burst_window: Vec<RecentMessage> = recent
                .iter()
                .filter(|message| message.created_at >= burst_cutoff)
                .cloned()
                .collect();
            let trigger = if is_new_account(event.author_created_at, event.author_joined_at, now) {
                BehaviorTriggerType::YoungAccountBurst
            } else {
                BehaviorTriggerType::BurstRate
            };
            return Some(build_signal(
                event,
                now,
                trigger,
                BehaviorSeverity::Suspicious,
                BehaviorActionHint::Proposal,
                "behavior:burst_rate",
                BURST_WINDOW_SECONDS,
                burst_window,
                None,
            ));
        }

        if is_image_multi_channel(recent, now) {
            return Some(build_signal(
                event,
                now,
                BehaviorTriggerType::ImageMultichannel,
                BehaviorSeverity::Suspicious,
                BehaviorActionHint::Proposal,
                "behavior:image_multichannel",
                IMAGE_MULTICHANNEL_WINDOW_SECONDS,
                recent.to_vec(),
                None,
            ));
        }

        if contains_suspicious_text(&event.content) {
            return Some(build_signal(
                event,
                now,
                BehaviorTriggerType::Keyword,
                BehaviorSeverity::Suspicious,
                BehaviorActionHint::Proposal,
                "behavior:keyword",
                WINDOW_SECONDS,
                vec![event_to_recent(event, now)],
                None,
            ));
        }

        None
    }
}

fn event_to_recent(event: &dl_discord::MessageEvent, now: i64) -> RecentMessage {
    RecentMessage {
        channel_id: event.channel_id,
        message_id: event.message_id,
        created_at: now,
        content: event.content.clone(),
        attachment_count: event.attachment_count,
        image_count: event.image_attachment_count,
        image_urls: event.image_attachment_urls.clone(),
    }
}

fn prune_history(
    history: &mut HashMap<u64, VecDeque<RecentMessage>>,
    now: i64,
    protected_user_id: u64,
) {
    let cutoff = now - WINDOW_SECONDS;
    history.retain(|_, messages| {
        messages.retain(|message| message.created_at >= cutoff);
        !messages.is_empty()
    });
    while history.len() >= HISTORY_USER_MAX && !history.contains_key(&protected_user_id) {
        if !evict_oldest_history(history, protected_user_id) {
            break;
        }
    }
}

fn evict_oldest_history(
    history: &mut HashMap<u64, VecDeque<RecentMessage>>,
    protected_user_id: u64,
) -> bool {
    let victim = history
        .iter()
        .filter(|(user_id, _)| **user_id != protected_user_id)
        .min_by_key(|(user_id, messages)| {
            (
                messages
                    .back()
                    .map(|message| message.created_at)
                    .unwrap_or(i64::MIN),
                **user_id,
            )
        })
        .map(|(user_id, _)| *user_id);
    if let Some(user_id) = victim {
        history.remove(&user_id);
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn build_signal(
    event: &dl_discord::MessageEvent,
    now: i64,
    trigger_type: BehaviorTriggerType,
    severity: BehaviorSeverity,
    action_hint: BehaviorActionHint,
    reason_code: &str,
    window_seconds: i64,
    messages: Vec<RecentMessage>,
    invite_code: Option<String>,
) -> BehaviorSignal {
    let account_age_hours = ((now - event.author_created_at).max(0)) / 3600;
    let join_age_hours = event
        .author_joined_at
        .map(|joined_at| ((now - joined_at).max(0)) / 3600);
    BehaviorSignal {
        trigger_type,
        severity,
        action_hint,
        reason_code: reason_code.to_string(),
        account_is_new: is_new_account(event.author_created_at, event.author_joined_at, now),
        evidence: build_evidence(
            window_seconds,
            &messages,
            event.message_id,
            account_age_hours,
            join_age_hours,
            invite_code,
        ),
        messages,
    }
}

fn build_evidence(
    window_seconds: i64,
    messages: &[RecentMessage],
    event_message_id: u64,
    account_age_hours: i64,
    join_age_hours: Option<i64>,
    invite_code: Option<String>,
) -> BehaviorEvidence {
    let mut channel_ids = messages
        .iter()
        .map(|message| message.channel_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    channel_ids.sort_unstable();
    let message_ids = messages
        .iter()
        .map(|message| message.message_id)
        .collect::<Vec<_>>();
    let attachment_count = messages
        .iter()
        .map(|message| message.attachment_count)
        .sum();
    let image_count = messages.iter().map(|message| message.image_count).sum();
    let keyword_hit = messages
        .iter()
        .any(|message| contains_suspicious_text(&message.content));
    let image_urls = single_message_image_urls(messages, event_message_id);

    BehaviorEvidence {
        window_seconds,
        channel_ids,
        message_ids,
        message_count: messages.len(),
        attachment_count,
        image_count,
        keyword_hit,
        account_age_hours,
        join_age_hours,
        invite_code,
        image_urls,
    }
}

/// Bilder GENAU EINER Nachricht als Beweis. Bevorzugt die ausloesende Event-Nachricht;
/// hat sie keine Bilder, die bildreichste Wellennachricht. So entsteht kein Mix ueber
/// mehrere Nachrichten. moderation_system haengt die Anhaenge der Event-Nachricht ohnehin
/// an, deshalb bleibt es bei der Wahl der Event-Nachricht genau eine Nachricht.
fn single_message_image_urls(messages: &[RecentMessage], event_message_id: u64) -> Vec<String> {
    let chosen = messages
        .iter()
        .find(|message| message.message_id == event_message_id && !message.image_urls.is_empty())
        .or_else(|| {
            messages
                .iter()
                .filter(|message| !message.image_urls.is_empty())
                .max_by_key(|message| message.image_urls.len())
        });
    let Some(message) = chosen else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    message
        .image_urls
        .iter()
        .filter(|url| seen.insert((*url).clone()))
        .take(EVIDENCE_IMAGE_LIMIT)
        .cloned()
        .collect()
}

fn takeover_action_hint(created_at: i64, joined_at: Option<i64>, now: i64) -> BehaviorActionHint {
    if is_new_account(created_at, joined_at, now) {
        BehaviorActionHint::Ban
    } else {
        BehaviorActionHint::Timeout
    }
}

pub fn contains_suspicious_text(text: &str) -> bool {
    static KEYWORD_RE: OnceLock<Option<Regex>> = OnceLock::new();
    let re = KEYWORD_RE.get_or_init(|| {
        let alternatives = KEYWORDS
            .iter()
            .map(|keyword| regex::escape(keyword))
            .collect::<Vec<_>>()
            .join("|");
        Regex::new(&format!("(?i)({alternatives})")).ok()
    });
    if let Some(re) = re {
        return re.is_match(text);
    }
    let lower = text.to_lowercase();
    KEYWORDS.iter().any(|keyword| lower.contains(keyword))
}

pub fn extract_invite_codes(text: &str) -> Vec<String> {
    static INVITE_RE: OnceLock<Option<Regex>> = OnceLock::new();
    let re = INVITE_RE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (?:^|[^A-Z0-9_.-])
            (?:https?://)?
            (?:(?:ptb|canary)\.)?
            (?:
                discord\.gg/
                |
                discord(?:app)?\.com/invite/
            )
            ([A-Z0-9-]+)
            ",
        )
        .ok()
    });
    let Some(re) = re else {
        tracing::warn!("BehaviorDetector: invite regex unavailable");
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut codes = Vec::new();
    for caps in re.captures_iter(text) {
        let Some(code) = caps.get(1).map(|m| m.as_str().to_string()) else {
            continue;
        };
        if seen.insert(code.clone()) {
            codes.push(code);
        }
    }
    codes
}

pub fn detect_takeover(
    messages: &[RecentMessage],
    now: i64,
) -> Option<(String, Vec<RecentMessage>)> {
    // Ausloesekriterium bleibt eng: Bilder in mindestens zwei Kanaelen innerhalb von 30 s.
    let trigger_cutoff = now - TAKEOVER_WINDOW_SECONDS;
    let mut image_channels: HashMap<u64, u32> = HashMap::new();
    let mut image_count = 0u32;
    for message in messages
        .iter()
        .filter(|message| message.created_at >= trigger_cutoff)
    {
        if message.image_count > 0 {
            *image_channels.entry(message.channel_id).or_default() += message.image_count;
            image_count += message.image_count;
        }
    }
    if image_channels.len() < TAKEOVER_IMAGE_CHANNELS {
        return None;
    }
    // Loesch- und Beweisziel ist die ganze erfasste Welle, nicht nur das 30-s-Fenster:
    // die Welle postet ueber laenger als 30 s, und alle diese Nachrichten sollen weg.
    let wave_cutoff = now - TAKEOVER_WAVE_SECONDS;
    let wave = messages
        .iter()
        .filter(|message| message.created_at >= wave_cutoff)
        .cloned()
        .collect::<Vec<_>>();
    Some((
        format!(
            "account_takeover:{image_count}:{}:{TAKEOVER_WINDOW_SECONDS}",
            image_channels.len()
        ),
        wave,
    ))
}

pub fn should_trigger(messages: &[RecentMessage], now: i64) -> Option<(String, [i64; 4])> {
    let cutoff = now - BURST_WINDOW_SECONDS;
    let windowed: Vec<&RecentMessage> = messages
        .iter()
        .filter(|message| message.created_at >= cutoff)
        .collect();
    if windowed.is_empty() {
        return None;
    }
    let unique_channels = windowed
        .iter()
        .map(|message| message.channel_id)
        .collect::<HashSet<_>>();
    let total = windowed.len();
    let attachment_count = windowed
        .iter()
        .filter(|message| message.attachment_count > 0)
        .count();
    let attachment_channels = windowed
        .iter()
        .filter(|message| message.attachment_count > 0)
        .map(|message| message.channel_id)
        .collect::<HashSet<_>>();
    let keyword_hit = windowed
        .iter()
        .any(|message| contains_suspicious_text(&message.content));

    let multi_channel_burst =
        unique_channels.len() >= CHANNEL_THRESHOLD && total >= MESSAGE_THRESHOLD;
    let two_channel_sus =
        unique_channels.len() >= 2 && total >= 2 && (keyword_hit || attachment_count > 0);
    let attachment_multi_channel = attachment_channels.len() >= 2;

    if !(multi_channel_burst || two_channel_sus || attachment_multi_channel) {
        return None;
    }

    Some((
        "behavior:burst_rate".to_string(),
        [
            unique_channels.len() as i64,
            total as i64,
            attachment_count as i64,
            i64::from(keyword_hit),
        ],
    ))
}

pub fn is_image_multi_channel(messages: &[RecentMessage], now: i64) -> bool {
    let cutoff = now - IMAGE_MULTICHANNEL_WINDOW_SECONDS;
    let channels_with_images = messages
        .iter()
        .filter(|message| message.image_count > 0 && message.created_at >= cutoff)
        .map(|message| message.channel_id)
        .collect::<HashSet<_>>();
    channels_with_images.len() >= IMAGE_CHANNEL_THRESHOLD
}

pub fn is_new_account(created_at: i64, joined_at: Option<i64>, now: i64) -> bool {
    let account_new = now - created_at < NEW_ACCOUNT_MAX_AGE_HOURS * 3600;
    let joined_new = joined_at
        .map(|joined| now - joined < NEW_MEMBER_MAX_JOIN_HOURS * 3600)
        .unwrap_or(false);
    account_new && joined_new
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    fn recent(
        channel_id: u64,
        message_id: u64,
        secs_ago: i64,
        content: &str,
        images: u32,
    ) -> RecentMessage {
        RecentMessage {
            channel_id,
            message_id,
            created_at: 1_000_000 - secs_ago,
            content: content.to_string(),
            attachment_count: images,
            image_count: images,
            image_urls: (0..images)
                .map(|idx| format!("https://img/{message_id}-{idx}.png"))
                .collect(),
        }
    }

    fn event(
        user_id: u64,
        channel_id: u64,
        message_id: u64,
        content: &str,
        created_at: i64,
        joined_at: Option<i64>,
    ) -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id: Some(1),
            channel_id,
            message_id,
            author_id: user_id,
            author_display_name: format!("user-{user_id}"),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.to_string(),
            message_created_at: chrono::Utc::now().timestamp(),
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: created_at,
            author_joined_at: joined_at,
        }
    }

    fn image_event(
        user_id: u64,
        channel_id: u64,
        message_id: u64,
        created_at: i64,
        joined_at: Option<i64>,
    ) -> dl_discord::MessageEvent {
        let mut event = event(user_id, channel_id, message_id, "", created_at, joined_at);
        event.attachment_count = 1;
        event.image_attachment_count = 1;
        event.image_attachment_urls = vec![format!("https://img/{message_id}.png")];
        event
    }

    #[derive(Default)]
    struct FakePort {
        resolves: Mutex<HashMap<String, Option<u64>>>,
    }

    #[async_trait::async_trait]
    impl BehaviorDetectorPort for FakePort {
        async fn resolve_invite_guild(&self, code: &str) -> Option<u64> {
            self.resolves.lock().await.get(code).copied().flatten()
        }
    }

    #[test]
    fn invite_extraction_detects_real_invite_urls_only() {
        let text = "\
            discord.gg/Vanity-Code \
            https://discord.com/invite/AbC123?utm=1 \
            http://discordapp.com/invite/oldCode. \
            https://ptb.discord.com/invite/PTB \
            https://canary.discord.com/invite/CANARY \
            https://discord.com/channels/1/2/3 \
            https://discord.com/users/42";

        assert_eq!(
            extract_invite_codes(text),
            vec!["Vanity-Code", "AbC123", "oldCode", "PTB", "CANARY"]
        );
        assert!(extract_invite_codes("https://discord.com/channels/1/2/3").is_empty());
    }

    #[test]
    fn takeover_detection_requires_images_in_two_channels_within_30s() {
        let now = 1_000_000;
        assert!(
            detect_takeover(&[recent(1, 10, 10, "", 1), recent(2, 11, 5, "", 1)], now).is_some()
        );
        assert!(
            detect_takeover(&[recent(1, 10, 10, "", 1), recent(1, 11, 5, "", 2)], now).is_none()
        );
        assert!(
            detect_takeover(&[recent(1, 10, 60, "", 1), recent(2, 11, 5, "", 1)], now).is_none()
        );
    }

    #[test]
    fn burst_trigger_keeps_channel_message_attachment_keyword_meta() {
        let now = 1_000_000;
        let messages = vec![
            recent(1, 10, 30, "join my telegram", 0),
            recent(2, 11, 20, "hi", 0),
        ];
        let (_reason, meta) = should_trigger(&messages, now).expect("trigger");

        assert_eq!(meta, [2, 2, 0, 1]);
        assert!(should_trigger(
            &[recent(1, 10, 30, "hi", 0), recent(2, 11, 20, "ho", 0)],
            now
        )
        .is_none());
    }

    #[test]
    fn image_multichannel_uses_five_minute_window() {
        let now = 1_000_000;
        assert!(is_image_multi_channel(
            &[recent(1, 10, 30, "", 1), recent(2, 11, 10, "", 1)],
            now
        ));
        assert!(!is_image_multi_channel(
            &[recent(1, 10, 301, "", 1), recent(2, 11, 10, "", 1)],
            now
        ));
    }

    #[test]
    fn should_trigger_ignores_activity_spread_beyond_burst_window() {
        let now = 1_000_000;
        assert!(should_trigger(
            &[
                recent(1, 10, 200, "hi", 0),
                recent(2, 11, 120, "ho", 0),
                recent(3, 12, 0, "he", 0),
            ],
            now
        )
        .is_none());
        assert!(should_trigger(
            &[
                recent(1, 10, 40, "hi", 0),
                recent(2, 11, 20, "ho", 0),
                recent(3, 12, 0, "he", 0),
            ],
            now
        )
        .is_some());
    }

    #[test]
    fn should_trigger_attachment_multichannel_respects_burst_window() {
        let now = 1_000_000;
        assert!(
            should_trigger(&[recent(1, 10, 200, "", 1), recent(2, 11, 0, "", 1)], now).is_none()
        );
        assert!(
            should_trigger(&[recent(1, 10, 40, "", 1), recent(2, 11, 0, "", 1)], now).is_some()
        );
    }

    #[tokio::test]
    async fn detector_returns_takeover_signal_once_and_suppresses_followups() {
        let now = chrono::Utc::now().timestamp();
        let detector = BehaviorDetector::new(Arc::new(FakePort::default()));
        let created_at = now - 10 * 3600;
        let joined_at = Some(now - 3600);

        assert_eq!(
            detector
                .detect(1, &image_event(100, 10, 1000, created_at, joined_at))
                .await,
            DetectOutcome::Ignore
        );
        let signal = detector
            .detect(1, &image_event(100, 11, 1001, created_at, joined_at))
            .await
            .into_signal()
            .expect("takeover signal");

        assert_eq!(signal.trigger_type, BehaviorTriggerType::AccountTakeover);
        assert_eq!(signal.severity, BehaviorSeverity::Critical);
        assert_eq!(signal.action_hint, BehaviorActionHint::Ban);
        assert_eq!(signal.evidence.channel_ids, vec![10, 11]);
        assert_eq!(signal.evidence.image_count, 2);
        // Folgenachricht derselben Welle: solange die Durchsetzung das Cleanup-Fenster
        // NICHT armiert hat, bleibt sie schlicht unterdrueckt (Ignore). Der Detektor
        // armiert nicht mehr selbst beim Erkennen.
        assert_eq!(
            detector
                .detect(1, &image_event(100, 12, 1002, created_at, joined_at))
                .await,
            DetectOutcome::Ignore
        );
        // Erst nachdem moderation_system den Takeover durchgesetzt und das Fenster
        // armiert hat, wird die Restwelle geloescht (Cleanup), kein neuer Case.
        detector.arm_takeover_cleanup(100).await;
        assert_eq!(
            detector
                .detect(1, &image_event(100, 13, 1003, created_at, joined_at))
                .await,
            DetectOutcome::Cleanup
        );
    }

    #[tokio::test]
    async fn established_account_fast_multichannel_burst_triggers_burst_rate() {
        let detector = BehaviorDetector::new(Arc::new(FakePort::default()));
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 67508 * 3600;
        let joined_at = Some(now - 6317 * 3600);
        let channels = [10, 11, 12];
        let mut last = DetectOutcome::Ignore;

        for (idx, channel_id) in channels.into_iter().enumerate() {
            last = detector
                .detect(
                    1,
                    &event(
                        300,
                        channel_id,
                        4000 + idx as u64,
                        "hello everyone",
                        created_at,
                        joined_at,
                    ),
                )
                .await;
        }

        let sig = last
            .into_signal()
            .expect("established account fast multichannel burst signal");
        assert_eq!(sig.trigger_type, BehaviorTriggerType::BurstRate);
    }

    #[tokio::test]
    async fn new_account_plain_multichannel_burst_still_triggers_young_burst() {
        let detector = BehaviorDetector::new(Arc::new(FakePort::default()));
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 10 * 3600;
        let joined_at = Some(now - 2 * 3600);
        let channels = [10, 11, 12];
        let mut last = DetectOutcome::Ignore;

        for (idx, channel_id) in channels.into_iter().enumerate() {
            last = detector
                .detect(
                    1,
                    &event(
                        200,
                        channel_id,
                        3000 + idx as u64,
                        "hello everyone",
                        created_at,
                        joined_at,
                    ),
                )
                .await;
        }

        let sig = last
            .into_signal()
            .expect("new account plain multichannel burst signal");
        assert_eq!(sig.trigger_type, BehaviorTriggerType::YoungAccountBurst);
    }

    #[tokio::test]
    async fn detector_history_stays_bounded_for_many_one_shot_users() {
        let detector = BehaviorDetector::new(Arc::new(FakePort::default()));
        let now = chrono::Utc::now().timestamp();
        let created_at = now - 90 * 24 * 3600;
        let joined_at = Some(now - 30 * 24 * 3600);

        for idx in 0..160 {
            detector
                .detect(
                    1,
                    &event(
                        10_000 + idx,
                        20,
                        30_000 + idx,
                        "normale nachricht",
                        created_at,
                        joined_at,
                    ),
                )
                .await;
        }

        assert!(detector.history.lock().await.len() <= 128);
    }
}
