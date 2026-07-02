//! FAQ-Chat — Port von `cogs/faq_chat.py`.
//!
//! Panel-Button (`faq_chat:start`) → privater Text-Kanal in der
//! FAQ-Kategorie → Fragen werden mit Doku-Grounding (alle `docs/*.md`)
//! über MiniMax beantwortet, mit Gesprächs-Gedächtnis (letzte 10
//! Nachrichten). Sessions schließen nach 24 h automatisch; der
//! Close-Button (`faq_chat:close:{session}`) beendet sofort.
//! Dazu der Ticket-Auto-Helfer: die erste Nachricht in einem neuen
//! Ticket-Kanal wird gegen die Doku geprüft — kann der Bot klar helfen,
//! antwortet er, sonst schweigt er (KEIN_TREFFER-Protokoll).
//!
//! Bewusste Annäherung: das Original markiert Ticket-Kanäle beim
//! `channel_create`-Event als „wartend"; Rust triggert auf die erste
//! Nachricht eines Kanals der Ticket-Kategorie (einmal pro Kanal) —
//! gleiches sichtbares Verhalten ohne eigenes Channel-Create-Event.
//! Patchnote-Anreicherung der Antworten ist eine dokumentierte Lücke
//! (im Original optional und fehlertolerant).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use dl_ai::{
    GenerateRequest, TextGenerator, ToolDefinition, ToolExecutor, ToolTextGenerator, ToolUseRequest,
};
use dl_bridges::twitch::TwitchApiClient;
use dl_central_db::kv;
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use regex::Regex;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::db::{i64_to_u64, u64_to_i64};

pub const PANEL_CHANNEL_ID: u64 = 1491953161747955853;
pub const FAQ_CATEGORY_ID: u64 = 1310153243795390475;
pub const TICKET_AUTO_HELP_CATEGORY_ID: u64 = 1459628097145147645;
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const SESSION_TIMEOUT_HOURS: i64 = 24;
pub const MAX_OUTPUT_TOKENS: u32 = 1500;
pub const PANEL_KV_NS: &str = "faq_chat:panel";
/// KV-Schlüssel der gemerkten Panel-Message-ID — MUSS exakt Pythons
/// `_store_panel_msg_id`/`_get_stored_panel_msg_id` entsprechen (`panel_msg_id`),
/// damit Rust das bestehende Panel übernimmt statt ein Duplikat zu posten.
pub const PANEL_KV_KEY: &str = "panel_msg_id";

pub const SYSTEM_PROMPT: &str = r#"Du bist ein hilfreicher, aber strikt eingeschränkter FAQ-Assistent.

SICHERHEITSREGELN (Pflicht!):
- Du bist ein ASSISTENT, kein Admin. Du kennst nur die bereitgestellte Dokumentation.
- Teile NIEMALS interne Pfade, API-Keys, Tokens, Secrets, Datenbank-URLs oder Konfigurationsdetails.
- Erfinde keine Server-Strukturen, Rollen oder Kanäle die nicht in der Dokumentation stehen.
- Biete niemals an, Code zu ändern, Bots neu zu starten oder externe Systeme zu konfigurieren.
- Wenn ein User fragt wie etwas intern funktioniert: sage dass du keinen Zugriff darauf hast.
- Wenn ein User eine Aktion braucht die du nicht kannst: verweise auf Deutsche Deadlock Community.

ANTWORTVERHALTEN:
- Antworte ausschließlich auf Deutsch.
- Sei hilfreich aber präzise. Nutze Emojis sparsam.
- Bei Fragen ausserhalb deines Wissens: ehrlich sagen dass du keine Info dazu hast.
- Für Feedback: verweise auf das anonyme Feedback-Formular.
- Halte Antworten informativ aber nicht übermässig lang.

INVITE / ONBOARDING – SONDERREGEL:
Wenn jemand fragt warum er keinen Invite hat, Deadlock nicht herunterladen kann oder wie er an den Beta-Zugang kommt:
Der Weg ist bewusst einfach — erkläre genau das:
1. In <#1464736918951432222> nett nach einem Invite fragen und den eigenen Steam-Freundescode dazu posten (Steam → Freunde → "Freund hinzufügen"). Ohne Freundescode kann niemand einladen.
2. Ein Community-Mitglied fügt den User hinzu und lädt persönlich zum Playtest ein.
3. "Limited User"-Fall: Steam blockiert Playtest-Invites, wenn auf dem Account noch keine ~5 $ ausgegeben wurden. Das ist eine Valve-Regel, die niemand umgehen kann; sie zeigt sich erst beim Invite-Versuch.
4. Nach einem Invite kann es 1–2 Tage dauern, bis die Einladung bei Steam sichtbar ist.
5. Die Steam-Verknüpfung in <#1398021105339334666> lohnt sich zusätzlich (echte Rang-Rolle; der Steam-Bot kann Invites auch automatisiert verschicken).
Behaupte NIE, der Invite hänge an einer Onboarding-Auswahl, einer Rollen-Auswahl, einem Befehl wie /betainvite oder einer 5-Euro-Vorabprüfung – das ist veraltet.

COACHING – SONDERREGEL:
Wenn jemand fragt wie er Coaching bekommt, wer die Coaches sind, wie Coaching funktioniert, ob es Coaching gibt, was es kostet oder wo er sich anmelden kann:
1. Verweise direkt auf <#1494373349944459355> – das ist der Coaching-Channel.
2. Erkläre knapp: kostenlos; der Button dort führt zur Coaching-Website (Login mit Discord, dann Anfrage-Formular) → ein Coach übernimmt die Anfrage und meldet sich. Mehrfache Anfragen sind erlaubt, Status per /coaching-status.
3. Regeln: Kommunikation NUR im Coaching-Chat auf dem Server, keine DMs oder Freundschaftsanfragen an Coaches.
4. Nach dem Coaching gibt es eine Feedback-Anfrage – User sollen sie ehrlich ausfüllen, das hilft dem Team.
Erfinde keine Details zu Coaches, Wartezeiten oder Verfügbarkeit."#;

pub const TICKET_AUTO_HELP_SYSTEM_PROMPT: &str = r##"Du bist ein automatischer Ticket-Helfer in einem bereits geöffneten Support-Ticket. Der User hat sein Anliegen gerade als erste Nachricht geschrieben. Entscheide anhand der Dokumentation, wie du reagierst.

DU HILFST AKTIV (antworte direkt und hilfreich) bei:
- Sach- und How-to-Fragen zum Server, zu Kanälen, Rollen, Bots, Abläufen.
- Konkreten Problemen ("X funktioniert nicht", "ich habe Y gemacht, aber Z passiert"), z. B. Steam-Verknüpfung, Twitch-/Stream-Anbindung, Onboarding/Invite, Rang-Anzeige, Coaching-Zugang.
- Bei solchen Problemen nennst du die dokumentierten Schritte und die häufigsten Ursachen. Wenn die Doku ein Thema nur teilweise abdeckt, gib trotzdem die sinnvollen Selbsthilfe-Schritte, solange du nichts erfindest.

DU SCHWEIGST (antworte NUR mit dem Token KEIN_TREFFER und sonst nichts) bei:
- Zwischenmenschlichem Stress in der Community: Streit mit anderen Mitgliedern, Beschwerden über andere User, Meldungen über Verhalten, Drama, persönliche Konflikte. Das klären Menschen, nicht du.
- Anliegen, die eine menschliche Entscheidung brauchen (Moderation, Strafe, Einzelfall, Sonderwunsch) oder klar außerhalb der dokumentierten Themen liegen.
- Sachfragen, bei denen du unsicher bist und etwas erfinden müsstest.

