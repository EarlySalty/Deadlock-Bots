//! SecurityGuard — Port des Kerns von `cogs/security_guard.py`.
//!
//! Drei Detektions-Pfade wie das Original:
//! 1. **Account-Takeover** (alle Accounts, deterministisch, KEIN AI):
//!    Bilder in ≥ 2 verschiedenen Channels innerhalb von 30 s → sofortige
//!    Quarantäne (Ban/Timeout laut Konfiguration).
//! 2. **Junge Accounts** (< 30 Tage): Mehrkanal-Burst (3 Kanäle/3
//!    Nachrichten in 1 h, oder 2 Kanäle mit Keyword/Anhängen) → AI-Scam-
//!    Check (MiniMax, ≥ 0,78) → bestätigt: Vollzug; unbestätigt:
//!    60-min-Holding-Timeout als Mod-Vorschlag.
//! 3. **Keyword-Einzeltreffer**: AI-Check; etablierte Accounts (≥ 30 Tage
//!    Account + ≥ 24 h auf dem Server) bekommen einen Vorschlag, junge den
//!    Vollzug.
//!
//! Bewusste Lücke (dokumentiert): Die Bild-Scam-Bestätigung lief über die
//! externe `mmx`-Vision-CLI — hier noch nicht angebunden. Folge: Bursts,
//! die NUR über Bilder bestätigt würden, landen als reversibler
//! Holding-Vorschlag statt als Auto-Vollzug (konservativer als das
//! Original, nie schärfer).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use dl_db::Db;
use serde_json::Value;

pub const REVIEW_CHANNEL_ID: u64 = 1374364800817303632;
pub const MOD_CHANNEL_ID: u64 = 1315684135175716978;
pub const WINDOW_SECONDS: i64 = 3600;
pub const CHANNEL_THRESHOLD: usize = 3;
pub const MESSAGE_THRESHOLD: usize = 3;
pub const ACCOUNT_MAX_AGE_HOURS: i64 = 720;
pub const ESTABLISHED_ACCOUNT_MIN_AGE_HOURS: i64 = 720;
pub const ESTABLISHED_MIN_JOIN_HOURS: i64 = 24;
pub const AI_SCAM_CONFIDENCE: f64 = 0.78;
pub const TAKEOVER_WINDOW_SECONDS: i64 = 30;
pub const TAKEOVER_IMAGE_CHANNELS: usize = 2;
pub const TIMEOUT_MINUTES: i64 = 1440;
pub const PROPOSAL_TIMEOUT_MINUTES: i64 = 60;
pub const HISTORY_MAX: usize = 20;

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

/// System-Prompt wortgleich (SCAM_DETECTION_SYSTEM_PROMPT).
pub const SCAM_DETECTION_SYSTEM_PROMPT: &str =
    "You are a scam detector for a Discord gaming server. \
Decide if the message is financial spam or a scam (earnings promises, investment schemes, \
Telegram/contact requests, profit-sharing, referral schemes, or similar). \
Reply only with valid JSON, no other text: \
{\"is_scam\": true|false, \"confidence\": 0.0-1.0, \"reason\": \"max one sentence\"}";

// ── Pure Detektion ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RecentMsg {
    pub channel_id: u64,
    pub message_id: u64,
    pub created_at: i64,
    pub content: String,
    pub attachment_count: u32,
    pub image_count: u32,
}

