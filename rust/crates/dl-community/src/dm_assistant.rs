//! KI-DM-Assistent — Port von `cogs/welcome_dm/dm_assistant.py`.
//!
//! Reagiert auf Freitext-DMs an den Bot mit einer KI-Antwort. Die KI liefert
//! ein JSON-Objekt `{intent, message, action}`; je nach Intent wird die Antwort
//! um ein passendes Embed (streamer/beta/steam) ergänzt. Schlägt die KI fehl
//! oder ist sie nicht konfiguriert, kommt ein Fallback-Menü mit den
//! `dma:fallback:*`-Buttons (deren Handler liegen im Onboarding-Glue).
//!
//! Provider-Hinweis: das Original nutzt Gemini→OpenAI, der Rewrite nutzt wie der
//! Rest (FAQ, Moderation) den konfigurierten [`TextGenerator`] (MiniMax) — das
//! sichtbare Verhalten (KI beantwortet DMs) bleibt gleich.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use dl_ai::{GenerateRequest, TextGenerator};
use dl_discord::Dispatcher;
use serde_json::{json, Value};

/// Rate-Limit: max. Aufrufe pro Fenster, Fensterlänge, Mindestabstand (Sek.).
const MAX_CALLS_PER_WINDOW: usize = 3;
const WINDOW_SECONDS: f64 = 60.0;
const MIN_INTERVAL_SECONDS: f64 = 10.0;
const MAX_OUTPUT_TOKENS: u32 = 400;

pub const SYSTEM_PROMPT: &str = "Du bist der freundliche Bot der Deutschen Deadlock Community (Discord-Server ID 1289721245281292288).\n\
Deine Aufgabe: Beantworte DMs von Nutzern auf Deutsch, kurz und freundlich.\n\n\
=== Server-Features ===\n\
- Streamer-Partnerschaft: Auto-Raid, Chat Guard, Analytics Dashboard (/twitch/), Discord Auto-Post (#🎥twitch), Chat-Promos alle ~30 Min. → Start mit /streamer oder DM-Flow.\n\
- Beta-Invite: Deadlock Beta-Zugang via Ko-fi oder Invite → /betainvite im Server.\n\
- Steam-Verifizierung: Steam-Account verknüpfen → Rolle \"Steam Verifiziert\" → /steamlink.\n\
- Twitch Analytics: /twitch/ – Retention, Unique Chatters, Leaderboard.\n\
- LFG: Mitspieler finden → #spieler-suche Channel.\n\
- FAQ: /faq oder /serverfaq <frage>.\n\
- Voice: Temp-Voice-Channels, eigene Lanes.\n\
- Ranking: Deadlock-Rang eintragen, Rollen nach Rank.\n\n\
=== Antwort-Regeln ===\n\
- Immer auf Deutsch, freundlich, max. 3–4 Sätze.\n\
- Antworte NUR mit einem JSON-Objekt (kein Markdown, kein Codeblock darum herum).\n\
- JSON-Format: {\"intent\": \"...\", \"message\": \"...\", \"action\": true/false}\n\
- intent-Werte:\n\
    \"streamer\"  → User fragt nach Streamer-Partnerschaft, Auto-Raid, Chat Guard, Analytics\n\
    \"beta\"      → User braucht Deadlock Beta-Zugang\n\
    \"steam\"     → User will Steam-Account verknüpfen oder Steam-Rolle erhalten\n\
    \"faq\"       → FAQ-Frage oder allgemeine Server-Info\n\
    \"general\"   → Alles andere (Begrüßung, Smalltalk, unklare Anfragen)\n\
- action=true wenn ein spezieller Discord-View/Embed sinnvoll ist (bei streamer, beta, steam).\n\
- action=false bei general/faq oder wenn kein View nötig ist.";

/// Discord-Anbindung des DM-Assistenten.
#[async_trait::async_trait]
pub trait DmPort: Send + Sync {
    /// Sendet eine Nachricht (Text/Embeds/Components) in den DM-Kanal.
    async fn send_dm(&self, channel_id: u64, body: serde_json::Map<String, Value>);
}

pub struct DmAssistant {
    ai: Option<Arc<dyn TextGenerator>>,
    port: Arc<dyn DmPort>,
    /// Pro User die monotonen Zeitstempel der letzten Aufrufe (Cooldown).
    cooldowns: Mutex<HashMap<u64, Vec<f64>>>,
    /// Monotone Referenz für die Cooldown-Sekunden.
    start: Instant,
}

