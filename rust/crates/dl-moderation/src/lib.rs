//! dl-moderation — Moderationslogik fuer Discord-Faelle.
//!
//! Legacy-Pipeline: Nachricht im Scan-Kanal → Text-Klassifikation
//! (System-Prompt wortgleich) → Schwellen-Routing:
//! - `delete` + Auto-Kategorie + confidence ≥ 0.90 → Auto-Delete + Case
//! - `delete`/`propose` + confidence ≥ 0.78 → Mod-Review-Case (Buttons
//!   `aimod:accept|ban|deny:{case_id}` — custom_ids unverändert)
//! - `ragebait_ok` → Fenster-Zählung (4 Treffer / 120 min → Eskalation
//!   als `persistent_ragebait`-Vorschlag)
//! - `needs_context`/`ok` → nichts
//!
//! Der neue aktive Scanner sitzt in den sprechenden Modulen
//! `content_analyzer`, `content_verifier`, `behavior_detector`,
//! `moderation_verdict`, `action_policy`, `case_embed` und
//! `moderation_system`.
//!
//! Der Ragebaiter-Free-Warnhinweis ist portiert: in Kanälen mit
//! `required_tone_tag = "ragebaiter_free"` (aus `tempvoice_lane_tag_filter`)
//! bekommt ein `ragebait_ok`-Treffer mit Confidence 0.40–0.78 eine
//! niederschwellige Verwarn-DM; ein zweiter Treffer innerhalb von 30 min
//! eskaliert stattdessen zum Mod-Vorschlag.
//!
//! Bild-only-Nachrichten (Original: `image_attachments`) laufen über die
//! Vision-Klassifikation (`generate_multimodal`, gleicher System-Prompt,
//! Temperatur 0.2): leerer Text wird nur dann übersprungen, wenn auch keine
//! Bild-Anhänge vorliegen oder kein Vision-Generator verdrahtet ist.
//!
//! Kontext-Backfill-Eskalation (12 Nachrichten bei `needs_context` oder
//! 0.55–0.78) und der reichere Prompt-Payload sind portiert. Der Staff-Skip
//! nutzt die Dispatcher-Rechte fuer Admins/`manage_messages`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use sqlx::PgPool;

pub mod action_policy;
pub mod behavior_detector;
pub mod case_embed;
pub mod content_analyzer;
pub mod content_verifier;
pub mod moderation_channel;
pub mod moderation_system;
pub mod moderation_verdict;
pub mod store;

pub use moderation_system::ModerationSystem;

pub const SCAN_CHANNEL_IDS: [u64; 1] = [1289721245281292291];
pub const MOD_REVIEW_CHANNEL_ID: u64 = 1315684135175716978;
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const TIMEOUT_MINUTES: i64 = 1440;
pub const RAGEBAIT_WINDOW_MINUTES: i64 = 120;
pub const RAGEBAIT_ESCALATE_THRESHOLD: i64 = 4;
pub const AUTO_DELETE_CONFIDENCE: f64 = 0.90;
pub const PROPOSE_CONFIDENCE: f64 = 0.78;
pub const CONTEXT_ESCALATE_LOWER: f64 = 0.55;
pub const CONTEXT_ESCALATE_UPPER: f64 = 0.78;
pub const CONTEXT_BACKFILL_MESSAGES: usize = 12;
pub const PER_USER_COOLDOWN_SECONDS: f64 = 2.0;
pub const MAX_PROMPT_CHARS: usize = 4000;
/// Niederschwellige Confidence-Untergrenze für die Ragebaiter-Free-Warnung
/// (Original: RAGEBAITER_FREE_PROPOSE_CONFIDENCE).
pub const RAGEBAITER_FREE_PROPOSE_CONFIDENCE: f64 = 0.40;
/// Fenster, in dem nicht erneut gewarnt, sondern eskaliert wird (30 min).
pub const RAGEBAITER_FREE_WARNING_WINDOW_SECONDS: i64 = 1800;

pub const AUTO_DELETE_CATEGORIES: [&str; 5] =
    ["nsfw_explicit", "csam", "raping", "epstein_child", "scam"];
pub const DISABLED_CATEGORIES: [&str; 1] = ["racism"];

pub const ALLOWED_VERDICTS: [&str; 4] = ["ok", "delete", "propose", "needs_context"];
pub const ALLOWED_CATEGORIES: [&str; 11] = [
    "nsfw_explicit",
    "csam",
    "raping",
    "epstein_child",
    "racism",
    "harassment",
    "hate_speech",
    "ragebait_ok",
    "game_related_ok",
    "scam",
    "other",
];

/// System-Prompt wortgleich zum Original (MODERATION_SYSTEM_PROMPT).
pub const MODERATION_SYSTEM_PROMPT: &str = r#"Du bist ein Discord-Moderations-Klassifikator fuer einen kompetitiven Gaming-Server zu Deadlock.
Deine Aufgabe: Bewerte die letzte Nachricht eines Users. Antworte ausschliesslich mit gueltigem JSON und ohne weiteren Text.

WICHTIG — Standardmaessig ist der Ton auf diesem Server rau und direkt. Sei NICHT ueberempfindlich.

Klare Lösch-Faelle (verdict="delete", hohe confidence):
- NSFW explizit, Pornografie, sexuelle Gewalt, Raping, CSAM, Minderjaehrigen-Sexualisierung, Epstein-Anspielungen.
- Crypto-Scam, Advance-Fee-Fraud, Investment-Betrug, Gewinnversprechen mit Rueckzahlungspflicht (z. B. "I'll help the first 10 people earn $100k from crypto").

