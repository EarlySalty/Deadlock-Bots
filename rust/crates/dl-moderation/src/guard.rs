//! SecurityGuard — Port des Kerns von `cogs/security_guard.py`.
//!
//! Detektions-Pfade:
//! 1. **Account-Takeover**: Bilder in ≥ 2 verschiedenen Channels innerhalb
//!    von 30 s → Treffer.
//! 2. **Fremd-Invite**: echte Discord-Invite-Links werden per Port auf die
//!    Ziel-Guild aufgelöst; nur eindeutig fremde Guilds → Treffer.
//! 3. **Junge Accounts**: Mehrkanal-Burst → Text-/Bild-Scam-Check; bestätigt
//!    = Treffer, unbestätigt = 60-min-Holding-Timeout als Mod-Vorschlag.
//! 4. **Bild-Multichannel** und **Keyword-Einzeltreffer**: AI-bestätigte Scam-
//!    Signale → Treffer.
//!
//! Treffer laufen durch das vereinheitlichte Modell: ban-fähig neu (<30 d
//! Account und <7 d Server) → Ban; etablierter Einzelchannel → Soft-Warn;
//! etablierte Streuung oder Takeover → Hijack-Timeout.
//!
//! Takeover-Bild-Label (Original: `_finalize_takeover_ai_label`): Bei einem
//! Takeover mit Bild-Anhängen holt der Guard best-effort ein Vision-
//! Label (`generate_multimodal`) und hängt es als Mod-Kontext an den Grund —
//! KEIN Gate, die Quarantäne bleibt deterministisch.
//!
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;
use sqlx::PgPool;

pub const REVIEW_CHANNEL_ID: u64 = 1374364800817303632;
pub const MOD_CHANNEL_ID: u64 = 1315684135175716978;
pub const WINDOW_SECONDS: i64 = 3600;
pub const CHANNEL_THRESHOLD: usize = 3;
pub const MESSAGE_THRESHOLD: usize = 3;
pub const NEW_ACCOUNT_MAX_AGE_HOURS: i64 = 720;
pub const NEW_MEMBER_MAX_JOIN_HOURS: i64 = 168;
pub const ACCOUNT_MAX_AGE_HOURS: i64 = NEW_ACCOUNT_MAX_AGE_HOURS;
pub const ESTABLISHED_ACCOUNT_MIN_AGE_HOURS: i64 = NEW_ACCOUNT_MAX_AGE_HOURS;
pub const ESTABLISHED_MIN_JOIN_HOURS: i64 = NEW_MEMBER_MAX_JOIN_HOURS;
pub const AI_SCAM_CONFIDENCE: f64 = 0.78;
pub const AI_IMAGE_CONFIDENCE: f64 = 0.75;
pub const IMAGE_CHANNEL_THRESHOLD: usize = 2;
pub const IMAGE_MULTICHANNEL_WINDOW_SECONDS: i64 = 300;
pub const TAKEOVER_WINDOW_SECONDS: i64 = 30;
pub const TAKEOVER_IMAGE_CHANNELS: usize = 2;
pub const TIMEOUT_MINUTES: i64 = 1440;
pub const PROPOSAL_TIMEOUT_MINUTES: i64 = 60;
pub const CASE_COOLDOWN_SECONDS: i64 = 600;
pub const HISTORY_MAX: usize = 20;
pub const EVIDENCE_IMAGE_LIMIT: usize = 4;
/// Einspruch-Modal-Grenzen (Original: APPEAL_MIN_CHARS / APPEAL_MAX_CHARS).
pub const APPEAL_MIN_CHARS: u16 = 4;
pub const APPEAL_MAX_CHARS: u16 = 800;
pub const DEFAULT_ESCALATION_CONTACT_HANDLE: &str = "@earlysalty";
pub const SOFT_WARN_DELETE_AFTER_SECONDS: u64 = 12;

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

/// System-Prompt für den Text-Scam-Check.
pub const SCAM_DETECTION_SYSTEM_PROMPT: &str =
    "You are a scam detector for a Discord gaming server. \
Decide if the message is financial spam or a scam (earnings promises, investment schemes, \
Telegram/contact requests, profit-sharing, referral schemes, or similar). \
Reply only with valid JSON, no other text: \
{\"is_scam\": true|false, \"confidence\": 0.0-1.0, \"reason\": \"max one sentence, in German\"}";

/// Vision-Prompt für den Bild-Scam-Check. Liefert dasselbe JSON-Schema wie der Text-Scam-Check.
pub const VISION_SCAM_PROMPT: &str =
    "You are a scam detector for a Discord gaming server. Look at the image. \
Decide if it shows financial/crypto/casino/gambling/giveaway scam content \
(fake withdrawals, betting bonuses, promo codes, fake celebrity crypto promos, \
trading/earnings proof). Reply ONLY with valid JSON, no other text: \
{\"is_scam\": true|false, \"confidence\": 0.0-1.0, \"reason\": \"max one sentence, in German\"}";

// ── Pure Detektion ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RecentMsg {
    pub channel_id: u64,
    pub message_id: u64,
    pub created_at: i64,
    pub content: String,
    pub attachment_count: u32,
    pub image_count: u32,
    /// URLs der Bild-Anhänge — für das Takeover-Vision-Label.
    pub image_urls: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EvidenceImage {
    pub filename: String,
    pub data: Vec<u8>,
}

