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
use std::sync::Arc;
use std::time::Duration;

use dl_ai::{GenerateRequest, TextGenerator};
use dl_db::Db;
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use rusqlite::OptionalExtension;
use serde_json::json;

pub const PANEL_CHANNEL_ID: u64 = 1491953161747955853;
pub const FAQ_CATEGORY_ID: u64 = 1310153243795390475;
pub const TICKET_AUTO_HELP_CATEGORY_ID: u64 = 1459628097145147645;
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const SESSION_TIMEOUT_HOURS: i64 = 24;
pub const MAX_OUTPUT_TOKENS: u32 = 1500;
pub const PANEL_KV_NS: &str = "faq_chat:panel";

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
Wenn jemand fragt warum er keinen Invite hat, Deadlock nicht herunterladen kann, keinen Zugang zum Beta-Kanal hat oder einen Channel nicht sieht:
Gehe sofort die Checkliste durch:
1. Onboarding vollständig abgeschlossen? Bei Frage 4 "Ich brauche einen Invite / Betazugang" gewählt?
2. Rollen in #customize ausgewählt?
3. /betainvite im #beta-zugang verwendet?
4. Steam-Account hat min. 5 € Kaufhistorie?
Nenne alle offenen Punkte direkt und klar. Wenn das Problem offensichtlich daran liegt, dass jemand das Onboarding nicht gelesen hat, darf der Bot das freundlich aber ohne Umschweife sagen – zum Beispiel: "Das Onboarding enthält den genauen Hinweis dazu – wer es liest, spart sich das Ticket."

COACHING – SONDERREGEL:
Wenn jemand fragt wie er Coaching bekommt, wer die Coaches sind, wie Coaching funktioniert, ob es Coaching gibt, was es kostet oder wo er sich anmelden kann:
1. Verweise direkt auf <#1494373349944459355> – das ist der Coaching-Channel.
2. Erkläre knapp: kostenlos, Button klicken → Formular ausfüllen → Coach meldet sich. Mehrfache Anfragen sind erlaubt.
3. Regeln: Kommunikation NUR im Coaching-Chat auf dem Server, keine DMs oder Freundschaftsanfragen an Coaches.
4. Nach dem Coaching gibt es eine Feedback-Anfrage – User sollen sie ehrlich ausfüllen, das hilft dem Team.
Erfinde keine Details zu Coaches, Wartezeiten oder Verfügbarkeit."#;

pub const TICKET_AUTO_HELP_SYSTEM_PROMPT: &str = r#"Du bist ein automatischer Ticket-Helfer. Entscheide anhand der Dokumentation ob du die Frage des Users lösen kannst.

REGEL:
- Wenn du die Frage aus der Dokumentation klar und vollständig beantworten kannst: antworte direkt und hilfreich.
- Wenn die Frage außerhalb deines Wissens liegt, unklar ist, oder menschlichen Support erfordert: antworte NUR mit dem Token KEIN_TREFFER und nichts weiter.
- Erfinde keine Informationen. Wenn du dir nicht sicher bist: KEIN_TREFFER.
- Antworte auf Deutsch, kurz und direkt."#;

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

// ── Store ──────────────────────────────────────────────────────────────────

pub struct FaqStore {
    pub db: Db,
}

