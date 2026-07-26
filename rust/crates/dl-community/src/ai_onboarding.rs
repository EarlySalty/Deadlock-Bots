//! AI onboarding flow for the legacy `aiob:*` Discord components.
//!
//! This module intentionally complements the static onboarding wizard. It only
//! owns the AI tour panel, modal submit, quick-action buttons, KV persistence,
//! and the `aiob:rules_confirm` role grant.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use dl_ai::{GenerateRequest, TextGenerator};
use dl_central_db::kv;
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter, ResponseMessageHook,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::PgPool;

pub const CUSTOM_ID_START: &str = "aiob:start";
pub const CUSTOM_ID_RULES_CONFIRM: &str = "aiob:rules_confirm";
const CUSTOM_ID_QUESTIONS_PREFIX: &str = "aiob:questions:";

pub const NS_PERSIST_VIEWS: &str = "ai_onboarding:persistent_views";
pub const NS_SESSION_LOG: &str = "ai_onboarding:sessions";

const FIELD_INTERESTS: &str = "interests";
const FIELD_EXPECTATIONS: &str = "expectations";
const FIELD_STYLE: &str = "style";

pub const AI_ONBOARDING_DEFAULT_MAX_OUTPUT_TOKENS: u32 = 700;
const TEMPERATURE: f64 = 0.45;

const LFG_LEGACY_CHANNEL_ID: u64 = 1376335502919335936;
const LFG_FORUM_CHANNEL_ID_ENV: &str = "DL_LFG_FORUM_CHANNEL_ID";
const LFG_FORUM_CUTOVER_ENV: &str = "DL_LFG_FORUM_CUTOVER";
const TEMPVOICE_PANEL_CHANNEL_ID: u64 = 1371927143537315890;
const FEEDBACK_CHANNEL_ID: u64 = 1289721245281292291;
const RULES_CHANNEL_ID: u64 = 1315684135175716975;

const ROLE_IGNORE_ID: u64 = 1304216250649415771;
pub const ROLE_STREAMER_ONBOARD_ID: u64 = 1468365558293598268;
pub const ROLE_STREAMER_PARTNER_ID: u64 = 1411798947936342097;
pub const ROLE_LFG_PING_ID: u64 = 1407086020331311144;
pub const ROLE_CUSTOM_GAMES_PING_ID: u64 = 1407085699374649364;
pub const ROLE_PATCHNOTES_PING_ID: u64 = 1330994309524357140;
pub const ROLE_RANKED_ID: u64 = 1420466763262591120;
pub const ROLE_CASUAL_ID: u64 = 1420466468746690621;

const ROLE_LABELS: &[(u64, &str)] = &[
    (ROLE_STREAMER_ONBOARD_ID, "Streamer Onboarding Rolle"),
    (ROLE_STREAMER_PARTNER_ID, "Streamer Partner Rolle"),
    (ROLE_LFG_PING_ID, "Spieler-Suche Ping Rolle"),
    (ROLE_CUSTOM_GAMES_PING_ID, "Custom Games Ping Rolle"),
    (ROLE_PATCHNOTES_PING_ID, "Patchnotes Ping Rolle"),
    (ROLE_RANKED_ID, "Ranked/Rang-Spieler Rolle"),
    (ROLE_CASUAL_ID, "Casual/Spaß-Spieler Rolle"),
];

const STREAMING_KEYWORDS: [&str; 9] = [
    "stream",
    "streamer",
    "twitch",
    "kick",
    "youtube",
    "yt",
    "livestream",
    "live gehen",
    "obs",
];

pub const AI_ONBOARDING_START_BUTTON_LABEL: &str = "Persönliche Tour starten 🚀";
pub const AI_ONBOARDING_RULES_CONFIRM_BUTTON_LABEL: &str = "Regeln gelesen ✅";
pub const AI_ONBOARDING_QUICK_LFG_BUTTON_LABEL: &str = "Spieler-Suche";
pub const AI_ONBOARDING_QUICK_TEMPVOICE_BUTTON_LABEL: &str = "Voice-Lanes";
pub const AI_ONBOARDING_QUICK_FEEDBACK_BUTTON_LABEL: &str = "Feedback";
pub const AI_ONBOARDING_QUICK_RULES_BUTTON_LABEL: &str = "Regelwerk";
pub const AI_ONBOARDING_MODAL_TITLE: &str = "Kurz zu dir";
pub const AI_ONBOARDING_MODAL_INTERESTS_LABEL: &str = "Was reizt dich an Deadlock?";
pub const AI_ONBOARDING_MODAL_INTERESTS_PLACEHOLDER: &str =
    "z. B. Ranked, Custom Games, Community, Builds";
pub const AI_ONBOARDING_MODAL_EXPECTATIONS_LABEL: &str = "Was erhoffst du dir vom Server?";
pub const AI_ONBOARDING_MODAL_EXPECTATIONS_PLACEHOLDER: &str = "z. B. Mitspieler, Tipps, Turniere";
pub const AI_ONBOARDING_MODAL_STYLE_LABEL: &str = "Wie spielst du am liebsten?";
pub const AI_ONBOARDING_MODAL_STYLE_PLACEHOLDER: &str = "z. B. entspannt, kompetitiv, Team-Play";
pub const AI_ONBOARDING_WRONG_USER_MESSAGE: &str =
    "Dieses Onboarding gehört jemand anderem – bitte nutze deinen eigenen Button.";
pub const AI_ONBOARDING_RULES_CONFIRM_SUCCESS_MESSAGE: &str = "Danke! Viel Spaß auf dem Server. 😊";
pub const AI_ONBOARDING_RULES_CONFIRM_MEMBER_ERROR_MESSAGE: &str =
    "Ich konnte dich gerade nicht als Server-Mitglied zuordnen. Probier es kurz später erneut.";
pub const AI_ONBOARDING_RULES_CONFIRM_ROLE_ERROR_MESSAGE: &str =
    "Ich konnte die Onboarding-Rolle nicht setzen. Bitte gib kurz dem Team Bescheid.";
pub const AI_ONBOARDING_PANEL_TITLE: &str = "Willkommen in der Deadlock Community! 👋";
pub const AI_ONBOARDING_PANEL_DESCRIPTION: &str =
    "Schön, dass du da bist! Starte deine persönliche Tour: beantworte 3 kurze Fragen und ich zeige dir, wo du am besten loslegst. Wenn du die Regeln gelesen hast, bestätige sie unten.";
pub const AI_ONBOARDING_PERSONALIZED_EMBED_TITLE: &str = "Speziell für dich 🎯";
pub const AI_ONBOARDING_PERSONALIZED_EMBED_FOOTER: &str =
    "Automatisch erstellt – passt nicht alles? Frag einfach im Chat.";