pub fn contains_suspicious_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Extrahiert Discord-Invite-Codes aus echten Invite-URLs.
///
/// Erkannt werden `discord.gg/<code>`, `discord.com/invite/<code>`,
/// `discordapp.com/invite/<code>` sowie `ptb.`/`canary.`-Hosts. Andere
/// Discord-Links wie `/channels/...` werden bewusst ignoriert.
pub fn extract_invite_codes(text: &str) -> Vec<String> {
    static INVITE_RE: OnceLock<Regex> = OnceLock::new();
    let re = INVITE_RE.get_or_init(|| {
        match Regex::new(
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
        ) {
            Ok(re) => re,
            Err(err) => panic!("invalid invite regex: {err}"),
        }
    });
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

/// Takeover-Fingerabdruck: Bilder über ≥ 2 Channels in ≤ 30 s.
pub fn detect_takeover(msgs: &[RecentMsg], now: i64) -> Option<(String, Vec<RecentMsg>)> {
    let cutoff = now - TAKEOVER_WINDOW_SECONDS;
    let window: Vec<RecentMsg> = msgs
        .iter()
        .filter(|m| m.created_at >= cutoff)
        .cloned()
        .collect();
    let mut image_channels: HashMap<u64, u32> = HashMap::new();
    let mut image_count = 0u32;
    for msg in &window {
        if msg.image_count > 0 {
            *image_channels.entry(msg.channel_id).or_default() += msg.image_count;
            image_count += msg.image_count;
        }
    }
    if image_channels.len() < TAKEOVER_IMAGE_CHANNELS {
        return None;
    }
    Some((
        format!(
            "Account-Takeover-Muster: {image_count} Bild(er) in {} Channels in <= {TAKEOVER_WINDOW_SECONDS}s",
            image_channels.len()
        ),
        window,
    ))
}

/// Burst-Trigger für junge Accounts (wie `_should_trigger`).
pub fn should_trigger(msgs: &[RecentMsg]) -> Option<(String, [i64; 4])> {
    if msgs.is_empty() {
        return None;
    }
    let unique_channels: std::collections::HashSet<u64> =
        msgs.iter().map(|m| m.channel_id).collect();
    let total = msgs.len();
    let attachment_count = msgs.iter().filter(|m| m.attachment_count > 0).count();
    let attachment_channels: std::collections::HashSet<u64> = msgs
        .iter()
        .filter(|m| m.attachment_count > 0)
        .map(|m| m.channel_id)
        .collect();
    let keyword_hit = msgs.iter().any(|m| contains_suspicious_text(&m.content));

    let multi_channel_burst =
        unique_channels.len() >= CHANNEL_THRESHOLD && total >= MESSAGE_THRESHOLD;
    let two_channel_sus =
        unique_channels.len() >= 2 && total >= 2 && (keyword_hit || attachment_count > 0);
    let attachment_multi_channel = attachment_channels.len() >= 2;

    if !(multi_channel_burst || two_channel_sus || attachment_multi_channel) {
        return None;
    }
    let mut reason_bits: Vec<String> = Vec::new();
    if multi_channel_burst {
        reason_bits.push("Burst über mehrere Kanäle".to_string());
    }
    if two_channel_sus && !multi_channel_burst {
        reason_bits.push("verdächtige Inhalte in 2+ Kanälen".to_string());
    }
    if attachment_multi_channel && !multi_channel_burst {
        reason_bits.push("Anhänge in 2+ Kanälen".to_string());
    }
    if keyword_hit {
        reason_bits.push("Schlagwort-Treffer".to_string());
    }
    if attachment_count > 0 {
        if attachment_count == 1 {
            reason_bits.push("1 Anhang".to_string());
        } else {
            reason_bits.push(format!("{attachment_count} Anhänge"));
        }
    }
    let reason = if reason_bits.is_empty() {
        "Burst von neuem Account".to_string()
    } else {
        reason_bits.join("; ")
    };
    Some((
        reason,
        [
            unique_channels.len() as i64,
            total as i64,
            attachment_count as i64,
            i64::from(keyword_hit),
        ],
    ))
}

/// Bilder in mindestens `IMAGE_CHANNEL_THRESHOLD` verschiedenen Channels im
/// 5-Minuten-Fenster.
pub fn is_image_multi_channel(msgs: &[RecentMsg], now: i64) -> bool {
    let cutoff = now - IMAGE_MULTICHANNEL_WINDOW_SECONDS;
    let channels_with_images: std::collections::HashSet<u64> = msgs
        .iter()
        .filter(|m| m.image_count > 0 && m.created_at >= cutoff)
        .map(|m| m.channel_id)
        .collect();
    channels_with_images.len() >= IMAGE_CHANNEL_THRESHOLD
}

/// `<think>`-Strip + greedy `\{.*\}` → (is_scam, confidence, reason).
pub fn parse_scam_json(text: Option<&str>) -> (bool, f64, String) {
    let fallback = (false, 0.0, "parse_error".to_string());
    let Some(text) = text else { return fallback };
    let cleaned = dl_ai::strip_think(text);
    let (Some(open), Some(close)) = (cleaned.find('{'), cleaned.rfind('}')) else {
        return fallback;
    };
    if close <= open {
        return fallback;
    }
    let Ok(payload) = serde_json::from_str::<Value>(&cleaned[open..=close]) else {
        return fallback;
    };
    let is_scam = payload
        .get("is_scam")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let confidence = payload
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    (is_scam, confidence, reason)
}

pub fn ban_dm_text(contact_handle: &str) -> String {
    format!(
        "Du wurdest auf der Deutschen Deadlock Community gebannt, weil dein Account ein Scam-/Fremdlink-Muster ausgelöst hat. Wenn du denkst, das ist ein Fehler: Schick {contact_handle} eine Freundschaftsanfrage **und** eine kurze Nachricht — dann schauen wir uns das an."
    )
}

/// Einheitliche Hijack-DM für Takeover-Muster und etablierte gestreute Treffer.
pub fn hijack_dm_text(case_id: &str) -> String {
    format!(
        "Du wurdest auf der Deutschen Deadlock Community vorübergehend stummgeschaltet (24 Stunden).\nGrund: Auf deinem Account wurde eine verdächtige Scam-Nachricht erkannt. Falls dein Account gehackt wurde, melde dich bitte beim Mod-Team, sobald du ihn zurück hast.\nCase: {case_id}\nWende dich an das Mod-Team, sobald dein Account wieder sicher ist."
    )
}

pub fn soft_warn_notice(user_id: u64) -> String {
    format!(
        "<@{user_id}> — fremder Discord-Invite entfernt. Bitte keine fremden Server-Einladungen posten."
    )
}

pub fn is_new_account(created_at: i64, joined_at: Option<i64>, now: i64) -> bool {
    let account_new = now - created_at < NEW_ACCOUNT_MAX_AGE_HOURS * 3600;
    let joined_new = joined_at
        .map(|joined| now - joined < NEW_MEMBER_MAX_JOIN_HOURS * 3600)
        .unwrap_or(false);
    account_new && joined_new
}

pub fn is_established_account(created_at: i64, joined_at: Option<i64>, now: i64) -> bool {
    !is_new_account(created_at, joined_at, now)
}

pub fn distinct_channel_count(msgs: &[RecentMsg]) -> usize {
    msgs.iter()
        .map(|m| m.channel_id)
        .collect::<HashSet<_>>()
        .len()
}

pub fn decide_hit_action(
    created_at: i64,
    joined_at: Option<i64>,
    now: i64,
    window_msgs: &[RecentMsg],
    takeover_pattern: bool,
) -> GuardAction {
    if is_new_account(created_at, joined_at, now) {
        return GuardAction::Enforce;
    }
    if takeover_pattern || distinct_channel_count(window_msgs) >= 2 {
        GuardAction::Hijack
    } else {
        GuardAction::SoftWarn
    }
}

// ── Engine ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAction {
    /// Vollzug (Ban, da PUNISHMENT="ban") + Beweise + Logs.
    Enforce,
    /// Reversibler 60-min-Holding-Timeout mit Mod-Review.
    Propose,
    /// Etablierter Einzelchannel-Treffer: löschen + selbstlöschende Notiz.
    SoftWarn,
    /// Etablierter gestreuter Treffer oder Takeover-Muster: 24h-Timeout +
    /// einheitliche Hijack-DM.
    Hijack,
}