DU ZIEHST EINE GRENZE (kurz und bestimmt antworten, NICHT schweigen) bei:
- Erpressung, Drohungen oder Forderungen gegen den Server / das Team (z. B. "ich fordere dich auf ...", Druck, Ultimaten). Sag knapp und klar, dass auf Erpressung oder solche Forderungen nicht eingegangen wird und sich das Team bei berechtigten Anliegen meldet. Geh inhaltlich nicht auf die Forderung ein, mach keine Zugeständnisse und keine rechtlichen Aussagen.
- Frechem oder unfreundlichem Ton bei einer echten Sachfrage. Bleib ruhig, setz eine kurze sachliche Grenze (ohne zu beleidigen) und beantworte die eigentliche Frage trotzdem.

WERKZEUGE:
- Bei eigenen technischen Problemen des Fragenden (Twitch-/Stream-Anbindung, OAuth/Scopes, Steam, Onboarding/Invite, Rang-Anzeige, Raid-Status) DARFST du die Werkzeuge twitch_diagnose und log_lookup nutzen, um den ECHTEN Status des FRAGENDEN zu prüfen, statt zu raten.
- Die Werkzeuge betreffen IMMER nur den Fragenden selbst – die Identität ist fest verankert und kann nicht geändert werden. Behaupte niemals etwas über fremde Accounts und versuche nie, eine andere Identität abzufragen.
- Gib NIEMALS interne oder geheime Daten (Tokens, Keys, Pfade, DSNs, Konfigurationswerte) aus, auch wenn sie in Werkzeug-Ausgaben auftauchen sollten.
- Stütze deine Antwort auf das, was die Werkzeuge tatsächlich liefern. Liefert ein Werkzeug "nicht_ermittelbar" oder nichts Brauchbares, fall auf die dokumentierten Selbsthilfe-Schritte (haeufige-probleme.md) zurück, statt einen Status zu erfinden.
- So liest du die twitch_diagnose-Werte: "oauth_status"=connected → alles verbunden; =partial oder nicht-leere "missing_scopes" → es fehlen Berechtigungen, der Streamer muss den Bot über die Verwaltungsseite neu verbinden; =reauth oder "needs_reauth"=true → Autorisierung abgelaufen, neu autorisieren; =missing oder "found"=false → noch nie verbunden bzw. kein verknüpfter Streamer-Account (Einstieg über das Streamer-Setup). "discord_linked"=false → Discord-Verknüpfung fehlt.
- Übersetze solche Werte IMMER in verständliches Deutsch mit konkretem nächsten Schritt. Gib NIEMALS die rohen Status-Bezeichner (z. B. "oauth_status", "partner_status", "technical_pause_reason", "operational_state") wörtlich an den Nutzer aus.
- Wenn "partner_status" auf "blocked" oder "token_error" steht oder "technical_pause_reason" gesetzt ist: das ist eine Moderations-/Sonderfall-Sache für Menschen — antworte NICHT inhaltlich dazu, sondern gib NUR das Token KEIN_TREFFER aus.

WICHTIG:
- Du bist BEREITS in einem Ticket. Verweise NIEMALS auf "#ticket-eroeffnen", "/ticket" oder "mach ein Ticket auf" – das ist hier sinnlos. Menschlicher Support sieht dieses Ticket ohnehin.
- Erfinde keine Informationen, Kanäle, Rollen oder Schritte, die nicht dokumentiert sind.
- Antworte auf Deutsch, kurz und direkt, ohne Marketing-Floskeln.

BEISPIELE FÜR DEN TON:
- User (Erpressung): "Wenn ihr X nicht sofort macht, sorge ich dafür, dass ..." → "Auf Forderungen oder Druck dieser Art gehen wir hier nicht ein. Wenn du ein echtes Anliegen hast, schildere es sachlich – das Team sieht das Ticket."
- User (frech + Sachfrage): "Sag mir endlich wie ich den Bot verbinde, oder kriegt ihr das nicht hin?" → "Lass uns das sachlich klären, dann geht es schneller. Zum Verbinden: <die dokumentierten Schritte>.""##;

/// Doku-Grounding wie `_load_docs`: alle *.md aus dem Docs-Verzeichnis.
pub fn load_docs(docs_path: &std::path::Path) -> String {
    let Ok(entries) = std::fs::read_dir(docs_path) else {
        tracing::warn!(path = %docs_path.display(), "FAQ: Docs-Pfad nicht gefunden");
        return String::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|ext| ext == "md").unwrap_or(false))
        .collect();
    files.sort();
    let mut parts = Vec::new();
    for file in files {
        if let Ok(content) = std::fs::read_to_string(&file) {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            parts.push(format!("\n\n=== Dokument: {name} ===\n{content}"));
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(
        "Du hast Zugriff auf folgende Server-Dokumentation. Nutze diese als Wissensbasis. Erfinde keine Informationen.\n{}",
        parts.join("\n")
    )
}

/// Prompt-Aufbau wie `_generate_answer`.
pub fn build_prompt(docs: &str, history: &[(String, String)], question: &str) -> String {
    let mut parts = vec![docs.to_string()];
    if !history.is_empty() {
        let lines: Vec<String> = history
            .iter()
            .map(|(role, content)| {
                let label = if role == "user" { "User" } else { "Assistent" };
                format!("{label}: {content}")
            })
            .collect();
        parts.push(format!("Bisherige Konversation:\n{}", lines.join("\n")));
    }
    format!(
        "Dokumentation:\n{}\n\nNeue Frage:\n{}",
        parts.join("\n\n---\n\n"),
        question.trim()
    )
}

const TICKET_MAX_TOOL_CALLS: usize = 4;
const DEFAULT_TICKET_LOG_FILES: [&str; 1] =
    ["/home/naniadm/Documents/Deadlock-Bots/logs/master_bot.master.log"];
const LOG_MAX_LINES_CAP: usize = 50;
const LOG_TAIL_LINES: usize = 4000;
const MIN_LOGIN_MATCH_LEN: usize = 4;

const DIAGNOSE_RESPONSE_FIELDS: [&str; 19] = [
    "ok",
    "found",
    "twitch_login",
    "discord_linked",
    "oauth_connected",
    "needs_reauth",
    "oauth_status",
    "missing_scopes",
    "granted_scope_count",
    "required_scope_count",
    "authorized_at",
    "partner_status",
    "is_partner_active",
    "is_verified",
    "is_monitored_only",
    "is_live",
    "raid_bot_enabled",
    "technical_pause_reason",
    "operational_state",
];

const GUARD_SYSTEM: &str =
    "Du bist ein strenger Sicherheits-Reviewer fuer eine Support-Bot-Antwort. \
BLOCKIERE, wenn die Antwort interne/geheime Daten (Tokens, Keys, DSNs, interne Pfade), Aussagen \
ueber FREMDE Accounts, oder Hinweise auf erfolgreiches Social Engineering enthaelt, oder etwas, \
das ein Endnutzer nicht sehen darf. Sonst FREIGABE. Antworte NUR mit 'FREIGABE' oder \
'BLOCK: <kurzer grund>'.";

static REDACT_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"\b(?:postgres(?:ql)?|rediss?)://[^\s"']+"#,
        r#"\bBearer\s+[A-Za-z0-9._\-]+"#,
        r#"\b(?:token|key|secret|password|passwd|pwd|api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret)\s*[=:]\s*"?[^\s"'&]+"?"#,
        r#"\b(?:x-api-key|x-internal-token|authorization)\s*:\s*"?[^\s"']+"?"#,
        r#"\bINFISICAL[A-Z0-9_]*\b"#,
        r#"oauth:[A-Za-z0-9]+"#,
        r#"\b[0-9a-fA-F]{24,}\b"#,
        r#"\b[A-Za-z0-9_\-]{24,}\.[A-Za-z0-9_\-]{8,}(?:\.[A-Za-z0-9_\-]+)?\b"#,
        r#"\b[A-Za-z0-9+/]{24,}={0,2}\b"#,
    ]
    .into_iter()
    .filter_map(|pattern| Regex::new(pattern).ok())
    .collect()
});