pub const AI_ONBOARDING_FALLBACK_RESPONSE: &str =
    "Hey, willkommen auf dem Server! 🎉 Schau am besten zuerst ins Regelwerk. Über die Buttons oben findest du schnell Mitspieler, deine Voice-Lanes und die Patchnotes. Viel Spaß!";
pub const AI_ONBOARDING_SYSTEM_PROMPT: &str = "Du bist der herzliche Onboarding-Guide der Deutschen Deadlock Community. Antworte immer auf Deutsch.\n\nZiele:\n- Begrüße den User warm und freundlich (max. 2 Sätze).\n- Spiegle grob den Stil des Users (locker/kurz, gern ein paar Emojis), bleib aber sauber.\n- Schreib kurze Absätze, keine Kanal-Listen als Fließtext.\n- Nutze höchstens 4 relevante Kanäle insgesamt.\n- Pro Bullet/Schritt maximal 1 Kanal.\n- Sei kompakt: 6–9 Sätze gesamt, kein Roman.\n- Nutze nur den gegebenen Kontext; was du nicht weißt, lässt du weg.";
pub const AI_ONBOARDING_USER_PROMPT_TEMPLATE: &str = "Erstelle eine persönliche Willkommens-Nachricht für ein neues Mitglied. Nutze nur den folgenden Kontext.\n\nForm:\n- Drei Blöcke:\n  1) Begrüßung (1–2 Sätze)\n  2) Kurz für dich: 3–4 Bullets (je Bullet max. 1 Kanal)\n  3) Nächste Schritte: 2–3 nummerierte Punkte\n- Keine doppelten Einleitungen, kein Fließtext mit vielen Kanälen.\n- Maximal 4 Kanäle insgesamt, nur aus dem Kontext.";
pub const AI_ONBOARDING_ANSWERS_PROMPT_TEMPLATE: &str = "Antworten des Users:";
pub const AI_ONBOARDING_SERVER_CONTEXT: &str = "Server: Deutsche Deadlock Community (Discord).\nWichtige Bereiche:\n- #📝patchnotes: Patchnotes auf Deutsch.\n- #📢ankündigungen: Updates & News.\n- #💬build-discussion: Fragen zu Builds und Helden.\n- #🎮spieler-suche: Mitspieler finden (Spieler-Suche Ping).\n- #🧩custom-games-chat und #📍Sammelpunkt: Custom Games organisieren.\n- #🚧sprach-kanal-verwalten: eigene Voice-Lanes erstellen und verwalten.\n- #🏆rang-auswahl: Rang setzen für Ranked-Lanes.";
pub const AI_ONBOARDING_ROLE_CONTEXT_HEADER: &str = "Rollen-Kontext (Discord Onboarding):";
pub const AI_ONBOARDING_ROLE_CONTEXT_EMPTY: &str = "- (keine relevanten Rollen erkannt)";
pub const AI_ONBOARDING_ROLE_CONTEXT_HINTS_HEADER: &str = "Hinweise:";
pub const AI_ONBOARDING_ROLE_CONTEXT_STREAMING_HINT: &str =
    "- Streaming erkannt: Streamer-Partner explizit vorschlagen und /streamer nennen.";
pub const AI_ONBOARDING_ROLE_CONTEXT_LFG_PING_HINT: &str =
    "- Spieler-Suche Ping: #🎮spieler-suche erwähnen.";
pub const AI_ONBOARDING_ROLE_CONTEXT_CUSTOM_GAMES_HINT: &str =
    "- Custom Games Ping: #🧩custom-games-chat und #📍Sammelpunkt erwähnen.";
pub const AI_ONBOARDING_ROLE_CONTEXT_PATCHNOTES_HINT: &str =
    "- Patchnotes Ping: #📝patchnotes erwähnen.";
pub const AI_ONBOARDING_ROLE_CONTEXT_RANKED_HINT: &str =
    "- Ranked/Rang: #🏆rang-auswahl und Ranked/Competitiv Lane erwähnen.";
pub const AI_ONBOARDING_ROLE_CONTEXT_CASUAL_HINT: &str = "- Casual/Spaß: Spaß Lane erwähnen.";