impl FaqStore {
    pub async fn ensure_schema(&self) -> Result<(), dl_db::DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS faq_chat_sessions(
                        session_id TEXT PRIMARY KEY,
                        user_id INTEGER NOT NULL,
                        user_name TEXT,
                        channel_id INTEGER NOT NULL,
                        guild_id INTEGER NOT NULL,
                        created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                        expires_at DATETIME NOT NULL,
                        status TEXT NOT NULL DEFAULT 'active',
                        last_activity_at DATETIME DEFAULT CURRENT_TIMESTAMP
                    );
                    CREATE TABLE IF NOT EXISTS faq_chat_messages(
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        session_id TEXT NOT NULL,
                        role TEXT NOT NULL,
                        content TEXT NOT NULL,
                        created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                        FOREIGN KEY(session_id) REFERENCES faq_chat_sessions(session_id)
                    );
                    CREATE INDEX IF NOT EXISTS idx_faq_messages_session
                        ON faq_chat_messages(session_id, created_at);
                    CREATE INDEX IF NOT EXISTS idx_faq_sessions_expires
                        ON faq_chat_sessions(expires_at, status);",
                )
            })
            .await
    }

    pub async fn active_session_of_user(&self, user_id: u64) -> Option<(String, u64)> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT session_id, channel_id FROM faq_chat_sessions
                      WHERE user_id = ?1 AND status = 'active'",
                    [user_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    pub async fn active_session_in_channel(&self, channel_id: u64) -> Option<(String, u64)> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT session_id, user_id FROM faq_chat_sessions
                      WHERE channel_id = ?1 AND status = 'active'",
                    [channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    pub async fn create_session(
        &self,
        session_id: String,
        user_id: u64,
        user_name: String,
        channel_id: u64,
        guild_id: u64,
    ) {
        let expires = (chrono::Utc::now() + chrono::Duration::hours(SESSION_TIMEOUT_HOURS))
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO faq_chat_sessions(session_id, user_id, user_name, channel_id, guild_id, expires_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![session_id, user_id, user_name, channel_id, guild_id, expires],
                )
                .map(|_| ())
            })
            .await;
    }

    pub async fn add_message(&self, session_id: &str, role: &str, content: &str) {
        let (session_id, role, content) = (
            session_id.to_string(),
            role.to_string(),
            content.to_string(),
        );
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO faq_chat_messages(session_id, role, content) VALUES (?1, ?2, ?3)",
                    rusqlite::params![session_id, role, content],
                )?;
                conn.execute(
                    "UPDATE faq_chat_sessions SET last_activity_at = CURRENT_TIMESTAMP
                      WHERE session_id = ?1",
                    [session_id],
                )
                .map(|_| ())
            })
            .await;
    }

    /// Letzte 10 Nachrichten (chronologisch) für das Gesprächs-Gedächtnis.
    pub async fn recent_messages(&self, session_id: &str) -> Vec<(String, String)> {
        let session_id = session_id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT role, content FROM (
                        SELECT role, content, id FROM faq_chat_messages
                         WHERE session_id = ?1 ORDER BY id DESC LIMIT 10
                     ) ORDER BY id ASC",
                )?;
                let rows = stmt.query_map([session_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }

    pub async fn close_session(&self, session_id: &str) {
        let session_id = session_id.to_string();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE faq_chat_sessions SET status = 'closed' WHERE session_id = ?1",
                    [session_id],
                )
                .map(|_| ())
            })
            .await;
    }

    /// (session_id, channel_id) aller abgelaufenen aktiven Sessions.
    pub async fn expired_sessions(&self) -> Vec<(String, u64)> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT session_id, channel_id FROM faq_chat_sessions
                      WHERE status = 'active' AND expires_at <= datetime('now')",
                )?;
                let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
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
}

pub struct FaqChat {
    pub store: FaqStore,
    pub port: Arc<dyn FaqPort>,
    pub ai: Option<Arc<dyn TextGenerator>>,
    pub docs: String,
    answered_tickets: tokio::sync::Mutex<HashSet<u64>>,
}

impl FaqChat {
    pub fn new(
        db: Db,
        port: Arc<dyn FaqPort>,
        ai: Option<Arc<dyn TextGenerator>>,
        docs: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: FaqStore { db },
            port,
            ai,
            docs,
            answered_tickets: tokio::sync::Mutex::new(HashSet::new()),
        })
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
        content: &str,
    ) {
        {
            let mut answered = self.answered_tickets.lock().await;
            if answered.contains(&channel_id) {
                return;
            }
            answered.insert(channel_id);
        }
        if self.port.channel_category(guild_id, channel_id).await
            != Some(TICKET_AUTO_HELP_CATEGORY_ID)
        {
            return;
        }
        let problem = content.trim();
        if problem.is_empty() {
            return;
        }
        let Some(ai) = &self.ai else { return };
        let answer = ai
            .generate_text(GenerateRequest {
                prompt: format!("Dokumentation:\n{}\n\nTicket-Inhalt:\n{problem}", self.docs),
                system_prompt: Some(TICKET_AUTO_HELP_SYSTEM_PROMPT.to_string()),
                model: None,
                max_output_tokens: Some(MAX_OUTPUT_TOKENS),
                temperature: 0.2,
            })
            .await;
        let Some(answer) = answer else { return };
        let trimmed = answer.trim();
        if trimmed.is_empty() || trimmed.contains("KEIN_TREFFER") {
            return; // kein klarer Treffer → schweigen wie das Original
        }
        self.port.send_message(channel_id, trimmed, None).await;
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
        if interaction.custom_id == "faq_chat:start" {
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

    #[tokio::test]
    async fn session_lifecycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let store = FaqStore { db };
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
}