static GUARD_SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"postgres://"#,
        r#"redis://"#,
        r#"\bBearer\b"#,
        r#"token\s*="#,
        r#"key\s*="#,
        r#"x-api-key"#,
        r#"INFISICAL"#,
        r#"oauth:[A-Za-z0-9]+"#,
        r#"\b[0-9a-fA-F]{24,}\b"#,
        r#"\b[A-Za-z0-9+/]{24,}={1,2}"#,
    ]
    .into_iter()
    .filter_map(|pattern| Regex::new(&format!("(?i){pattern}")).ok())
    .collect()
});

static DISCORD_ID_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\b\d{17,20}\b").ok());

pub fn diagnose_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "twitch_diagnose".to_string(),
            description: "Prüft den Twitch-Streamer-Status (OAuth/Scopes/aktiv) des FRAGENDEN selbst. Keine Parameter — die Identität ist fest.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "log_lookup".to_string(),
            description: "Sucht relevante, redigierte Log-Zeilen zum FRAGENDEN selbst (Twitch-/Bot-Logs).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "max_lines": { "type": "integer" },
                },
                "additionalProperties": false,
            }),
        },
    ]
}

fn redact(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in REDACT_PATTERNS.iter() {
        result = pattern.replace_all(&result, "[redacted]").into_owned();
    }
    result
}

fn has_foreign_discord_id(text: &str, own_id: u64) -> bool {
    let Some(re) = DISCORD_ID_RE.as_ref() else {
        return false;
    };
    let own = own_id.to_string();
    re.find_iter(text).any(|found| found.as_str() != own)
}

fn requested_log_lines(tool_input: &Value) -> usize {
    let parsed = tool_input
        .get("max_lines")
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .or_else(|| value.as_str()?.trim().parse::<usize>().ok())
        })
        .filter(|n| *n > 0)
        .unwrap_or(15);
    parsed.min(LOG_MAX_LINES_CAP)
}

async fn read_tail_lines(path: &PathBuf) -> Vec<String> {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(LOG_TAIL_LINES);
    lines[start..]
        .iter()
        .map(|line| (*line).to_string())
        .collect()
}

#[derive(Clone)]
pub struct TicketDiagnostics {
    twitch: Option<Arc<TwitchApiClient>>,
    log_files: Vec<PathBuf>,
}

impl TicketDiagnostics {
    pub fn new(twitch: Option<Arc<TwitchApiClient>>) -> Arc<Self> {
        Arc::new(Self {
            twitch,
            log_files: DEFAULT_TICKET_LOG_FILES.iter().map(PathBuf::from).collect(),
        })
    }

    #[cfg(test)]
    fn with_log_files(twitch: Option<Arc<TwitchApiClient>>, log_files: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(Self { twitch, log_files })
    }

    async fn execute_tool(&self, author_id: u64, tool_name: &str, tool_input: &Value) -> Value {
        match tool_name {
            "twitch_diagnose" => self.collect_twitch_diagnose(author_id).await,
            "log_lookup" => {
                let diagnose = self.collect_twitch_diagnose(author_id).await;
                let twitch_login = diagnose
                    .get("twitch_login")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|login| !login.is_empty())
                    .map(str::to_string);
                self.collect_log_lookup(
                    author_id,
                    twitch_login.as_deref(),
                    requested_log_lines(tool_input),
                )
                .await
            }
            _ => json!({ "error": "unknown_tool" }),
        }
    }

    async fn collect_twitch_diagnose(&self, author_id: u64) -> Value {
        let Some(twitch) = &self.twitch else {
            return json!({ "status": "nicht_ermittelbar" });
        };
        let Ok(payload) = twitch.diagnose_discord_user(author_id).await else {
            return json!({ "status": "nicht_ermittelbar" });
        };
        let Some(payload) = payload.as_object() else {
            return json!({ "status": "nicht_ermittelbar" });
        };
        let mut sanitized = serde_json::Map::new();
        for field in DIAGNOSE_RESPONSE_FIELDS {
            sanitized.insert(
                field.to_string(),
                payload.get(field).cloned().unwrap_or(Value::Null),
            );
        }
        Value::Object(sanitized)
    }

    async fn collect_log_lookup(
        &self,
        author_id: u64,
        twitch_login: Option<&str>,
        max_lines: usize,
    ) -> Value {
        let own_id = author_id.to_string();
        let login_lower = twitch_login
            .map(str::trim)
            .filter(|login| login.len() >= MIN_LOGIN_MATCH_LEN)
            .map(str::to_lowercase);

        let mut matched = Vec::new();
        for path in &self.log_files {
            for line in read_tail_lines(path).await {
                let mut relevant = line.contains(&own_id);
                if !relevant {
                    if let Some(login) = &login_lower {
                        relevant = line.to_lowercase().contains(login);
                    }
                }
                if !relevant || has_foreign_discord_id(&line, author_id) {
                    continue;
                }
                matched.push(redact(&line));
            }
        }

        if matched.len() > max_lines {
            matched = matched.split_off(matched.len() - max_lines);
        }
        let note = if matched.is_empty() {
            "Keine zuordenbaren Log-Zeilen gefunden.".to_string()
        } else {
            format!(
                "{} redigierte Log-Zeile(n) zum Fragenden gefunden.",
                matched.len()
            )
        };
        json!({ "lines": matched, "note": note })
    }
}

struct TicketToolExecutor {
    diagnostics: Arc<TicketDiagnostics>,
    author_id: u64,
}

#[async_trait::async_trait]
impl ToolExecutor for TicketToolExecutor {
    async fn execute(&self, tool_name: &str, tool_input: &Value) -> Result<Value, String> {
        Ok(self
            .diagnostics
            .execute_tool(self.author_id, tool_name, tool_input)
            .await)
    }
}

fn deterministic_guard_scan(candidate_answer: &str, author_id: u64) -> Option<String> {
    for pattern in GUARD_SECRET_PATTERNS.iter() {
        if pattern.is_match(candidate_answer) {
            return Some(format!("deterministic:secret_pattern:{}", pattern.as_str()));
        }
    }
    if contains_mixed_long_token(candidate_answer) {
        return Some("deterministic:secret_pattern:mixed_token".to_string());
    }
    if has_foreign_discord_id(candidate_answer, author_id) {
        return Some("deterministic:foreign_discord_id".to_string());
    }
    None
}

