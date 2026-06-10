//! dl-moderation — Port des Kerns von `cogs/ai_moderator.py`.
//!
//! Pipeline: Nachricht im Scan-Kanal → MiniMax-Klassifikation
//! (System-Prompt wortgleich) → Schwellen-Routing:
//! - `delete` + Auto-Kategorie + confidence ≥ 0.90 → Auto-Delete + Case
//! - `delete`/`propose` + confidence ≥ 0.78 → Mod-Review-Case (Buttons
//!   `aimod:accept|ban|deny:{case_id}` — custom_ids unverändert)
//! - `ragebait_ok` → Fenster-Zählung (4 Treffer / 120 min → Eskalation
//!   als `persistent_ragebait`-Vorschlag)
//! - `needs_context`/`ok` → nichts
//!
//! Bewusste Lücken (dokumentiert): Kontext-Backfill-Eskalation (12
//! Nachrichten bei 0.55–0.78), Bild-Anhänge (multimodal), Tone-Tag-
//! Schwellen und der Ragebaiter-Free-Warnhinweis — folgen mit dem
//! Tag-System; der Admin-Skip nähert manage_messages über das
//! Administrator-Flag des Dispatchers an.

use std::collections::HashMap;
use std::sync::Arc;

use dl_db::Db;
use serde_json::Value;

pub mod guard;
pub mod store;

pub const SCAN_CHANNEL_IDS: [u64; 1] = [1289721245281292291];
pub const MOD_REVIEW_CHANNEL_ID: u64 = 1315684135175716978;
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const TIMEOUT_MINUTES: i64 = 1440;
pub const RAGEBAIT_WINDOW_MINUTES: i64 = 120;
pub const RAGEBAIT_ESCALATE_THRESHOLD: i64 = 4;
pub const AUTO_DELETE_CONFIDENCE: f64 = 0.90;
pub const PROPOSE_CONFIDENCE: f64 = 0.78;
pub const PER_USER_COOLDOWN_SECONDS: f64 = 2.0;
pub const MAX_PROMPT_CHARS: usize = 4000;

pub const AUTO_DELETE_CATEGORIES: [&str; 5] =
    ["nsfw_explicit", "csam", "raping", "epstein_child", "scam"];

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
}

fn parse_error_verdict() -> AiVerdict {
    AiVerdict {
        verdict: "needs_context".to_string(),
        category: "other".to_string(),
        confidence: 0.0,
        reason: "parse_error".to_string(),
    }
}

/// JSON aus der AI-Antwort ziehen — greedy `\{.*\}` über Zeilen hinweg
/// (erstes `{` bis letztes `}`), Felder validieren und klemmen.
pub fn parse_ai_verdict(raw_text: Option<&str>) -> AiVerdict {
    let Some(raw) = raw_text else {
        return parse_error_verdict();
    };
    let (Some(open), Some(close)) = (raw.find('{'), raw.rfind('}')) else {
        return parse_error_verdict();
    };
    if close <= open {
        return parse_error_verdict();
    }
    let Ok(payload) = serde_json::from_str::<Value>(&raw[open..=close]) else {
        return parse_error_verdict();
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

// ── Engine ─────────────────────────────────────────────────────────────────

/// Discord-Aktionen des Moderators (Tests mocken sie).
#[async_trait::async_trait]
pub trait ModPort: Send + Sync {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool;
    async fn timeout_member(&self, guild_id: u64, user_id: u64, minutes: i64) -> bool;
    /// Review-Embed mit aimod:*-Buttons posten → message_id.
    async fn post_review(&self, case: &store::CaseDraft, buttons_case_id: &str) -> Option<u64>;
    async fn post_log(&self, text: String);
}

pub struct AiModerator {
    pub store: store::ModerationStore,
    pub generator: Arc<dyn dl_ai::TextGenerator>,
    pub port: Arc<dyn ModPort>,
    cooldown: tokio::sync::Mutex<HashMap<u64, std::time::Instant>>,
}

impl AiModerator {
    pub fn new(
        db: Db,
        generator: Arc<dyn dl_ai::TextGenerator>,
        port: Arc<dyn ModPort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: store::ModerationStore { db },
            generator,
            port,
            cooldown: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub async fn handle_message(self: &Arc<Self>, event: &dl_discord::MessageEvent) {
        let Some(guild_id) = event.guild_id else {
            return;
        };
        if !SCAN_CHANNEL_IDS.contains(&event.channel_id) {
            return;
        }
        if event.author_is_admin {
            return; // Original skippt manage_messages — Annäherung
        }
        if event.content.trim().is_empty() {
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

        let prompt: String = event.content.chars().take(MAX_PROMPT_CHARS).collect();
        let raw = self
            .generator
            .generate_text(dl_ai::GenerateRequest {
                prompt,
                system_prompt: Some(MODERATION_SYSTEM_PROMPT.to_string()),
                model: None,
                max_output_tokens: Some(300),
                temperature: 0.0,
            })
            .await;
        let verdict = parse_ai_verdict(raw.as_deref());

        match decide_action(&verdict, PROPOSE_CONFIDENCE) {
            ModAction::Ignore => {}
            ModAction::AutoDelete => {
                let deleted = self
                    .port
                    .delete_message(
                        event.channel_id,
                        event.message_id,
                        "AI-Moderation: Auto-Delete",
                    )
                    .await;
                let case_id = self
                    .store
                    .insert_case(self.draft(guild_id, event, &verdict, "auto_deleted"))
                    .await;
                self.port
                    .post_log(format!(
                        "🤖 Auto-Delete ({}, {:.0}%): <@{}> in <#{}> — {} {}",
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
                    ))
                    .await;
                tracing::info!(case_id, "AI-Moderation: Auto-Delete-Case angelegt");
            }
            ModAction::Propose => {
                self.create_proposal(guild_id, event, &verdict).await;
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
                    };
                    self.create_proposal(guild_id, event, &escalated).await;
                }
            }
        }
    }

    fn draft(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: &AiVerdict,
        action: &str,
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
        }
    }

    async fn create_proposal(
        &self,
        guild_id: u64,
        event: &dl_discord::MessageEvent,
        verdict: &AiVerdict,
    ) {
        let draft = self.draft(guild_id, event, verdict, "proposed");
        let case_id = self.store.insert_case(draft.clone()).await;
        if let Some(message_id) = self.port.post_review(&draft, &case_id).await {
            self.store.set_review_message(&case_id, message_id).await;
        }
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
    use super::*;

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
        // delete in Nicht-Auto-Kategorie trotz 0.95 → Vorschlag
        assert_eq!(
            decide_action(&verdict("delete", "racism", 0.95), PROPOSE_CONFIDENCE),
            ModAction::Propose
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
}