impl DmAssistant {
    pub fn new(ai: Option<Arc<dyn TextGenerator>>, port: Arc<dyn DmPort>) -> Arc<Self> {
        Arc::new(Self {
            ai,
            port,
            cooldowns: Mutex::new(HashMap::new()),
            start: Instant::now(),
        })
    }

    /// Verarbeitet eine DM. `now` = monotone Sekunden seit [`Self::start`].
    async fn handle_dm(&self, channel_id: u64, user_id: u64, content: &str, now: f64) {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return;
        }

        // Rate-Limit prüfen.
        let cooldown_msg = {
            let mut map = self.cooldowns.lock().expect("cooldowns");
            check_cooldown(map.entry(user_id).or_default(), now)
        };
        if let Some(msg) = cooldown_msg {
            self.port.send_dm(channel_id, text_body(&msg)).await;
            return;
        }

        // Keine KI konfiguriert → Fallback-Menü.
        let Some(ai) = &self.ai else {
            self.port.send_dm(channel_id, fallback_body()).await;
            return;
        };

        let answer = ai
            .generate_text(GenerateRequest {
                prompt: trimmed.to_string(),
                system_prompt: Some(SYSTEM_PROMPT.to_string()),
                model: None,
                max_output_tokens: Some(MAX_OUTPUT_TOKENS),
                temperature: 0.7,
            })
            .await
            .filter(|t| !t.trim().is_empty());

        let Some(answer) = answer else {
            self.port.send_dm(channel_id, fallback_body()).await;
            return;
        };

        let parsed = parse_ai_response(&answer);
        if parsed.message.is_empty() {
            self.port.send_dm(channel_id, fallback_body()).await;
            return;
        }
        self.port.send_dm(channel_id, intent_body(&parsed)).await;
    }
}

/// Geparste KI-Antwort (`{intent, message, action}`).
struct ParsedAi {
    intent: String,
    message: String,
    action: bool,
}

/// Prüft das Rate-Limit (Port von `_check_cooldown`): gibt eine Hinweis-Nachricht
/// zurück, wenn der User warten muss; sonst `None` und der Zeitstempel wird
/// vermerkt.
fn check_cooldown(stamps: &mut Vec<f64>, now: f64) -> Option<String> {
    stamps.retain(|&t| now - t < WINDOW_SECONDS);
    if let Some(&last) = stamps.last() {
        if now - last < MIN_INTERVAL_SECONDS {
            let wait = (MIN_INTERVAL_SECONDS - (now - last)) as i64 + 1;
            return Some(format!(
                "Bitte warte noch **{wait} Sekunden**, bevor du mir erneut schreibst. 😊"
            ));
        }
    }
    if stamps.len() >= MAX_CALLS_PER_WINDOW {
        return Some(
            "Du hast mich gerade zu oft angeschrieben. Bitte warte kurz (ca. 1 Minute) und versuche es dann erneut. 😊"
                .to_string(),
        );
    }
    stamps.push(now);
    None
}

/// Extrahiert `{intent, message, action}` aus der KI-Antwort (Port von
/// `_parse_ai_response`): erstes `{` bis letztes `}`; schlägt das Parsen fehl,
/// gilt der ganze Text als `general`-Message.
fn parse_ai_response(text: &str) -> ParsedAi {
    let t = text.trim();
    if let (Some(start), Some(end)) = (t.find('{'), t.rfind('}')) {
        if end >= start {
            if let Ok(v) = serde_json::from_str::<Value>(&t[start..=end]) {
                return ParsedAi {
                    intent: v
                        .get("intent")
                        .and_then(Value::as_str)
                        .unwrap_or("general")
                        .to_lowercase()
                        .trim()
                        .to_string(),
                    message: v
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                    action: v.get("action").and_then(Value::as_bool).unwrap_or(false),
                };
            }
        }
    }
    ParsedAi {
        intent: "general".to_string(),
        message: t.to_string(),
        action: false,
    }
}

fn text_body(content: &str) -> serde_json::Map<String, Value> {
    let mut body = serde_json::Map::new();
    body.insert("content".into(), json!(content));
    body
}