fn contains_mixed_long_token(text: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '+' | '/' | '_' | '-'))
        .any(|part| {
            part.len() >= 24
                && part.bytes().any(|b| b.is_ascii_digit())
                && part.bytes().any(|b| b.is_ascii_alphabetic())
        })
}

async fn diagnose_guard_check(
    ai: &Arc<dyn TextGenerator>,
    candidate_answer: &str,
    ticket_text: &str,
    author_id: u64,
    _tool_trace: &[String],
) -> (bool, String) {
    if let Some(reason) = deterministic_guard_scan(candidate_answer, author_id) {
        return (false, reason);
    }

    let prompt =
        format!("Kandidatenantwort:\n{candidate_answer}\n\nTicket des Nutzers:\n{ticket_text}");
    let text = ai
        .generate_text(GenerateRequest {
            prompt,
            system_prompt: Some(GUARD_SYSTEM.to_string()),
            model: None,
            max_output_tokens: Some(200),
            temperature: 0.0,
        })
        .await;
    let Some(text) = text
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    else {
        return (false, "guard_error".to_string());
    };
    let upper = text.to_uppercase();
    if upper.contains("BLOCK") {
        return (false, text);
    }
    if upper.contains("FREIGABE") {
        return (true, String::new());
    }
    (false, "guard_error".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TicketAutoOutcome {
    answer: Option<String>,
    decision: &'static str,
    tool_calls: Vec<String>,
    guard_reason: Option<String>,
}

impl TicketAutoOutcome {
    fn silence(decision: &'static str) -> Self {
        Self {
            answer: None,
            decision,
            tool_calls: Vec::new(),
            guard_reason: None,
        }
    }
}

/// Embed + „Frage stellen"-Button des FAQ-Panels (Port von `_build_panel_embed`
/// + `FAQPanelView`).
fn panel_body() -> serde_json::Map<String, serde_json::Value> {
    let embed = json!({
        "title": "FAQ - Häufig gestellte Fragen",
        "description": "Stell eine Frage zum Server, zu Kanälen, Rollen oder Deadlock.\n\
                        Klicke auf den Button – **ein Bot** versucht deine Frage zu beantworten.\n\
                        Deine Frage geht **nicht** an die Community.\n\n\
                        ⏱️ Chats werden nach 24 Stunden automatisch geschlossen.",
        "color": 0x5865F2, // blurple
        "footer": { "text": "Deadlock Master Bot • FAQ Chat" }
    });
    let components = json!([{ "type": 1, "components": [{
        "type": 2, "style": 1, "label": "Frage stellen",
        "emoji": { "name": "💬" }, "custom_id": "faq_chat:start"
    }]}]);
    let mut body = serde_json::Map::new();
    body.insert("embeds".into(), json!([embed]));
    body.insert("components".into(), components);
    body
}

// ── Store ──────────────────────────────────────────────────────────────────

pub struct FaqStore {
    pub pool: PgPool,
}

impl FaqStore {
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            SELECT to_regclass('bot.faq_chat_sessions') IS NOT NULL AS "exists!"
            "#
        )
        .fetch_one(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn active_session_of_user(&self, user_id: u64) -> Option<(String, u64)> {
        let user_id = u64_to_i64(user_id, "user_id").ok()?;
        let row = sqlx::query!(
            r#"
            SELECT session_id, channel_id
              FROM bot.faq_chat_sessions
             WHERE user_id = $1 AND status = 'active'
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()??;
        Some((row.session_id, i64_to_u64(row.channel_id, "channel_id")?))
    }

    pub async fn active_session_in_channel(&self, channel_id: u64) -> Option<(String, u64)> {
        let channel_id = u64_to_i64(channel_id, "channel_id").ok()?;
        let row = sqlx::query!(
            r#"
            SELECT session_id, user_id
              FROM bot.faq_chat_sessions
             WHERE channel_id = $1 AND status = 'active'
            "#,
            channel_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()??;
        Some((row.session_id, i64_to_u64(row.user_id, "user_id")?))
    }

    pub async fn create_session(
        &self,
        session_id: String,
        user_id: u64,
        user_name: String,
        channel_id: u64,
        guild_id: u64,
    ) {
        let (Ok(user_id), Ok(channel_id), Ok(guild_id)) = (
            u64_to_i64(user_id, "user_id"),
            u64_to_i64(channel_id, "channel_id"),
            u64_to_i64(guild_id, "guild_id"),
        ) else {
            return;
        };
        let expires = chrono::Utc::now() + chrono::Duration::hours(SESSION_TIMEOUT_HOURS);
        let _ = sqlx::query!(
            r#"
            INSERT INTO bot.faq_chat_sessions(
                session_id, user_id, user_name, channel_id, guild_id, expires_at
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            session_id,
            user_id,
            user_name,
            channel_id,
            guild_id,
            expires,
        )
        .execute(&self.pool)
        .await;
    }

    pub async fn add_message(&self, session_id: &str, role: &str, content: &str) {
        let (session_id, role, content) = (
            session_id.to_string(),
            role.to_string(),
            content.to_string(),
        );
        let _ = sqlx::query!(
            r#"
            INSERT INTO bot.faq_chat_messages(session_id, role, content)
            VALUES ($1, $2, $3)
            "#,
            session_id,
            role,
            content,
        )
        .execute(&self.pool)
        .await;
        let _ = sqlx::query!(
            r#"
            UPDATE bot.faq_chat_sessions
               SET last_activity_at = now()
             WHERE session_id = $1
            "#,
            session_id,
        )
        .execute(&self.pool)
        .await;
    }

    /// Letzte 10 Nachrichten (chronologisch) für das Gesprächs-Gedächtnis.
    pub async fn recent_messages(&self, session_id: &str) -> Vec<(String, String)> {
        sqlx::query!(
            r#"
            SELECT role, content
              FROM (
                    SELECT role, content, id
                      FROM bot.faq_chat_messages
                     WHERE session_id = $1
                     ORDER BY id DESC
                     LIMIT 10
                   ) recent
             ORDER BY id ASC
            "#,
            session_id,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| (row.role, row.content))
        .collect()
    }

    pub async fn close_session(&self, session_id: &str) {
        let _ = sqlx::query!(
            r#"
            UPDATE bot.faq_chat_sessions
               SET status = 'closed'
             WHERE session_id = $1
            "#,
            session_id,
        )
        .execute(&self.pool)
        .await;
    }

    /// (session_id, channel_id) aller abgelaufenen aktiven Sessions.
    pub async fn expired_sessions(&self) -> Vec<(String, u64)> {
        let rows = sqlx::query!(
            r#"
            SELECT session_id, channel_id
              FROM bot.faq_chat_sessions
             WHERE status = 'active' AND expires_at <= $1
            "#,
            chrono::Utc::now(),
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .filter_map(|row| Some((row.session_id, i64_to_u64(row.channel_id, "channel_id")?)))
            .collect()
    }
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait FaqPort: Send + Sync {
    /// Privaten FAQ-Kanal anlegen (nur User + Bot sichtbar) → channel_id.
    async fn create_faq_channel(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_name: &str,
    ) -> Result<u64, String>;
    async fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        components: Option<serde_json::Value>,
    );
    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64>;
    async fn user_name(&self, user_id: u64) -> String;
    /// Postet eine Rich-Nachricht (Embed + Components) → message_id (fürs Panel).
    async fn post_rich(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String>;
    /// Editiert eine zuvor gepostete Rich-Nachricht (Panel-Refresh).
    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String>;
    /// Löscht eine Nachricht (Aufräumen eines Duplikat-Panels).
    async fn delete_panel(&self, channel_id: u64, message_id: u64);
}

pub struct FaqChat {
    pub store: FaqStore,
    pub port: Arc<dyn FaqPort>,
    pub ai: Option<Arc<dyn TextGenerator>>,
    pub tool_ai: Option<Arc<dyn ToolTextGenerator>>,
    pub ticket_diagnostics: Option<Arc<TicketDiagnostics>>,
    pub docs: String,
    answered_tickets: tokio::sync::Mutex<HashSet<u64>>,
}

impl FaqChat {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn FaqPort>,
        ai: Option<Arc<dyn TextGenerator>>,
        docs: String,
    ) -> Arc<Self> {
        Self::new_with_ticket_support(pool, port, ai, None, None, docs)
    }

    pub fn new_with_ticket_support(
        pool: PgPool,
        port: Arc<dyn FaqPort>,
        ai: Option<Arc<dyn TextGenerator>>,
        tool_ai: Option<Arc<dyn ToolTextGenerator>>,
        ticket_diagnostics: Option<Arc<TicketDiagnostics>>,
        docs: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: FaqStore { pool },
            port,
            ai,
            tool_ai,
            ticket_diagnostics,
            docs,
            answered_tickets: tokio::sync::Mutex::new(HashSet::new()),
        })
    }

    /// Postet/editiert das FAQ-Panel im [`PANEL_CHANNEL_ID`] (Port von
    /// `_ensure_panel`). Idempotent über den KV-Store: ist eine Panel-Nachricht
    /// gemerkt, wird sie editiert; nur wenn das fehlschlägt (z. B. gelöscht),
    /// wird eine neue gepostet. Wird beim Start aufgerufen.
    pub async fn ensure_panel(&self) {
        // Selbstheilung: eine frühere (fehlerhafte) Rust-Version merkte die
        // Panel-ID unter dem falschen Key `message_id` und postete dadurch beim
        // Cutover ein Duplikat. Ist dort eine ID gemerkt, die nicht dem
        // kanonischen `panel_msg_id` entspricht, wird das Duplikat gelöscht und
        // der Alt-Key entfernt.
        self.heal_legacy_panel().await;

        let body = panel_body();
        let stored = self.panel_message_id().await;
        if let Some(message_id) = stored {
            if self
                .port
                .edit_rich(PANEL_CHANNEL_ID, message_id, body.clone())
                .await
                .is_ok()
            {
                return;
            }
        }
        match self.port.post_rich(PANEL_CHANNEL_ID, body).await {
            Ok(message_id) => {
                if let Err(err) = kv::set(
                    &self.store.pool,
                    PANEL_KV_NS,
                    PANEL_KV_KEY,
                    &message_id.to_string(),
                )
                .await
                {
                    tracing::warn!(%err, "FAQ-Panel-ID konnte nicht gespeichert werden");
                }
            }
            Err(err) => tracing::warn!(%err, "FAQ-Panel konnte nicht gepostet werden"),
        }
    }

    /// `/faqpanel` (Admin): meldet ein bestehendes Panel oder erstellt es neu.
    pub async fn faqpanel_command(&self, guild_id: u64) -> BridgeReply {
        if let Some(message_id) = self.panel_message_id().await {
            return BridgeReply::ephemeral_text(format!(
                "✅ FAQ Panel existiert bereits: https://discord.com/channels/{guild_id}/{PANEL_CHANNEL_ID}/{message_id}"
            ));
        }
        self.ensure_panel().await;
        match self.panel_message_id().await {
            Some(message_id) => BridgeReply::ephemeral_text(format!(
                "✅ FAQ Panel wurde erstellt: https://discord.com/channels/{guild_id}/{PANEL_CHANNEL_ID}/{message_id}"
            )),
            None => BridgeReply::ephemeral_text(format!(
                "❌ Konnte Panel nicht erstellen. Channel {PANEL_CHANNEL_ID} prüfen."
            )),
        }
    }

    /// Gemerkte Panel-Message-ID aus dem KV-Store.
    async fn panel_message_id(&self) -> Option<u64> {
        kv::get(&self.store.pool, PANEL_KV_NS, PANEL_KV_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok())
    }

    /// Entfernt ein unter dem alten Key (`message_id`) gemerktes Duplikat-Panel.
    async fn heal_legacy_panel(&self) {
        const LEGACY_KEY: &str = "message_id";
        let legacy = kv::get(&self.store.pool, PANEL_KV_NS, LEGACY_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok());
        let Some(legacy_id) = legacy else {
            return;
        };
        // Nur löschen, wenn es NICHT das kanonische Panel ist.
        if self.panel_message_id().await != Some(legacy_id) {
            self.port.delete_panel(PANEL_CHANNEL_ID, legacy_id).await;
            tracing::info!(legacy_id, "FAQ: Duplikat-Panel aus Cutover-Bug gelöscht");
        }
        let _ = kv::delete(&self.store.pool, PANEL_KV_NS, LEGACY_KEY).await;
    }

    async fn generate_answer(&self, session_id: &str, question: &str) -> String {
        let Some(ai) = &self.ai else {
            return "Der FAQ-Service ist aktuell nicht verfügbar.".to_string();
        };
        let history = self.store.recent_messages(session_id).await;
        let prompt = build_prompt(&self.docs, &history, question);
        let answer = ai
            .generate_text(GenerateRequest {
                prompt,
                system_prompt: Some(SYSTEM_PROMPT.to_string()),
                model: None,
                max_output_tokens: Some(MAX_OUTPUT_TOKENS),
                temperature: 0.3,
            })
            .await;
        match answer {
            Some(text) if !text.trim().is_empty() => text.trim().to_string(),
            _ => "Ich konnte keine Antwort generieren.".to_string(),
        }
    }

    async fn ticket_auto_answer(&self, problem: &str, author_id: u64) -> TicketAutoOutcome {
        let Some(ai) = &self.ai else {
            return TicketAutoOutcome::silence("no_ai");
        };
        let full_prompt = format!(
            "Dokumentation:\n{}\n\nTicket-Inhalt:\n{}",
            self.docs,
            problem.trim()
        );

        let mut tool_calls = Vec::new();
        let mut answer_text = None;
        if let (Some(tool_ai), Some(diagnostics)) = (&self.tool_ai, &self.ticket_diagnostics) {
            let tool_result = tool_ai
                .generate_text_with_tools(
                    ToolUseRequest {
                        prompt: full_prompt.clone(),
                        system_prompt: Some(TICKET_AUTO_HELP_SYSTEM_PROMPT.to_string()),
                        model: None,
                        max_output_tokens: Some(MAX_OUTPUT_TOKENS),
                        temperature: 0.2,
                        tools: diagnose_tools(),
                        max_tool_calls: TICKET_MAX_TOOL_CALLS,
                    },
                    Arc::new(TicketToolExecutor {
                        diagnostics: diagnostics.clone(),
                        author_id,
                    }),
                )
                .await;
            tool_calls = tool_result.tool_calls;
            answer_text = tool_result.text;
        }

        if answer_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .is_none()
        {
            answer_text = ai
                .generate_text(GenerateRequest {
                    prompt: full_prompt,
                    system_prompt: Some(TICKET_AUTO_HELP_SYSTEM_PROMPT.to_string()),
                    model: None,
                    max_output_tokens: Some(MAX_OUTPUT_TOKENS),
                    temperature: 0.2,
                })
                .await;
        }

        let Some(answer) = answer_text.map(|answer| answer.trim().to_string()) else {
            let mut outcome = TicketAutoOutcome::silence("empty");
            outcome.tool_calls = tool_calls;
            return outcome;
        };
        if answer.is_empty() {
            let mut outcome = TicketAutoOutcome::silence("empty");
            outcome.tool_calls = tool_calls;
            return outcome;
        }
        if answer.contains("KEIN_TREFFER") {
            let mut outcome = TicketAutoOutcome::silence("kein_treffer");
            outcome.tool_calls = tool_calls;
            return outcome;
        }

        let (allowed, reason) =
            diagnose_guard_check(ai, &answer, problem, author_id, &tool_calls).await;
        if !allowed {
            return TicketAutoOutcome {
                answer: None,
                decision: "guard_block",
                tool_calls,
                guard_reason: Some(reason),
            };
        }

        TicketAutoOutcome {
            answer: Some(answer),
            decision: "answered",
            tool_calls,
            guard_reason: None,
        }
    }

    /// Frage im FAQ-Kanal beantworten (vom Message-Subscriber gerufen).
    pub async fn handle_chat_message(
        self: &Arc<Self>,
        channel_id: u64,
        author_id: u64,
        author_name: &str,
        content: &str,
    ) -> bool {
        let Some((session_id, owner_id)) = self.store.active_session_in_channel(channel_id).await
        else {
            return false;
        };
        if author_id != owner_id {
            return true; // Kanal gehört dem FAQ-System, aber fremde Nachricht
        }
        let question = content.trim();
        if question.is_empty() {
            return true;
        }
        self.store.add_message(&session_id, "user", question).await;
        self.port
            .send_message(
                LOG_CHANNEL_ID,
                &format!("❓ FAQ-Frage von **{author_name}** (<#{channel_id}>): {question}"),
                None,
            )
            .await;
        let answer = self.generate_answer(&session_id, question).await;
        self.store
            .add_message(&session_id, "assistant", &answer)
            .await;
        self.port.send_message(channel_id, &answer, None).await;
        true
    }

    /// Ticket-Auto-Helfer: erste Nachricht eines Ticket-Kanals prüfen.
    pub async fn handle_ticket_message(
        self: &Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        author_id: u64,
        content: &str,
    ) {
        if self.port.channel_category(guild_id, channel_id).await
            != Some(TICKET_AUTO_HELP_CATEGORY_ID)
        {
            return;
        }
        {
            let mut answered = self.answered_tickets.lock().await;
            if answered.contains(&channel_id) {
                return;
            }
            answered.insert(channel_id);
        }
        let problem = content.trim();
        if problem.is_empty() {
            return;
        }
        let outcome = self.ticket_auto_answer(problem, author_id).await;
        tracing::debug!(
            channel_id,
            author_id,
            decision = outcome.decision,
            tool_calls = ?outcome.tool_calls,
            guard_reason = outcome.guard_reason.as_deref().unwrap_or(""),
            "FAQ-Ticket-Auto-Hilfe entschieden"
        );
        if let Some(answer) = outcome.answer {
            self.port.send_message(channel_id, &answer, None).await;
        }
    }

    /// Abgelaufene Sessions schließen (1-h-Loop).
    pub async fn cleanup_expired(&self) {
        for (session_id, channel_id) in self.store.expired_sessions().await {
            self.store.close_session(&session_id).await;
            self.port
                .send_message(
                    channel_id,
                    "⏱️ Chat wurde automatisch geschlossen (24h Timeout).",
                    None,
                )
                .await;
        }
    }
}

// ── Interaction-Handler ────────────────────────────────────────────────────

struct FaqHandler {
    faq: Arc<FaqChat>,
}

#[async_trait::async_trait]
impl InteractionHandler for FaqHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        // /faqpanel (Admin): Panel posten/melden.
        if interaction.command == "faqpanel" {
            if interaction.guild_id == 0 {
                return BridgeReply::ephemeral_text("❌ Das funktioniert nur auf dem Server.");
            }
            return self.faq.faqpanel_command(interaction.guild_id).await;
        }

        // Start über den Panel-Button ODER den /faq-Slash-Command.
        if interaction.custom_id == "faq_chat:start" || interaction.command == "faq" {
            if interaction.guild_id == 0 {
                return BridgeReply::ephemeral_text("❌ Das funktioniert nur auf dem Server.");
            }
            if let Some((_, channel_id)) = self
                .faq
                .store
                .active_session_of_user(interaction.user_id)
                .await
            {
                return BridgeReply::ephemeral_text(format!(
                    "❌ Du hast bereits einen aktiven Chat: <#{channel_id}>"
                ));
            }
            let user_name = self.faq.port.user_name(interaction.user_id).await;
            let channel_name = format!("faq-{}", user_name.to_lowercase().replace(' ', "-"));
            let channel_id = match self
                .faq
                .port
                .create_faq_channel(interaction.guild_id, interaction.user_id, &channel_name)
                .await
            {
                Ok(id) => id,
                Err(err) => {
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Konnte keinen Chat erstellen: {err}"
                    ))
                }
            };
            let session_id = format!(
                "faq-{}-{}",
                interaction.user_id,
                chrono::Utc::now().format("%Y%m%d%H%M%S")
            );
            self.faq
                .store
                .create_session(
                    session_id.clone(),
                    interaction.user_id,
                    user_name.clone(),
                    channel_id,
                    interaction.guild_id,
                )
                .await;
            let welcome = format!(
                "👋 **{user_name}**, willkommen zum FAQ-Chat!\n\n\
Stell mir Fragen zum Server, zu Kanälen, Rollen, Bots oder Deadlock.\n\
Ich kann mich an unsere Unterhaltung erinnern - du kannst auch Rückfragen stellen.\n\n\
⏱️ Dieser Chat wird nach 24 Stunden automatisch geschlossen.\n\
🛑 Du kannst den Chat jederzeit mit dem Button unten beenden."
            );
            let close_button = json!([{ "type": 1, "components": [{
                "type": 2, "style": 4, "label": "Chat beenden",
                "custom_id": format!("faq_chat:close:{session_id}"),
            }]}]);
            self.faq
                .port
                .send_message(channel_id, &welcome, Some(close_button))
                .await;
            return BridgeReply::ephemeral_text(format!(
                "✅ Dein FAQ-Chat wurde erstellt: <#{channel_id}>\n\nStell deine Frage(n) dort."
            ));
        }

        // faq_chat:close:{session_id} (Original-View nutzt faq_chat:close —
        // beide Formen werden über das Präfix gematcht)
        let session_id = interaction
            .custom_id
            .strip_prefix("faq_chat:close")
            .map(|rest| rest.trim_start_matches(':').to_string())
            .unwrap_or_default();
        // Session bestimmen: über die ID im Button, sonst über den Kanal
        let session = if session_id.is_empty() {
            self.faq
                .store
                .active_session_in_channel(interaction.channel_id)
                .await
        } else {
            self.faq
                .store
                .active_session_in_channel(interaction.channel_id)
                .await
                .filter(|(sid, _)| *sid == session_id)
        };
        let Some((session_id, owner_id)) = session else {
            return BridgeReply::ephemeral_text("❌ Session nicht gefunden.");
        };
        if interaction.user_id != owner_id {
            return BridgeReply::ephemeral_text("❌ Das ist nicht dein Chat.");
        }
        self.faq.store.close_session(&session_id).await;
        self.faq
            .port
            .send_message(interaction.channel_id, "🛑 Chat beendet.", None)
            .await;
        BridgeReply::ephemeral_text("✅ Chat beendet.")
    }
}

