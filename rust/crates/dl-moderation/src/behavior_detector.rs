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
pub const CHANNEL_THRESHOLD: usize = 3;
pub const MESSAGE_THRESHOLD: usize = 3;
pub const NEW_ACCOUNT_MAX_AGE_HOURS: i64 = 720;
pub const NEW_MEMBER_MAX_JOIN_HOURS: i64 = 168;
pub const IMAGE_CHANNEL_THRESHOLD: usize = 2;
pub const IMAGE_MULTICHANNEL_WINDOW_SECONDS: i64 = 300;
pub const TAKEOVER_WINDOW_SECONDS: i64 = 30;
pub const TAKEOVER_IMAGE_CHANNELS: usize = 2;
pub const TIMEOUT_MINUTES: i64 = 1440;
pub const PROPOSAL_TIMEOUT_MINUTES: i64 = 60;
pub const CASE_COOLDOWN_SECONDS: i64 = 600;
pub const HISTORY_MAX: usize = 20;
pub const HISTORY_USER_MAX: usize = 128;
pub const SUPPRESSED_USER_MAX: usize = 512;
pub const EVIDENCE_IMAGE_LIMIT: usize = 4;

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
        })
    }

    pub async fn detect(
        self: &Arc<Self>,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
    ) -> Option<BehaviorSignal> {
        if !event.author_staff_status_known || event.author_is_staff {
            return None;
        }
        let now = chrono::Utc::now().timestamp();
        if self.is_suppressed(event.author_id, now).await {
            return None;
        }

        let recent = self.record_recent(event, now).await;
        if !self.try_claim(event.author_id).await {
            return None;
        }
        let signal = self.run_detection(guild_id, event, &recent, now).await;
        self.release(event.author_id).await;
        if signal.is_some() {
            self.history.lock().await.remove(&event.author_id);
            self.suppress_user(event.author_id, now).await;
        }
        signal
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

        if should_trigger(recent).is_some() {
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
                WINDOW_SECONDS,
                recent.to_vec(),
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
    let mut seen_images = HashSet::new();
    let image_urls = messages
        .iter()
        .flat_map(|message| message.image_urls.iter())
        .filter(|url| seen_images.insert((*url).clone()))
        .take(EVIDENCE_IMAGE_LIMIT)
        .cloned()
        .collect();

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
    let cutoff = now - TAKEOVER_WINDOW_SECONDS;
    let window = messages
        .iter()
        .filter(|message| message.created_at >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let mut image_channels: HashMap<u64, u32> = HashMap::new();
    let mut image_count = 0u32;
    for message in &window {
        if message.image_count > 0 {
            *image_channels.entry(message.channel_id).or_default() += message.image_count;
            image_count += message.image_count;
        }
    }
    if image_channels.len() < TAKEOVER_IMAGE_CHANNELS {
        return None;
    }
    Some((
        format!(
            "account_takeover:{image_count}:{}:{TAKEOVER_WINDOW_SECONDS}",
            image_channels.len()
        ),
        window,
    ))
}

pub fn should_trigger(messages: &[RecentMessage]) -> Option<(String, [i64; 4])> {
    if messages.is_empty() {
        return None;
    }
    let unique_channels = messages
        .iter()
        .map(|message| message.channel_id)
        .collect::<HashSet<_>>();
    let total = messages.len();
    let attachment_count = messages
        .iter()
        .filter(|message| message.attachment_count > 0)
        .count();
    let attachment_channels = messages
        .iter()
        .filter(|message| message.attachment_count > 0)
        .map(|message| message.channel_id)
        .collect::<HashSet<_>>();
    let keyword_hit = messages
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
        let messages = vec![
            recent(1, 10, 30, "join my telegram", 0),
            recent(2, 11, 20, "hi", 0),
        ];
        let (_reason, meta) = should_trigger(&messages).expect("trigger");

        assert_eq!(meta, [2, 2, 0, 1]);
        assert!(
            should_trigger(&[recent(1, 10, 30, "hi", 0), recent(2, 11, 20, "ho", 0)]).is_none()
        );
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

    #[tokio::test]
    async fn detector_returns_takeover_signal_once_and_suppresses_followups() {
        let now = chrono::Utc::now().timestamp();
        let detector = BehaviorDetector::new(Arc::new(FakePort::default()));
        let created_at = now - 10 * 3600;
        let joined_at = Some(now - 3600);

        assert!(detector
            .detect(1, &image_event(100, 10, 1000, created_at, joined_at))
            .await
            .is_none());
        let signal = detector
            .detect(1, &image_event(100, 11, 1001, created_at, joined_at))
            .await
            .expect("takeover signal");

        assert_eq!(signal.trigger_type, BehaviorTriggerType::AccountTakeover);
        assert_eq!(signal.severity, BehaviorSeverity::Critical);
        assert_eq!(signal.action_hint, BehaviorActionHint::Ban);
        assert_eq!(signal.evidence.channel_ids, vec![10, 11]);
        assert_eq!(signal.evidence.image_count, 2);
        assert!(detector
            .detect(1, &image_event(100, 12, 1002, created_at, joined_at))
            .await
            .is_none());
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