/// Baut die Antwort: KI-Text + bei `action` ein Intent-Embed (Port von
/// `_handle_intent`).
fn intent_body(parsed: &ParsedAi) -> serde_json::Map<String, Value> {
    let mut body = serde_json::Map::new();
    body.insert("content".into(), json!(parsed.message));
    if parsed.action {
        let embed = match parsed.intent.as_str() {
            "streamer" => Some(streamer_embed()),
            "beta" => Some(beta_embed()),
            "steam" => Some(steam_embed()),
            _ => None,
        };
        if let Some(embed) = embed {
            body.insert("embeds".into(), json!([embed]));
        }
    }
    body
}

fn streamer_embed() -> Value {
    json!({
        "title": "🎮 Streamer-Partnerschaft",
        "description": "Nutze **/streamer** im Server, um den Streamer-Partner-Prozess zu starten!\n\n\
                        Als Partner bekommst du: Auto-Raid, Chat Guard, Analytics-Dashboard und mehr.",
        "color": 0x5865F2
    })
}

fn beta_embed() -> Value {
    json!({
        "title": "🎟️ Deadlock Beta-Invite",
        "description": "So bekommst du einen Beta-Invite:\n\n\
                        **1.** Betritt unseren Discord-Server\n\
                        **2.** Nutze `/betainvite` im richtigen Channel\n\n\
                        Du kannst auch direkt im <#1428745737323155679> Channel nachschauen.",
        "color": 0x3498DB
    })
}

fn steam_embed() -> Value {
    json!({
        "title": "🔗 Steam-Account verknüpfen",
        "description": "Das bringt dir die Verknüpfung:\n\
                        • korrekter Rang auf dem Server\n\
                        • zuverlässiger Live-Status in den Voice Lanes\n\
                        • bessere Einstufung in der Spielersuche\n\n\
                        So verknüpfst du deinen Steam-Account:\n\n\
                        **1.** Betritt unseren Discord-Server\n\
                        **2.** Nutze den Befehl `/steamlink`\n\
                        **3.** Folge den Anweisungen\n\
                        **4.** Sende dem Steam-Bot eine Freundschaftsanfrage (Freundescode **820142646**) und nimm sie an\n\n\
                        Nach der Verknüpfung erhältst du die Rolle **\"Steam Verifiziert\"**.",
        "color": 0x2ECC71
    })
}

/// Fallback-Menü (Port von `_send_fallback` + `FallbackView`).
fn fallback_body() -> serde_json::Map<String, Value> {
    let embed = json!({
        "title": "Hallo! Ich bin der Deadlock Community Bot 👋",
        "description": "Womit kann ich dir helfen? Wähle eine Option:",
        "color": 0x5865F2
    });
    let components = json!([{ "type": 1, "components": [
        { "type": 2, "style": 1, "label": "🎮 Streamer werden", "custom_id": "dma:fallback:streamer" },
        { "type": 2, "style": 2, "label": "🎟️ Beta-Invite", "custom_id": "dma:fallback:beta" },
        { "type": 2, "style": 2, "label": "🔗 Steam verknüpfen", "custom_id": "dma:fallback:steam" },
        { "type": 2, "style": 2, "label": "❓ FAQ", "custom_id": "dma:fallback:faq" }
    ]}]);
    let mut body = serde_json::Map::new();
    body.insert("embeds".into(), json!([embed]));
    body.insert("components".into(), components);
    body
}