pub fn contains_suspicious_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    KEYWORDS.iter().any(|kw| lower.contains(kw))
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
        reason_bits.push("multi-channel burst".to_string());
    }
    if two_channel_sus && !multi_channel_burst {
        reason_bits.push("suspicious content across 2+ channels".to_string());
    }
    if attachment_multi_channel && !multi_channel_burst {
        reason_bits.push("attachments across 2+ channels".to_string());
    }
    if keyword_hit {
        reason_bits.push("keyword match".to_string());
    }
    if attachment_count > 0 {
        reason_bits.push(format!("{attachment_count} attachment(s)"));
    }
    let reason = if reason_bits.is_empty() {
        "burst from new account".to_string()
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

/// Takeover-DM-Text (Original: `_send_takeover_dm`). Dauer aus `TIMEOUT_MINUTES`
/// abgeleitet (1440 → „24 Stunden"). Als Plain-Text, wie die übrige Guard-DM.
pub fn takeover_dm_text(case_id: &str) -> String {
    let hours = TIMEOUT_MINUTES / 60;
    let dauer = if TIMEOUT_MINUTES % 60 == 0 && hours > 0 {
        if hours == 1 {
            "1 Stunde".to_string()
        } else {
            format!("{hours} Stunden")
        }
    } else {
        format!("{TIMEOUT_MINUTES} Minuten")
    };
    format!(
        "Du wurdest vorübergehend stummgeschaltet.\n\
         Grund: Dein Account hat in Sekunden Bilder in mehreren Kanälen gepostet — ein typisches \
         Muster für einen gekaperten Account. Falls du gehackt wurdest: Passwort ändern, \
         2FA aktivieren und beim Mod-Team melden, sobald du den Account zurück hast.\n\
         Dauer: {dauer}\n\
         Case: {case_id}\n\
         Falls das ein Irrtum war, wende dich ans Mod-Team — der Timeout wird aufgehoben."
    )
}

pub fn is_new_account(created_at: i64, now: i64) -> bool {
    now - created_at < ACCOUNT_MAX_AGE_HOURS * 3600
}

pub fn is_established_account(created_at: i64, joined_at: Option<i64>, now: i64) -> bool {
    let account_old = now - created_at >= ESTABLISHED_ACCOUNT_MIN_AGE_HOURS * 3600;
    let joined_long = joined_at
        .map(|joined| now - joined >= ESTABLISHED_MIN_JOIN_HOURS * 3600)
        .unwrap_or(false);
    account_old && joined_long
}

// ── Engine ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAction {
    /// Vollzug (Ban, da PUNISHMENT="ban") + Beweise + Logs.
    Enforce,
    /// Reversibler 60-min-Holding-Timeout mit Mod-Review.
    Propose,
    /// Account-Takeover-Quarantäne: reversibler 24h-Timeout + eigene
    /// Takeover-DM (Original: `_handle_takeover`/`_send_takeover_dm`).
    Takeover,
}

/// Discord-Seite (Tests mocken sie).
#[async_trait::async_trait]
pub trait GuardPort: Send + Sync {
    async fn ban(&self, guild_id: u64, user_id: u64, reason: &str) -> bool;
    async fn timeout(&self, guild_id: u64, user_id: u64, minutes: i64, reason: &str) -> bool;
    async fn delete_message(&self, channel_id: u64, message_id: u64) -> bool;
    async fn send_dm(&self, user_id: u64, text: String) -> bool;
    /// Mod-Alarm mit sg:*-Buttons in den Mod-Kanal.
    async fn post_mod_alert(&self, case: &Incident, action: &GuardAction);
    /// Kurze öffentliche Notiz in den betroffenen Kanal.
    async fn post_public_notice(&self, channel_id: u64, text: String);
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
}

pub struct SecurityGuard {
    pub db: Db,
    pub generator: Option<Arc<dyn dl_ai::TextGenerator>>,
    pub port: Arc<dyn GuardPort>,
    history: tokio::sync::Mutex<HashMap<u64, VecDeque<RecentMsg>>>,
    active: tokio::sync::Mutex<std::collections::HashSet<u64>>,
}

