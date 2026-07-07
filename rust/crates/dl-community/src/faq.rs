//! FAQ-Chat — Port von `cogs/faq_chat.py`.
//!
//! Panel-Button (`faq_chat:start`) → privater Text-Kanal in der
//! FAQ-Kategorie → Fragen werden über den dl-knowledge-Dienst beantwortet,
//! mit Gesprächs-Gedächtnis (letzte Nutzerfragen der Session).
//! Nachrichten). Sessions schließen nach 24 h automatisch; der
//! Close-Button (`faq_chat:close:{session}`) beendet sofort.
//! Dazu der Ticket-Auto-Helfer: die erste Nachricht in einem neuen
//! Ticket-Kanal wird gegen dl-knowledge geprüft — kann der Bot klar helfen,
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

use dl_central_db::kv;
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;

use crate::db::{i64_to_u64, u64_to_i64};

pub const PANEL_CHANNEL_ID: u64 = 1491953161747955853;
pub const FAQ_CATEGORY_ID: u64 = 1310153243795390475;
pub const TICKET_AUTO_HELP_CATEGORY_ID: u64 = 1459628097145147645;
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const SESSION_TIMEOUT_HOURS: i64 = 24;
pub const PANEL_KV_NS: &str = "faq_chat:panel";
pub const DEFAULT_KNOWLEDGE_URL: &str = "http://127.0.0.1:8896";
const KNOWLEDGE_TIMEOUT: Duration = Duration::from_secs(20);
const FAQ_NO_ANSWER: &str = "Da müssen wir passen, das haben wir gerade selbst nicht parat. Stell die Frage gern nochmal anders, oder mach ein Ticket auf, dann schaut sich das jemand von uns persönlich an.";
const TICKET_SHADOW_PREFIX: &str = "🧪 **FAQ-Shadow**: so hätte der Bot im Ticket geantwortet:";
/// KV-Schlüssel der gemerkten Panel-Message-ID — MUSS exakt Pythons
/// `_store_panel_msg_id`/`_get_stored_panel_msg_id` entsprechen (`panel_msg_id`),
/// damit Rust das bestehende Panel übernimmt statt ein Duplikat zu posten.
pub const PANEL_KV_KEY: &str = "panel_msg_id";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KnowledgeAnswer {
    pub answerable: bool,
    pub answer: Option<String>,
    #[serde(default)]
    pub sources: Vec<KnowledgeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KnowledgeSource {
    pub title: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
struct KnowledgeQuestion<'a> {
    question: &'a str,
}

fn knowledge_url_from_env() -> String {
    std::env::var("DL_KNOWLEDGE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_KNOWLEDGE_URL.to_string())
}

pub async fn ask_knowledge(question: &str) -> Option<KnowledgeAnswer> {
    ask_knowledge_with_timeout(&knowledge_url_from_env(), question, KNOWLEDGE_TIMEOUT).await
}

async fn ask_knowledge_at(base_url: &str, question: &str) -> Option<KnowledgeAnswer> {
    ask_knowledge_with_timeout(base_url, question, KNOWLEDGE_TIMEOUT).await
}

async fn ask_knowledge_with_timeout(
    base_url: &str,
    question: &str,
    timeout: Duration,
) -> Option<KnowledgeAnswer> {
    let client = reqwest::Client::builder().timeout(timeout).build().ok()?;
    let url = format!("{}/public/v1/ask", base_url.trim_end_matches('/'));
    client
        .post(url)
        .json(&KnowledgeQuestion { question })
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<KnowledgeAnswer>()
        .await
        .ok()
}

fn answer_text(answer: Option<KnowledgeAnswer>) -> Option<String> {
    let answer = answer?;
    if !answer.answerable {
        return None;
    }
    answer
        .answer
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TicketAutoOutcome {
    answer: Option<String>,
    decision: &'static str,
}

impl TicketAutoOutcome {
    fn silence(decision: &'static str) -> Self {
        Self {
            answer: None,
            decision,
        }
    }
}

fn knowledge_question_from_history(history: &[(String, String)], question: &str) -> String {
    let mut questions: Vec<String> = history
        .iter()
        .filter(|(role, _)| role == "user")
        .map(|(_, content)| content.trim())
        .filter(|content| !content.is_empty())
        .map(str::to_string)
        .collect();
    let question = question.trim();
    if !question.is_empty() && questions.last().map(String::as_str) != Some(question) {
        questions.push(question.to_string());
    }
    questions.join("\n")
}

fn ticket_auto_outcome_from_knowledge(answer: Option<KnowledgeAnswer>) -> TicketAutoOutcome {
    match answer_text(answer) {
        Some(answer) => TicketAutoOutcome {
            answer: Some(answer),
            decision: "answered",
        },
        None => TicketAutoOutcome::silence("kein_treffer"),
    }
}

fn shadow_channel_from_env() -> Option<u64> {
    std::env::var("DL_FAQ_SHADOW_CHANNEL_ID")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn shadow_ticket_message(ticket_channel_id: u64, answer: &str) -> String {
    format!("{TICKET_SHADOW_PREFIX}\nTicket: <#{ticket_channel_id}>\n\n{answer}")
}

fn ticket_answer_target(ticket_channel_id: u64, shadow_channel_id: Option<u64>) -> u64 {
    shadow_channel_id.unwrap_or(ticket_channel_id)
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
    pub knowledge_url: String,
    pub shadow_channel_id: Option<u64>,
    answered_tickets: tokio::sync::Mutex<HashSet<u64>>,
}

impl FaqChat {
    pub fn new(pool: PgPool, port: Arc<dyn FaqPort>) -> Arc<Self> {
        Self::new_with_config(
            pool,
            port,
            knowledge_url_from_env(),
            shadow_channel_from_env(),
        )
    }

    fn new_with_config(
        pool: PgPool,
        port: Arc<dyn FaqPort>,
        knowledge_url: String,
        shadow_channel_id: Option<u64>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: FaqStore { pool },
            port,
            knowledge_url,
            shadow_channel_id,
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
        let history = self.store.recent_messages(session_id).await;
        let question = knowledge_question_from_history(&history, question);
        answer_text(ask_knowledge_at(&self.knowledge_url, &question).await)
            .unwrap_or_else(|| FAQ_NO_ANSWER.to_string())
    }

    async fn ticket_auto_answer(&self, problem: &str, _author_id: u64) -> TicketAutoOutcome {
        ticket_auto_outcome_from_knowledge(ask_knowledge_at(&self.knowledge_url, problem).await)
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
        if guild_id == 0 {
            return;
        }
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
            "FAQ-Ticket-Auto-Hilfe entschieden"
        );
        if let Some(answer) = outcome.answer {
            let target = ticket_answer_target(channel_id, self.shadow_channel_id);
            let content = if self.shadow_channel_id.is_some() {
                shadow_ticket_message(channel_id, &answer)
            } else {
                answer
            };
            self.port.send_message(target, &content, None).await;
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn knowledge_frage_nutzt_nur_nutzerfragen_als_kontext() {
        let history = vec![
            ("user".to_string(), " Erste Frage ".to_string()),
            ("assistant".to_string(), "Antwort 1".to_string()),
            ("user".to_string(), "Zweite Frage".to_string()),
        ];
        assert_eq!(
            knowledge_question_from_history(&history, " Neue Frage "),
            "Erste Frage\nZweite Frage\nNeue Frage"
        );
        assert_eq!(
            knowledge_question_from_history(&history, "Zweite Frage"),
            "Erste Frage\nZweite Frage"
        );
    }

    #[test]
    fn ticket_auto_help_entscheidet_answerable_true_false_none() {
        let answered = ticket_auto_outcome_from_knowledge(Some(KnowledgeAnswer {
            answerable: true,
            answer: Some("  Antwort aus Knowledge  ".to_string()),
            sources: Vec::new(),
        }));
        assert_eq!(answered.decision, "answered");
        assert_eq!(answered.answer.as_deref(), Some("Antwort aus Knowledge"));

        let unanswerable = ticket_auto_outcome_from_knowledge(Some(KnowledgeAnswer {
            answerable: false,
            answer: None,
            sources: Vec::new(),
        }));
        assert_eq!(unanswerable.decision, "kein_treffer");
        assert_eq!(unanswerable.answer, None);

        let none = ticket_auto_outcome_from_knowledge(None);
        assert_eq!(none.decision, "kein_treffer");
        assert_eq!(none.answer, None);
    }

    #[test]
    fn shadow_mode_routing_entscheidung() {
        assert_eq!(ticket_answer_target(10, None), 10);
        assert_eq!(ticket_answer_target(10, Some(99)), 99);
        let message = shadow_ticket_message(10, "Antwort");
        assert!(message.contains(TICKET_SHADOW_PREFIX));
        assert!(message.contains("<#10>"));
        assert!(message.contains("Antwort"));
    }

    async fn knowledge_server(
        status: u16,
        body: &'static str,
        delay: Duration,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await;
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let status_line = match status {
                200 => "200 OK",
                500 => "500 Internal Server Error",
                _ => "400 Bad Request",
            };
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn ask_knowledge_liefert_answerable_true() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"Ja.","sources":[{"title":"T","path":"p.md"}]}"#,
            Duration::ZERO,
        )
        .await;

        let answer = ask_knowledge_with_timeout(&url, "Frage?", Duration::from_secs(1))
            .await
            .expect("knowledge answer");
        let _ = handle.await;

        assert!(answer.answerable);
        assert_eq!(answer.answer.as_deref(), Some("Ja."));
        assert_eq!(answer.sources[0].title, "T");
    }

    #[tokio::test]
    async fn ask_knowledge_liefert_answerable_false() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":false,"answer":null,"sources":[]}"#,
            Duration::ZERO,
        )
        .await;

        let answer = ask_knowledge_with_timeout(&url, "Frage?", Duration::from_secs(1))
            .await
            .expect("knowledge answer");
        let _ = handle.await;

        assert!(!answer.answerable);
        assert_eq!(answer.answer, None);
    }

    #[tokio::test]
    async fn ask_knowledge_timeout_ist_none() {
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"zu spaet","sources":[]}"#,
            Duration::from_millis(200),
        )
        .await;

        let answer = ask_knowledge_with_timeout(&url, "Frage?", Duration::from_millis(30)).await;
        handle.abort();

        assert_eq!(answer, None);
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
        sent: std::sync::Mutex<Vec<(u64, String)>>,
        category: Option<u64>,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl FaqPort for MockPanelPort {
        async fn create_faq_channel(&self, _g: u64, _u: u64, _n: &str) -> Result<u64, String> {
            Ok(1)
        }
        async fn send_message(
            &self,
            channel_id: u64,
            text: &str,
            _comp: Option<serde_json::Value>,
        ) {
            self.sent
                .lock()
                .unwrap()
                .push((channel_id, text.to_string()));
        }
        async fn channel_category(&self, _g: u64, _c: u64) -> Option<u64> {
            self.category
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
            sent: std::sync::Mutex::new(Vec::new()),
            category: None,
        })
    }

    #[cfg(feature = "testing")]
    fn ticket_port() -> Arc<MockPanelPort> {
        Arc::new(MockPanelPort {
            posts: std::sync::Mutex::new(0),
            edits: std::sync::Mutex::new(0),
            deleted: std::sync::Mutex::new(Vec::new()),
            sent: std::sync::Mutex::new(Vec::new()),
            category: Some(TICKET_AUTO_HELP_CATEGORY_ID),
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
        let faq = FaqChat::new(db.pool().clone(), port.clone());

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
        let faq = FaqChat::new(db.pool().clone(), port.clone());

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
        let faq = FaqChat::new(db.pool().clone(), port.clone());

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
    #[tokio::test]
    async fn ticket_auto_help_postet_answerable_true() {
        let db = db_with_kv().await;
        let port = ticket_port();
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"Ticket-Antwort","sources":[]}"#,
            Duration::ZERO,
        )
        .await;
        let faq = FaqChat::new_with_config(db.pool().clone(), port.clone(), url, None);

        faq.handle_ticket_message(1, 222, 111111111111111111, "Steam geht nicht")
            .await;
        let _ = handle.await;

        assert_eq!(
            *port.sent.lock().unwrap(),
            vec![(222, "Ticket-Antwort".to_string())]
        );
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn ticket_auto_help_shadow_postet_nicht_ins_ticket() {
        let db = db_with_kv().await;
        let port = ticket_port();
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"Ticket-Antwort","sources":[]}"#,
            Duration::ZERO,
        )
        .await;
        let faq = FaqChat::new_with_config(db.pool().clone(), port.clone(), url, Some(999));

        faq.handle_ticket_message(1, 222, 111111111111111111, "Steam geht nicht")
            .await;
        let _ = handle.await;

        let sent = port.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, 999);
        assert!(sent[0].1.contains("<#222>"));
        assert!(sent[0].1.contains("Ticket-Antwort"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn ticket_auto_help_ignoriert_events_ohne_guild() {
        let db = db_with_kv().await;
        let port = ticket_port();
        let (url, handle) = knowledge_server(
            200,
            r#"{"answerable":true,"answer":"Ticket-Antwort","sources":[]}"#,
            Duration::ZERO,
        )
        .await;
        let faq = FaqChat::new_with_config(db.pool().clone(), port.clone(), url, None);

        faq.handle_ticket_message(0, 222, 111111111111111111, "Steam geht nicht")
            .await;
        handle.abort();

        assert!(port.sent.lock().unwrap().is_empty());
    }
}