/// Discord-Seite (Tests mocken sie).
#[async_trait::async_trait]
pub trait GuardPort: Send + Sync {
    async fn ban(&self, guild_id: u64, user_id: u64, reason: &str) -> bool;
    async fn timeout(&self, guild_id: u64, user_id: u64, minutes: i64, reason: &str) -> bool;
    async fn delete_message(&self, channel_id: u64, message_id: u64) -> bool;
    async fn send_dm(&self, user_id: u64, text: String) -> bool;
    /// User-DM mit „Einspruch"-Button (`sg:appeal:{case_id}`) für den
    /// verbliebenen Holding-Proposal-Pfad.
    async fn send_dm_with_appeal(&self, user_id: u64, text: String, case_id: &str) -> bool;
    /// Mod-Alarm mit sg:*-Buttons in den Mod-Kanal.
    async fn post_mod_alert(&self, case: &Incident, action: &GuardAction);
    /// Kurze öffentliche Notiz in den betroffenen Kanal, danach Best-Effort-Löschung.
    async fn post_self_deleting_notice(
        &self,
        channel_id: u64,
        text: String,
        delete_after_secs: u64,
    );
    async fn fetch_evidence_image(&self, _url: &str) -> Option<EvidenceImage> {
        None
    }
    /// Invite-Code auf die Ziel-Guild auflösen. `None` bedeutet unauflösbar
    /// und darf nicht als fremd gewertet werden.
    async fn resolve_invite_guild(&self, code: &str) -> Option<u64>;
}

#[derive(Debug, Clone)]
pub struct Incident {
    pub case_id: String,
    pub guild_id: u64,
    pub user_id: u64,
    pub user_tag: String,
    pub action: String,
    pub reason: String,
    pub meta: [i64; 4],
    pub messages: Vec<RecentMsg>,
    /// Account-Erstellung (Unix) — für „Account-Alter" im Log.
    pub account_created_at: i64,
    /// Guild-Join (Unix) — für „Zeit seit Join" im Log.
    pub joined_at: Option<i64>,
    /// Wurde die User-DM zugestellt? (Log-Feld „DM sent").
    pub dm_sent: bool,
    /// Hat Ban/Timeout geklappt? (Log-Feld „Aktionen").
    pub action_ok: bool,
    /// Wie viele Nachrichten gelöscht? (Log-Feld „Deleted").
    pub deleted_count: i64,
    pub delete_attempted: bool,
    pub evidence_images: Vec<EvidenceImage>,
    /// Auslöser für Mod-Embed/Diagnose: Scam, Fremd-Invite oder Takeover.
    pub trigger: String,
}

#[derive(Debug, Clone)]
pub struct SecurityGuardConfig {
    pub escalation_contact_handle: String,
    /// `false` = Shadow/Log-only: Erkennung und Mod-Sichtbarkeit laufen,
    /// strafende Seiteneffekte werden nicht ausgeführt.
    pub enforce: bool,
}

impl Default for SecurityGuardConfig {
    fn default() -> Self {
        Self {
            escalation_contact_handle: DEFAULT_ESCALATION_CONTACT_HANDLE.to_string(),
            enforce: false,
        }
    }
}

pub struct SecurityGuard {
    pub pool: PgPool,
    pub generator: Option<Arc<dyn dl_ai::TextGenerator>>,
    /// Optionaler Vision-Pfad für das Takeover-Bild-Label (Original:
    /// `_finalize_takeover_ai_label` → `_ai_check_image_scam`). Best-effort,
    /// kein Gate: ändert die Quarantäne nie, liefert nur Mod-Kontext.
    pub vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
    pub port: Arc<dyn GuardPort>,
    pub config: SecurityGuardConfig,
    history: tokio::sync::Mutex<HashMap<u64, VecDeque<RecentMsg>>>,
    active: tokio::sync::Mutex<std::collections::HashSet<u64>>,
    suppressed_until: tokio::sync::Mutex<HashMap<u64, i64>>,
}

impl SecurityGuard {
    pub fn new(
        pool: PgPool,
        generator: Option<Arc<dyn dl_ai::TextGenerator>>,
        vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
        port: Arc<dyn GuardPort>,
    ) -> Arc<Self> {
        Self::new_with_config(
            pool,
            generator,
            vision,
            port,
            SecurityGuardConfig::default(),
        )
    }