Moderationsvorschlag nur bei echtem Verstoss (verdict="propose", confidence >= 0.80):
- Gezielte rassistische Beleidigungen oder Slurs mit klarer Diskriminierungsabsicht.
- Nackte Hassrede gegen eine Gruppe (ethnisch, religioese etc.) ohne jeglichen Gaming-Kontext.
- Anhaltende persoenliche Angriffe gegen einen konkreten User ueber mehrere Nachrichten.

Immer OK — NICHT flaggen (verdict="ok"):
- Gaming-typische Laender-Klischees oder Nationalitaets-Kommentare im Spiel-Kontext ("Russe", "typisch Brasilianer", etc.) — das ist Spielkultur, kein Hate Speech.
- Einmalige kurze Beleidigungen wie "halts Maul", "noob", "trash", "fick dich" im Spielfluss.
- Einzelne kurze Nachrichten oder Ein-Wort-Antworten (z.B. "lol", "ok", "nein") — fast nie Harassment.
- Jemand der schlechtes Verhalten MELDET oder KOMMENTIERT ("da war ein Hakenkreuz im GIF", "der hat gerade X gesagt") — das ist Reporting, kein Verstoss.
- Ragebait, Flame, Trash Talk generell — der Bot zaehlt das separat, du klassifizierst es als "ragebait_ok".
- Spielkritik, Balance-Beschwerden, Patchnotes, Matchmaking-Frust.

Kontext korrekt lesen:
- Nachrichten mit ">>>" am Anfang stammen vom GLEICHEN User der bewertet wird — so siehst du sein Verhaltensmuster.
- Zeitstempel wie "[2min ago]" zeigen wie frisch der Kontext ist — alles ueber 30 Minuten ist wahrscheinlich ein anderes Gespraech.
- Wenn "is_reply_to" vorhanden: die Nachricht ist eine DIREKTE ANTWORT darauf — bewerte sie immer im Kontext dieser Provokation.
- Wenn jemand auf eine Beleidigung reagiert, ist die Reaktion milder zu werten als der Ausloser.

Zweifelsfaelle:
- Wenn der Kontext unklar ist oder du dir nicht sicher bist: verdict="needs_context".
- Lieber zu wenig flaggen als zu viel — Mods koennen selbst eingreifen.

Erlaubte Kategorien:
nsfw_explicit, csam, raping, epstein_child, racism, harassment, hate_speech, ragebait_ok, game_related_ok, scam, other

Output-Format strikt:
{"verdict":"ok|delete|propose|needs_context","category":"...","confidence":0.0,"reason":"1-2 Saetze Deutsch","needs_context":true}
"#;

// ── Verdict-Parsing (wie _parse_ai_verdict) ────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct AiVerdict {
    pub verdict: String,
    pub category: String,
    pub confidence: f64,
    pub reason: String,
    pub raw_json: String,
}

fn raw_envelope(raw_text: Option<&str>, parsed: Option<Value>) -> String {
    let mut envelope = serde_json::Map::new();
    envelope.insert("response_text".to_string(), serde_json::json!(raw_text));
    if let Some(parsed) = parsed {
        envelope.insert("parsed".to_string(), parsed);
    }
    Value::Object(envelope).to_string()
}

fn parse_error_verdict(raw_text: Option<&str>) -> AiVerdict {
    AiVerdict {
        verdict: "needs_context".to_string(),
        category: "other".to_string(),
        confidence: 0.0,
        reason: "parse_error".to_string(),
        raw_json: raw_envelope(raw_text, None),
    }
}