#[derive(Debug, thiserror::Error)]
pub enum AiOnboardingError {
    #[error("Discord port error: {0}")]
    Discord(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Central(#[from] dl_central_db::CentralDbError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct AiOnboardingConfig {
    pub guild_id: u64,
    pub onboard_complete_role_id: u64,
    pub max_output_tokens: u32,
    pub ignored_role_ids: Vec<u64>,
}

impl AiOnboardingConfig {
    pub fn new(guild_id: u64, onboard_complete_role_id: u64) -> Self {
        Self {
            guild_id,
            onboard_complete_role_id,
            max_output_tokens: AI_ONBOARDING_DEFAULT_MAX_OUTPUT_TOKENS,
            ignored_role_ids: vec![onboard_complete_role_id],
        }
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAnswers {
    pub interests: String,
    pub expectations: String,
    pub style: String,
}

impl UserAnswers {
    pub fn as_prompt_block(&self) -> String {
        format!(
            "{}\nanswers.interests={}\nanswers.expectations={}\nanswers.style={}",
            AI_ONBOARDING_ANSWERS_PROMPT_TEMPLATE,
            sanitize_prompt_fragment(&self.interests),
            sanitize_prompt_fragment(&self.expectations),
            sanitize_prompt_fragment(&self.style)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptUser {
    pub user_id: u64,
    pub display_name: String,
    pub guild_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRole {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedView {
    user_id: Option<u64>,
    thread_id: Option<u64>,
}

struct PersistResponseView {
    pool: PgPool,
    user_id: Option<u64>,
    thread_id: Option<u64>,
}

#[async_trait]
impl ResponseMessageHook for PersistResponseView {
    async fn on_response_message(&self, message_id: u64) {
        let payload = PersistedView {
            user_id: self.user_id,
            thread_id: self.thread_id,
        };
        let raw = match serde_json::to_string(&payload) {
            Ok(raw) => raw,
            Err(err) => {
                tracing::debug!(%err, message_id, "AI onboarding response view serialize failed");
                return;
            }
        };
        if let Err(err) = kv::set(&self.pool, NS_PERSIST_VIEWS, &message_id.to_string(), &raw).await
        {
            tracing::debug!(%err, message_id, "AI onboarding response view persist failed");
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub scanned: usize,
    pub restored: usize,
    pub removed_invalid: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalizedText {
    pub text: String,
    pub llm_meta: Value,
}

#[async_trait]
pub trait AiOnboardingPort: Send + Sync {
    async fn post_message(&self, channel_id: u64, body: Map<String, Value>) -> Result<u64, String>;

    async fn add_member_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String>;

    async fn member_roles(&self, guild_id: u64, user_id: u64) -> Vec<MemberRole>;
}

pub struct AiOnboarding {
    pool: PgPool,
    port: Arc<dyn AiOnboardingPort>,
    ai: Option<Arc<dyn TextGenerator>>,
    config: AiOnboardingConfig,
}

impl AiOnboarding {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn AiOnboardingPort>,
        ai: Option<Arc<dyn TextGenerator>>,
        config: AiOnboardingConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            ai,
            config,
        })
    }

    /// Posts the AI onboarding start panel and persists the `aiob:start` view.
    ///
    /// Cutover scope for this Rust module is serving existing persistent
    /// `aiob:*` buttons plus restore. The production post trigger still lives in
    /// `ai_connector` and will be wired from a separate port.
    pub async fn start_in_channel(
        &self,
        channel_id: u64,
        member_id: u64,
    ) -> Result<u64, AiOnboardingError> {
        let message_id = self
            .port
            .post_message(channel_id, panel_body())
            .await
            .map_err(AiOnboardingError::Discord)?;
        self.persist_view(message_id, Some(member_id), Some(channel_id))
            .await?;
        Ok(message_id)
    }

    pub async fn restore_persistent_views(&self) -> Result<RestoreReport, AiOnboardingError> {
        let rows = sqlx::query!(
            r#"
            SELECT k, v
              FROM bot.kv_store
             WHERE ns = $1
             ORDER BY k
            "#,
            NS_PERSIST_VIEWS,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| (row.k, row.v))
        .collect::<Vec<_>>();

        let mut report = RestoreReport {
            scanned: rows.len(),
            ..RestoreReport::default()
        };

        for (key, value) in rows {
            let valid_message_id = key.parse::<u64>().ok().filter(|id| *id > 0).is_some();
            let valid_payload = serde_json::from_str::<PersistedView>(&value).is_ok();
            if valid_message_id && valid_payload {
                report.restored += 1;
            } else {
                kv::delete(&self.pool, NS_PERSIST_VIEWS, &key).await?;
                report.removed_invalid += 1;
            }
        }
        Ok(report)
    }

    async fn persisted_view(
        &self,
        message_id: Option<u64>,
    ) -> Result<Option<PersistedView>, AiOnboardingError> {
        let Some(message_id) = message_id else {
            return Ok(None);
        };
        let Some(raw) = kv::get(&self.pool, NS_PERSIST_VIEWS, &message_id.to_string()).await?
        else {
            return Ok(None);
        };
        match serde_json::from_str::<PersistedView>(&raw) {
            Ok(view) => Ok(Some(view)),
            Err(err) => {
                tracing::debug!(%err, message_id, "invalid AI onboarding persisted view");
                let _ = kv::delete(&self.pool, NS_PERSIST_VIEWS, &message_id.to_string()).await;
                Ok(None)
            }
        }
    }

    async fn persist_view(
        &self,
        message_id: u64,
        user_id: Option<u64>,
        thread_id: Option<u64>,
    ) -> Result<(), AiOnboardingError> {
        let payload = PersistedView { user_id, thread_id };
        kv::set(
            &self.pool,
            NS_PERSIST_VIEWS,
            &message_id.to_string(),
            &serde_json::to_string(&payload)?,
        )
        .await?;
        Ok(())
    }

    async fn clear_persisted_view(&self, message_id: Option<u64>) {
        if let Some(message_id) = message_id {
            if let Err(err) =
                kv::delete(&self.pool, NS_PERSIST_VIEWS, &message_id.to_string()).await
            {
                tracing::debug!(%err, message_id, "AI onboarding persisted view delete failed");
            }
        }
    }

    pub async fn generate_personalized_text(
        &self,
        answers: &UserAnswers,
        user: PromptUser,
    ) -> PersonalizedText {
        let guild_id = if user.guild_id > 0 {
            user.guild_id
        } else {
            self.config.guild_id
        };
        let roles = self.port.member_roles(guild_id, user.user_id).await;
        let role_context = build_role_context_block(&roles, answers, &self.config.ignored_role_ids);
        let prompt = assemble_prompt(&user.display_name, &role_context, answers);

        if let Some(ai) = &self.ai {
            let request = GenerateRequest {
                prompt,
                system_prompt: Some(AI_ONBOARDING_SYSTEM_PROMPT.to_string()),
                model: None,
                max_output_tokens: Some(self.config.max_output_tokens),
                reasoning_effort: None,
                temperature: TEMPERATURE,
            };
            if let Some(text) = ai.generate_text(request).await {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return PersonalizedText {
                        text: trimmed.to_string(),
                        llm_meta: json!({
                            "provider": "minimax",
                            "ok": true,
                        }),
                    };
                }
            }
        }

        PersonalizedText {
            text: AI_ONBOARDING_FALLBACK_RESPONSE.to_string(),
            llm_meta: json!({
                "provider": "fallback",
                "error": "no_ai_available",
            }),
        }
    }

    async fn log_session(
        &self,
        user_id: u64,
        thread_id: Option<u64>,
        answers: &UserAnswers,
        llm_meta: Value,
    ) {
        let Ok(user_id_i64) = crate::db::u64_to_i64(user_id, "user_id") else {
            return;
        };
        if crate::privacy::is_opted_out(&self.pool, user_id_i64).await {
            return;
        }
        let payload = json!({
            "user_id": user_id,
            "thread_id": thread_id,
            "answers": answers,
            "llm": llm_meta,
        });
        if let Err(err) = kv::set(
            &self.pool,
            NS_SESSION_LOG,
            &user_id.to_string(),
            &payload.to_string(),
        )
        .await
        {
            tracing::debug!(%err, user_id, "AI onboarding session log failed");
        }
    }

    async fn handle_start(&self, interaction: BridgeInteraction) -> BridgeReply {
        let persisted = match self.persisted_view(interaction.message_id).await {
            Ok(view) => view,
            Err(err) => {
                tracing::warn!(%err, "AI onboarding persisted view lookup failed");
                None
            }
        };
        if let Some(allowed) = persisted.as_ref().and_then(|view| view.user_id) {
            if allowed != interaction.user_id {
                return BridgeReply::ephemeral_text(AI_ONBOARDING_WRONG_USER_MESSAGE);
            }
        }

        self.clear_persisted_view(interaction.message_id).await;
        let allowed_user_id = persisted.as_ref().and_then(|view| view.user_id);
        let thread_id = persisted
            .as_ref()
            .and_then(|view| view.thread_id)
            .unwrap_or(interaction.channel_id);

        BridgeReply {
            modal: Some(questions_modal(allowed_user_id, Some(thread_id))),
            ..BridgeReply::default()
        }
    }

    async fn handle_rules_confirm(&self, interaction: BridgeInteraction) -> BridgeReply {
        let guild_id = interaction.guild_id;
        if guild_id == 0 || interaction.user_id == 0 || !interaction.member_present {
            return BridgeReply::ephemeral_text(AI_ONBOARDING_RULES_CONFIRM_MEMBER_ERROR_MESSAGE);
        }
        let persisted = match self.persisted_view(interaction.message_id).await {
            Ok(view) => view,
            Err(err) => {
                tracing::warn!(%err, "AI onboarding persisted quick-actions lookup failed");
                None
            }
        };
        if let Some(owner) = persisted.as_ref().and_then(|view| view.user_id) {
            if owner != interaction.user_id {
                return BridgeReply::ephemeral_text(AI_ONBOARDING_WRONG_USER_MESSAGE);
            }
        }

        match self
            .port
            .add_member_role(
                guild_id,
                interaction.user_id,
                self.config.onboard_complete_role_id,
                "AI onboarding: rules confirmed",
            )
            .await
        {
            Ok(()) => BridgeReply::ephemeral_text(AI_ONBOARDING_RULES_CONFIRM_SUCCESS_MESSAGE),
            Err(err) => {
                tracing::warn!(
                    %err,
                    guild_id,
                    user_id = interaction.user_id,
                    role_id = self.config.onboard_complete_role_id,
                    "AI onboarding role grant failed"
                );
                BridgeReply::ephemeral_text(AI_ONBOARDING_RULES_CONFIRM_ROLE_ERROR_MESSAGE)
            }
        }
    }

    async fn handle_questions(&self, interaction: BridgeInteraction) -> BridgeReply {
        let context = parse_modal_context(&interaction.custom_id);
        if let Some(allowed) = context.allowed_user_id {
            if allowed != interaction.user_id {
                return BridgeReply::ephemeral_text(AI_ONBOARDING_WRONG_USER_MESSAGE);
            }
        }

        let answers = parse_answers(&interaction.options);
        let user = PromptUser {
            user_id: interaction.user_id,
            display_name: if interaction.author_name.trim().is_empty() {
                interaction.user_id.to_string()
            } else {
                interaction.author_name.clone()
            },
            guild_id: interaction.guild_id,
        };
        let generated = self.generate_personalized_text(&answers, user).await;
        let thread_id = context.thread_id.or(Some(interaction.channel_id));
        self.log_session(
            interaction.user_id,
            thread_id,
            &answers,
            generated.llm_meta.clone(),
        )
        .await;

        BridgeReply {
            embeds: vec![json!({
                "title": AI_ONBOARDING_PERSONALIZED_EMBED_TITLE,
                "description": generated.text,
                "color": 0x5865F2,
                "footer": { "text": AI_ONBOARDING_PERSONALIZED_EMBED_FOOTER },
            })],
            components: Some(quick_actions_components(self.config.guild_id)),
            response_message_hook: Some(Arc::new(PersistResponseView {
                pool: self.pool.clone(),
                user_id: context.allowed_user_id,
                thread_id,
            })),
            ..BridgeReply::default()
        }
    }
}

struct AiOnboardingHandler {
    service: Arc<AiOnboarding>,
}

#[async_trait]
impl InteractionHandler for AiOnboardingHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        match interaction.custom_id.as_str() {
            CUSTOM_ID_START => self.service.handle_start(interaction).await,
            CUSTOM_ID_RULES_CONFIRM => self.service.handle_rules_confirm(interaction).await,
            custom_id if custom_id.starts_with(CUSTOM_ID_QUESTIONS_PREFIX) => {
                self.service.handle_questions(interaction).await
            }
            _ => BridgeReply::ephemeral_text(AI_ONBOARDING_RULES_CONFIRM_MEMBER_ERROR_MESSAGE),
        }
    }
}

pub fn register(router: &mut InteractionRouter, service: Arc<AiOnboarding>) {
    let handler = Arc::new(AiOnboardingHandler { service });
    router.on_custom_id(CUSTOM_ID_START, handler.clone());
    router.on_custom_id(CUSTOM_ID_RULES_CONFIRM, handler.clone());
    router.on_prefix(CUSTOM_ID_QUESTIONS_PREFIX, handler);
}

pub fn parse_answers(options: &HashMap<String, Value>) -> UserAnswers {
    UserAnswers {
        interests: modal_value(options, FIELD_INTERESTS),
        expectations: modal_value(options, FIELD_EXPECTATIONS),
        style: modal_value(options, FIELD_STYLE),
    }
}

fn modal_value(options: &HashMap<String, Value>, key: &str) -> String {
    options
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn questions_modal(allowed_user_id: Option<u64>, thread_id: Option<u64>) -> ModalSpec {
    ModalSpec {
        custom_id: modal_custom_id(allowed_user_id, thread_id),
        title: AI_ONBOARDING_MODAL_TITLE.to_string(),
        fields: vec![
            ModalField {
                custom_id: FIELD_INTERESTS.to_string(),
                label: AI_ONBOARDING_MODAL_INTERESTS_LABEL.to_string(),
                placeholder: AI_ONBOARDING_MODAL_INTERESTS_PLACEHOLDER.to_string(),
                value: None,
                required: true,
                min_length: 0,
                max_length: 200,
                paragraph: false,
            },
            ModalField {
                custom_id: FIELD_EXPECTATIONS.to_string(),
                label: AI_ONBOARDING_MODAL_EXPECTATIONS_LABEL.to_string(),
                placeholder: AI_ONBOARDING_MODAL_EXPECTATIONS_PLACEHOLDER.to_string(),
                value: None,
                required: true,
                min_length: 0,
                max_length: 300,
                paragraph: true,
            },
            ModalField {
                custom_id: FIELD_STYLE.to_string(),
                label: AI_ONBOARDING_MODAL_STYLE_LABEL.to_string(),
                placeholder: AI_ONBOARDING_MODAL_STYLE_PLACEHOLDER.to_string(),
                value: None,
                required: false,
                min_length: 0,
                max_length: 200,
                paragraph: false,
            },
        ],
    }
}

fn modal_custom_id(allowed_user_id: Option<u64>, thread_id: Option<u64>) -> String {
    format!(
        "{CUSTOM_ID_QUESTIONS_PREFIX}{}:{}",
        allowed_user_id.unwrap_or_default(),
        thread_id.unwrap_or_default()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModalContext {
    allowed_user_id: Option<u64>,
    thread_id: Option<u64>,
}

fn parse_modal_context(custom_id: &str) -> ModalContext {
    let rest = custom_id
        .strip_prefix(CUSTOM_ID_QUESTIONS_PREFIX)
        .unwrap_or_default();
    let mut parts = rest.split(':');
    let allowed_user_id = parts
        .next()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|id| *id > 0);
    let thread_id = parts
        .next()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|id| *id > 0);
    ModalContext {
        allowed_user_id,
        thread_id,
    }
}

fn panel_body() -> Map<String, Value> {
    let mut body = Map::new();
    body.insert(
        "embeds".into(),
        json!([{
            "title": AI_ONBOARDING_PANEL_TITLE,
            "description": AI_ONBOARDING_PANEL_DESCRIPTION,
            "color": 0x5865F2,
        }]),
    );
    body.insert("components".into(), panel_components());
    body
}

fn panel_components() -> Value {
    json!([{ "type": 1, "components": [{
        "type": 2,
        "style": 1,
        "label": AI_ONBOARDING_START_BUTTON_LABEL,
        "custom_id": CUSTOM_ID_START,
    }]}])
}

fn quick_actions_components(guild_id: u64) -> Value {
    let url = |channel_id: u64| format!("https://discord.com/channels/{guild_id}/{channel_id}");
    json!([
        { "type": 1, "components": [
            { "type": 2, "style": 5, "label": AI_ONBOARDING_QUICK_LFG_BUTTON_LABEL, "emoji": { "name": "🎮" }, "url": url(lfg_target_channel_id()) },
            { "type": 2, "style": 5, "label": AI_ONBOARDING_QUICK_TEMPVOICE_BUTTON_LABEL, "emoji": { "name": "🛠️" }, "url": url(TEMPVOICE_PANEL_CHANNEL_ID) },
            { "type": 2, "style": 5, "label": AI_ONBOARDING_QUICK_FEEDBACK_BUTTON_LABEL, "emoji": { "name": "💬" }, "url": url(FEEDBACK_CHANNEL_ID) },
            { "type": 2, "style": 5, "label": AI_ONBOARDING_QUICK_RULES_BUTTON_LABEL, "emoji": { "name": "📜" }, "url": url(RULES_CHANNEL_ID) }
        ]},
        { "type": 1, "components": [{
            "type": 2,
            "style": 3,
            "label": AI_ONBOARDING_RULES_CONFIRM_BUTTON_LABEL,
            "custom_id": CUSTOM_ID_RULES_CONFIRM,
        }]}
    ])
}

fn lfg_target_channel_id() -> u64 {
    lfg_target_channel_id_from_lookup(|key| std::env::var(key).ok())
}

fn lfg_target_channel_id_from_lookup<F>(lookup: F) -> u64
where
    F: Fn(&str) -> Option<String>,
{
    lfg_cutover_target_channel_id_from_lookup(lookup).unwrap_or(LFG_LEGACY_CHANNEL_ID)
}

fn lfg_cutover_target_channel_id_from_lookup<F>(lookup: F) -> Option<u64>
where
    F: Fn(&str) -> Option<String>,
{
    let cutover_enabled = lookup(LFG_FORUM_CUTOVER_ENV)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if !cutover_enabled {
        return None;
    }
    lookup(LFG_FORUM_CHANNEL_ID_ENV)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|id| *id > 0)
}

pub fn build_role_context_block(
    roles: &[MemberRole],
    answers: &UserAnswers,
    ignored_role_ids: &[u64],
) -> String {
    let role_ids = roles
        .iter()
        .map(|role| role.id)
        .filter(|role_id| *role_id != ROLE_IGNORE_ID)
        .filter(|role_id| !ignored_role_ids.contains(role_id))
        .collect::<HashSet<_>>();

    let streamer_onboarding = role_ids.contains(&ROLE_STREAMER_ONBOARD_ID);
    let streamer_partner = role_ids.contains(&ROLE_STREAMER_PARTNER_ID);
    let lfg_ping = role_ids.contains(&ROLE_LFG_PING_ID);
    let custom_games_ping = role_ids.contains(&ROLE_CUSTOM_GAMES_PING_ID);
    let patchnotes_ping = role_ids.contains(&ROLE_PATCHNOTES_PING_ID);
    let ranked_role = role_ids.contains(&ROLE_RANKED_ID);
    let casual_role = role_ids.contains(&ROLE_CASUAL_ID);

    let mut lines = ROLE_LABELS
        .iter()
        .filter(|(role_id, _)| role_ids.contains(role_id))
        .map(|(_, label)| format!("- {label}: ja"))
        .collect::<Vec<_>>();

    if lines.is_empty() {
        lines.push(AI_ONBOARDING_ROLE_CONTEXT_EMPTY.to_string());
    }

    let mut hints = Vec::new();
    if (looks_like_streamer(answers) || streamer_onboarding) && !streamer_partner {
        hints.push(AI_ONBOARDING_ROLE_CONTEXT_STREAMING_HINT.to_string());
    }
    if lfg_ping {
        hints.push(AI_ONBOARDING_ROLE_CONTEXT_LFG_PING_HINT.to_string());
    }
    if custom_games_ping {
        hints.push(AI_ONBOARDING_ROLE_CONTEXT_CUSTOM_GAMES_HINT.to_string());
    }
    if patchnotes_ping {
        hints.push(AI_ONBOARDING_ROLE_CONTEXT_PATCHNOTES_HINT.to_string());
    }
    if ranked_role {
        hints.push(AI_ONBOARDING_ROLE_CONTEXT_RANKED_HINT.to_string());
    }
    if casual_role {
        hints.push(AI_ONBOARDING_ROLE_CONTEXT_CASUAL_HINT.to_string());
    }

    let context = format!(
        "{}\n{}",
        AI_ONBOARDING_ROLE_CONTEXT_HEADER,
        lines.join("\n")
    );
    if hints.is_empty() {
        context
    } else {
        format!(
            "{context}\n\n{}\n{}",
            AI_ONBOARDING_ROLE_CONTEXT_HINTS_HEADER,
            hints.join("\n")
        )
    }
}

fn assemble_prompt(display_name: &str, role_context: &str, answers: &UserAnswers) -> String {
    format!(
        "{}\n{}\nuser.display_name={}\n{}\n{}",
        AI_ONBOARDING_USER_PROMPT_TEMPLATE,
        AI_ONBOARDING_SERVER_CONTEXT,
        sanitize_prompt_fragment(display_name),
        role_context,
        answers.as_prompt_block()
    )
}

fn looks_like_streamer(answers: &UserAnswers) -> bool {
    let text = [
        answers.interests.as_str(),
        answers.expectations.as_str(),
        answers.style.as_str(),
    ]
    .into_iter()
    .map(str::trim)
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();
    !text.is_empty()
        && STREAMING_KEYWORDS
            .iter()
            .any(|keyword| text.contains(keyword))
}

fn sanitize_prompt_fragment(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    #[test]
    fn lfg_quick_action_target_respektiert_effektiven_forum_cutover() {
        assert_eq!(
            lfg_target_channel_id_from_lookup(|_| None),
            LFG_LEGACY_CHANNEL_ID
        );
        assert_eq!(
            lfg_target_channel_id_from_lookup(|key| match key {
                LFG_FORUM_CUTOVER_ENV => Some("0".to_string()),
                LFG_FORUM_CHANNEL_ID_ENV => Some("222".to_string()),
                _ => None,
            }),
            LFG_LEGACY_CHANNEL_ID
        );
        assert_eq!(
            lfg_target_channel_id_from_lookup(|key| match key {
                LFG_FORUM_CUTOVER_ENV => Some("1".to_string()),
                LFG_FORUM_CHANNEL_ID_ENV => Some("222".to_string()),
                "DL_LFG_PANEL_CHANNEL_ID" => Some("999".to_string()),
                _ => None,
            }),
            222
        );
        assert_eq!(
            lfg_target_channel_id_from_lookup(|key| match key {
                LFG_FORUM_CUTOVER_ENV => Some("1".to_string()),
                LFG_FORUM_CHANNEL_ID_ENV => Some("0".to_string()),
                _ => None,
            }),
            LFG_LEGACY_CHANNEL_ID
        );
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use dl_central_db::{
        kv,
        testing::{test_pool, TestDb},
    };

    fn config() -> AiOnboardingConfig {
        AiOnboardingConfig::new(1289721245281292288, 1304216250649415771)
    }

    async fn test_db() -> TestDb {
        test_pool().await.expect("test_pool")
    }

    #[derive(Default)]
    struct MockPort {
        posts: Mutex<Vec<(u64, Map<String, Value>)>>,
        roles: Mutex<Vec<(u64, u64, u64, String)>>,
        member_roles: Mutex<Vec<MemberRole>>,
        role_error: Mutex<Option<String>>,
    }

    #[async_trait]
    impl AiOnboardingPort for MockPort {
        async fn post_message(
            &self,
            channel_id: u64,
            body: Map<String, Value>,
        ) -> Result<u64, String> {
            self.posts.lock().expect("posts").push((channel_id, body));
            Ok(555)
        }

        async fn add_member_role(
            &self,
            guild_id: u64,
            user_id: u64,
            role_id: u64,
            reason: &str,
        ) -> Result<(), String> {
            if let Some(err) = self.role_error.lock().expect("role_error").clone() {
                return Err(err);
            }
            self.roles.lock().expect("roles").push((
                guild_id,
                user_id,
                role_id,
                reason.to_string(),
            ));
            Ok(())
        }

        async fn member_roles(&self, _guild_id: u64, _user_id: u64) -> Vec<MemberRole> {
            self.member_roles.lock().expect("member_roles").clone()
        }
    }

    struct MockAi {
        response: Mutex<Option<String>>,
        seen: Mutex<Vec<GenerateRequest>>,
    }

    #[async_trait]
    impl TextGenerator for MockAi {
        async fn generate_text(&self, request: GenerateRequest) -> Option<String> {
            self.seen.lock().expect("seen").push(request);
            self.response.lock().expect("response").clone()
        }
    }

    #[test]
    fn modal_parse_trims_fields() {
        let options = HashMap::from([
            (FIELD_INTERESTS.to_string(), json!("  ranked  ")),
            (FIELD_EXPECTATIONS.to_string(), json!(" team ")),
            (FIELD_STYLE.to_string(), json!(" short ")),
        ]);
        assert_eq!(
            parse_answers(&options),
            UserAnswers {
                interests: "ranked".into(),
                expectations: "team".into(),
                style: "short".into(),
            }
        );
    }

    #[test]
    fn modal_uses_named_text_constants() {
        let modal = questions_modal(Some(7), Some(8));
        assert_eq!(modal.title, AI_ONBOARDING_MODAL_TITLE);
        assert_eq!(modal.fields[0].label, AI_ONBOARDING_MODAL_INTERESTS_LABEL);
        assert_eq!(
            modal.fields[0].placeholder,
            AI_ONBOARDING_MODAL_INTERESTS_PLACEHOLDER
        );
        assert_eq!(
            modal.fields[1].label,
            AI_ONBOARDING_MODAL_EXPECTATIONS_LABEL
        );
        assert_eq!(
            modal.fields[2].placeholder,
            AI_ONBOARDING_MODAL_STYLE_PLACEHOLDER
        );
        assert_eq!(
            parse_modal_context(&modal.custom_id).allowed_user_id,
            Some(7)
        );
        assert_eq!(parse_modal_context(&modal.custom_id).thread_id, Some(8));
    }

    #[test]
    fn components_mirror_custom_ids_and_link_buttons() {
        let panel = panel_components();
        assert_eq!(
            panel[0]["components"][0]["label"],
            AI_ONBOARDING_START_BUTTON_LABEL
        );
        assert_eq!(panel[0]["components"][0]["custom_id"], CUSTOM_ID_START);

        let quick = quick_actions_components(1289721245281292288);
        assert_eq!(quick[0]["components"][0]["style"], 5);
        assert_eq!(
            quick[0]["components"][0]["label"],
            AI_ONBOARDING_QUICK_LFG_BUTTON_LABEL
        );
        assert_eq!(
            quick[1]["components"][0]["label"],
            AI_ONBOARDING_RULES_CONFIRM_BUTTON_LABEL
        );
        assert_eq!(
            quick[1]["components"][0]["custom_id"],
            CUSTOM_ID_RULES_CONFIRM
        );
    }

    #[tokio::test]
    async fn router_registers_all_aiob_handlers() {
        let db = test_db().await;
        let service = AiOnboarding::new(
            db.pool().clone(),
            Arc::new(MockPort::default()),
            None,
            config(),
        );
        let mut router = InteractionRouter::new();
        register(&mut router, service);

        assert!(router.resolve_component(CUSTOM_ID_START).is_some());
        assert!(router.resolve_component(CUSTOM_ID_RULES_CONFIRM).is_some());
        assert!(router.resolve_component("aiob:questions:7:8").is_some());
    }

    #[tokio::test]
    async fn start_in_channel_posts_panel_and_persists_view() {
        let db = test_db().await;
        let port = Arc::new(MockPort::default());
        let service = AiOnboarding::new(db.pool().clone(), port.clone(), None, config());

        let message_id = service.start_in_channel(123, 77).await.expect("start");
        assert_eq!(message_id, 555);
        assert_eq!(port.posts.lock().expect("posts").len(), 1);

        let raw = kv::get(db.pool(), NS_PERSIST_VIEWS, "555")
            .await
            .expect("kv")
            .expect("view");
        let view: PersistedView = serde_json::from_str(&raw).expect("json");
        assert_eq!(view.user_id, Some(77));
        assert_eq!(view.thread_id, Some(123));
    }

    #[tokio::test]
    async fn start_in_channel_persists_owner_even_when_user_is_opted_out() {
        let db = test_db().await;
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
            VALUES (77, TRUE, now())
            "#
        )
        .execute(db.pool())
        .await
        .expect("privacy");
        let service = AiOnboarding::new(
            db.pool().clone(),
            Arc::new(MockPort::default()),
            None,
            config(),
        );

        service.start_in_channel(123, 77).await.expect("start");

        let raw = kv::get(db.pool(), NS_PERSIST_VIEWS, "555")
            .await
            .expect("kv")
            .expect("view");
        let view: PersistedView = serde_json::from_str(&raw).expect("json");
        assert_eq!(view.user_id, Some(77));
        assert_eq!(view.thread_id, Some(123));
    }

    #[tokio::test]
    async fn restore_counts_valid_and_removes_invalid_views() {
        let db = test_db().await;
        kv::set(
            db.pool(),
            NS_PERSIST_VIEWS,
            "100",
            &serde_json::to_string(&PersistedView {
                user_id: Some(1),
                thread_id: Some(2),
            })
            .expect("json"),
        )
        .await
        .expect("set valid");
        kv::set(db.pool(), NS_PERSIST_VIEWS, "bad", "{}")
            .await
            .expect("set invalid key");
        kv::set(db.pool(), NS_PERSIST_VIEWS, "101", "{")
            .await
            .expect("set invalid json");

        let service = AiOnboarding::new(
            db.pool().clone(),
            Arc::new(MockPort::default()),
            None,
            config(),
        );
        let report = service.restore_persistent_views().await.expect("restore");
        assert_eq!(
            report,
            RestoreReport {
                scanned: 3,
                restored: 1,
                removed_invalid: 2,
            }
        );
        assert!(kv::get(db.pool(), NS_PERSIST_VIEWS, "100")
            .await
            .expect("valid")
            .is_some());
        assert!(kv::get(db.pool(), NS_PERSIST_VIEWS, "101")
            .await
            .expect("invalid")
            .is_none());
    }

    #[tokio::test]
    async fn rules_confirm_dm_aborts_without_guild_fallback() {
        let db = test_db().await;
        let port = Arc::new(MockPort::default());
        let service = AiOnboarding::new(db.pool().clone(), port.clone(), None, config());

        let reply = service
            .handle_rules_confirm(BridgeInteraction {
                custom_id: CUSTOM_ID_RULES_CONFIRM.to_string(),
                user_id: 42,
                guild_id: 0,
                member_present: false,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some(AI_ONBOARDING_RULES_CONFIRM_MEMBER_ERROR_MESSAGE)
        );
        assert!(port.roles.lock().expect("roles").is_empty());
    }

    #[tokio::test]
    async fn rules_confirm_grants_configured_complete_role() {
        let db = test_db().await;
        let port = Arc::new(MockPort::default());
        let service = AiOnboarding::new(db.pool().clone(), port.clone(), None, config());
        let mut router = InteractionRouter::new();
        register(&mut router, service);
        let handler = router
            .resolve_component(CUSTOM_ID_RULES_CONFIRM)
            .expect("handler");

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: CUSTOM_ID_RULES_CONFIRM.to_string(),
                user_id: 42,
                guild_id: 1289721245281292288,
                member_present: true,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(
            reply.content.as_deref(),
            Some(AI_ONBOARDING_RULES_CONFIRM_SUCCESS_MESSAGE)
        );
        assert_eq!(
            port.roles.lock().expect("roles").as_slice(),
            &[(
                1289721245281292288,
                42,
                1304216250649415771,
                "AI onboarding: rules confirmed".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn rules_confirm_uses_persisted_view_owner_even_when_session_log_missing() {
        let db = test_db().await;
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
            VALUES (42, TRUE, now())
            "#
        )
        .execute(db.pool())
        .await
        .expect("privacy");
        kv::set(
            db.pool(),
            NS_PERSIST_VIEWS,
            "777",
            &serde_json::to_string(&PersistedView {
                user_id: Some(42),
                thread_id: Some(333),
            })
            .expect("json"),
        )
        .await
        .expect("view");
        let port = Arc::new(MockPort::default());
        let service = AiOnboarding::new(db.pool().clone(), port.clone(), None, config());

        let rejected = service
            .handle_rules_confirm(BridgeInteraction {
                custom_id: CUSTOM_ID_RULES_CONFIRM.to_string(),
                user_id: 99,
                guild_id: 1289721245281292288,
                channel_id: 333,
                message_id: Some(777),
                member_present: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            rejected.content.as_deref(),
            Some(AI_ONBOARDING_WRONG_USER_MESSAGE)
        );
        assert!(port.roles.lock().expect("roles").is_empty());

        let accepted = service
            .handle_rules_confirm(BridgeInteraction {
                custom_id: CUSTOM_ID_RULES_CONFIRM.to_string(),
                user_id: 42,
                guild_id: 1289721245281292288,
                channel_id: 333,
                message_id: Some(777),
                member_present: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            accepted.content.as_deref(),
            Some(AI_ONBOARDING_RULES_CONFIRM_SUCCESS_MESSAGE)
        );
        assert_eq!(port.roles.lock().expect("roles").len(), 1);
    }

    #[tokio::test]
    async fn start_button_opens_modal_and_clears_persisted_view() {
        let db = test_db().await;
        kv::set(
            db.pool(),
            NS_PERSIST_VIEWS,
            "900",
            &serde_json::to_string(&PersistedView {
                user_id: Some(42),
                thread_id: Some(321),
            })
            .expect("json"),
        )
        .await
        .expect("set");
        let service = AiOnboarding::new(
            db.pool().clone(),
            Arc::new(MockPort::default()),
            None,
            config(),
        );

        let reply = service
            .handle_start(BridgeInteraction {
                custom_id: CUSTOM_ID_START.to_string(),
                user_id: 42,
                channel_id: 111,
                message_id: Some(900),
                ..BridgeInteraction::default()
            })
            .await;
        let modal = reply.modal.expect("modal");
        assert_eq!(
            parse_modal_context(&modal.custom_id),
            ModalContext {
                allowed_user_id: Some(42),
                thread_id: Some(321),
            }
        );
        assert!(kv::get(db.pool(), NS_PERSIST_VIEWS, "900")
            .await
            .expect("kv")
            .is_none());
    }

    #[tokio::test]
    async fn prompt_assembly_uses_minimax_path_and_constants() {
        let db = test_db().await;
        let port = Arc::new(MockPort::default());
        port.member_roles.lock().expect("roles").push(MemberRole {
            id: ROLE_RANKED_ID,
            name: "Cache Name Ranked".to_string(),
        });
        port.member_roles.lock().expect("roles").push(MemberRole {
            id: 1,
            name: "AdminRole".to_string(),
        });
        let ai = Arc::new(MockAi {
            response: Mutex::new(Some("  generated  ".to_string())),
            seen: Mutex::new(Vec::new()),
        });
        let service = AiOnboarding::new(
            db.pool().clone(),
            port,
            Some(ai.clone()),
            config().with_max_output_tokens(321),
        );
        let answers = UserAnswers {
            interests: "stream ranked".to_string(),
            expectations: "team".to_string(),
            style: "short".to_string(),
        };

        let generated = service
            .generate_personalized_text(
                &answers,
                PromptUser {
                    user_id: 7,
                    display_name: "Nani".to_string(),
                    guild_id: 1289721245281292288,
                },
            )
            .await;

        assert_eq!(generated.text, "generated");
        assert_eq!(generated.llm_meta["provider"], "minimax");
        let seen = ai.seen.lock().expect("seen");
        let request = seen.first().expect("request");
        assert_eq!(
            request.system_prompt.as_deref(),
            Some(AI_ONBOARDING_SYSTEM_PROMPT)
        );
        assert_eq!(request.max_output_tokens, Some(321));
        assert_eq!(request.temperature, TEMPERATURE);
        assert!(request.prompt.contains(AI_ONBOARDING_USER_PROMPT_TEMPLATE));
        assert!(request
            .prompt
            .contains(AI_ONBOARDING_ANSWERS_PROMPT_TEMPLATE));
        assert!(request.prompt.contains(AI_ONBOARDING_SERVER_CONTEXT));
        assert!(request.prompt.contains(AI_ONBOARDING_ROLE_CONTEXT_HEADER));
        assert!(request
            .prompt
            .contains(AI_ONBOARDING_ROLE_CONTEXT_STREAMING_HINT));
        assert!(request.prompt.contains("Ranked/Rang-Spieler Rolle: ja"));
        assert!(!request.prompt.contains("AdminRole"));
        assert!(!request.prompt.contains("Cache Name Ranked"));
        assert!(request.prompt.contains("stream ranked"));
    }

    #[test]
    fn role_context_whitelists_known_onboarding_roles() {
        let roles = vec![
            MemberRole {
                id: ROLE_IGNORE_ID,
                name: "Complete".to_string(),
            },
            MemberRole {
                id: ROLE_LFG_PING_ID,
                name: "Cache LFG".to_string(),
            },
            MemberRole {
                id: ROLE_CUSTOM_GAMES_PING_ID,
                name: "Cache Custom".to_string(),
            },
            MemberRole {
                id: 999,
                name: "AdminRole".to_string(),
            },
        ];
        let answers = UserAnswers {
            interests: String::new(),
            expectations: String::new(),
            style: String::new(),
        };

        let context = build_role_context_block(&roles, &answers, &[]);

        assert!(context.contains("Spieler-Suche Ping Rolle: ja"));
        assert!(context.contains("Custom Games Ping Rolle: ja"));
        assert!(context.contains(AI_ONBOARDING_ROLE_CONTEXT_HINTS_HEADER));
        assert!(context.contains(AI_ONBOARDING_ROLE_CONTEXT_LFG_PING_HINT));
        assert!(context.contains(AI_ONBOARDING_ROLE_CONTEXT_CUSTOM_GAMES_HINT));
        assert!(!context.contains("AdminRole"));
        assert!(!context.contains("Complete"));
    }

    #[tokio::test]
    async fn modal_submit_logs_session_and_returns_quick_actions() {
        let db = test_db().await;
        let ai = Arc::new(MockAi {
            response: Mutex::new(Some("generated".to_string())),
            seen: Mutex::new(Vec::new()),
        });
        let service = AiOnboarding::new(
            db.pool().clone(),
            Arc::new(MockPort::default()),
            Some(ai),
            config(),
        );
        let reply = service
            .handle_questions(BridgeInteraction {
                custom_id: modal_custom_id(Some(42), Some(333)),
                user_id: 42,
                guild_id: 1289721245281292288,
                channel_id: 999,
                author_name: "User".to_string(),
                options: HashMap::from([
                    (FIELD_INTERESTS.to_string(), json!("a")),
                    (FIELD_EXPECTATIONS.to_string(), json!("b")),
                    (FIELD_STYLE.to_string(), json!("c")),
                ]),
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.embeds[0]["title"],
            AI_ONBOARDING_PERSONALIZED_EMBED_TITLE
        );
        assert_eq!(
            reply.components.as_ref().expect("components")[1]["components"][0]["custom_id"],
            CUSTOM_ID_RULES_CONFIRM
        );
        reply
            .response_message_hook
            .as_ref()
            .expect("response hook")
            .on_response_message(777)
            .await;
        let raw_view = kv::get(db.pool(), NS_PERSIST_VIEWS, "777")
            .await
            .expect("kv")
            .expect("view");
        let view: PersistedView = serde_json::from_str(&raw_view).expect("json");
        assert_eq!(view.user_id, Some(42));
        assert_eq!(view.thread_id, Some(333));
        let raw = kv::get(db.pool(), NS_SESSION_LOG, "42")
            .await
            .expect("kv")
            .expect("session");
        let logged: Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(logged["thread_id"], 333);
        assert_eq!(logged["answers"]["interests"], "a");
        assert_eq!(logged["llm"]["provider"], "minimax");
    }

    #[tokio::test]
    async fn opt_out_skips_session_logging() {
        let db = test_db().await;
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
            VALUES (42, TRUE, now())
            "#
        )
        .execute(db.pool())
        .await
        .expect("privacy");
        let service = AiOnboarding::new(
            db.pool().clone(),
            Arc::new(MockPort::default()),
            None,
            config(),
        );
        service
            .log_session(
                42,
                Some(1),
                &UserAnswers {
                    interests: "a".into(),
                    expectations: "b".into(),
                    style: "c".into(),
                },
                json!({"provider": "fallback"}),
            )
            .await;
        assert!(kv::get(db.pool(), NS_SESSION_LOG, "42")
            .await
            .expect("kv")
            .is_none());
    }
}