impl SecurityGuard {
    pub fn new(
        db: Db,
        generator: Option<Arc<dyn dl_ai::TextGenerator>>,
        port: Arc<dyn GuardPort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            generator,
            port,
            history: tokio::sync::Mutex::new(HashMap::new()),
            active: tokio::sync::Mutex::new(std::collections::HashSet::new()),
        })
    }

    pub async fn ensure_schema(&self) -> Result<(), dl_db::DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS security_guard_incidents (
                        case_id      TEXT PRIMARY KEY,
                        guild_id     INTEGER NOT NULL,
                        user_id      INTEGER NOT NULL,
                        user_tag     TEXT NOT NULL,
                        action       TEXT NOT NULL,
                        reason       TEXT NOT NULL,
                        channel_count   INTEGER DEFAULT 0,
                        message_count   INTEGER DEFAULT 0,
                        attachment_count INTEGER DEFAULT 0,
                        keyword_hit  INTEGER DEFAULT 0,
                        messages_json TEXT,
                        created_at   TEXT NOT NULL
                    )",
                )
            })
            .await
    }

    pub async fn handle_message(self: &Arc<Self>, event: &dl_discord::MessageEvent) {
        let Some(guild_id) = event.guild_id else {
            return;
        };
        if event.author_is_admin {
            return; // Staff wird nicht überwacht (manage_messages-Näherung)
        }
        let now = chrono::Utc::now().timestamp();

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
        }
    }

    async fn try_claim(&self, user_id: u64) -> bool {
        self.active.lock().await.insert(user_id)
    }

    async fn release(&self, user_id: u64) {
        self.active.lock().await.remove(&user_id);
    }

    /// → true wenn ein Case ausgelöst wurde (History wird dann geleert).
    async fn run_detection(
        self: &Arc<Self>,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        recent: &[RecentMsg],
        now: i64,
    ) -> bool {
        // 1. Takeover (alle Accounts, kein AI)
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
            self.execute(guild_id, event, burst, reason, meta, GuardAction::Takeover)
                .await;
            return true;
        }

        let young = is_new_account(event.author_created_at, now);

        // 2. Burst junger Accounts → AI-Text-Check
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
                let (is_scam, confidence, ai_reason) = self.ai_check(&combined).await;
                let action = if is_scam && confidence >= AI_SCAM_CONFIDENCE {
                    GuardAction::Enforce
                } else {
                    GuardAction::Propose
                };
                // `action` ist hier ausschließlich Enforce oder Propose
                // (zwei Zeilen darüber gesetzt); der Takeover-Pfad läuft
                // separat über `run_detection` und kommt hier nie an.
                let full_reason = match action {
                    GuardAction::Enforce => {
                        format!("{reason}; AI conf {:.0}%: {ai_reason}", confidence * 100.0)
                    }
                    GuardAction::Propose | GuardAction::Takeover => format!(
                        "{reason}; AI unbestätigt ({:.0}%: {ai_reason})",
                        confidence * 100.0
                    ),
                };
                self.execute(guild_id, event, recent.to_vec(), full_reason, meta, action)
                    .await;
                return true;
            }
        }

        // 3. Keyword-Einzeltreffer → AI; etabliert = Vorschlag, jung = Vollzug
        if contains_suspicious_text(&event.content) {
            let (is_scam, confidence, ai_reason) = self.ai_check(&event.content).await;
            if is_scam && confidence >= AI_SCAM_CONFIDENCE {
                let established =
                    is_established_account(event.author_created_at, event.author_joined_at, now);
                let action = if established {
                    GuardAction::Propose
                } else {
                    GuardAction::Enforce
                };
                let reason = format!("AI-Scam (conf {:.0}%): {ai_reason}", confidence * 100.0);
                let single = vec![RecentMsg {
                    channel_id: event.channel_id,
                    message_id: event.message_id,
                    created_at: now,
                    content: event.content.clone(),
                    attachment_count: event.attachment_count,
                    image_count: event.image_attachment_count,
                }];
                let meta = [1, 1, event.attachment_count as i64, 1];
                self.execute(guild_id, event, single, reason, meta, action)
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

    /// Vollzug oder Vorschlag: DM → Aktion → Nachrichten löschen →
    /// öffentliche Notiz → Mod-Alarm → Persistenz (Reihenfolge wie Original).
    async fn execute(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        messages: Vec<RecentMsg>,
        reason: String,
        meta: [i64; 4],
        action: GuardAction,
    ) {
        let case_id = format!("sg-{}-{}", event.author_id, chrono::Utc::now().timestamp());
        let incident = Incident {
            case_id: case_id.clone(),
            guild_id,
            user_id: event.author_id,
            user_tag: event.author_display_name.clone(),
            action: match action {
                GuardAction::Enforce => "ban".to_string(),
                GuardAction::Propose => "timeout-proposal".to_string(),
                GuardAction::Takeover => "takeover-quarantine".to_string(),
            },
            reason: reason.clone(),
            meta,
            messages: messages.clone(),
        };
        self.persist(&incident).await;

        // Takeover hat eine eigene DM (Hinweis auf möglichen Hack, reversibel),
        // alle anderen Pfade die generische Sicherheits-Muster-DM.
        let dm_text = match action {
            GuardAction::Takeover => takeover_dm_text(&case_id),
            _ => format!(
                "Dein Account hat auf der Deutschen Deadlock Community ein Sicherheits-Muster ausgelöst.\nGrund: {reason}\nCase: {case_id}\nWenn das ein Irrtum ist, melde dich beim Mod-Team."
            ),
        };
        let _ = self.port.send_dm(event.author_id, dm_text).await;

        let acted = match action {
            GuardAction::Enforce => self.port.ban(guild_id, event.author_id, &reason).await,
            GuardAction::Propose => {
                self.port
                    .timeout(guild_id, event.author_id, PROPOSAL_TIMEOUT_MINUTES, &reason)
                    .await
            }
            // Reversibler 24h-Timeout statt Ban (Original: `_apply_timeout`,
            // timeout_minutes=1440). Mod kann eskalieren oder aufheben.
            GuardAction::Takeover => {
                self.port
                    .timeout(guild_id, event.author_id, TIMEOUT_MINUTES, &reason)
                    .await
            }
        };
        if !acted {
            tracing::warn!(case_id, "SecurityGuard: Aktion fehlgeschlagen");
        }

        for msg in &messages {
            let _ = self
                .port
                .delete_message(msg.channel_id, msg.message_id)
                .await;
        }
        if let Some(last) = messages.last() {
            self.port
                .post_public_notice(
                    last.channel_id,
                    "🛡️ Eine Scam-Nachricht wurde entfernt und der Account in Quarantäne genommen."
                        .to_string(),
                )
                .await;
        }
        self.port.post_mod_alert(&incident, &action).await;
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
        .unwrap_or_default();
        let incident = incident.clone();
        let result = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO security_guard_incidents
                       (case_id, guild_id, user_id, user_tag, action, reason,
                        channel_count, message_count, attachment_count, keyword_hit,
                        messages_json, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11, datetime('now'))",
                    rusqlite::params![
                        incident.case_id,
                        incident.guild_id,
                        incident.user_id,
                        incident.user_tag,
                        incident.action,
                        incident.reason,
                        incident.meta[0],
                        incident.meta[1],
                        incident.meta[2],
                        incident.meta[3],
                        messages_json,
                    ],
                )
                .map(|_| ())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, "SecurityGuard: Incident-Persist fehlgeschlagen");
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
    use super::*;

    fn msg(channel: u64, secs_ago: i64, content: &str, images: u32) -> RecentMsg {
        RecentMsg {
            channel_id: channel,
            message_id: 1,
            created_at: 1_000_000 - secs_ago,
            content: content.to_string(),
            attachment_count: images,
            image_count: images,
        }
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
        // 3 Nachrichten in 3 Channels → multi-channel burst
        let msgs = vec![
            msg(1, 30, "hi", 0),
            msg(2, 20, "hi", 0),
            msg(3, 10, "hi", 0),
        ];
        let (reason, meta) = should_trigger(&msgs).expect("burst");
        assert!(reason.contains("multi-channel burst"));
        assert_eq!(meta, [3, 3, 0, 0]);
        // 2 Channels + Keyword → suspicious
        let msgs = vec![msg(1, 30, "join my telegram", 0), msg(2, 10, "hi", 0)];
        let (reason, meta) = should_trigger(&msgs).expect("sus");
        assert!(reason.contains("suspicious content"));
        assert!(reason.contains("keyword match"));
        assert_eq!(meta[3], 1);
        // 2 harmlose Nachrichten in 2 Channels → nichts
        let msgs = vec![msg(1, 30, "hi", 0), msg(2, 10, "ho", 0)];
        assert!(should_trigger(&msgs).is_none());
        // Anhänge in 2 Channels → attachments-Trigger
        let msgs = vec![msg(1, 30, "", 1), msg(2, 10, "", 1)];
        let (reason, _) = should_trigger(&msgs).expect("attach");
        assert!(reason.contains("attachments across 2+ channels"));
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
        assert!(is_new_account(now - 100 * hour, now)); // 100h alt < 720h
        assert!(!is_new_account(now - 800 * hour, now));
        assert!(is_established_account(
            now - 800 * hour,
            Some(now - 30 * hour),
            now
        ));
        // Account alt genug, aber erst 1h auf dem Server → nicht etabliert
        assert!(!is_established_account(
            now - 800 * hour,
            Some(now - hour),
            now
        ));
        assert!(!is_established_account(now - 800 * hour, None, now));
    }

    #[test]
    fn takeover_dm_24h() {
        let text = takeover_dm_text("sg-1-2");
        // 1440 min → "24 Stunden", Case-ID enthalten, Hack-Hinweis vorhanden.
        assert!(text.contains("24 Stunden"));
        assert!(text.contains("sg-1-2"));
        assert!(text.contains("gekaperten Account"));
    }

    #[test]
    fn keywords() {
        assert!(contains_suspicious_text("Join my TELEGRAM channel"));
        assert!(contains_suspicious_text("first 10 people earning $"));
        assert!(!contains_suspicious_text("wer bock auf deadlock"));
    }
}