/// Message-Subscriber: beantwortet Freitext-DMs an den Bot.
pub fn spawn_dm_assistant(
    assistant: Arc<DmAssistant>,
    dispatcher: &Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    // Nur DMs (keine Gilde); Bot-Nachrichten filtert das Gateway.
                    if event.guild_id.is_some() || event.content.trim().is_empty() {
                        continue;
                    }
                    let now = assistant.start.elapsed().as_secs_f64();
                    assistant
                        .handle_dm(event.channel_id, event.author_id, &event.content, now)
                        .await;
                }
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
    fn cooldown_min_intervall_und_max_calls() {
        let mut stamps = Vec::new();
        // Erster Aufruf ok.
        assert!(check_cooldown(&mut stamps, 0.0).is_none());
        // Sofort nochmal (< 10 s Abstand) → Hinweis.
        assert!(check_cooldown(&mut stamps, 2.0).unwrap().contains("Sekunden"));
        // Nach 11 s wieder ok (2. Aufruf gezählt).
        assert!(check_cooldown(&mut stamps, 11.0).is_none());
        // Nach weiteren 11 s der 3. Aufruf ok.
        assert!(check_cooldown(&mut stamps, 22.0).is_none());
        // 4. Aufruf im 60-s-Fenster → zu oft.
        assert!(check_cooldown(&mut stamps, 33.0).unwrap().contains("zu oft"));
    }

    #[test]
    fn cooldown_fenster_laeuft_ab() {
        let mut stamps = vec![0.0, 10.0, 20.0];
        // 85 s später sind alle alten Stempel (Abstand ≥ 60 s) aus dem Fenster.
        assert!(check_cooldown(&mut stamps, 85.0).is_none());
        assert_eq!(stamps, vec![85.0]);
    }

    #[test]
    fn parse_json_und_fallback() {
        let p = parse_ai_response("Vorspann {\"intent\": \"Steam\", \"message\": \" Hi \", \"action\": true} Nachspann");
        assert_eq!(p.intent, "steam");
        assert_eq!(p.message, "Hi");
        assert!(p.action);
        // Kein JSON → ganzer Text als general-Message.
        let p = parse_ai_response("nur text");
        assert_eq!(p.intent, "general");
        assert_eq!(p.message, "nur text");
        assert!(!p.action);
    }

    #[test]
    fn intent_embed_nur_bei_action() {
        let with = intent_body(&ParsedAi {
            intent: "steam".into(),
            message: "m".into(),
            action: true,
        });
        assert!(with.contains_key("embeds"));
        // action=false → kein Embed, nur Text.
        let without = intent_body(&ParsedAi {
            intent: "steam".into(),
            message: "m".into(),
            action: false,
        });
        assert!(!without.contains_key("embeds"));
        // unbekannter Intent mit action → kein Embed.
        let general = intent_body(&ParsedAi {
            intent: "general".into(),
            message: "m".into(),
            action: true,
        });
        assert!(!general.contains_key("embeds"));
    }

    // ── Handler-Integration mit Mocks ──────────────────────────────────────

    struct MockAi {
        reply: Option<String>,
    }
    #[async_trait::async_trait]
    impl TextGenerator for MockAi {
        async fn generate_text(&self, _req: GenerateRequest) -> Option<String> {
            self.reply.clone()
        }
    }

    struct MockPort {
        sent: Mutex<Vec<serde_json::Map<String, Value>>>,
    }
    #[async_trait::async_trait]
    impl DmPort for MockPort {
        async fn send_dm(&self, _c: u64, body: serde_json::Map<String, Value>) {
            self.sent.lock().unwrap().push(body);
        }
    }

    fn port() -> Arc<MockPort> {
        Arc::new(MockPort { sent: Mutex::new(Vec::new()) })
    }

    #[tokio::test]
    async fn antwortet_mit_intent_embed() {
        let ai: Arc<dyn TextGenerator> = Arc::new(MockAi {
            reply: Some("{\"intent\":\"beta\",\"message\":\"Klar!\",\"action\":true}".into()),
        });
        let p = port();
        let dm = DmAssistant::new(Some(ai), p.clone());
        dm.handle_dm(10, 1, "Wie komme ich in die Beta?", 0.0).await;
        let sent = p.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["content"], json!("Klar!"));
        assert!(sent[0].contains_key("embeds"));
    }

    #[tokio::test]
    async fn ohne_ki_kommt_fallback() {
        let p = port();
        let dm = DmAssistant::new(None, p.clone());
        dm.handle_dm(10, 1, "Hallo", 0.0).await;
        let sent = p.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        // Fallback-Embed + Buttons.
        assert!(sent[0].contains_key("components"));
        assert!(sent[0]["embeds"][0]["title"].as_str().unwrap().contains("Community Bot"));
    }

    #[tokio::test]
    async fn cooldown_blockt_zweite_dm() {
        let ai: Arc<dyn TextGenerator> = Arc::new(MockAi {
            reply: Some("{\"intent\":\"general\",\"message\":\"Hi\",\"action\":false}".into()),
        });
        let p = port();
        let dm = DmAssistant::new(Some(ai), p.clone());
        dm.handle_dm(10, 1, "erste", 0.0).await;
        // sofort danach (gleiche Sekunde) → Cooldown-Hinweis statt KI-Antwort.
        dm.handle_dm(10, 1, "zweite", 1.0).await;
        let sent = p.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[1]["content"].as_str().unwrap().contains("Sekunden"));
    }
}