/// JSON aus der AI-Antwort ziehen — greedy `\{.*\}` über Zeilen hinweg
/// (erstes `{` bis letztes `}`), Felder validieren und klemmen.
pub fn parse_ai_verdict(raw_text: Option<&str>) -> AiVerdict {
    let Some(raw) = raw_text else {
        return parse_error_verdict(raw_text);
    };
    let (Some(open), Some(close)) = (raw.find('{'), raw.rfind('}')) else {
        return parse_error_verdict(raw_text);
    };
    if close <= open {
        return parse_error_verdict(raw_text);
    }
    let Ok(payload) = serde_json::from_str::<Value>(&raw[open..=close]) else {
        return parse_error_verdict(raw_text);
    };
    let mut verdict = payload
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("needs_context")
        .trim()
        .to_lowercase();
    let mut category = payload
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("other")
        .trim()
        .to_lowercase();
    let confidence = payload
        .get("confidence")
        .and_then(|c| {
            c.as_f64()
                .or_else(|| c.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let mut reason: String = payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect();
    if reason.is_empty() {
        reason = "Keine Begruendung geliefert.".to_string();
    }
    if !ALLOWED_VERDICTS.contains(&verdict.as_str()) {
        verdict = "needs_context".to_string();
    }
    if !ALLOWED_CATEGORIES.contains(&category.as_str()) {
        category = "other".to_string();
    }
    AiVerdict {
        verdict,
        category,
        confidence,
        reason,
        raw_json: raw_envelope(raw_text, Some(payload)),
    }
}

// ── Schwellen-Routing (wie on_message) ─────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModAction {
    /// Sofort löschen + Case anlegen.
    AutoDelete,
    /// Mod-Review-Case mit Buttons.
    Propose,
    /// Ragebait zählen (Eskalation entscheidet der Fenster-Stand).
    CountRagebait,
    /// Nichts tun.
    Ignore,
}

pub fn decide_action(verdict: &AiVerdict, proposal_threshold: f64) -> ModAction {
    if verdict.verdict == "needs_context" {
        return ModAction::Ignore;
    }
    if DISABLED_CATEGORIES.contains(&verdict.category.as_str()) {
        return ModAction::Ignore;
    }
    if verdict.verdict == "ok" {
        return if verdict.category == "ragebait_ok" {
            ModAction::CountRagebait
        } else {
            ModAction::Ignore
        };
    }
    if verdict.verdict == "delete"
        && AUTO_DELETE_CATEGORIES.contains(&verdict.category.as_str())
        && verdict.confidence >= AUTO_DELETE_CONFIDENCE
    {
        return ModAction::AutoDelete;
    }
    if verdict.confidence < proposal_threshold {
        return ModAction::Ignore;
    }
    if verdict.verdict == "delete" || verdict.verdict == "propose" {
        return ModAction::Propose;
    }
    ModAction::Ignore
}

pub fn needs_context_escalation(verdict: &AiVerdict) -> bool {
    verdict.verdict == "needs_context"
        || (CONTEXT_ESCALATE_LOWER..CONTEXT_ESCALATE_UPPER).contains(&verdict.confidence)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyContext {
    pub author: String,
    pub content: String,
}

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

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

fn build_prompt_payload(
    event: &dl_discord::MessageEvent,
    context_lines: &[String],
    attachment_count: usize,
    include_full_context: bool,
    replied_to: Option<&ReplyContext>,
) -> String {
    let mut focus_message = strip_mentions(&event.content);
    if focus_message.is_empty() {
        focus_message = "[kein Text]".to_string();
    }
    focus_message = truncate_chars(&focus_message, 1000);

    let context_limit = if include_full_context { 110 } else { 200 };
    let mut recent_context: Vec<String> = context_lines
        .iter()
        .map(|line| truncate_chars(line, context_limit))
        .collect();

    let mut payload = serde_json::json!({
        "user_tag": truncate_chars(&event.author_display_name, 80),
        "user_message": focus_message,
        "attachment_count": attachment_count,
        "recent_context": recent_context,
        "context_note": "Lines starting with '>>>' are from the SAME user being evaluated. Timestamps show how long ago each message was sent.",
    });
    if let Some(reply) = replied_to {
        payload["is_reply_to"] = serde_json::json!({
            "author": truncate_chars(&reply.author, 60),
            "content": truncate_chars(&reply.content, 300),
        });
        payload["reply_note"] = serde_json::json!(
            "The evaluated message is a DIRECT REPLY to 'is_reply_to'. Read it as a response to that specific message, not in isolation."
        );
    }
    if include_full_context {
        payload["analysis_stage"] = serde_json::json!("context_escalation");
        payload["focus_message_id"] = serde_json::json!(event.message_id.to_string());
    }

    let mut prompt = payload.to_string();
    while prompt.len() > MAX_PROMPT_CHARS && !recent_context.is_empty() {
        recent_context.remove(0);
        payload["recent_context"] = serde_json::json!(recent_context);
        prompt = payload.to_string();
    }
    if prompt.len() > MAX_PROMPT_CHARS {
        let overflow = prompt.len().saturating_sub(MAX_PROMPT_CHARS);
        let reduced_limit = 1000usize.saturating_sub(overflow).max(200);
        payload["user_message"] = serde_json::json!(truncate_chars(
            payload
                .get("user_message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            reduced_limit,
        ));
        prompt = payload.to_string();
    }
    prompt
}

// ── Engine ─────────────────────────────────────────────────────────────────

/// Discord-Aktionen des Moderators (Tests mocken sie).
#[async_trait::async_trait]
pub trait ModPort: Send + Sync {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool;
    async fn timeout_member(&self, guild_id: u64, user_id: u64, minutes: i64) -> bool;
    async fn ban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool;
    /// Review-Embed mit aimod:*-Buttons posten → message_id.
    async fn post_review(&self, case: &store::CaseDraft, buttons_case_id: &str) -> Option<u64>;
    async fn post_log(&self, text: String);
    async fn post_case_log(
        &self,
        case: &store::CaseDraft,
        _case_id: &str,
        action: &str,
    ) -> Option<u64> {
        self.post_log(format!(
            "{action} ({}, {:.0}%): <@{}> in <#{}> — {}",
            case.category,
            case.confidence * 100.0,
            case.user_id,
            case.channel_id,
            case.reason,
        ))
        .await;
        None
    }
    /// Reine Text-DM an einen User (Original: `_send_ragebaiter_free_hint`).
    async fn send_dm(&self, user_id: u64, text: String);
    async fn add_mod_tag(&self, _user_id: u64, _tag: &str, _reason: &str) -> bool {
        false
    }
    async fn fetch_context_lines(
        &self,
        _guild_id: u64,
        _channel_id: u64,
        _before_message_id: u64,
        _author_id: u64,
        _message_created_at: i64,
        _limit: usize,
    ) -> Vec<String> {
        Vec::new()
    }
    async fn fetch_reply_context(
        &self,
        _guild_id: u64,
        _channel_id: u64,
        _reply_channel_id: Option<u64>,
        _reply_message_id: Option<u64>,
    ) -> Option<ReplyContext> {
        None
    }
}

/// Ergebnis einer Review-Aktion (Button-Reply-Text für den Mod).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewOutcome {
    NotFound,
    AlreadyHandled,
    Done(String),
}

pub struct AiModerator {
    pub store: store::ModerationStore,
    pub generator: Arc<dyn dl_ai::TextGenerator>,
    /// Optionaler Vision-Pfad für Bild-only-Nachrichten (Original:
    /// `generate_multimodal`). Ohne Vision-Generator werden reine Bild-
    /// nachrichten wie bisher übersprungen.
    pub vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
    pub port: Arc<dyn ModPort>,
    cooldown: tokio::sync::Mutex<HashMap<u64, std::time::Instant>>,
    /// (user_id, channel_id) → letzter Ragebaiter-Free-Warnzeitpunkt (Unix).
    /// Doppel-Schutz wie das Original-Dict `_ragebaiter_free_warnings`.
    ragebaiter_free_warnings: tokio::sync::Mutex<HashMap<(u64, u64), i64>>,
}

impl AiModerator {
    pub fn new(
        pool: PgPool,
        generator: Arc<dyn dl_ai::TextGenerator>,
        vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
        port: Arc<dyn ModPort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: store::ModerationStore { pool },
            generator,
            vision,
            port,
            cooldown: tokio::sync::Mutex::new(HashMap::new()),
            ragebaiter_free_warnings: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Tone-Tag des Kanals aus `tempvoice_lane_tag_filter` (Original:
    /// `_get_required_tone_tag_sync`). Tabelle/Spalte werden von dl-voice
    /// gepflegt — hier nur lesend.
    async fn required_tone_tag(&self, channel_id: u64) -> Option<String> {
        let channel_id = match i64::try_from(channel_id) {
            Ok(channel_id) => channel_id,
            Err(err) => {
                tracing::warn!(%err, channel_id, "Moderation: Channel-ID passt nicht in BIGINT");
                return None;
            }
        };
        let raw = sqlx::query!(
            r#"
            SELECT required_tone_tag
            FROM voice.tempvoice_lane_tag_filter
            WHERE channel_id = $1
            "#,
            channel_id,
        )
        .fetch_optional(&self.store.pool)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.required_tone_tag);
        raw.map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
    }

    /// Niederschwellige Verwarn-DM im Ragebaiter-Free-Channel, mit Doppel-Schutz
    /// (Original: `_maybe_handle_ragebaiter_free_warning`). → true, wenn der
    /// Pfad behandelt wurde (Warnung gesendet ODER eskaliert).
    async fn maybe_handle_ragebaiter_free_warning(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: &AiVerdict,
        escalated_with_context: bool,
    ) -> bool {
        if self.required_tone_tag(event.channel_id).await.as_deref() != Some("ragebaiter_free") {
            return false;
        }
        if !(RAGEBAITER_FREE_PROPOSE_CONFIDENCE..PROPOSE_CONFIDENCE).contains(&verdict.confidence) {
            return false;
        }
        let now = chrono::Utc::now().timestamp();
        let key = (event.author_id, event.channel_id);
        let recently_warned = {
            let mut warnings = self.ragebaiter_free_warnings.lock().await;
            warnings.retain(|_, ts| now - *ts <= RAGEBAITER_FREE_WARNING_WINDOW_SECONDS);
            match warnings.get(&key) {
                Some(_) => true,
                None => {
                    warnings.insert(key, now);
                    false
                }
            }
        };
        if recently_warned {
            // Innerhalb von 30 min schon gewarnt → statt erneuter Warnung ein
            // Mod-Vorschlag (Original: eskaliert zu `propose`).
            let escalated = AiVerdict {
                verdict: "propose".to_string(),
                category: verdict.category.clone(),
                confidence: RAGEBAITER_FREE_PROPOSE_CONFIDENCE.max(verdict.confidence),
                reason: format!(
                    "{}\nHinweis fuer Ragebaiter-Free-Channel bereits innerhalb von 30 Minuten erfolgt.",
                    verdict.reason
                )
                .chars()
                .take(900)
                .collect(),
                raw_json: verdict.raw_json.clone(),
            };
            self.create_proposal(
                guild_id,
                event,
                &escalated,
                escalated_with_context,
                "proposed",
            )
            .await;
        } else {
            self.port
                .send_dm(
                    event.author_id,
                    "Hinweis: Dieser Voice ist als `Ragebaiter-Free` markiert, bitte Provokationen reduzieren.".to_string(),
                )
                .await;
        }
        true
    }

    pub async fn handle_message(self: &Arc<Self>, event: &dl_discord::MessageEvent) {
        let Some(guild_id) = event.guild_id else {
            return;
        };
        if !SCAN_CHANNEL_IDS.contains(&event.channel_id) {
            return;
        }
        if event.author_is_admin || event.author_can_manage_messages {
            return;
        }
        // Original (`ai_moderator.py`): überspringe nur, wenn WEDER Text NOCH
        // Bild-Anhänge vorliegen. Reine Bild-Nachrichten laufen über die
        // Vision-Klassifikation (sofern ein Vision-Generator verdrahtet ist).
        let has_text = !event.content.trim().is_empty();
        let image_urls: Vec<String> = if self.vision.is_some() {
            event.image_attachment_urls.clone()
        } else {
            Vec::new()
        };
        if !has_text && image_urls.is_empty() {
            return;
        }
        {
            let mut cooldown = self.cooldown.lock().await;
            let now = std::time::Instant::now();
            if let Some(last) = cooldown.get(&event.author_id) {
                if now.duration_since(*last).as_secs_f64() < PER_USER_COOLDOWN_SECONDS {
                    return;
                }
            }
            cooldown.insert(event.author_id, now);
        }

        let (verdict, escalated_with_context) =
            self.classify_message(guild_id, event, &image_urls).await;

        match decide_action(&verdict, PROPOSE_CONFIDENCE) {
            ModAction::Ignore => {}
            ModAction::AutoDelete => {
                let Some(case_id) = self
                    .store
                    .insert_case(self.draft(
                        guild_id,
                        event,
                        &verdict,
                        "auto_delete",
                        escalated_with_context,
                    ))
                    .await
                else {
                    tracing::warn!(
                        guild_id,
                        channel_id = event.channel_id,
                        message_id = event.message_id,
                        user_id = event.author_id,
                        "AI-Moderation: Auto-Delete wegen fehlgeschlagener Case-Persistenz uebersprungen"
                    );
                    return;
                };
                let deleted = self
                    .port
                    .delete_message(
                        event.channel_id,
                        event.message_id,
                        "AI-Moderation: Auto-Delete",
                    )
                    .await;
                let timed_out = self
                    .port
                    .timeout_member(guild_id, event.author_id, TIMEOUT_MINUTES)
                    .await;
                let action = if deleted && timed_out {
                    "auto_delete"
                } else {
                    "auto_delete_failed"
                };
                self.store.update_case_action(&case_id, action).await;
                self.port
                    .post_log(format!(
                        "🤖 Auto-Delete ({}, {:.0}%): <@{}> in <#{}> — {} {}{}",
                        verdict.category,
                        verdict.confidence * 100.0,
                        event.author_id,
                        event.channel_id,
                        verdict.reason,
                        if deleted {
                            ""
                        } else {
                            "(Delete fehlgeschlagen)"
                        },
                        if timed_out {
                            ""
                        } else {
                            "(Timeout fehlgeschlagen)"
                        },
                    ))
                    .await;
                tracing::info!(case_id, "AI-Moderation: Auto-Delete-Case angelegt");
            }
            ModAction::Propose => {
                self.create_proposal(
                    guild_id,
                    event,
                    &verdict,
                    escalated_with_context,
                    "proposed",
                )
                .await;
            }
            ModAction::CountRagebait => {
                let (count, previews) = self
                    .store
                    .insert_ragebait_hit(
                        guild_id,
                        event.author_id,
                        event.message_id,
                        event.channel_id,
                        &event.content,
                    )
                    .await;
                if count >= RAGEBAIT_ESCALATE_THRESHOLD {
                    let mut lines = vec![
                        "Wiederholtes Ragebait innerhalb des Zeitfensters erkannt:".to_string(),
                    ];
                    for preview in previews
                        .iter()
                        .rev()
                        .take(RAGEBAIT_ESCALATE_THRESHOLD as usize)
                    {
                        lines.push(format!(
                            "- {}",
                            preview.chars().take(90).collect::<String>()
                        ));
                    }
                    let escalated = AiVerdict {
                        verdict: "propose".to_string(),
                        category: "persistent_ragebait".to_string(),
                        confidence: PROPOSE_CONFIDENCE.max(verdict.confidence),
                        reason: format!("{}\n{}", verdict.reason, lines.join("\n"))
                            .chars()
                            .take(900)
                            .collect(),
                        raw_json: verdict.raw_json.clone(),
                    };
                    self.port
                        .add_mod_tag(event.author_id, "ragebaiter", "auto: persistent_ragebait")
                        .await;
                    self.create_proposal(
                        guild_id,
                        event,
                        &escalated,
                        escalated_with_context,
                        "ragebait_escalated",
                    )
                    .await;
                    return;
                }
                // Keine Eskalation: in Ragebaiter-Free-Kanälen ggf. eine
                // niederschwellige Verwarn-DM (mit Doppel-Schutz).
                self.maybe_handle_ragebaiter_free_warning(
                    guild_id,
                    event,
                    &verdict,
                    escalated_with_context,
                )
                .await;
            }
        }
    }

    async fn classify_message(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        image_urls: &[String],
    ) -> (AiVerdict, bool) {
        let minimal_context = self
            .port
            .fetch_context_lines(
                guild_id,
                event.channel_id,
                event.message_id,
                event.author_id,
                event.message_created_at,
                2,
            )
            .await;
        let verdict = self
            .run_moderation_call(guild_id, event, image_urls, &minimal_context, false)
            .await;
        if !needs_context_escalation(&verdict) {
            return (verdict, false);
        }

        let full_context = self
            .port
            .fetch_context_lines(
                guild_id,
                event.channel_id,
                event.message_id,
                event.author_id,
                event.message_created_at,
                CONTEXT_BACKFILL_MESSAGES,
            )
            .await;
        let escalated = self
            .run_moderation_call(guild_id, event, image_urls, &full_context, true)
            .await;
        (escalated, true)
    }

    async fn run_moderation_call(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        image_urls: &[String],
        context_lines: &[String],
        include_full_context: bool,
    ) -> AiVerdict {
        let replied_to = self
            .port
            .fetch_reply_context(
                guild_id,
                event.channel_id,
                event.reply_channel_id,
                event.reply_message_id,
            )
            .await;
        let prompt = build_prompt_payload(
            event,
            context_lines,
            image_urls.len(),
            include_full_context,
            replied_to.as_ref(),
        );

        if image_urls.is_empty() {
            let raw = self
                .generator
                .generate_text(dl_ai::GenerateRequest {
                    prompt,
                    system_prompt: Some(MODERATION_SYSTEM_PROMPT.to_string()),
                    model: None,
                    max_output_tokens: Some(300),
                    reasoning_effort: None,
                    temperature: 0.0,
                })
                .await;
            return parse_ai_verdict(raw.as_deref());
        }

        let Some(vision) = &self.vision else {
            return parse_error_verdict(None);
        };
        let raw = vision
            .generate_multimodal(dl_ai::GenerateMultimodalRequest {
                prompt,
                image_urls: image_urls.to_vec(),
                system_prompt: Some(MODERATION_SYSTEM_PROMPT.to_string()),
                model: None,
                max_output_tokens: Some(300),
                temperature: 0.2,
            })
            .await;
        parse_ai_verdict(raw.as_deref())
    }

    fn draft(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: &AiVerdict,
        action: &str,
        escalated_with_context: bool,
    ) -> store::CaseDraft {
        store::CaseDraft {
            guild_id,
            channel_id: event.channel_id,
            message_id: event.message_id,
            user_id: event.author_id,
            user_tag: event.author_display_name.clone(),
            content: event.content.clone(),
            category: verdict.category.clone(),
            confidence: verdict.confidence,
            reason: verdict.reason.clone(),
            action: action.to_string(),
            source: "content".to_string(),
            trigger_type: Some("content".to_string()),
            attachments: event
                .attachments
                .iter()
                .map(|attachment| store::CaseAttachment {
                    url: attachment.url.clone(),
                    content_type: attachment.content_type.clone(),
                    filename: attachment.filename.clone(),
                })
                .collect(),
            ai_raw_json: verdict.raw_json.clone(),
            timeout_minutes: None,
            escalated_with_context,
        }
    }

    async fn create_proposal(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: &AiVerdict,
        escalated_with_context: bool,
        action: &str,
    ) {
        let draft = self.draft(guild_id, event, verdict, action, escalated_with_context);
        let Some(case_id) = self.store.insert_case(draft.clone()).await else {
            tracing::warn!(
                guild_id,
                channel_id = event.channel_id,
                message_id = event.message_id,
                user_id = event.author_id,
                action,
                "AI-Moderation: Proposal wegen fehlgeschlagener Case-Persistenz uebersprungen"
            );
            return;
        };
        if let Some(message_id) = self.port.post_review(&draft, &case_id).await {
            self.store.set_review_message(&case_id, message_id).await;
        }
        if let Some(message_id) = self.port.post_case_log(&draft, &case_id, action).await {
            self.store.set_log_message(&case_id, message_id).await;
        }
    }

    // ── Review-Aktionen (aimod:accept|ban|deny) ─────────────────────────────
    //
    // Spiegelt `handle_accept_interaction`/`handle_ban_interaction`/
    // `handle_deny_submit` aus `cogs/ai_moderator.py`: gleiche Idempotenz-
    // Checks, gleiche Reihenfolge (Aktion → DB-Status → Action-Log).

    /// Annehmen: Nachricht löschen + Member 24h timeouten + Log posten.
    pub async fn accept_case(&self, case_id: &str, mod_id: u64) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if matches!(case.action.as_str(), "accepted" | "denied") {
            return ReviewOutcome::AlreadyHandled;
        }
        let deleted = self
            .port
            .delete_message(
                case.channel_id,
                case.message_id,
                &format!("AI-Moderator Case {}", case.case_id),
            )
            .await;
        let timed_out = self
            .port
            .timeout_member(case.guild_id, case.user_id, TIMEOUT_MINUTES)
            .await;
        self.store.resolve_case(case_id, "accepted", mod_id).await;
        self.post_action_log(
            &case,
            "accepted",
            mod_id,
            &[
                (deleted, "Delete fehlgeschlagen"),
                (timed_out, "Timeout fehlgeschlagen"),
            ],
            None,
        )
        .await;
        ReviewOutcome::Done("Moderationsvorschlag akzeptiert.".to_string())
    }

    /// Ban: Nachricht löschen + Member bannen + Log posten.
    pub async fn ban_case(&self, case_id: &str, mod_id: u64) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if matches!(case.action.as_str(), "accepted" | "denied" | "banned") {
            return ReviewOutcome::AlreadyHandled;
        }
        let deleted = self
            .port
            .delete_message(
                case.channel_id,
                case.message_id,
                &format!("AI-Moderator Case {}", case.case_id),
            )
            .await;
        let banned = self
            .port
            .ban_member(
                case.guild_id,
                case.user_id,
                &format!("AI-Moderator Ban {}", case.case_id),
            )
            .await;
        self.store.resolve_case(case_id, "banned", mod_id).await;
        self.post_action_log(
            &case,
            "banned",
            mod_id,
            &[
                (deleted, "Delete fehlgeschlagen"),
                (banned, "Ban fehlgeschlagen"),
            ],
            None,
        )
        .await;
        ReviewOutcome::Done("User gebannt.".to_string())
    }

    /// Ablehnen: Pflicht-Grund speichern + Log posten (keine Member-Aktion).
    pub async fn deny_case(&self, case_id: &str, mod_id: u64, reason: &str) -> ReviewOutcome {
        let Some(case) = self.store.fetch_case(case_id).await else {
            return ReviewOutcome::NotFound;
        };
        if matches!(case.action.as_str(), "accepted" | "denied") {
            return ReviewOutcome::AlreadyHandled;
        }
        self.store
            .resolve_case_denied(case_id, mod_id, reason)
            .await;
        self.post_action_log(&case, "denied", mod_id, &[], Some(reason))
            .await;
        ReviewOutcome::Done("Moderationsvorschlag abgelehnt.".to_string())
    }

    /// Action-Log wie `_post_action_log`/`_build_log_embed` (hier als Text-
    /// Zeile im selben Stil wie der bestehende Auto-Delete-Log).
    async fn post_action_log(
        &self,
        case: &store::CaseRecord,
        action: &str,
        mod_id: u64,
        notes: &[(bool, &str)],
        deny_reason: Option<&str>,
    ) {
        let label = match action {
            "accepted" => "✅ Accepted",
            "banned" => "🔨 Banned",
            "denied" => "❌ Denied",
            other => other,
        };
        let mut text = format!(
            "{label} ({}, {:.0}%): <@{}> in <#{}> durch <@{}> — {}",
            case.category,
            case.confidence * 100.0,
            case.user_id,
            case.channel_id,
            mod_id,
            case.reason,
        );
        if let Some(reason) = deny_reason {
            let trimmed: String = reason.chars().take(200).collect();
            text.push_str(&format!("\nDeny-Grund: {trimmed}"));
        }
        let failures: Vec<&str> = notes
            .iter()
            .filter(|(ok, _)| !ok)
            .map(|(_, note)| *note)
            .collect();
        if !failures.is_empty() {
            text.push_str(&format!(" ({})", failures.join("; ")));
        }
        self.port.post_log(text).await;
    }
}

/// Subscriber am Message-Dispatcher.
pub fn spawn(
    moderator: Arc<AiModerator>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => moderator.handle_message(&event).await,
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
    use tokio::sync::Mutex;

    struct StaticGenerator {
        response: String,
    }

    #[async_trait::async_trait]
    impl dl_ai::TextGenerator for StaticGenerator {
        async fn generate_text(&self, _request: dl_ai::GenerateRequest) -> Option<String> {
            Some(self.response.clone())
        }
    }

    #[derive(Default)]
    struct TestPort {
        reviews: Mutex<Vec<String>>,
        logs: Mutex<Vec<String>>,
        tags: Mutex<Vec<(u64, String, String)>>,
        dms: Mutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl ModPort for TestPort {
        async fn delete_message(&self, _channel_id: u64, _message_id: u64, _reason: &str) -> bool {
            true
        }

        async fn timeout_member(&self, _guild_id: u64, _user_id: u64, _minutes: i64) -> bool {
            true
        }

        async fn ban_member(&self, _guild_id: u64, _user_id: u64, _reason: &str) -> bool {
            true
        }

        async fn post_review(
            &self,
            case: &store::CaseDraft,
            _buttons_case_id: &str,
        ) -> Option<u64> {
            self.reviews.lock().await.push(case.action.clone());
            Some(42)
        }

        async fn post_log(&self, text: String) {
            self.logs.lock().await.push(text);
        }

        async fn post_case_log(
            &self,
            _case: &store::CaseDraft,
            _case_id: &str,
            action: &str,
        ) -> Option<u64> {
            self.logs.lock().await.push(action.to_string());
            Some(43)
        }

        async fn send_dm(&self, user_id: u64, _text: String) {
            self.dms.lock().await.push(user_id);
        }

        async fn add_mod_tag(&self, user_id: u64, tag: &str, reason: &str) -> bool {
            self.tags
                .lock()
                .await
                .push((user_id, tag.to_string(), reason.to_string()));
            true
        }
    }

    fn event(message_id: u64, content: &str) -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id: Some(1),
            channel_id: SCAN_CHANNEL_IDS[0],
            message_id,
            author_id: 100,
            author_display_name: "Anna".into(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.into(),
            message_created_at: 1_000,
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

    #[test]
    fn verdict_parsing_wie_python() {
        let v = parse_ai_verdict(Some(
            "<think>x</think> {\"verdict\":\"delete\",\"category\":\"scam\",\"confidence\":0.95,\"reason\":\"Crypto-Scam\"}",
        ));
        assert_eq!(v.verdict, "delete");
        assert_eq!(v.category, "scam");
        assert_eq!(v.confidence, 0.95);
        assert_eq!(v.reason, "Crypto-Scam");

        // ungültige Werte → needs_context/other, confidence geklemmt
        let v = parse_ai_verdict(Some(
            "{\"verdict\":\"BAN\",\"category\":\"weird\",\"confidence\":7,\"reason\":\"\"}",
        ));
        assert_eq!(v.verdict, "needs_context");
        assert_eq!(v.category, "other");
        assert_eq!(v.confidence, 1.0);
        assert_eq!(v.reason, "Keine Begruendung geliefert.");

        assert_eq!(parse_ai_verdict(None).reason, "parse_error");
        assert_eq!(parse_ai_verdict(Some("kein json")).reason, "parse_error");
    }

    #[test]
    fn schwellen_routing() {
        let verdict = |v: &str, c: &str, conf: f64| AiVerdict {
            verdict: v.into(),
            category: c.into(),
            confidence: conf,
            reason: String::new(),
            raw_json: "{}".into(),
        };
        // Auto-Delete nur bei Auto-Kategorie + ≥0.90
        assert_eq!(
            decide_action(&verdict("delete", "scam", 0.95), PROPOSE_CONFIDENCE),
            ModAction::AutoDelete
        );
        // delete unter 0.90 aber ≥0.78 → Vorschlag
        assert_eq!(
            decide_action(&verdict("delete", "scam", 0.85), PROPOSE_CONFIDENCE),
            ModAction::Propose
        );
        // deaktivierte Kategorien werden wie im Python-Pfad verworfen
        assert_eq!(
            decide_action(&verdict("delete", "racism", 0.95), PROPOSE_CONFIDENCE),
            ModAction::Ignore
        );
        // propose unter Schwelle → nichts
        assert_eq!(
            decide_action(&verdict("propose", "harassment", 0.70), PROPOSE_CONFIDENCE),
            ModAction::Ignore
        );
        assert_eq!(
            decide_action(&verdict("ok", "ragebait_ok", 0.6), PROPOSE_CONFIDENCE),
            ModAction::CountRagebait
        );
        assert_eq!(
            decide_action(&verdict("ok", "game_related_ok", 0.6), PROPOSE_CONFIDENCE),
            ModAction::Ignore
        );
        assert_eq!(
            decide_action(&verdict("needs_context", "other", 0.99), PROPOSE_CONFIDENCE),
            ModAction::Ignore
        );
    }

    #[test]
    fn context_escalation_thresholds_match_python() {
        let verdict = |v: &str, conf: f64| AiVerdict {
            verdict: v.into(),
            category: "other".into(),
            confidence: conf,
            reason: String::new(),
            raw_json: "{}".into(),
        };
        assert!(needs_context_escalation(&verdict("needs_context", 0.1)));
        assert!(!needs_context_escalation(&verdict("ok", 0.54)));
        assert!(needs_context_escalation(&verdict("ok", 0.55)));
        assert!(needs_context_escalation(&verdict("propose", 0.779)));
        assert!(!needs_context_escalation(&verdict("propose", 0.78)));
    }

    #[test]
    fn prompt_payload_contains_context_and_reply() {
        let event = dl_discord::MessageEvent {
            guild_id: Some(1),
            channel_id: 2,
            message_id: 3,
            author_id: 4,
            author_display_name: "Anna".into(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: "hallo <@123> test".into(),
            message_created_at: 1_000,
            is_reply: true,
            reply_message_id: Some(9),
            reply_channel_id: Some(2),
            attachment_count: 1,
            image_attachment_count: 1,
            image_attachment_urls: vec!["https://img".into()],
            attachments: vec![dl_discord::MessageAttachment {
                url: "https://img".into(),
                content_type: "image/png".into(),
                filename: "img.png".into(),
            }],
            author_created_at: 0,
            author_joined_at: None,
        };
        let prompt = build_prompt_payload(
            &event,
            &[
                ">>> [2min ago] Anna: vorher".to_string(),
                "    [1min ago] Ben: antwort".to_string(),
            ],
            1,
            true,
            Some(&ReplyContext {
                author: "Ben".into(),
                content: "provokation".into(),
            }),
        );
        let payload: Value = serde_json::from_str(&prompt).expect("payload");
        assert_eq!(payload["user_message"], "hallo test");
        assert_eq!(payload["attachment_count"], 1);
        assert_eq!(payload["recent_context"][0], ">>> [2min ago] Anna: vorher");
        assert_eq!(payload["is_reply_to"]["author"], "Ben");
        assert_eq!(payload["analysis_stage"], "context_escalation");
        assert_eq!(payload["focus_message_id"], "3");
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn persistent_ragebait_setzt_mod_tag_und_loggt_case(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let port = Arc::new(TestPort::default());
        let moderator = AiModerator::new(
            db.pool().clone(),
            Arc::new(StaticGenerator {
                response:
                    r#"{"verdict":"ok","category":"ragebait_ok","confidence":0.80,"reason":"Bait"}"#
                        .into(),
            }),
            None,
            port.clone(),
        );
        moderator.store.ensure_schema().await.expect("schema");

        for idx in 0..RAGEBAIT_ESCALATE_THRESHOLD {
            moderator.cooldown.lock().await.clear();
            moderator
                .handle_message(&event(5_000 + idx as u64, &format!("bait {idx}")))
                .await;
        }

        assert_eq!(
            port.tags.lock().await.as_slice(),
            &[(
                100,
                "ragebaiter".to_string(),
                "auto: persistent_ragebait".to_string()
            )]
        );
        assert_eq!(
            port.reviews.lock().await.as_slice(),
            &["ragebait_escalated".to_string()]
        );
        assert_eq!(
            port.logs.lock().await.as_slice(),
            &["ragebait_escalated".to_string()]
        );
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn ragebaiter_free_lane_tag_filter_reads_voice_schema(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let channel_id = i64::try_from(SCAN_CHANNEL_IDS[0])?;
        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_lane_tag_filter
                (channel_id, required_tone_tag, updated_at)
            VALUES ($1, $2, now())
            "#,
            channel_id,
            "ragebaiter_free",
        )
        .execute(db.pool())
        .await?;

        let port = Arc::new(TestPort::default());
        let moderator = AiModerator::new(
            db.pool().clone(),
            Arc::new(StaticGenerator {
                response:
                    r#"{"verdict":"ok","category":"ragebait_ok","confidence":0.50,"reason":"Bait"}"#
                        .into(),
            }),
            None,
            port.clone(),
        );
        moderator.store.ensure_schema().await?;

        moderator.handle_message(&event(6_000, "milder bait")).await;

        assert_eq!(port.dms.lock().await.as_slice(), &[100]);
        assert!(port.reviews.lock().await.is_empty());
        assert!(port.logs.lock().await.is_empty());
        Ok(())
    }
}