    pub fn new_with_config(
        pool: PgPool,
        generator: Option<Arc<dyn dl_ai::TextGenerator>>,
        vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
        port: Arc<dyn GuardPort>,
        config: SecurityGuardConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            generator,
            vision,
            port,
            config,
            history: tokio::sync::Mutex::new(HashMap::new()),
            active: tokio::sync::Mutex::new(std::collections::HashSet::new()),
            suppressed_until: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            SELECT 1 AS "ok!"
            FROM moderation.security_guard_incidents
            LIMIT 0
            "#
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn handle_message(self: &Arc<Self>, event: &dl_discord::MessageEvent) {
        let Some(guild_id) = event.guild_id else {
            return;
        };
        if !event.author_staff_status_known {
            tracing::warn!(
                guild_id,
                user_id = event.author_id,
                "SecurityGuard: Staff-Status nicht im Cache, fail-closed skip"
            );
            return;
        }
        if event.author_is_staff {
            return; // do not police staff: administrator || manage_messages || manage_guild
        }
        let now = chrono::Utc::now().timestamp();
        if self.is_suppressed(event.author_id, now).await {
            return;
        }

        // History pflegen (Ringpuffer 20, Fenster 1 h)
        let recent: Vec<RecentMsg> = {
            let mut history = self.history.lock().await;
            let entry = history.entry(event.author_id).or_default();
            entry.push_back(RecentMsg {
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
            entry.retain(|m| m.created_at >= cutoff);
            entry.iter().cloned().collect()
        };

        if !self.try_claim(event.author_id).await {
            return;
        }
        let result = self.run_detection(guild_id, event, &recent, now).await;
        self.release(event.author_id).await;
        if result {
            self.history.lock().await.remove(&event.author_id);
            self.suppress_user(event.author_id, now).await;
        }
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
        self.suppressed_until
            .lock()
            .await
            .insert(user_id, now + CASE_COOLDOWN_SECONDS);
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
                        "SecurityGuard: Invite unauflösbar, nicht als fremd gewertet"
                    );
                }
            }
        }
        None
    }

    /// → true wenn ein Case ausgelöst wurde (History wird dann geleert).
    async fn run_detection(
        self: &Arc<Self>,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        recent: &[RecentMsg],
        now: i64,
    ) -> bool {
        // 1. Takeover (alle Accounts, deterministisch, kein AI-Gate)
        if let Some((reason, burst)) = detect_takeover(recent, now) {
            let meta = [
                burst
                    .iter()
                    .map(|m| m.channel_id)
                    .collect::<std::collections::HashSet<_>>()
                    .len() as i64,
                burst.len() as i64,
                burst.iter().map(|m| m.image_count as i64).sum(),
                0,
            ];
            // Best-effort-KI-Bild-Label (kein Gate) als Mod-Kontext an den
            // Grund anhängen — wie das Original-Feld „KI-Einschätzung".
            let reason = match self.takeover_ai_label(&burst).await {
                Some(label) => format!("{reason}\nKI-Einschätzung: {label}"),
                None => reason,
            };
            let action = decide_hit_action(
                event.author_created_at,
                event.author_joined_at,
                now,
                &burst,
                true,
            );
            self.execute(guild_id, event, burst, reason, meta, action, "Takeover")
                .await;
            return true;
        }

        if let Some(code) = self.foreign_invite_code(guild_id, &event.content).await {
            let single = vec![RecentMsg {
                channel_id: event.channel_id,
                message_id: event.message_id,
                created_at: now,
                content: event.content.clone(),
                attachment_count: event.attachment_count,
                image_count: event.image_attachment_count,
                image_urls: event.image_attachment_urls.clone(),
            }];
            let action = decide_hit_action(
                event.author_created_at,
                event.author_joined_at,
                now,
                recent,
                false,
            );
            let meta = [
                distinct_channel_count(recent) as i64,
                recent.len() as i64,
                event.attachment_count as i64,
                0,
            ];
            self.execute(
                guild_id,
                event,
                single,
                format!("Fremder Discord-Invite: {code}"),
                meta,
                action,
                "Fremd-Invite",
            )
            .await;
            return true;
        }

        let young = is_new_account(event.author_created_at, event.author_joined_at, now);

        // 2. Burst junger Accounts → Text- und ggf. Bild-Scam-Check
        if young {
            if let Some((reason, meta)) = should_trigger(recent) {
                let combined: String = recent
                    .iter()
                    .filter(|m| !m.content.is_empty())
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(2000)
                    .collect();
                let (txt_scam, txt_conf, txt_reason) = self.ai_check(&combined).await;
                let text_confirmed = txt_scam && txt_conf >= AI_SCAM_CONFIDENCE;
                let (img_scam, img_conf, img_reason) =
                    if !text_confirmed && recent.iter().any(|m| m.image_count > 0) {
                        self.ai_check_image_scam(recent).await
                    } else {
                        (false, 0.0, "no_attachments".to_string())
                    };
                let (is_scam, confidence, ai_reason) = if img_scam && img_conf > txt_conf {
                    (img_scam, img_conf, img_reason.clone())
                } else {
                    (txt_scam, txt_conf, txt_reason.clone())
                };
                let action = if is_scam && confidence >= AI_SCAM_CONFIDENCE {
                    decide_hit_action(
                        event.author_created_at,
                        event.author_joined_at,
                        now,
                        recent,
                        false,
                    )
                } else {
                    GuardAction::Propose
                };
                let full_reason = match action {
                    GuardAction::Enforce => {
                        format!("{reason}; AI conf {:.0}%: {ai_reason}", confidence * 100.0)
                    }
                    GuardAction::Propose | GuardAction::SoftWarn | GuardAction::Hijack => {
                        format!(
                            "{reason}; AI unbestaetigt (Text {:.0}%: {}; Bild {:.0}%: {})",
                            txt_conf * 100.0,
                            txt_reason,
                            img_conf * 100.0,
                            img_reason
                        )
                    }
                };
                self.execute(
                    guild_id,
                    event,
                    recent.to_vec(),
                    full_reason,
                    meta,
                    action,
                    "Scam",
                )
                .await;
                return true;
            }
        }

        // 3. Bilder in mehreren Channels → AI-Bild-Scam-Check (alle Accounts)
        if is_image_multi_channel(recent, now) {
            let (is_scam, confidence, ai_reason) = self.ai_check_image_scam(recent).await;
            if is_scam && confidence >= AI_IMAGE_CONFIDENCE {
                let image_channels = recent
                    .iter()
                    .filter(|m| m.image_count > 0)
                    .map(|m| m.channel_id)
                    .collect::<std::collections::HashSet<_>>()
                    .len() as i64;
                let image_count: i64 = recent.iter().map(|m| m.image_count as i64).sum();
                let reason = format!(
                    "Bild-Scam in {image_channels} Channels (conf {:.0}%): {ai_reason}",
                    confidence * 100.0
                );
                let meta = [image_channels, recent.len() as i64, image_count, 0];
                let action = decide_hit_action(
                    event.author_created_at,
                    event.author_joined_at,
                    now,
                    recent,
                    false,
                );
                self.execute(
                    guild_id,
                    event,
                    recent.to_vec(),
                    reason,
                    meta,
                    action,
                    "Scam",
                )
                .await;
                return true;
            }
        }

        // 4. Keyword-Einzeltreffer → AI; etabliert = 24h-Timeout+Warn-DM, jung = Vollzug
        if contains_suspicious_text(&event.content) {
            let (is_scam, confidence, ai_reason) = self.ai_check(&event.content).await;
            if is_scam && confidence >= AI_SCAM_CONFIDENCE {
                let action = decide_hit_action(
                    event.author_created_at,
                    event.author_joined_at,
                    now,
                    recent,
                    false,
                );
                let reason = format!("AI-Scam (conf {:.0}%): {ai_reason}", confidence * 100.0);
                let single = vec![RecentMsg {
                    channel_id: event.channel_id,
                    message_id: event.message_id,
                    created_at: now,
                    content: event.content.clone(),
                    attachment_count: event.attachment_count,
                    image_count: event.image_attachment_count,
                    image_urls: event.image_attachment_urls.clone(),
                }];
                let meta = [
                    distinct_channel_count(recent) as i64,
                    recent.len() as i64,
                    event.attachment_count as i64,
                    1,
                ];
                self.execute(guild_id, event, single, reason, meta, action, "Scam")
                    .await;
                return true;
            }
        }
        false
    }

    async fn ai_check(&self, content: &str) -> (bool, f64, String) {
        let Some(generator) = &self.generator else {
            return (false, 0.0, "ai_unavailable".to_string());
        };
        if content.trim().is_empty() {
            return (false, 0.0, "no_text".to_string());
        }
        let raw = generator
            .generate_text(dl_ai::GenerateRequest {
                prompt: content.to_string(),
                system_prompt: Some(SCAM_DETECTION_SYSTEM_PROMPT.to_string()),
                model: None,
                max_output_tokens: Some(200),
                temperature: 0.0,
            })
            .await;
        parse_scam_json(raw.as_deref())
    }

    async fn ai_check_image_scam(&self, msgs: &[RecentMsg]) -> (bool, f64, String) {
        let Some(vision) = &self.vision else {
            return (false, 0.0, "ai_unavailable".to_string());
        };
        let image_urls: Vec<String> = msgs
            .iter()
            .flat_map(|m| m.image_urls.iter().cloned())
            .take(1)
            .collect();
        if image_urls.is_empty() {
            return (false, 0.0, "no_images".to_string());
        }
        let raw = vision
            .generate_multimodal(dl_ai::GenerateMultimodalRequest {
                prompt: VISION_SCAM_PROMPT.to_string(),
                image_urls,
                system_prompt: None,
                model: None,
                max_output_tokens: Some(400),
                temperature: 0.0,
            })
            .await;
        parse_scam_json(raw.as_deref())
    }

    /// Best-effort-Bild-Label für den Takeover-Alarm (Original:
    /// `_takeover_ai_label` → `_ai_check_image_scam`). KEIN Gate: ändert die
    /// Quarantäne nie, liefert den Mods nur Kontext, ob die KI die Bilder
    /// ebenfalls als Scam einschätzt. Gibt None zurück, wenn kein Vision-
    /// Generator verdrahtet ist oder keine Bilder vorliegen.
    async fn takeover_ai_label(&self, burst: &[RecentMsg]) -> Option<String> {
        let vision = self.vision.as_ref()?;
        let image_urls: Vec<String> = burst
            .iter()
            .flat_map(|m| m.image_urls.iter().cloned())
            .take(4)
            .collect();
        if image_urls.is_empty() {
            return None;
        }
        let raw = vision
            .generate_multimodal(dl_ai::GenerateMultimodalRequest {
                prompt: VISION_SCAM_PROMPT.to_string(),
                image_urls,
                system_prompt: None,
                model: None,
                max_output_tokens: Some(400),
                temperature: 0.0,
            })
            .await;
        if raw.is_none() {
            return Some("nicht verfuegbar".to_string());
        }
        let (is_scam, confidence, reason) = parse_scam_json(raw.as_deref());
        if reason == "parse_error" {
            return Some("nicht verfuegbar".to_string());
        }
        let verdict = if is_scam { "Scam" } else { "kein Scam" };
        let reason = if reason.is_empty() { "-" } else { &reason };
        Some(format!("{verdict} ({:.0}%) — {reason}", confidence * 100.0))
    }

    /// DM vor Ban/Timeout, dann Aktion, Löschung, optional Soft-Warn-Notiz
    /// und Mod-Alarm. DM-Fehler blockieren die Aktion nicht.
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        messages: Vec<RecentMsg>,
        reason: String,
        meta: [i64; 4],
        action: GuardAction,
        trigger: &str,
    ) {
        let case_id = format!("sg-{}-{}", event.author_id, event.message_id);
        let mut incident = Incident {
            case_id: case_id.clone(),
            guild_id,
            user_id: event.author_id,
            user_tag: event.author_display_name.clone(),
            action: match action {
                GuardAction::Enforce => "ban".to_string(),
                GuardAction::Propose => "timeout-proposal".to_string(),
                GuardAction::SoftWarn => "soft-warn".to_string(),
                GuardAction::Hijack => "hijack-timeout".to_string(),
            },
            reason: reason.clone(),
            meta,
            messages: messages.clone(),
            account_created_at: event.author_created_at,
            joined_at: event.author_joined_at,
            dm_sent: false,
            action_ok: false,
            deleted_count: 0,
            delete_attempted: false,
            evidence_images: Vec::new(),
            trigger: trigger.to_string(),
        };
        self.persist(&incident).await;
        incident.evidence_images = self.collect_evidence_images(&messages).await;

        if !self.config.enforce {
            tracing::warn!(
                case_id = %case_id,
                guild_id,
                user_id = event.author_id,
                action = ?action,
                trigger,
                reason = %reason,
                "SHADOW: würde {:?} gegen User {} — Grund {}, nicht durchgesetzt",
                action,
                event.author_id,
                reason
            );
            self.port.post_mod_alert(&incident, &action).await;
            return;
        }

        let dm_sent = match action {
            GuardAction::Enforce => {
                self.port
                    .send_dm(
                        event.author_id,
                        ban_dm_text(&self.config.escalation_contact_handle),
                    )
                    .await
            }
            GuardAction::Hijack => {
                self.port
                    .send_dm(event.author_id, hijack_dm_text(&case_id))
                    .await
            }
            GuardAction::Propose => {
                self.port
                    .send_dm_with_appeal(
                        event.author_id,
                        format!(
                            "Dein Account hat auf der Deutschen Deadlock Community ein Sicherheits-Muster ausgelöst.\nGrund: {reason}\nCase: {case_id}\nWenn du das für einen Fehler hältst, nutze den Einspruch-Button."
                        ),
                        &case_id,
                    )
                    .await
            }
            GuardAction::SoftWarn => false,
        };

        let acted = match action {
            GuardAction::Enforce => self.port.ban(guild_id, event.author_id, &reason).await,
            GuardAction::Propose => {
                self.port
                    .timeout(guild_id, event.author_id, PROPOSAL_TIMEOUT_MINUTES, &reason)
                    .await
            }
            GuardAction::Hijack => {
                self.port
                    .timeout(guild_id, event.author_id, TIMEOUT_MINUTES, &reason)
                    .await
            }
            GuardAction::SoftWarn => true,
        };
        if !acted {
            tracing::warn!(case_id, "SecurityGuard: Aktion fehlgeschlagen");
        }

        let mut deleted = 0i64;
        for msg in &messages {
            if self
                .port
                .delete_message(msg.channel_id, msg.message_id)
                .await
            {
                deleted += 1;
            }
        }
        if matches!(action, GuardAction::SoftWarn) {
            if let Some(channel_id) = messages.first().map(|m| m.channel_id) {
                self.port
                    .post_self_deleting_notice(
                        channel_id,
                        soft_warn_notice(event.author_id),
                        SOFT_WARN_DELETE_AFTER_SECONDS,
                    )
                    .await;
            }
        }
        incident.dm_sent = dm_sent;
        incident.action_ok = acted;
        incident.deleted_count = deleted;
        incident.delete_attempted = true;
        if !matches!(action, GuardAction::SoftWarn) {
            self.port.post_mod_alert(&incident, &action).await;
        }
    }

    async fn collect_evidence_images(&self, messages: &[RecentMsg]) -> Vec<EvidenceImage> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for url in messages.iter().flat_map(|m| m.image_urls.iter()) {
            if out.len() >= EVIDENCE_IMAGE_LIMIT {
                break;
            }
            if !seen.insert(url.clone()) {
                continue;
            }
            if let Some(image) = self.port.fetch_evidence_image(url).await {
                out.push(image);
            }
        }
        out
    }

    async fn persist(&self, incident: &Incident) {
        let messages_json = serde_json::to_string(
            &incident
                .messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "channel_id": m.channel_id,
                        "ts": m.created_at,
                        "content": m.content.chars().take(500).collect::<String>(),
                        "attachments": m.attachment_count,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        let Some(guild_id) = discord_id_to_i64(incident.guild_id, "guild_id") else {
            return;
        };
        let Some(user_id) = discord_id_to_i64(incident.user_id, "user_id") else {
            return;
        };
        let Some(channel_count) = i64_to_i32(incident.meta[0], "channel_count") else {
            return;
        };
        let Some(message_count) = i64_to_i32(incident.meta[1], "message_count") else {
            return;
        };
        let Some(attachment_count) = i64_to_i32(incident.meta[2], "attachment_count") else {
            return;
        };
        let keyword_hit = incident.meta[3] != 0;
        let incident = incident.clone();
        let result = sqlx::query!(
            r#"
            INSERT INTO moderation.security_guard_incidents(
                case_id, guild_id, user_id, user_tag, action, reason,
                channel_count, message_count, attachment_count, keyword_hit,
                messages, created_at
            )
            VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::text::jsonb, now())
            ON CONFLICT (case_id) DO NOTHING
            "#,
            incident.case_id,
            guild_id,
            user_id,
            incident.user_tag,
            incident.action,
            incident.reason,
            channel_count,
            message_count,
            attachment_count,
            keyword_hit,
            messages_json,
        )
        .execute(&self.pool)
        .await;
        if let Err(err) = result {
            tracing::warn!(%err, "SecurityGuard: Incident-Persist fehlgeschlagen");
        }
    }
}