pub fn register(router: &mut InteractionRouter, faq: Arc<FaqChat>) {
    let handler = Arc::new(FaqHandler { faq });
    router.on_command(
        "faq",
        CommandSpec {
            definition: json!({
                "name": "faq",
                "description": "Startet einen FAQ-Chat mit dem Server-Assistenten.",
                "type": 1,
                "dm_permission": false,
            }),
        },
        handler.clone(),
    );
    router.on_command(
        "faqpanel",
        CommandSpec {
            definition: json!({
                "name": "faqpanel",
                "description": "Erstellt das FAQ Panel (Admin)",
                "type": 1,
                "dm_permission": false,
                "default_member_permissions": "8", // Administrator
            }),
        },
        handler.clone(),
    );
    router.on_custom_id("faq_chat:start", handler.clone());
    router.on_prefix("faq_chat:close", handler);
}

/// Message-Subscriber (FAQ-Kanäle + Ticket-Auto-Help) + Cleanup-Loop.
pub fn spawn(
    faq: Arc<FaqChat>,
    dispatcher: &dl_discord::Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut messages = dispatcher.subscribe_messages();
    let message_task = {
        let faq = faq.clone();
        tokio::spawn(async move {
            if let Err(err) = faq.store.ensure_schema().await {
                tracing::warn!(%err, "FAQ-Schema-Anlage fehlgeschlagen");
            }
            // Panel beim Start posten/auffrischen (wie `_ensure_panel` in cog_load).
            faq.ensure_panel().await;
            loop {
                match messages.recv().await {
                    Ok(event) => {
                        // Bot-Nachrichten filtert bereits das Gateway
                        let handled = faq
                            .handle_chat_message(
                                event.channel_id,
                                event.author_id,
                                &event.author_display_name,
                                &event.content,
                            )
                            .await;
                        if !handled {
                            faq.handle_ticket_message(
                                event.guild_id.unwrap_or_default(),
                                event.channel_id,
                                event.author_id,
                                &event.content,
                            )
                            .await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };
    let cleanup_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            faq.cleanup_expired().await;
        }
    });
    vec![message_task, cleanup_task]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "testing")]
    use dl_ai::ToolGeneration;

    #[test]
    fn prompt_aufbau_wie_python() {
        let docs = "DOCS".to_string();
        let history = vec![
            ("user".to_string(), "Frage 1".to_string()),
            ("assistant".to_string(), "Antwort 1".to_string()),
        ];
        let prompt = build_prompt(&docs, &history, "  Neue Frage?  ");
        assert!(prompt.starts_with("Dokumentation:\nDOCS"));
        assert!(prompt.contains("Bisherige Konversation:\nUser: Frage 1\nAssistent: Antwort 1"));
        assert!(prompt.ends_with("Neue Frage:\nNeue Frage?"));
        // ohne Verlauf kein Konversations-Block
        let prompt = build_prompt(&docs, &[], "x");
        assert!(!prompt.contains("Bisherige Konversation"));
    }

    #[test]
    fn docs_grounding() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("b.md"), "Inhalt B").expect("write");
        std::fs::write(dir.path().join("a.md"), "Inhalt A").expect("write");
        std::fs::write(dir.path().join("c.txt"), "ignoriert").expect("write");
        let docs = load_docs(dir.path());
        assert!(docs.contains("=== Dokument: a.md ===\nInhalt A"));
        assert!(docs.contains("=== Dokument: b.md ===\nInhalt B"));
        assert!(!docs.contains("ignoriert"));
        // a vor b (sortiert)
        assert!(docs.find("a.md").expect("a") < docs.find("b.md").expect("b"));
        assert_eq!(load_docs(std::path::Path::new("/nope")), "");
    }

    #[test]
    fn ticket_prompt_enthaelt_status_uebersetzung_und_ticket_hinweis() {
        assert!(TICKET_AUTO_HELP_SYSTEM_PROMPT.contains(
            "Übersetze solche Werte IMMER in verständliches Deutsch mit konkretem nächsten Schritt"
        ));
        assert!(TICKET_AUTO_HELP_SYSTEM_PROMPT.contains("Gib NIEMALS die rohen Status-Bezeichner"));
        assert!(TICKET_AUTO_HELP_SYSTEM_PROMPT
            .contains("\"partner_status\" auf \"blocked\" oder \"token_error\""));
        assert!(TICKET_AUTO_HELP_SYSTEM_PROMPT
            .contains("Du bist BEREITS in einem Ticket. Verweise NIEMALS"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn session_lifecycle() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let store = FaqStore {
            pool: db.pool().clone(),
        };
        store.ensure_schema().await.expect("schema");
        store
            .create_session("s1".into(), 42, "Nani".into(), 100, 1)
            .await;
        assert_eq!(
            store.active_session_of_user(42).await,
            Some(("s1".to_string(), 100))
        );
        assert_eq!(
            store.active_session_in_channel(100).await,
            Some(("s1".to_string(), 42))
        );
        // Verlauf: nur die letzten 10, chronologisch
        for i in 0..12 {
            store.add_message("s1", "user", &format!("m{i}")).await;
        }
        let recent = store.recent_messages("s1").await;
        assert_eq!(recent.len(), 10);
        assert_eq!(recent[0].1, "m2");
        assert_eq!(recent[9].1, "m11");
        store.close_session("s1").await;
        assert!(store.active_session_of_user(42).await.is_none());
        assert!(store.expired_sessions().await.is_empty());
    }

    // Port-Mock, der Panel-Post/-Edit/-Delete zählt.
    #[cfg(feature = "testing")]
    struct MockPanelPort {
        posts: std::sync::Mutex<u32>,
        edits: std::sync::Mutex<u32>,
        deleted: std::sync::Mutex<Vec<u64>>,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl FaqPort for MockPanelPort {
        async fn create_faq_channel(&self, _g: u64, _u: u64, _n: &str) -> Result<u64, String> {
            Ok(1)
        }
        async fn send_message(&self, _c: u64, _t: &str, _comp: Option<serde_json::Value>) {}
        async fn channel_category(&self, _g: u64, _c: u64) -> Option<u64> {
            None
        }
        async fn user_name(&self, _u: u64) -> String {
            "U".to_string()
        }
        async fn post_rich(
            &self,
            _c: u64,
            _b: serde_json::Map<String, serde_json::Value>,
        ) -> Result<u64, String> {
            *self.posts.lock().unwrap() += 1;
            Ok(55501)
        }
        async fn edit_rich(
            &self,
            _c: u64,
            _m: u64,
            _b: serde_json::Map<String, serde_json::Value>,
        ) -> Result<(), String> {
            *self.edits.lock().unwrap() += 1;
            Ok(())
        }
        async fn delete_panel(&self, _c: u64, message_id: u64) {
            self.deleted.lock().unwrap().push(message_id);
        }
    }

    #[cfg(feature = "testing")]
    fn panel_port() -> Arc<MockPanelPort> {
        Arc::new(MockPanelPort {
            posts: std::sync::Mutex::new(0),
            edits: std::sync::Mutex::new(0),
            deleted: std::sync::Mutex::new(Vec::new()),
        })
    }

    #[cfg(feature = "testing")]
    async fn db_with_kv() -> dl_central_db::testing::TestDb {
        dl_central_db::testing::test_pool()
            .await
            .expect("test_pool")
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn panel_postet_einmal_dann_editiert() {
        let db = db_with_kv().await;
        let port = panel_port();
        let faq = FaqChat::new(db.pool().clone(), port.clone(), None, String::new());

        // Erster ensure: ein Post, kein Edit; ID wird gemerkt.
        faq.ensure_panel().await;
        assert_eq!(*port.posts.lock().unwrap(), 1);
        assert_eq!(*port.edits.lock().unwrap(), 0);
        assert_eq!(faq.panel_message_id().await, Some(55501));

        // Zweiter ensure: nur Edit, kein neuer Post (idempotent).
        faq.ensure_panel().await;
        assert_eq!(*port.posts.lock().unwrap(), 1);
        assert_eq!(*port.edits.lock().unwrap(), 1);
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn faqpanel_command_meldet_bestehend_und_erstellt() {
        let db = db_with_kv().await;
        let port = panel_port();
        let faq = FaqChat::new(db.pool().clone(), port.clone(), None, String::new());

        // Noch kein Panel → Command erstellt es und meldet „wurde erstellt".
        let reply = faq.faqpanel_command(42).await;
        let text = reply.content.unwrap();
        assert!(text.contains("wurde erstellt"), "text: {text}");
        assert!(
            text.contains("/42/1491953161747955853/55501"),
            "jump: {text}"
        );
        assert_eq!(*port.posts.lock().unwrap(), 1);

        // Erneuter Command → meldet „existiert bereits", postet nicht erneut.
        let reply = faq.faqpanel_command(42).await;
        assert!(reply.content.unwrap().contains("existiert bereits"));
        assert_eq!(*port.posts.lock().unwrap(), 1);
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn heilt_duplikat_aus_altem_key() {
        let db = db_with_kv().await;
        // Kanonisches Panel (Python-Key) = 100, Duplikat unter Alt-Key = 200.
        dl_central_db::kv::set(db.pool(), PANEL_KV_NS, PANEL_KV_KEY, "100")
            .await
            .unwrap();
        dl_central_db::kv::set(db.pool(), PANEL_KV_NS, "message_id", "200")
            .await
            .unwrap();
        let port = panel_port();
        let faq = FaqChat::new(db.pool().clone(), port.clone(), None, String::new());

        faq.ensure_panel().await;

        // Das Duplikat (200) wurde gelöscht, das kanonische (100) nur editiert.
        assert_eq!(*port.deleted.lock().unwrap(), vec![200]);
        assert_eq!(*port.posts.lock().unwrap(), 0);
        assert_eq!(*port.edits.lock().unwrap(), 1);
        // Der Alt-Key ist entfernt.
        assert_eq!(
            dl_central_db::kv::get(db.pool(), PANEL_KV_NS, "message_id")
                .await
                .unwrap(),
            None
        );
    }

    #[cfg(feature = "testing")]
    struct SequenceAi {
        responses: std::sync::Mutex<std::collections::VecDeque<Option<String>>>,
    }

    #[cfg(feature = "testing")]
    impl SequenceAi {
        fn new(responses: Vec<Option<&str>>) -> Arc<Self> {
            Arc::new(Self {
                responses: std::sync::Mutex::new(
                    responses
                        .into_iter()
                        .map(|item| item.map(str::to_string))
                        .collect(),
                ),
            })
        }
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl TextGenerator for SequenceAi {
        async fn generate_text(&self, _request: GenerateRequest) -> Option<String> {
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .flatten()
        }
    }

    #[cfg(feature = "testing")]
    struct StaticToolAi {
        result: std::sync::Mutex<Option<ToolGeneration>>,
    }

    #[cfg(feature = "testing")]
    impl StaticToolAi {
        fn new(result: ToolGeneration) -> Arc<Self> {
            Arc::new(Self {
                result: std::sync::Mutex::new(Some(result)),
            })
        }
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl ToolTextGenerator for StaticToolAi {
        async fn generate_text_with_tools(
            &self,
            _request: ToolUseRequest,
            _tool_executor: Arc<dyn ToolExecutor>,
        ) -> ToolGeneration {
            self.result
                .lock()
                .expect("tool result lock")
                .take()
                .unwrap_or_default()
        }
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn ticket_guard_fehler_schweigt_fail_closed() {
        let db = db_with_kv().await;
        let port = panel_port();
        let ai: Arc<dyn TextGenerator> = SequenceAi::new(vec![
            Some("Dokumentierte Antwort"),
            None, // Guard-Reviewer leer/Fehler => fail-closed
        ]);
        let faq = FaqChat::new(db.pool().clone(), port, Some(ai), "DOCS".to_string());

        let outcome = faq
            .ticket_auto_answer("Steam geht nicht", 111111111111111111)
            .await;

        assert_eq!(outcome.decision, "guard_block");
        assert_eq!(outcome.answer, None);
        assert_eq!(outcome.guard_reason.as_deref(), Some("guard_error"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn ticket_tool_loop_faellt_auf_textpfad_zurueck_und_behaelt_tool_trace() {
        let db = db_with_kv().await;
        let port = panel_port();
        let ai: Arc<dyn TextGenerator> =
            SequenceAi::new(vec![Some("Fallback Antwort"), Some("FREIGABE")]);
        let tool_ai: Arc<dyn ToolTextGenerator> = StaticToolAi::new(ToolGeneration {
            text: None,
            tool_calls: vec!["twitch_diagnose".to_string()],
        });
        let faq = FaqChat::new_with_ticket_support(
            db.pool().clone(),
            port,
            Some(ai),
            Some(tool_ai),
            Some(TicketDiagnostics::new(None)),
            "DOCS".to_string(),
        );

        let outcome = faq
            .ticket_auto_answer("Twitch ist kaputt", 111111111111111111)
            .await;

        assert_eq!(outcome.decision, "answered");
        assert_eq!(outcome.answer.as_deref(), Some("Fallback Antwort"));
        assert_eq!(outcome.tool_calls, vec!["twitch_diagnose"]);
    }

    #[tokio::test]
    async fn log_lookup_filtert_fremde_ids_und_redigiert_secrets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let own_id = 111111111111111111u64;
        let other_id = 222222222222222222u64;
        let log_path = dir.path().join("bot.log");
        std::fs::write(
            &log_path,
            format!(
                "irrelevant\n\
                 own {own_id} token=secret12345678901234567890\n\
                 mixed {own_id} and {other_id} should_skip\n\
                 login naniworks oauth:abc123\n"
            ),
        )
        .expect("write log");
        let diagnostics = TicketDiagnostics::with_log_files(None, vec![log_path]);

        let result = diagnostics
            .collect_log_lookup(own_id, Some("naniworks"), 10)
            .await;

        let lines = result["lines"].as_array().expect("lines");
        assert_eq!(lines.len(), 2);
        let joined = lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("[redacted]"));
        assert!(joined.contains("login naniworks"));
        assert!(!joined.contains("should_skip"));
        assert!(result["note"]
            .as_str()
            .expect("note")
            .contains("2 redigierte Log-Zeile"));
    }
}