fn discord_id_to_i64(value: u64, field: &'static str) -> Option<i64> {
    match i64::try_from(value) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::warn!(%err, field, value, "Discord-ID passt nicht in PostgreSQL BIGINT");
            None
        }
    }
}

fn i64_to_i32(value: i64, field: &'static str) -> Option<i32> {
    match i32::try_from(value) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::warn!(%err, field, value, "SecurityGuard-Metadatum passt nicht in INTEGER");
            None
        }
    }
}

pub fn spawn(
    guard: Arc<SecurityGuard>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => guard.handle_message(&event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(dead_code)]

    use super::*;
    use std::collections::HashMap;
    use std::io;
    use std::sync::Arc;

    fn msg(channel: u64, secs_ago: i64, content: &str, images: u32) -> RecentMsg {
        RecentMsg {
            channel_id: channel,
            message_id: 1,
            created_at: 1_000_000 - secs_ago,
            content: content.to_string(),
            attachment_count: images,
            image_count: images,
            image_urls: (0..images).map(|i| format!("https://img/{i}")).collect(),
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
        event.attachment_count = 2;
        event.image_attachment_count = 2;
        event.image_attachment_urls = vec![
            format!("https://img/{message_id}-1.png"),
            format!("https://img/{message_id}-2.png"),
        ];
        event
    }

    #[derive(Default)]
    struct FakePort {
        resolves: HashMap<String, Option<u64>>,
        calls: tokio::sync::Mutex<Vec<String>>,
    }

    impl FakePort {
        fn with_resolves(entries: &[(&str, Option<u64>)]) -> Arc<Self> {
            Arc::new(Self {
                resolves: entries
                    .iter()
                    .map(|(code, guild_id)| ((*code).to_string(), *guild_id))
                    .collect(),
                calls: tokio::sync::Mutex::new(Vec::new()),
            })
        }

        async fn record(&self, call: impl Into<String>) {
            self.calls.lock().await.push(call.into());
        }

        async fn calls(&self) -> Vec<String> {
            self.calls.lock().await.clone()
        }
    }

    #[derive(Clone, Default)]
    struct LogCapture {
        bytes: Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl LogCapture {
        fn text(&self) -> String {
            let bytes = self.bytes.lock().expect("log capture").clone();
            String::from_utf8_lossy(&bytes).to_string()
        }
    }

    struct LogCaptureWriter {
        bytes: Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl io::Write for LogCaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .expect("log capture")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            LogCaptureWriter {
                bytes: self.bytes.clone(),
            }
        }
    }

    #[async_trait::async_trait]
    impl GuardPort for FakePort {
        async fn ban(&self, _guild_id: u64, _user_id: u64, _reason: &str) -> bool {
            self.record("ban").await;
            true
        }

        async fn timeout(
            &self,
            _guild_id: u64,
            _user_id: u64,
            minutes: i64,
            _reason: &str,
        ) -> bool {
            self.record(format!("timeout:{minutes}")).await;
            true
        }

        async fn delete_message(&self, channel_id: u64, message_id: u64) -> bool {
            self.record(format!("delete:{channel_id}:{message_id}"))
                .await;
            true
        }

        async fn send_dm(&self, _user_id: u64, _text: String) -> bool {
            self.record("dm").await;
            true
        }

        async fn send_dm_with_appeal(&self, _user_id: u64, _text: String, _case_id: &str) -> bool {
            self.record("dm_appeal").await;
            true
        }

        async fn post_mod_alert(&self, _case: &Incident, action: &GuardAction) {
            self.record(format!("mod:{action:?}")).await;
        }

        async fn post_self_deleting_notice(
            &self,
            channel_id: u64,
            _text: String,
            delete_after_secs: u64,
        ) {
            self.record(format!("notice:{channel_id}:{delete_after_secs}"))
                .await;
        }

        async fn resolve_invite_guild(&self, code: &str) -> Option<u64> {
            self.record(format!("resolve:{code}")).await;
            self.resolves.get(code).copied().flatten()
        }
    }

    fn enforcing_config() -> SecurityGuardConfig {
        SecurityGuardConfig {
            enforce: true,
            ..SecurityGuardConfig::default()
        }
    }

    #[cfg(feature = "testing")]
    async fn test_guard_with_config(
        port: Arc<dyn GuardPort>,
        config: SecurityGuardConfig,
    ) -> Result<(dl_central_db::TestDb, Arc<SecurityGuard>), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let guard = SecurityGuard::new_with_config(db.pool().clone(), None, None, port, config);
        guard.ensure_schema().await?;
        Ok((db, guard))
    }

    #[cfg(feature = "testing")]
    async fn test_guard(
        port: Arc<dyn GuardPort>,
    ) -> Result<(dl_central_db::TestDb, Arc<SecurityGuard>), Box<dyn std::error::Error>> {
        test_guard_with_config(port, enforcing_config()).await
    }

    #[test]
    fn invite_extraktion_erkennt_invites_und_ignoriert_andere_discord_links() {
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
    fn takeover_erkennung() {
        let now = 1_000_000;
        // Bilder in 2 Channels in 10s → Treffer
        let msgs = vec![msg(1, 10, "", 1), msg(2, 5, "", 1)];
        assert!(detect_takeover(&msgs, now).is_some());
        // Bilder nur in EINEM Channel → kein Treffer (Doppelpost ok)
        let msgs = vec![msg(1, 10, "", 1), msg(1, 5, "", 2)];
        assert!(detect_takeover(&msgs, now).is_none());
        // 2 Channels, aber außerhalb des 30s-Fensters → kein Treffer
        let msgs = vec![msg(1, 60, "", 1), msg(2, 5, "", 1)];
        assert!(detect_takeover(&msgs, now).is_none());
    }

    #[test]
    fn burst_trigger_wie_python() {
        // 3 Nachrichten in 3 Channels → Burst über mehrere Kanäle
        let msgs = vec![
            msg(1, 30, "hi", 0),
            msg(2, 20, "hi", 0),
            msg(3, 10, "hi", 0),
        ];
        let (reason, meta) = should_trigger(&msgs).expect("burst");
        assert!(reason.contains("Burst über mehrere Kanäle"));
        assert_eq!(meta, [3, 3, 0, 0]);
        // 2 Channels + Keyword → suspicious
        let msgs = vec![msg(1, 30, "join my telegram", 0), msg(2, 10, "hi", 0)];
        let (reason, meta) = should_trigger(&msgs).expect("sus");
        assert!(reason.contains("verdächtige Inhalte"));
        assert!(reason.contains("Schlagwort-Treffer"));
        assert_eq!(meta[3], 1);
        // 2 harmlose Nachrichten in 2 Channels → nichts
        let msgs = vec![msg(1, 30, "hi", 0), msg(2, 10, "ho", 0)];
        assert!(should_trigger(&msgs).is_none());
        // Anhänge in 2 Channels → attachments-Trigger
        let msgs = vec![msg(1, 30, "", 1), msg(2, 10, "", 1)];
        let (reason, _) = should_trigger(&msgs).expect("attach");
        assert!(reason.contains("Anhänge in 2+ Kanälen"));
    }

    #[test]
    fn image_multi_channel_threshold_wie_python() {
        let now = 1_000_000;
        let msgs = vec![msg(1, 30, "", 1), msg(2, 10, "", 1)];
        assert!(is_image_multi_channel(&msgs, now));
        let msgs = vec![msg(1, 30, "", 1), msg(1, 10, "", 2)];
        assert!(!is_image_multi_channel(&msgs, now));
        let msgs = vec![msg(1, 30, "telegram", 0), msg(2, 10, "", 0)];
        assert!(!is_image_multi_channel(&msgs, now));
        let msgs = vec![msg(1, 301, "", 1), msg(2, 10, "", 1)];
        assert!(!is_image_multi_channel(&msgs, now));
    }

    #[test]
    fn scam_json_parsing() {
        let (scam, conf, reason) = parse_scam_json(Some(
            "<think>hm</think> {\"is_scam\": true, \"confidence\": 0.92, \"reason\": \"crypto promo\"}",
        ));
        assert!(scam);
        assert_eq!(conf, 0.92);
        assert_eq!(reason, "crypto promo");
        assert_eq!(parse_scam_json(Some("garbage")).2, "parse_error");
        assert_eq!(parse_scam_json(None).2, "parse_error");
    }

    #[test]
    fn account_alter() {
        let now = 1_000_000_000;
        let hour = 3600;
        assert!(is_new_account(now - 100 * hour, Some(now - 10 * hour), now));
        assert!(!is_new_account(
            now - 100 * hour,
            Some(now - 200 * hour),
            now
        ));
        assert!(!is_new_account(now - 100 * hour, None, now));
        assert!(!is_new_account(now - 800 * hour, Some(now - hour), now));
        assert!(is_established_account(now - 100 * hour, None, now));
        assert!(is_established_account(
            now - 100 * hour,
            Some(now - 200 * hour),
            now
        ));
    }

    #[test]
    fn dm_texte() {
        let text = hijack_dm_text("sg-1-2");
        assert!(text.contains("24 Stunden"));
        assert!(text.contains("sg-1-2"));
        assert!(text.contains("gehackt"));
        assert_eq!(
            ban_dm_text(DEFAULT_ESCALATION_CONTACT_HANDLE),
            "Du wurdest auf der Deutschen Deadlock Community gebannt, weil dein Account ein Scam-/Fremdlink-Muster ausgelöst hat. Wenn du denkst, das ist ein Fehler: Schick @earlysalty eine Freundschaftsanfrage **und** eine kurze Nachricht — dann schauen wir uns das an."
        );
        assert_eq!(
            soft_warn_notice(42),
            "<@42> — fremder Discord-Invite entfernt. Bitte keine fremden Server-Einladungen posten."
        );
    }

    #[test]
    fn streuung_und_aktionsrouting() {
        let now = 1_000_000;
        let hour = 3600;
        let one_channel = vec![msg(1, 5, "hit", 0), msg(1, 1, "hit2", 0)];
        let two_channels = vec![msg(1, 5, "hit", 0), msg(2, 1, "hit2", 0)];
        assert_eq!(distinct_channel_count(&one_channel), 1);
        assert_eq!(distinct_channel_count(&two_channels), 2);
        assert_eq!(
            decide_hit_action(
                now - 100 * hour,
                Some(now - 10 * hour),
                now,
                &one_channel,
                false
            ),
            GuardAction::Enforce
        );
        assert_eq!(
            decide_hit_action(now - 800 * hour, Some(now - hour), now, &one_channel, false),
            GuardAction::SoftWarn
        );
        assert_eq!(
            decide_hit_action(
                now - 800 * hour,
                Some(now - hour),
                now,
                &two_channels,
                false
            ),
            GuardAction::Hijack
        );
        assert_eq!(
            decide_hit_action(now - 800 * hour, Some(now - hour), now, &one_channel, true),
            GuardAction::Hijack
        );
    }

    #[test]
    fn takeover_und_etabliert_zwei_channels_konsolidieren_auf_hijack() {
        let now = 1_000_000;
        let hour = 3600;
        let two_channels = vec![msg(1, 5, "", 1), msg(2, 1, "", 1)];
        assert_eq!(
            decide_hit_action(now - 800 * hour, Some(now - hour), now, &two_channels, true),
            decide_hit_action(
                now - 800 * hour,
                Some(now - hour),
                now,
                &two_channels,
                false
            )
        );
        assert_eq!(
            decide_hit_action(now - 800 * hour, Some(now - hour), now, &two_channels, true),
            GuardAction::Hijack
        );
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn fremd_invite_routing_own_foreign_unresolved_und_dm_vor_ban(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = chrono::Utc::now().timestamp();
        let hour = 3600;

        let own_port = FakePort::with_resolves(&[("own", Some(1))]);
        let (_db, guard) = test_guard(own_port.clone()).await?;
        guard
            .handle_message(&event(
                10,
                11,
                111,
                "https://discord.gg/own",
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        assert_eq!(own_port.calls().await, vec!["resolve:own"]);

        let foreign_port = FakePort::with_resolves(&[("foreign", Some(2))]);
        let (foreign_db, guard) = test_guard(foreign_port.clone()).await?;
        guard
            .handle_message(&event(
                20,
                21,
                211,
                "https://discord.gg/foreign",
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        let calls = foreign_port.calls().await;
        let dm_pos = calls.iter().position(|c| c == "dm").expect("dm");
        let ban_pos = calls.iter().position(|c| c == "ban").expect("ban");
        assert!(dm_pos < ban_pos);
        assert!(calls.contains(&"delete:21:211".to_string()));

        let row = sqlx::query!(
            r#"
            SELECT
                action AS "action!",
                keyword_hit AS "keyword_hit!",
                messages::text AS "messages!"
            FROM moderation.security_guard_incidents
            WHERE case_id = $1
            "#,
            "sg-20-211",
        )
        .fetch_one(foreign_db.pool())
        .await?;
        assert_eq!(row.action, "ban");
        assert!(!row.keyword_hit);
        assert!(row.messages.contains("discord.gg/foreign"));

        let unresolved_port = FakePort::with_resolves(&[("expired", None)]);
        let (_db, guard) = test_guard(unresolved_port.clone()).await?;
        guard
            .handle_message(&event(
                30,
                31,
                311,
                "https://discord.gg/expired",
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        let calls = unresolved_port.calls().await;
        assert_eq!(calls, vec!["resolve:expired"]);
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn takeover_case_setzt_user_cooldown_gegen_mehrfachalerts(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = chrono::Utc::now().timestamp();
        let hour = 3600;
        let port = FakePort::with_resolves(&[]);
        let (_db, guard) = test_guard(port.clone()).await?;

        guard
            .handle_message(&image_event(
                80,
                81,
                811,
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        guard
            .handle_message(&image_event(
                80,
                82,
                812,
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        let calls_after_first_case = port.calls().await;
        assert!(calls_after_first_case.contains(&"ban".to_string()));
        assert!(calls_after_first_case.contains(&"delete:81:811".to_string()));
        assert!(calls_after_first_case.contains(&"delete:82:812".to_string()));
        assert!(calls_after_first_case.contains(&"mod:Enforce".to_string()));

        guard
            .handle_message(&image_event(
                80,
                83,
                813,
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        guard
            .handle_message(&image_event(
                80,
                84,
                814,
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        assert_eq!(port.calls().await, calls_after_first_case);
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn shadow_mode_fuehrt_strafende_side_effects_nicht_aus_enforce_true_schon(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = chrono::Utc::now().timestamp();
        let hour = 3600;
        let log_capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(log_capture.clone())
            .with_ansi(false)
            .without_time()
            .finish();

        let shadow_port = FakePort::with_resolves(&[("foreign", Some(2))]);
        let (_dir, guard) =
            test_guard_with_config(shadow_port.clone(), SecurityGuardConfig::default()).await?;
        let _guard = tracing::subscriber::set_default(subscriber);
        guard
            .handle_message(&event(
                60,
                61,
                611,
                "https://discord.gg/foreign",
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        drop(_guard);
        let calls = shadow_port.calls().await;
        let logs = log_capture.text();
        assert!(logs.contains("SHADOW: würde Enforce gegen User 60"));
        assert!(logs.contains("nicht durchgesetzt"));
        assert!(calls.contains(&"resolve:foreign".to_string()));
        assert!(calls.contains(&"mod:Enforce".to_string()));
        assert!(!calls.iter().any(|c| {
            c == "dm"
                || c == "dm_appeal"
                || c == "ban"
                || c.starts_with("timeout:")
                || c.starts_with("delete:")
                || c.starts_with("notice:")
        }));

        let enforce_port = FakePort::with_resolves(&[("foreign", Some(2))]);
        let (_db, guard) = test_guard(enforce_port.clone()).await?;
        guard
            .handle_message(&event(
                61,
                62,
                612,
                "https://discord.gg/foreign",
                now - 100 * hour,
                Some(now - 10 * hour),
            ))
            .await;
        let calls = enforce_port.calls().await;
        assert!(calls.contains(&"dm".to_string()));
        assert!(calls.contains(&"ban".to_string()));
        assert!(calls.contains(&"delete:62:612".to_string()));
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn staff_cache_miss_fail_closed_ohne_resolve_oder_aktion(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = chrono::Utc::now().timestamp();
        let hour = 3600;
        let port = FakePort::with_resolves(&[("foreign", Some(2))]);
        let (_db, guard) = test_guard(port.clone()).await?;
        let mut event = event(
            70,
            71,
            711,
            "https://discord.gg/foreign",
            now - 100 * hour,
            Some(now - 10 * hour),
        );
        event.author_staff_status_known = false;
        guard.handle_message(&event).await;
        assert!(port.calls().await.is_empty());
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn etablierter_fremd_invite_softwarn_vs_hijack_nach_streuung(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = chrono::Utc::now().timestamp();
        let hour = 3600;

        let soft_port = FakePort::with_resolves(&[("foreign", Some(2))]);
        let (_db, guard) = test_guard(soft_port.clone()).await?;
        guard
            .handle_message(&event(
                40,
                41,
                411,
                "https://discord.gg/foreign",
                now - 800 * hour,
                None,
            ))
            .await;
        let calls = soft_port.calls().await;
        assert!(calls.contains(&"delete:41:411".to_string()));
        assert!(calls.contains(&format!("notice:41:{}", SOFT_WARN_DELETE_AFTER_SECONDS)));
        assert!(!calls.iter().any(|c| c == "dm" || c.starts_with("timeout")));

        let hijack_port = FakePort::with_resolves(&[("foreign", Some(2))]);
        let (_db, guard) = test_guard(hijack_port.clone()).await?;
        guard
            .handle_message(&event(50, 51, 511, "nur history", now - 800 * hour, None))
            .await;
        guard
            .handle_message(&event(
                50,
                52,
                512,
                "https://discord.gg/foreign",
                now - 800 * hour,
                None,
            ))
            .await;
        let calls = hijack_port.calls().await;
        assert!(calls.contains(&"dm".to_string()));
        assert!(calls.contains(&format!("timeout:{TIMEOUT_MINUTES}")));
        assert!(calls.iter().any(|c| c == "mod:Hijack"));
        Ok(())
    }

    #[test]
    fn keywords() {
        assert!(contains_suspicious_text("Join my TELEGRAM channel"));
        assert!(contains_suspicious_text("first 10 people earning $"));
        assert!(!contains_suspicious_text("wer bock auf deadlock"));
    }
}
