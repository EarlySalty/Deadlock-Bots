//! Team-Bewerbungen über ein öffentliches Components-V2-Panel und den internen Mod-Chat.

use std::path::Path;
use std::sync::Arc;

use dl_central_db::kv;
use dl_discord::{
    BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter, ModalField, ModalSpec,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Row};

pub const COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const STREAMER_PARTNER_URL: &str = "https://deutsche-deadlock-community.de/streamer";
pub const TEAM_PANEL_CHANNEL_NAME: &str = "🤝team-werden";
pub const TEAM_APPLICATION_NOTIFY_CHANNEL_ID: u64 = 1_315_684_135_175_716_978;
pub const TEAM_PANEL_CHANNEL_ID: u64 = 1_544_026_617_733_710_006;
const PANEL_KV_NS: &str = "team_applications";
const PANEL_KV_KEY: &str = "panel_message_id";
const PANEL_TEXT_FILE: &str = "assets/team_application_texts.toml";
const APPLICATION_ANSWER_MAX_LENGTH: usize = 550;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationKind {
    Moderation,
    Coach,
    Caster,
    Tournament,
    Coder,
    Pate,
    Other,
}

impl ApplicationKind {
    pub const ALL: [Self; 7] = [
        Self::Moderation,
        Self::Coach,
        Self::Caster,
        Self::Tournament,
        Self::Coder,
        Self::Pate,
        Self::Other,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Moderation => "moderation",
            Self::Coach => "coach",
            Self::Caster => "caster",
            Self::Tournament => "turnier",
            Self::Coder => "coder",
            Self::Pate => "pate",
            Self::Other => "sonstiges",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Moderation => "Moderation",
            Self::Coach => "Coach",
            Self::Caster => "Caster",
            Self::Tournament => "Turnier-Organisation",
            Self::Coder => "Coder",
            Self::Pate => "Pate",
            Self::Other => "Eigene Idee",
        }
    }

    pub fn from_slug(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.slug() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationStatus {
    Publishing,
    Open,
    Review,
    Question,
    Accepted,
    Rejected,
}

impl ApplicationStatus {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Publishing => "publishing",
            Self::Open => "open",
            Self::Review => "review",
            Self::Question => "question",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Publishing => "Wird veröffentlicht",
            Self::Open => "Offen",
            Self::Review => "In Prüfung",
            Self::Question => "Rückfrage",
            Self::Accepted => "Angenommen",
            Self::Rejected => "Abgelehnt",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelText {
    pub title: String,
    pub intro: String,
    pub process: String,
    pub footer: String,
}

impl Default for PanelText {
    fn default() -> Self {
        Self {
            title: "🤝 Werde Teil unseres Teams".to_string(),
            intro: "Du möchtest die Deutsche Deadlock Community mitgestalten? Wähle den Bereich, der zu dir passt, und erzähl uns kurz, wie du dich einbringen möchtest. Erfahrung ist hilfreich, aber kein Muss — wichtiger sind Verlässlichkeit, ein respektvoller Umgang und Lust, gemeinsam etwas aufzubauen.".to_string(),
            process: "Deine Bewerbung landet ausschließlich im internen Moderationsbereich. Wir lesen jede Bewerbung und melden uns per Discord-DM bei dir. Bitte teile keine sensiblen oder privaten Daten.".to_string(),
            footer: "Du findest dich in keinem Bereich wieder? Nutze „Eigene Idee“ — gute Ideen müssen nicht in eine Schublade passen.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PateText {
    pub intro: String,
    pub frage_erfahrung: String,
    pub frage_verfuegbarkeit: String,
}

impl Default for PateText {
    fn default() -> Self {
        Self {
            intro: "Als Pate nimmst du neue Mitglieder an die Hand und zeigst ihnen unseren Server und die ersten Schritte im Spiel. Du brauchst keine festen Zeiten, nur Lust, ansprechbar und freundlich zu sein. Rechne mit ein paar Minuten hier und da, wenn ein Neuling Fragen hat.".to_string(),
            frage_erfahrung: "Wie gut kennst du Deadlock und unseren Server schon?".to_string(),
            frage_verfuegbarkeit: "Wann bist du meistens da und wie schnell kannst du auf Fragen antworten?".to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextFile {
    #[serde(default)]
    publish: PublishConfig,
    panel: PanelText,
    #[serde(default)]
    pate: PateText,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishConfig {
    enabled: bool,
}

#[derive(Debug, Clone)]
struct LoadedPanelText {
    enabled: bool,
    text: PanelText,
    pate: PateText,
}

fn load_panel_text(repo_root: &Path) -> LoadedPanelText {
    let runtime_path = repo_root.join(PANEL_TEXT_FILE);
    match std::fs::read_to_string(&runtime_path) {
        Ok(raw) => match toml::from_str::<TextFile>(&raw) {
            Ok(file) => LoadedPanelText {
                enabled: file.publish.enabled,
                text: file.panel,
                pate: file.pate,
            },
            Err(error) => {
                tracing::error!(%error, path = %runtime_path.display(), "Team-Bewerbungstexte sind ungültig; Veröffentlichung bleibt gesperrt");
                embedded_disabled_panel()
            }
        },
        Err(error) => {
            tracing::warn!(%error, path = %runtime_path.display(), "Team-Bewerbungstexte konnten nicht gelesen werden; eingebetteter Entwurf wird verwendet");
            embedded_disabled_panel()
        }
    }
}

fn embedded_disabled_panel() -> LoadedPanelText {
    let file = toml::from_str::<TextFile>(include_str!(
        "../../../../assets/team_application_texts.toml"
    ))
    .ok();
    LoadedPanelText {
        enabled: false,
        text: file
            .as_ref()
            .map(|file| file.panel.clone())
            .unwrap_or_default(),
        pate: file.map(|file| file.pate).unwrap_or_default(),
    }
}

fn display(content: impl Into<String>) -> Value {
    json!({"type": 10, "content": content.into()})
}

fn publication_placeholder() -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".into(), json!(COMPONENTS_V2_FLAG));
    body.insert("allowed_mentions".into(), json!({"parse": []}));
    body.insert(
        "components".into(),
        json!([{"type": 17, "accent_color": 0xC8A86B, "components": [
            display("## Team-Bewerbung\n\nDie Bewerbung wird sicher vorbereitet …")
        ]}]),
    );
    body
}

const PATE_PANEL_EMOJI: (&str, &str) = ("dl_crown", "1522518265421631538");

fn button(kind: ApplicationKind, emoji: &str) -> Value {
    json!({
        "type": 2, "style": 2, "label": kind.label(), "emoji": {"name": emoji},
        "custom_id": format!("team_apply:open:{}", kind.slug())
    })
}

fn brand_button(kind: ApplicationKind, emoji: (&str, &str)) -> Value {
    json!({
        "type": 2, "style": 2, "label": kind.label(),
        "emoji": {"name": emoji.0, "id": emoji.1},
        "custom_id": format!("team_apply:open:{}", kind.slug())
    })
}

pub fn panel_body(text: &PanelText) -> Map<String, Value> {
    let first = json!({"type": 1, "components": [
        button(ApplicationKind::Moderation, "🛡️"),
        button(ApplicationKind::Coach, "🎓"),
        button(ApplicationKind::Caster, "🎙️"),
        button(ApplicationKind::Tournament, "🏆")
    ]});
    let second = json!({"type": 1, "components": [
        button(ApplicationKind::Coder, "💻"),
        brand_button(ApplicationKind::Pate, PATE_PANEL_EMOJI),
        button(ApplicationKind::Other, "💡"),
        {"type": 2, "style": 5, "label": "Streamer-Partner", "emoji": {"name": "📺"}, "url": STREAMER_PARTNER_URL}
    ]});
    let mut body = Map::new();
    body.insert("flags".into(), json!(COMPONENTS_V2_FLAG));
    body.insert("allowed_mentions".into(), json!({"parse": []}));
    body.insert(
        "components".into(),
        json!([{"type": 17, "accent_color": 0xC8A86B, "components": [
            display(format!("## {}\n\n{}", text.title, text.intro)),
            {"type": 14, "divider": true, "spacing": 1},
            display(format!("**So läuft es ab**\n{}\n\n{}", text.process, text.footer)),
            first,
            second
        ]}]),
    );
    body
}

fn modal_field(id: &str, label: &str, placeholder: &str, required: bool) -> ModalField {
    ModalField {
        custom_id: id.to_string(),
        label: label.to_string(),
        placeholder: placeholder.to_string(),
        value: None,
        required,
        min_length: if required { 10 } else { 0 },
        max_length: APPLICATION_ANSWER_MAX_LENGTH as u16,
        paragraph: true,
    }
}

pub fn application_modal(kind: ApplicationKind) -> ModalSpec {
    let focus = match kind {
        ApplicationKind::Moderation => "Moderator oder Community Moderator?",
        ApplicationKind::Coach => "Rank, Helden und Coaching-Schwerpunkte",
        ApplicationKind::Caster => "Streams, Turniere oder beides?",
        ApplicationKind::Tournament => "Custom, Grind, Funny oder eigene Formate?",
        ApplicationKind::Coder => "Technologien und Projekte",
        ApplicationKind::Pate => "Erfahrung und Verfügbarkeit als Pate",
        ApplicationKind::Other => "Wie möchtest du dich einbringen?",
    };
    ModalSpec {
        custom_id: format!("team_apply:submit:{}", kind.slug()),
        title: format!("Bewerbung: {}", kind.label())
            .chars()
            .take(45)
            .collect(),
        fields: vec![
            modal_field(
                "motivation",
                "Warum möchtest du ins Team?",
                "Was motiviert dich?",
                true,
            ),
            modal_field(
                "experience",
                "Erfahrung und Stärken",
                "Was bringst du mit?",
                true,
            ),
            modal_field(
                "availability",
                "Zeit und Verfügbarkeit",
                "Wann und wie oft?",
                true,
            ),
            modal_field(
                "contribution",
                "Dein konkreter Beitrag",
                "Was möchtest du umsetzen?",
                true,
            ),
            modal_field("focus", focus, "Kurz und konkret", false),
        ],
    }
}

pub fn pate_application_modal(text: &PateText) -> ModalSpec {
    ModalSpec {
        custom_id: format!("team_apply:submit:{}", ApplicationKind::Pate.slug()),
        title: "Bewerbung: Pate".to_string(),
        fields: vec![
            modal_field(
                "experience",
                "Erfahrung",
                &modal_placeholder(&text.frage_erfahrung),
                true,
            ),
            modal_field(
                "availability",
                "Verfügbarkeit",
                &modal_placeholder(&text.frage_verfuegbarkeit),
                true,
            ),
        ],
    }
}

const EPHEMERAL_FLAG: u64 = 64;

pub fn pate_intro_reply(text: &PateText) -> BridgeReply {
    let components = json!([{"type": 17, "accent_color": 0xC8A86B, "components": [
        display(format!("## Pate werden\n\n{}", text.intro)),
        {"type": 14, "divider": true, "spacing": 1},
        {"type": 1, "components": [
            {"type": 2, "style": 1, "label": "Weiter zum Formular",
             "custom_id": format!("team_apply:form:{}", ApplicationKind::Pate.slug())}
        ]}
    ]}]);
    BridgeReply {
        components: Some(components),
        message_flags: Some(COMPONENTS_V2_FLAG | EPHEMERAL_FLAG),
        allowed_mentions: Some(json!({"parse": []})),
        ..BridgeReply::default()
    }
}

fn modal_placeholder(text: &str) -> String {
    text.chars().take(100).collect()
}

fn decision_modal(action: &str, id: i64) -> Option<ModalSpec> {
    let (title, label, placeholder, required) = match action {
        "question" => (
            "Rückfrage senden",
            "Was möchtest du noch wissen?",
            "Die Nachricht wird per Discord-DM gesendet.",
            true,
        ),
        "accept" => (
            "Bewerbung annehmen",
            "Nächste Schritte (optional)",
            "Zum Beispiel: Wer meldet sich wann?",
            false,
        ),
        "reject" => (
            "Bewerbung ablehnen",
            "Kurze, faire Begründung",
            "Die Nachricht wird per Discord-DM gesendet.",
            true,
        ),
        _ => return None,
    };
    Some(ModalSpec {
        custom_id: format!("team_apply:decision:{action}:{id}"),
        title: title.to_string(),
        fields: vec![modal_field("note", label, placeholder, required)],
    })
}

pub fn sanitize_text(value: &str, limit: usize) -> String {
    let cleaned = value
        .split_whitespace()
        .filter(|part| {
            !((part.starts_with("<@") && part.ends_with('>'))
                || matches!(*part, "@everyone" | "@here"))
        })
        .map(|part| part.replace('@', "＠"))
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.chars().count() <= limit {
        return cleaned;
    }
    let keep = limit.saturating_sub(1);
    format!("{}…", cleaned.chars().take(keep).collect::<String>())
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut encoded = Vec::new();
    while value > 0 {
        encoded.push(DIGITS[(value % 36) as usize] as char);
        value /= 36;
    }
    encoded.iter().rev().collect()
}

fn publication_nonce(id: i64) -> String {
    format!("a:{}", base36(id.unsigned_abs()))
}

fn status_dm_nonce(id: i64, status_version: i32) -> String {
    format!(
        "d:{}:{}",
        base36(id.unsigned_abs()),
        base36(status_version.unsigned_abs().into())
    )
}

fn status_dm_audit_nonce(id: i64, status_version: i32) -> String {
    format!(
        "u:{}:{}",
        base36(id.unsigned_abs()),
        base36(status_version.unsigned_abs().into())
    )
}

fn status_dm_audit_body(id: i64) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".into(), json!(COMPONENTS_V2_FLAG));
    body.insert("allowed_mentions".into(), json!({"parse": []}));
    body.insert(
        "components".into(),
        json!([{"type": 17, "accent_color": 0xD97706, "components": [
            display(format!("## ⚠️ DM-Zustellung unklar\n\nBei Bewerbung #{id} konnte nach einem unterbrochenen Versand nicht sicher bestätigt werden, ob die Status-DM angekommen ist. Bitte kontaktiert den Bewerber manuell und sendet die Statusaktion nicht erneut."))
        ]}]),
    );
    body
}

fn add_nonce(body: &mut Map<String, Value>, nonce: String) {
    body.insert("nonce".into(), json!(nonce));
    body.insert("enforce_nonce".into(), json!(true));
}

pub fn staff_components(id: i64, status: ApplicationStatus) -> Value {
    let disabled = matches!(
        status,
        ApplicationStatus::Accepted | ApplicationStatus::Rejected
    );
    json!([{"type": 1, "components": [
        {"type": 2, "style": 2, "label": "In Prüfung", "custom_id": format!("team_apply:status:review:{id}"), "disabled": disabled},
        {"type": 2, "style": 1, "label": "Rückfrage", "custom_id": format!("team_apply:status:question:{id}"), "disabled": disabled},
        {"type": 2, "style": 3, "label": "Annehmen", "custom_id": format!("team_apply:status:accept:{id}"), "disabled": disabled},
        {"type": 2, "style": 4, "label": "Ablehnen", "custom_id": format!("team_apply:status:reject:{id}"), "disabled": disabled}
    ]}])
}

#[derive(Debug, Clone)]
pub struct ModeratorPost {
    pub body: Map<String, Value>,
}

#[derive(Debug)]
pub enum ModeratorEditError {
    NotFound,
    Other(String),
}

impl std::fmt::Display for ModeratorEditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("Discord-Nachricht nicht gefunden"),
            Self::Other(error) => formatter.write_str(error),
        }
    }
}

#[async_trait::async_trait]
pub trait TeamApplicationPort: Send + Sync {
    async fn post_panel(&self, channel_id: u64, body: Map<String, Value>) -> Result<u64, String>;
    async fn edit_panel(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), String>;
    async fn post_moderator_application(
        &self,
        channel_id: u64,
        body: Map<String, Value>,
    ) -> Result<u64, String>;
    async fn edit_moderator_application(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), ModeratorEditError>;
    async fn delete_moderator_application(
        &self,
        channel_id: u64,
        message_id: u64,
    ) -> Result<(), String>;
    async fn send_dm(&self, user_id: u64, body: Map<String, Value>) -> Result<(), String>;
    async fn add_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String>;
}

pub struct TeamApplications {
    pool: PgPool,
    port: Arc<dyn TeamApplicationPort>,
    guild_id: u64,
    panel_text: PanelText,
    pate_text: PateText,
    publish_enabled: bool,
}

impl TeamApplications {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn TeamApplicationPort>,
        guild_id: u64,
        repo_root: &Path,
    ) -> Arc<Self> {
        let loaded = load_panel_text(repo_root);
        Arc::new(Self {
            pool,
            port,
            guild_id,
            panel_text: loaded.text,
            pate_text: loaded.pate,
            publish_enabled: loaded.enabled,
        })
    }

    pub async fn ensure_panel(&self) {
        if !self.publish_enabled {
            tracing::info!("Team-Bewerbungs-Panel bleibt als Entwurf unveröffentlicht");
            return;
        }
        let channel_id = TEAM_PANEL_CHANNEL_ID;
        let body = panel_body(&self.panel_text);
        let stored = kv::get(&self.pool, PANEL_KV_NS, PANEL_KV_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|id| id.parse().ok());
        if let Some(message_id) = stored {
            if self
                .port
                .edit_panel(channel_id, message_id, body.clone())
                .await
                .is_ok()
            {
                return;
            }
        }
        match self.port.post_panel(channel_id, body).await {
            Ok(message_id) => {
                if let Err(error) = kv::set(
                    &self.pool,
                    PANEL_KV_NS,
                    PANEL_KV_KEY,
                    &message_id.to_string(),
                )
                .await
                {
                    tracing::warn!(%error, "Team-Bewerbungs-Panel-ID konnte nicht gespeichert werden");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "Team-Bewerbungs-Panel konnte nicht veröffentlicht werden")
            }
        }
    }

    pub async fn run_maintenance_loop(self: Arc<Self>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            self.process_pending_publications().await;
            self.process_status_dm_recovery().await;
            self.process_discord_erasure_queue().await;
        }
    }

    async fn process_status_dm_recovery(&self) {
        if let Err(error) = sqlx::query(
            r#"UPDATE community.team_applications
                  SET status_dm_claimed_at=NULL, updated_at=now()
                WHERE status_dm_sent_at IS NULL
                  AND status_dm_dispatch_started_at IS NULL
                  AND status_dm_claimed_at < now() - interval '5 minutes'"#,
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!(%error, "Nicht gestartete Team-Bewerbungs-DMs konnten nicht freigegeben werden");
        }

        let rows = match sqlx::query(
            r#"WITH candidates AS (
                   SELECT id
                     FROM community.team_applications
                    WHERE status_dm_sent_at IS NULL
                      AND status_dm_dispatch_started_at < now() - interval '5 minutes'
                      AND status_dm_audit_claimed_at IS NULL
                    ORDER BY status_dm_dispatch_started_at
                    FOR UPDATE SKIP LOCKED
                    LIMIT 25
               )
               UPDATE community.team_applications AS application
                  SET status_dm_audit_claimed_at=now(), updated_at=now()
                 FROM candidates
                WHERE application.id=candidates.id
               RETURNING application.id, application.status_version"#,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Unklare Team-Bewerbungs-DMs konnten nicht geprüft werden");
                return;
            }
        };
        for row in rows {
            let (Ok(id), Ok(status_version)) = (
                row.try_get::<i64, _>("id"),
                row.try_get::<i32, _>("status_version"),
            ) else {
                continue;
            };
            let mut body = status_dm_audit_body(id);
            add_nonce(&mut body, status_dm_audit_nonce(id, status_version));
            if let Err(error) = self
                .port
                .post_moderator_application(TEAM_APPLICATION_NOTIFY_CHANNEL_ID, body)
                .await
            {
                tracing::warn!(%error, id, status_version, "Unklare Team-Bewerbungs-DM konnte dem Mod-Team nicht gemeldet werden");
                continue;
            }
            if let Err(error) = sqlx::query(
                r#"UPDATE community.team_applications
                      SET status_dm_audited_at=now(), updated_at=now()
                    WHERE id=$1 AND status_version=$2
                      AND status_dm_sent_at IS NULL
                      AND status_dm_dispatch_started_at IS NOT NULL
                      AND status_dm_audit_claimed_at IS NOT NULL"#,
            )
            .bind(id)
            .bind(status_version)
            .execute(&self.pool)
            .await
            {
                tracing::warn!(%error, id, status_version, "Hinweis zur unklaren Team-Bewerbungs-DM konnte nicht bestätigt werden");
            }
        }
    }

    async fn process_pending_publications(&self) {
        let rows = match sqlx::query(
            r#"WITH candidates AS (
                   SELECT id
                     FROM community.team_applications
                    WHERE status='publishing'
                      AND (
                          publication_last_attempt_at IS NULL
                          OR publication_last_attempt_at < now() - interval '30 seconds'
                      )
                    ORDER BY created_at
                    FOR UPDATE SKIP LOCKED
                    LIMIT 25
               )
               UPDATE community.team_applications AS application
                  SET publication_started_at=COALESCE(application.publication_started_at, now()),
                      publication_last_attempt_at=now(),
                      updated_at=now()
                 FROM candidates
                WHERE application.id=candidates.id
               RETURNING application.id, application.applicant_user_id,
                         application.applicant_name, application.kind, application.answers,
                         application.moderator_message_id"#,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Offene Team-Bewerbungen konnten nicht gelesen werden");
                return;
            }
        };

        for row in rows {
            let (Ok(id), Some(applicant_user_id), Ok(applicant_name), Some(kind), Ok(answers)) = (
                row.try_get::<i64, _>("id"),
                row.try_get::<i64, _>("applicant_user_id")
                    .ok()
                    .and_then(|value| u64::try_from(value).ok()),
                row.try_get::<String, _>("applicant_name"),
                row.try_get::<String, _>("kind")
                    .ok()
                    .and_then(|value| ApplicationKind::from_slug(&value)),
                row.try_get::<Value, _>("answers"),
            ) else {
                tracing::error!("Offene Team-Bewerbung enthält ungültige Daten");
                continue;
            };
            let existing_message_id = row
                .try_get::<Option<i64>, _>("moderator_message_id")
                .ok()
                .flatten()
                .and_then(|value| u64::try_from(value).ok());
            let _ = self
                .publish_application(
                    id,
                    applicant_user_id,
                    &applicant_name,
                    kind,
                    &answers,
                    existing_message_id,
                )
                .await;
        }
    }

    async fn process_discord_erasure_queue(&self) {
        let rows = match sqlx::query(
            "SELECT application_id, moderator_message_id
               FROM community.team_application_discord_erasure_queue
              ORDER BY attempts, updated_at, created_at
              LIMIT 25",
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "Discord-Löschwarteschlange für Team-Bewerbungen konnte nicht gelesen werden");
                return;
            }
        };
        for row in rows {
            let Ok(application_id) = row.try_get::<i64, _>("application_id") else {
                continue;
            };
            let Some(message_id) = row
                .try_get::<i64, _>("moderator_message_id")
                .ok()
                .and_then(|value| u64::try_from(value).ok())
            else {
                tracing::error!(
                    application_id,
                    "Ungültige Nachrichten-ID in Discord-Löschwarteschlange"
                );
                continue;
            };
            match self
                .port
                .delete_moderator_application(TEAM_APPLICATION_NOTIFY_CHANNEL_ID, message_id)
                .await
            {
                Ok(()) => {
                    if let Err(error) = sqlx::query(
                        "DELETE FROM community.team_application_discord_erasure_queue WHERE application_id=$1",
                    )
                    .bind(application_id)
                    .execute(&self.pool)
                    .await
                    {
                        tracing::warn!(%error, application_id, "Erledigte Discord-Löschung konnte nicht bestätigt werden");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, application_id, "Team-Bewerbungsbeitrag konnte noch nicht gelöscht werden");
                    if let Err(update_error) = sqlx::query(
                        "UPDATE community.team_application_discord_erasure_queue SET attempts=attempts+1, updated_at=now() WHERE application_id=$1",
                    )
                    .bind(application_id)
                    .execute(&self.pool)
                    .await
                    {
                        tracing::error!(%update_error, application_id, message_id, "Fehlversuch der Discord-Löschung konnte nicht verbucht werden");
                    }
                }
            }
        }
    }

    async fn submit(&self, interaction: &BridgeInteraction, kind: ApplicationKind) -> BridgeReply {
        let answer = |key: &str| {
            sanitize_text(
                interaction
                    .options
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                APPLICATION_ANSWER_MAX_LENGTH,
            )
        };
        let answers = json!({
            "motivation": answer("motivation"), "experience": answer("experience"),
            "availability": answer("availability"), "contribution": answer("contribution"),
            "focus": answer("focus")
        });
        let required_keys: &[&str] = if kind == ApplicationKind::Pate {
            &["experience", "availability"]
        } else {
            &["motivation", "experience", "availability", "contribution"]
        };
        if required_keys.iter().any(|key| {
            answers[*key]
                .as_str()
                .is_none_or(|v| v.chars().count() < 10)
        }) {
            return safe_reply("Bitte fülle alle Pflichtfelder mit mindestens 10 Zeichen aus.");
        }
        let mut applicant_name = sanitize_text(
            if interaction.author_display_name.trim().is_empty() {
                &interaction.author_name
            } else {
                &interaction.author_display_name
            },
            80,
        );
        if applicant_name.is_empty() {
            applicant_name = "Discord-Nutzer".to_string();
        }
        let (Ok(guild_id), Ok(applicant_user_id)) = (
            i64::try_from(interaction.guild_id),
            i64::try_from(interaction.user_id),
        ) else {
            return safe_reply("Deine Discord-ID konnte nicht sicher verarbeitet werden.");
        };
        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(error) => {
                tracing::warn!(%error, "Team-Bewerbungs-Transaktion konnte nicht gestartet werden");
                return safe_reply("Deine Bewerbung konnte gerade nicht sicher gespeichert werden. Bitte versuche es später noch einmal.");
            }
        };
        match dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, applicant_user_id).await {
            Ok(true) => {
                return safe_reply("Du hast der Verarbeitung deiner Community-Daten widersprochen. Eine Bewerbung kann deshalb nicht gespeichert werden.");
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "Datenschutzstatus für Team-Bewerbung konnte nicht geprüft werden");
                return safe_reply("Deine Bewerbung konnte gerade nicht sicher gespeichert werden. Bitte versuche es später noch einmal.");
            }
        }
        let inserted = sqlx::query(
            r#"INSERT INTO community.team_applications
               (guild_id, applicant_user_id, applicant_name, kind, answers, status)
               VALUES ($1, $2, $3, $4, $5, 'publishing')
               ON CONFLICT (guild_id, applicant_user_id, kind)
               WHERE status IN ('publishing', 'open', 'review', 'question')
               DO NOTHING RETURNING id"#,
        )
        .bind(guild_id)
        .bind(applicant_user_id)
        .bind(&applicant_name)
        .bind(kind.slug())
        .bind(&answers)
        .fetch_optional(&mut *tx)
        .await;
        let id: i64 = match inserted {
            Ok(Some(row)) => match row.try_get("id") { Ok(id) => id, Err(_) => return safe_reply("Deine Bewerbung konnte gerade nicht sicher gespeichert werden. Bitte versuche es später noch einmal.") },
            Ok(None) => return safe_reply("Für diesen Bereich ist bereits eine Bewerbung von dir offen. Wir melden uns, sobald wir sie geprüft haben."),
            Err(error) => {
                tracing::warn!(%error, "Team-Bewerbung konnte nicht gespeichert werden");
                return safe_reply("Deine Bewerbung konnte gerade nicht sicher gespeichert werden. Bitte versuche es später noch einmal.");
            }
        };
        if let Err(error) = tx.commit().await {
            tracing::warn!(%error, id, "Team-Bewerbungs-Transaktion konnte nicht bestätigt werden");
            return safe_reply("Deine Bewerbung konnte gerade nicht sicher gespeichert werden. Bitte versuche es später noch einmal.");
        }
        let claimed = sqlx::query_scalar::<_, i64>(
            r#"UPDATE community.team_applications
                  SET publication_started_at=now(),
                      publication_last_attempt_at=now(),
                      updated_at=now()
                WHERE id=$1 AND status='publishing'
                  AND publication_started_at IS NULL
              RETURNING id"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await;
        match claimed {
            Ok(Some(_)) => {}
            Ok(None) => {
                return safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch an das Moderationsteam zugestellt. Bitte reiche sie nicht noch einmal ein.");
            }
            Err(error) => {
                tracing::warn!(%error, id, "Team-Bewerbung konnte nicht für die Veröffentlichung reserviert werden");
                return safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch an das Moderationsteam zugestellt. Bitte reiche sie nicht noch einmal ein.");
            }
        }
        if let Err(reply) = self
            .publish_application(
                id,
                interaction.user_id,
                &applicant_name,
                kind,
                &answers,
                None,
            )
            .await
        {
            return reply;
        }
        safe_reply(
            "Danke! Deine Bewerbung ist bei uns angekommen. Wir melden uns per Discord-DM bei dir.",
        )
    }

    async fn publish_application(
        &self,
        id: i64,
        applicant_user_id: u64,
        applicant_name: &str,
        kind: ApplicationKind,
        answers: &Value,
        existing_message_id: Option<u64>,
    ) -> Result<(), BridgeReply> {
        let message_id = if let Some(message_id) = existing_message_id {
            message_id
        } else {
            let mut placeholder = publication_placeholder();
            add_nonce(&mut placeholder, publication_nonce(id));
            let message_id = match self
                .port
                .post_moderator_application(TEAM_APPLICATION_NOTIFY_CHANNEL_ID, placeholder)
                .await
            {
                Ok(message_id) => message_id,
                Err(error) => {
                    tracing::warn!(%error, id, "Neutraler Platzhalter für Team-Bewerbung konnte noch nicht veröffentlicht werden");
                    return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch an das Moderationsteam zugestellt. Bitte reiche sie nicht noch einmal ein."));
                }
            };
            match self.stage_publication_message(id, message_id).await {
                Ok(stored_message_id) => stored_message_id,
                Err(reply) => return Err(reply),
            }
        };

        let Ok(message_db_id) = i64::try_from(message_id) else {
            tracing::error!(
                id,
                message_id,
                "Discord-Snowflake liegt außerhalb des DB-Bereichs"
            );
            return Err(self.compensate_published_message(id, message_id).await);
        };
        let Ok(applicant_db_id) = i64::try_from(applicant_user_id) else {
            return Err(self.compensate_published_message(id, message_id).await);
        };
        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(error) => {
                tracing::warn!(%error, id, "Datenschutztransaktion vor Team-Bewerbungs-Edit konnte nicht gestartet werden");
                return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch fertiggestellt. Bitte reiche sie nicht noch einmal ein."));
            }
        };
        match dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, applicant_db_id).await {
            Ok(false) => {}
            Ok(true) => {
                let _ = tx.rollback().await;
                return Err(self.compensate_published_message(id, message_id).await);
            }
            Err(error) => {
                tracing::warn!(%error, id, "Datenschutzstatus vor Team-Bewerbungs-Edit konnte nicht geprüft werden");
                let _ = tx.rollback().await;
                return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch fertiggestellt. Bitte reiche sie nicht noch einmal ein."));
            }
        }
        let stored = sqlx::query_as::<_, (String, Option<i64>)>(
            r#"SELECT status, moderator_message_id
                 FROM community.team_applications
                WHERE id=$1 AND applicant_user_id=$2
                FOR UPDATE"#,
        )
        .bind(id)
        .bind(applicant_db_id)
        .fetch_optional(&mut *tx)
        .await;
        match stored {
            Ok(Some((status, Some(stored_message_id))))
                if status == "open" && stored_message_id == message_db_id =>
            {
                let _ = tx.commit().await;
                return Ok(());
            }
            Ok(Some((status, Some(stored_message_id))))
                if status == "publishing" && stored_message_id == message_db_id => {}
            Ok(_) => {
                let _ = tx.rollback().await;
                return Err(self.compensate_published_message(id, message_id).await);
            }
            Err(error) => {
                tracing::warn!(%error, id, "Gespeicherte Team-Bewerbung konnte vor Discord-Edit nicht bestätigt werden");
                let _ = tx.rollback().await;
                return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch fertiggestellt. Bitte reiche sie nicht noch einmal ein."));
            }
        }

        let post = moderator_post(
            id,
            applicant_user_id,
            applicant_name,
            kind,
            answers,
            ApplicationStatus::Open,
            None,
        );
        match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            self.port.edit_moderator_application(
                TEAM_APPLICATION_NOTIFY_CHANNEL_ID,
                message_id,
                post.body,
            ),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(ModeratorEditError::NotFound)) => {
                let reset = sqlx::query(
                    r#"UPDATE community.team_applications
                          SET moderator_message_id=NULL,
                              publication_last_attempt_at=NULL,
                              updated_at=now()
                        WHERE id=$1 AND status='publishing'
                          AND moderator_message_id=$2"#,
                )
                .bind(id)
                .bind(message_db_id)
                .execute(&mut *tx)
                .await;
                match reset {
                    Ok(result) if result.rows_affected() == 1 => {
                        if let Err(error) = tx.commit().await {
                            tracing::warn!(%error, id, "Gelöschter Team-Bewerbungsplatzhalter konnte nicht zurückgesetzt werden");
                        }
                    }
                    Ok(_) => {
                        let _ = tx.rollback().await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, id, "Gelöschter Team-Bewerbungsplatzhalter konnte nicht zurückgesetzt werden");
                        let _ = tx.rollback().await;
                    }
                }
                return Err(safe_reply("Der gelöschte Bewerbungsbeitrag wird automatisch neu angelegt. Bitte reiche deine Bewerbung nicht noch einmal ein."));
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, id, message_id, "Gespeicherter Team-Bewerbungsplatzhalter konnte noch nicht befüllt werden");
                let _ = tx.rollback().await;
                return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch an das Moderationsteam zugestellt. Bitte reiche sie nicht noch einmal ein."));
            }
            Err(_) => {
                tracing::warn!(
                    id,
                    message_id,
                    "Discord-Edit der Team-Bewerbung hat das Zeitlimit überschritten"
                );
                let _ = tx.rollback().await;
                return Err(safe_reply("Discord antwortet gerade zu langsam. Deine Bewerbung bleibt sicher gespeichert und wird automatisch fertiggestellt."));
            }
        }

        let finalized = sqlx::query(
            "UPDATE community.team_applications SET status='open', updated_at=now() WHERE id=$1 AND status='publishing' AND moderator_message_id=$2",
        )
        .bind(id)
        .bind(message_db_id)
        .execute(&mut *tx)
        .await;
        match finalized {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => {
                let _ = tx.rollback().await;
                return Err(self.compensate_published_message(id, message_id).await);
            }
            Err(error) => {
                tracing::error!(%error, id, message_id, "Team-Bewerbung konnte nach Mod-Post nicht finalisiert werden");
                let _ = tx.rollback().await;
                return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch fertiggestellt. Bitte reiche sie nicht noch einmal ein."));
            }
        }
        if let Err(error) = tx.commit().await {
            tracing::error!(%error, id, message_id, "Team-Bewerbungsfinalisierung konnte nicht bestätigt werden");
            return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch fertiggestellt. Bitte reiche sie nicht noch einmal ein."));
        }
        Ok(())
    }

    async fn stage_publication_message(
        &self,
        id: i64,
        message_id: u64,
    ) -> Result<u64, BridgeReply> {
        let Ok(message_db_id) = i64::try_from(message_id) else {
            tracing::error!(
                id,
                message_id,
                "Discord-Platzhalter-ID liegt außerhalb des DB-Bereichs"
            );
            let _ = self
                .port
                .delete_moderator_application(TEAM_APPLICATION_NOTIFY_CHANNEL_ID, message_id)
                .await;
            return Err(safe_reply("Deine Bewerbung ist sicher gespeichert und wird automatisch an das Moderationsteam zugestellt. Bitte reiche sie nicht noch einmal ein."));
        };
        let staged = sqlx::query(
            r#"UPDATE community.team_applications
                  SET moderator_message_id=$2, updated_at=now()
                WHERE id=$1 AND status='publishing'
                  AND moderator_message_id IS NULL"#,
        )
        .bind(id)
        .bind(message_db_id)
        .execute(&self.pool)
        .await;
        if matches!(staged, Ok(ref result) if result.rows_affected() == 1) {
            return Ok(message_id);
        }
        if let Err(ref error) = staged {
            tracing::error!(%error, id, message_id, "Team-Bewerbungsplatzhalter konnte nicht gespeichert werden");
        }

        let stored = sqlx::query_as::<_, (String, Option<i64>)>(
            "SELECT status, moderator_message_id FROM community.team_applications WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await;
        if let Ok(Some((status, Some(stored_message_id)))) = stored {
            if let Ok(stored_message_id) = u64::try_from(stored_message_id) {
                if stored_message_id != message_id {
                    let _ = self
                        .port
                        .delete_moderator_application(
                            TEAM_APPLICATION_NOTIFY_CHANNEL_ID,
                            message_id,
                        )
                        .await;
                }
                if matches!(status.as_str(), "publishing" | "open") {
                    return Ok(stored_message_id);
                }
            }
        }

        let _ = self
            .port
            .delete_moderator_application(TEAM_APPLICATION_NOTIFY_CHANNEL_ID, message_id)
            .await;
        Err(safe_reply("Deine Bewerbung ist sicher gespeichert, konnte aber noch nicht für das Moderationsteam vorbereitet werden. Bitte reiche sie nicht noch einmal ein."))
    }

    async fn compensate_published_message(&self, id: i64, message_id: u64) -> BridgeReply {
        if let Err(error) = self
            .port
            .delete_moderator_application(TEAM_APPLICATION_NOTIFY_CHANNEL_ID, message_id)
            .await
        {
            tracing::error!(%error, id, message_id, "Unvollständiger Team-Bewerbungsbeitrag konnte nicht entfernt werden");
            let Ok(queue_message_id) = i64::try_from(message_id) else {
                return safe_reply("Deine Bewerbung ist im internen Mod-Bereich sichtbar, konnte aber nicht vollständig bestätigt werden. Das Team prüft den Vorgang.");
            };
            if let Err(queue_error) = sqlx::query(
                "INSERT INTO community.team_application_discord_erasure_queue(
                     application_id, moderator_message_id, updated_at
                 ) VALUES ($1, $2, now())
                 ON CONFLICT (application_id) DO UPDATE SET
                     moderator_message_id=EXCLUDED.moderator_message_id,
                     updated_at=EXCLUDED.updated_at",
            )
            .bind(id)
            .bind(queue_message_id)
            .execute(&self.pool)
            .await
            {
                tracing::error!(%queue_error, id, message_id, "Unvollständiger Team-Bewerbungsbeitrag konnte nicht zur Löschung vorgemerkt werden");
                return safe_reply("Deine Bewerbung ist im internen Mod-Bereich sichtbar, konnte aber nicht vollständig bestätigt werden. Das Team prüft den Vorgang.");
            }
        }
        if let Err(error) = sqlx::query(
            "DELETE FROM community.team_applications WHERE id=$1 AND status='publishing'",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        {
            tracing::error!(%error, id, "Kompensierte Team-Bewerbung konnte nicht aus der Datenbank entfernt werden");
        }
        safe_reply("Deine Bewerbung konnte gerade nicht sicher gespeichert werden. Bitte versuche es später noch einmal.")
    }

    async fn change_status(
        &self,
        interaction: &BridgeInteraction,
        id: i64,
        action: &str,
        note: &str,
    ) -> BridgeReply {
        if !interaction.author_can_manage_messages {
            return safe_reply("Diese Aktion ist nur für das Moderationsteam verfügbar.");
        }
        if interaction.channel_id != TEAM_APPLICATION_NOTIFY_CHANNEL_ID {
            return safe_reply("Statusaktionen sind nur am Bewerbungsbeitrag im Mod-Chat möglich.");
        }
        let Some(interaction_message_id) = interaction.message_id else {
            return safe_reply("Der Bewerbungsbeitrag konnte nicht eindeutig zugeordnet werden.");
        };
        let (Ok(reviewer_user_id), Ok(guild_id), Ok(message_id)) = (
            i64::try_from(interaction.user_id),
            i64::try_from(self.guild_id),
            i64::try_from(interaction_message_id),
        ) else {
            return safe_reply("Die Discord-IDs konnten nicht sicher verarbeitet werden.");
        };
        let target = match action {
            "review" => ApplicationStatus::Review,
            "question" => ApplicationStatus::Question,
            "accept" => ApplicationStatus::Accepted,
            "reject" => ApplicationStatus::Rejected,
            _ => return safe_reply("Unbekannte Statusaktion."),
        };
        let note = sanitize_text(note, APPLICATION_ANSWER_MAX_LENGTH);
        if matches!(
            target,
            ApplicationStatus::Question | ApplicationStatus::Rejected
        ) && note.chars().count() < 10
        {
            return safe_reply(
                "Bitte gib eine verständliche Nachricht mit mindestens 10 Zeichen ein.",
            );
        }
        let note_param = if note.is_empty() {
            None
        } else {
            Some(note.as_str())
        };

        let record = match sqlx::query(
            r#"UPDATE community.team_applications
               SET status=$1,
                   reviewer_user_id=$2,
                   status_note=$3,
                   status_version=CASE
                       WHEN status=$1 AND status_note IS NOT DISTINCT FROM $3
                       THEN status_version
                       ELSE status_version + 1
                   END,
                   status_dm_sent_at=CASE
                       WHEN status=$1 AND status_note IS NOT DISTINCT FROM $3
                       THEN status_dm_sent_at
                       ELSE NULL
                   END,
                   status_dm_claimed_at=CASE
                       WHEN status=$1 AND status_note IS NOT DISTINCT FROM $3
                       THEN status_dm_claimed_at
                       ELSE NULL
                   END,
                   status_dm_dispatch_started_at=CASE
                       WHEN status=$1 AND status_note IS NOT DISTINCT FROM $3
                       THEN status_dm_dispatch_started_at
                       ELSE NULL
                   END,
                   status_dm_audit_claimed_at=CASE
                       WHEN status=$1 AND status_note IS NOT DISTINCT FROM $3
                       THEN status_dm_audit_claimed_at
                       ELSE NULL
                   END,
                   status_dm_audited_at=CASE
                       WHEN status=$1 AND status_note IS NOT DISTINCT FROM $3
                       THEN status_dm_audited_at
                       ELSE NULL
                   END,
                   updated_at=now()
               WHERE id=$4 AND guild_id=$5
                 AND moderator_message_id=$6
                 AND (
                     status IN ('open', 'review', 'question')
                     OR (status=$1 AND status_note IS NOT DISTINCT FROM $3)
                 )
               RETURNING applicant_user_id, applicant_name, kind, answers,
                         moderator_message_id, status_dm_claimed_at,
                         status_dm_dispatch_started_at, status_dm_sent_at,
                         status_dm_audit_claimed_at, status_dm_audited_at,
                         status_version"#,
        )
        .bind(target.slug())
        .bind(reviewer_user_id)
        .bind(note_param)
        .bind(id)
        .bind(guild_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(Some(row)) => row,
            Ok(None) => {
                return safe_reply(
                    "Die Bewerbung ist bereits abgeschlossen oder wurde nicht gefunden.",
                )
            }
            Err(error) => {
                tracing::warn!(%error, id, "Team-Bewerbungsstatus konnte nicht gespeichert werden");
                return safe_reply("Der Status konnte gerade nicht sicher geändert werden.");
            }
        };

        let kind = record
            .try_get::<String, _>("kind")
            .ok()
            .and_then(|value| ApplicationKind::from_slug(&value));
        let applicant_user_id = record
            .try_get::<i64, _>("applicant_user_id")
            .ok()
            .and_then(|value| u64::try_from(value).ok());
        let applicant_name = record.try_get::<String, _>("applicant_name").ok();
        let answers = record.try_get::<Value, _>("answers").ok();
        let message_id = record
            .try_get::<Option<i64>, _>("moderator_message_id")
            .ok()
            .flatten()
            .and_then(|value| u64::try_from(value).ok());
        let dm_already_sent = record
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("status_dm_sent_at")
            .ok()
            .flatten()
            .is_some();
        let dm_already_claimed = record
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("status_dm_claimed_at")
            .ok()
            .flatten()
            .is_some();
        let status_version = record.try_get::<i32, _>("status_version").ok();
        let (
            Some(kind),
            Some(applicant_user_id),
            Some(applicant_name),
            Some(answers),
            Some(status_version),
        ) = (
            kind,
            applicant_user_id,
            applicant_name,
            answers,
            status_version,
        )
        else {
            tracing::error!(id, "Team-Bewerbung enthält unvollständige Metadaten");
            return safe_reply("Der Status wurde gespeichert, aber die Anzeige ist unvollständig.");
        };

        if target == ApplicationStatus::Accepted && kind == ApplicationKind::Pate {
            if let Err(error) = self
                .port
                .add_role(
                    self.guild_id,
                    applicant_user_id,
                    crate::concierge::PATE_ROLE_ID,
                    "team-bewerbung:pate-angenommen",
                )
                .await
            {
                tracing::warn!(%error, id, "Paten-Rolle konnte nach Annahme nicht vergeben werden");
                return safe_reply(
                    "Die Annahme wurde gespeichert, aber die Paten-Rolle konnte nicht vergeben werden. Bitte nimm die Bewerbung gleich noch einmal an, dann versucht der Bot die Rolle erneut. Erst danach geht die Willkommensnachricht raus.",
                );
            }
        }

        let mut delivery_warning = None;
        if !dm_already_sent {
            let claimed_now = if dm_already_claimed {
                false
            } else {
                match sqlx::query_scalar::<_, i64>(
                    r#"UPDATE community.team_applications
                          SET status_dm_claimed_at=now(), updated_at=now()
                        WHERE id=$1 AND status=$2 AND status_version=$3
                          AND status_dm_claimed_at IS NULL
                      RETURNING id"#,
                )
                .bind(id)
                .bind(target.slug())
                .bind(status_version)
                .fetch_optional(&self.pool)
                .await
                {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(error) => {
                        tracing::error!(%error, id, "Team-Bewerbungs-DM konnte nicht reserviert werden");
                        delivery_warning = Some("Der Status wurde gespeichert, aber die DM-Zustellung konnte nicht reserviert werden. Bitte kontaktiere den Bewerber manuell.");
                        false
                    }
                }
            };
            let dispatch_started = if claimed_now {
                match sqlx::query_scalar::<_, i64>(
                    r#"UPDATE community.team_applications
                          SET status_dm_dispatch_started_at=now(), updated_at=now()
                        WHERE id=$1 AND status=$2 AND status_version=$3
                          AND status_dm_claimed_at IS NOT NULL
                          AND status_dm_dispatch_started_at IS NULL
                      RETURNING id"#,
                )
                .bind(id)
                .bind(target.slug())
                .bind(status_version)
                .fetch_optional(&self.pool)
                .await
                {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(error) => {
                        tracing::error!(%error, id, "Start der Team-Bewerbungs-DM konnte nicht gespeichert werden");
                        delivery_warning = Some("Der Status wurde gespeichert, aber der DM-Versand konnte nicht gestartet werden. Der Vorgang wird automatisch wieder freigegeben.");
                        false
                    }
                }
            } else {
                false
            };
            if dispatch_started {
                let dm_text = status_dm_text(target, kind, &note);
                let mut dm = Map::new();
                dm.insert("content".into(), json!(dm_text));
                dm.insert("allowed_mentions".into(), json!({"parse": []}));
                add_nonce(&mut dm, status_dm_nonce(id, status_version));
                if let Err(error) = self.port.send_dm(applicant_user_id, dm).await {
                    tracing::warn!(%error, id, "Team-Bewerbungs-DM konnte nicht eindeutig zugestellt werden");
                    delivery_warning = Some("Der Status wurde gespeichert, aber die DM-Zustellung ist unklar. Zum Schutz vor Doppelversand wird sie nicht automatisch wiederholt; bitte kontaktiere den Bewerber manuell.");
                } else {
                    match sqlx::query(
                        "UPDATE community.team_applications SET status_dm_sent_at=now(), updated_at=now() WHERE id=$1 AND status=$2 AND status_version=$3 AND status_dm_dispatch_started_at IS NOT NULL",
                    )
                    .bind(id)
                    .bind(target.slug())
                    .bind(status_version)
                    .execute(&self.pool)
                    .await
                    {
                        Ok(result) if result.rows_affected() == 1 => {}
                        Ok(_) => {
                            tracing::error!(id, "Zugestellte Team-Bewerbungs-DM konnte keinem aktuellen Status zugeordnet werden");
                            delivery_warning = Some("Die DM wurde zugestellt, konnte intern aber nicht bestätigt werden. Bitte nicht erneut senden.");
                        }
                        Err(error) => {
                            tracing::error!(%error, id, "Zugestellte Team-Bewerbungs-DM konnte nicht bestätigt werden");
                            delivery_warning = Some("Die DM wurde zugestellt, konnte intern aber nicht bestätigt werden. Bitte nicht erneut senden.");
                        }
                    }
                }
            } else if delivery_warning.is_none() {
                delivery_warning = Some("Der Status wurde gespeichert. Die DM wurde bereits reserviert oder ihre Zustellung ist noch unklar; bitte nicht erneut senden.");
            }
        }

        if let Some(message_id) = message_id {
            let post = moderator_post(
                id,
                applicant_user_id,
                &applicant_name,
                kind,
                &answers,
                target,
                if note.is_empty() { None } else { Some(&note) },
            );
            if let Err(error) = self
                .port
                .edit_moderator_application(
                    TEAM_APPLICATION_NOTIFY_CHANNEL_ID,
                    message_id,
                    post.body,
                )
                .await
            {
                tracing::warn!(%error, id, "Team-Bewerbungs-Post im Mod-Chat konnte nicht aktualisiert werden");
                return safe_reply("Der Status wurde gespeichert, aber der Mod-Beitrag konnte nicht aktualisiert werden. Bitte prüfe den Vorgang im Mod-Chat.");
            }
        }
        if let Some(warning) = delivery_warning {
            return safe_reply(warning);
        }
        safe_reply(&format!("Status auf „{}“ gesetzt.", target.label()))
    }
}

pub const PATE_ACCEPTED_DM_TEXT: &str = "Schön, dass du dabei bist. Ab jetzt bist du Pate in der Deutschen Deadlock Community. Wie alles läuft, steht in deinem Leitfaden in der Paten-Zentrale: <#1524083665838276860>. Schau kurz rein, dann kann es losgehen.";

fn status_dm_text(status: ApplicationStatus, kind: ApplicationKind, note: &str) -> String {
    if status == ApplicationStatus::Accepted && kind == ApplicationKind::Pate {
        let note = if note.is_empty() {
            String::new()
        } else {
            format!("\n\n{note}")
        };
        return format!("{PATE_ACCEPTED_DM_TEXT}{note}");
    }
    let heading = match status {
        ApplicationStatus::Review => "Deine Bewerbung wird jetzt geprüft.",
        ApplicationStatus::Question => "Wir haben eine Rückfrage zu deiner Bewerbung.",
        ApplicationStatus::Accepted => "Wir möchten dich gern in unser Team aufnehmen!",
        ApplicationStatus::Rejected => "Wir können deine Bewerbung diesmal leider nicht annehmen.",
        ApplicationStatus::Publishing | ApplicationStatus::Open => "Deine Bewerbung ist offen.",
    };
    let note = if note.is_empty() {
        String::new()
    } else {
        format!("\n\n{note}")
    };
    format!("**Team-Bewerbung · {}**\n{heading}{note}", kind.label())
}

fn safe_reply(content: &str) -> BridgeReply {
    BridgeReply {
        content: Some(content.to_string()),
        ephemeral: true,
        allowed_mentions: Some(json!({"parse": []})),
        ..BridgeReply::default()
    }
}

fn moderator_post(
    id: i64,
    user_id: u64,
    name: &str,
    kind: ApplicationKind,
    answers: &Value,
    status: ApplicationStatus,
    status_note: Option<&str>,
) -> ModeratorPost {
    let sections = [
        ("Motivation", "motivation"),
        ("Erfahrung und Stärken", "experience"),
        ("Verfügbarkeit", "availability"),
        ("Konkreter Beitrag", "contribution"),
        ("Schwerpunkt", "focus"),
    ]
    .into_iter()
    .map(|(label, key)| {
        display(format!(
            "**{label}**\n{}",
            answers[key]
                .as_str()
                .filter(|v| !v.is_empty())
                .unwrap_or("—")
        ))
    })
    .collect::<Vec<_>>();
    let mut body = Map::new();
    let status_note = status_note
        .filter(|value| !value.is_empty())
        .map(|value| format!("\n**Notiz:** {value}"))
        .unwrap_or_default();
    body.insert("flags".into(), json!(COMPONENTS_V2_FLAG));
    body.insert("allowed_mentions".into(), json!({"parse": []}));
    let mut components = vec![
        display(format!("## Team-Bewerbung\n**Wer:** {name} (`{user_id}`)\n**Bereich:** {}\n**Status:** {}{status_note}", kind.label(), status.label())),
        json!({"type": 14, "divider": true, "spacing": 1}),
    ];
    components.extend(sections);
    components.push(json!({"type": 14, "divider": true, "spacing": 1}));
    components.push(staff_components(id, status)[0].clone());
    body.insert(
        "components".into(),
        json!([{"type": 17, "accent_color": 0xC8A86B, "components": components}]),
    );
    ModeratorPost { body }
}

struct Handler {
    service: Arc<TeamApplications>,
}

#[async_trait::async_trait]
impl InteractionHandler for Handler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.guild_id != self.service.guild_id {
            return safe_reply("Diese Bewerbungsaktion ist auf diesem Server nicht verfügbar.");
        }
        if let Some(slug) = interaction.custom_id.strip_prefix("team_apply:open:") {
            if !self.service.publish_enabled {
                return safe_reply("Das Bewerbungsportal ist noch nicht freigeschaltet.");
            }
            return ApplicationKind::from_slug(slug).map_or_else(
                || safe_reply("Unbekannter Bewerbungsbereich."),
                |kind| {
                    if kind == ApplicationKind::Pate {
                        pate_intro_reply(&self.service.pate_text)
                    } else {
                        BridgeReply {
                            modal: Some(application_modal(kind)),
                            ..BridgeReply::default()
                        }
                    }
                },
            );
        }
        if let Some(slug) = interaction.custom_id.strip_prefix("team_apply:form:") {
            if !self.service.publish_enabled {
                return safe_reply("Das Bewerbungsportal ist noch nicht freigeschaltet.");
            }
            return ApplicationKind::from_slug(slug).map_or_else(
                || safe_reply("Unbekannter Bewerbungsbereich."),
                |kind| {
                    let modal = if kind == ApplicationKind::Pate {
                        pate_application_modal(&self.service.pate_text)
                    } else {
                        application_modal(kind)
                    };
                    BridgeReply {
                        modal: Some(modal),
                        ..BridgeReply::default()
                    }
                },
            );
        }
        if let Some(slug) = interaction.custom_id.strip_prefix("team_apply:submit:") {
            if !self.service.publish_enabled {
                return safe_reply("Das Bewerbungsportal ist noch nicht freigeschaltet.");
            }
            return match ApplicationKind::from_slug(slug) {
                Some(kind) => self.service.submit(&interaction, kind).await,
                None => safe_reply("Unbekannter Bewerbungsbereich."),
            };
        }
        if let Some(rest) = interaction.custom_id.strip_prefix("team_apply:status:") {
            let Some((action, raw_id)) = rest.split_once(':') else {
                return safe_reply("Ungültige Statusaktion.");
            };
            let Ok(id) = raw_id.parse::<i64>() else {
                return safe_reply("Ungültige Bewerbungsnummer.");
            };
            if !interaction.author_can_manage_messages {
                return safe_reply("Diese Aktion ist nur für das Moderationsteam verfügbar.");
            }
            if action == "review" {
                return self
                    .service
                    .change_status(&interaction, id, action, "")
                    .await;
            }
            return decision_modal(action, id).map_or_else(
                || safe_reply("Unbekannte Statusaktion."),
                |modal| BridgeReply {
                    modal: Some(modal),
                    ..BridgeReply::default()
                },
            );
        }
        if let Some(rest) = interaction.custom_id.strip_prefix("team_apply:decision:") {
            let Some((action, raw_id)) = rest.split_once(':') else {
                return safe_reply("Ungültige Statusaktion.");
            };
            let Ok(id) = raw_id.parse::<i64>() else {
                return safe_reply("Ungültige Bewerbungsnummer.");
            };
            let note = interaction
                .options
                .get("note")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return self
                .service
                .change_status(&interaction, id, action, note)
                .await;
        }
        safe_reply("Unbekannte Bewerbungsaktion.")
    }
}

pub fn register(router: &mut InteractionRouter, service: Arc<TeamApplications>) {
    router.on_prefix("team_apply:", Arc::new(Handler { service }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn alle_teamwege_sind_vollstaendig_und_streamer_bleibt_externer_link() {
        assert_eq!(ApplicationKind::ALL.len(), 7);
        assert_eq!(ApplicationKind::Moderation.slug(), "moderation");
        assert_eq!(ApplicationKind::Coach.slug(), "coach");
        assert_eq!(ApplicationKind::Caster.slug(), "caster");
        assert_eq!(ApplicationKind::Tournament.slug(), "turnier");
        assert_eq!(ApplicationKind::Coder.slug(), "coder");
        assert_eq!(ApplicationKind::Pate.slug(), "pate");
        assert_eq!(ApplicationKind::Other.slug(), "sonstiges");

        let panel = panel_body(&PanelText::default());
        assert_eq!(panel["flags"], json!(COMPONENTS_V2_FLAG));
        assert_eq!(panel["allowed_mentions"], json!({"parse": []}));
        let raw = serde_json::to_string(&panel).expect("panel json");
        for kind in ApplicationKind::ALL {
            assert!(raw.contains(&format!("team_apply:open:{}", kind.slug())));
        }
        assert!(raw.contains(STREAMER_PARTNER_URL));
        assert!(!raw.contains("team_apply:open:streamer"));
    }

    #[test]
    fn jedes_modal_bleibt_unter_discords_fuenf_felder_grenze() {
        for kind in ApplicationKind::ALL {
            let modal = application_modal(kind);
            assert!(!modal.fields.is_empty());
            assert!(modal.fields.len() <= 5);
            assert!(modal
                .fields
                .iter()
                .all(|field| field.max_length as usize <= APPLICATION_ANSWER_MAX_LENGTH));
        }
    }

    #[test]
    fn pate_vorschalt_zeigt_intro_und_knopf_zum_formular() {
        let text = PateText::default();
        let reply = pate_intro_reply(&text);
        let raw = serde_json::to_string(&reply.components.clone().expect("components"))
            .expect("components json");
        assert!(raw.contains(text.intro.as_str()));
        assert!(raw.contains("team_apply:form:pate"));
        assert_eq!(reply.message_flags, Some(COMPONENTS_V2_FLAG | 64));

        for kind in [
            ApplicationKind::Moderation,
            ApplicationKind::Coach,
            ApplicationKind::Caster,
            ApplicationKind::Tournament,
            ApplicationKind::Coder,
            ApplicationKind::Other,
        ] {
            let modal = application_modal(kind);
            assert!(modal
                .fields
                .iter()
                .all(|field| !field.placeholder.contains(text.intro.as_str())
                    && !field.label.contains(text.intro.as_str())));
        }
        assert!(pate_application_modal(&text)
            .fields
            .iter()
            .all(|field| !field.placeholder.contains(text.intro.as_str())
                && !field.label.contains(text.intro.as_str())));
    }

    #[test]
    fn nutzertext_wird_mention_sicher_begrenzt_und_unicode_bleibt_intakt() {
        let value = sanitize_text("  Hallo <@123> @everyone 😊\n\nWelt  ", 24);
        assert!(!value.contains("@everyone"));
        assert!(!value.contains("<@123>"));
        assert!(value.contains('😊'));
        assert!(value.chars().count() <= 24);
    }

    #[test]
    fn staff_buttons_enthalten_alle_status_aktionen() {
        let components = staff_components(42, ApplicationStatus::Open);
        let raw = serde_json::to_string(&components).expect("components json");
        for action in ["review", "question", "accept", "reject"] {
            assert!(raw.contains(&format!("team_apply:status:{action}:42")));
        }
    }

    #[test]
    fn unbekannte_oder_leere_kategorie_wird_abgewiesen() {
        assert_eq!(ApplicationKind::from_slug(""), None);
        assert_eq!(ApplicationKind::from_slug("streamer"), None);
        assert_eq!(ApplicationKind::from_slug("MODERATION"), None);
    }

    #[test]
    fn panel_publish_ist_nach_freigabe_eingeschaltet() {
        let file = toml::from_str::<TextFile>(include_str!(
            "../../../../assets/team_application_texts.toml"
        ))
        .expect("Team-Bewerbungstexte");
        assert!(file.publish.enabled);
    }

    #[test]
    fn status_modals_trennen_rueckfrage_annahme_und_ablehnung() {
        for action in ["question", "accept", "reject"] {
            let modal = decision_modal(action, 42).expect("Statusmodal");
            assert_eq!(modal.custom_id, format!("team_apply:decision:{action}:42"));
            assert_eq!(modal.fields.len(), 1);
        }
        assert!(decision_modal("review", 42).is_none());
    }

    #[test]
    fn maximale_antworten_bleiben_in_eigenen_textanzeigen() {
        let long = "ä".repeat(APPLICATION_ANSWER_MAX_LENGTH);
        let answers = json!({
            "motivation": long,
            "experience": "b".repeat(APPLICATION_ANSWER_MAX_LENGTH),
            "availability": "c".repeat(APPLICATION_ANSWER_MAX_LENGTH),
            "contribution": "d".repeat(APPLICATION_ANSWER_MAX_LENGTH),
            "focus": "e".repeat(APPLICATION_ANSWER_MAX_LENGTH)
        });
        let post = moderator_post(
            42,
            u64::MAX,
            &"N".repeat(80),
            ApplicationKind::Tournament,
            &answers,
            ApplicationStatus::Question,
            Some(&"Z".repeat(APPLICATION_ANSWER_MAX_LENGTH)),
        );
        let inner = post.body["components"][0]["components"]
            .as_array()
            .expect("container components");
        assert!(inner.len() <= 10);
        let text_displays = inner
            .iter()
            .filter(|component| component["type"] == json!(10))
            .collect::<Vec<_>>();
        assert_eq!(text_displays.len(), 6);
        assert!(text_displays.iter().all(|component| component["content"]
            .as_str()
            .is_some_and(|content| content.chars().count() <= 4_000)));
        let total_text_length = text_displays
            .iter()
            .filter_map(|component| component["content"].as_str())
            .map(str::chars)
            .map(Iterator::count)
            .sum::<usize>();
        assert!(total_text_length <= 4_000);
    }

    #[test]
    fn discord_nonces_bleiben_eindeutig_und_unter_25_zeichen() {
        let publication = publication_nonce(i64::MAX);
        let first_status = status_dm_nonce(i64::MAX, i32::MAX);
        let next_status = status_dm_nonce(i64::MAX, i32::MAX - 1);
        let audit = status_dm_audit_nonce(i64::MAX, i32::MAX);
        assert!(publication.len() <= 25);
        assert!(first_status.len() <= 25);
        assert!(audit.len() <= 25);
        assert_ne!(first_status, next_status);
    }

    #[test]
    fn unverankerter_platzhalter_enthaelt_keine_bewerberdaten() {
        let raw = serde_json::to_string(&publication_placeholder()).expect("placeholder json");
        assert!(raw.contains("sicher vorbereitet"));
        assert!(!raw.contains("applicant"));
        assert!(!raw.contains("motivation"));
        assert!(!raw.contains("user_id"));
    }

    #[test]
    fn pate_bewerbungsformular_stellt_erfahrung_und_verfuegbarkeit() {
        let modal = pate_application_modal(&PateText::default());
        assert_eq!(modal.custom_id, "team_apply:submit:pate");
        assert_eq!(modal.fields.len(), 2);
        assert_eq!(modal.fields[0].custom_id, "experience");
        assert_eq!(modal.fields[1].custom_id, "availability");
    }

    #[cfg(feature = "testing")]
    #[derive(Default)]
    struct RecordingTeamPort {
        added_roles: std::sync::Mutex<Vec<(u64, u64, u64)>>,
        sent_dms: std::sync::Mutex<Vec<(u64, Map<String, Value>)>>,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl TeamApplicationPort for RecordingTeamPort {
        async fn post_panel(
            &self,
            _channel_id: u64,
            _body: Map<String, Value>,
        ) -> Result<u64, String> {
            Ok(1)
        }
        async fn edit_panel(
            &self,
            _channel_id: u64,
            _message_id: u64,
            _body: Map<String, Value>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn post_moderator_application(
            &self,
            _channel_id: u64,
            _body: Map<String, Value>,
        ) -> Result<u64, String> {
            Ok(1)
        }
        async fn edit_moderator_application(
            &self,
            _channel_id: u64,
            _message_id: u64,
            _body: Map<String, Value>,
        ) -> Result<(), ModeratorEditError> {
            Ok(())
        }
        async fn delete_moderator_application(
            &self,
            _channel_id: u64,
            _message_id: u64,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn send_dm(&self, user_id: u64, body: Map<String, Value>) -> Result<(), String> {
            self.sent_dms
                .lock()
                .expect("sent_dms lock")
                .push((user_id, body));
            Ok(())
        }
        async fn add_role(
            &self,
            guild_id: u64,
            user_id: u64,
            role_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            self.added_roles
                .lock()
                .expect("added_roles lock")
                .push((guild_id, user_id, role_id));
            Ok(())
        }
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn angenommene_paten_bewerbung_vergibt_paten_rolle_und_schickt_leitfaden_dm() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let guild_id: u64 = 1_289_721_245_281_292_288;
        let applicant: u64 = 424_242;
        let moderator_message_id: u64 = 999_001;
        sqlx::query(
            "INSERT INTO community.team_applications(
                 id, guild_id, applicant_user_id, applicant_name, kind, answers, status,
                 moderator_message_id, status_version, created_at, updated_at)
             VALUES(1, $1, $2, 'Testpate', 'pate',
                    '{\"experience\":\"kenne den Server gut\",\"availability\":\"abends\"}'::jsonb,
                    'open', $3, 0, now(), now())",
        )
        .bind(i64::try_from(guild_id).expect("guild_id i64"))
        .bind(i64::try_from(applicant).expect("applicant i64"))
        .bind(i64::try_from(moderator_message_id).expect("message_id i64"))
        .execute(&pool)
        .await
        .expect("Pate-Bewerbung seeden");

        let port = Arc::new(RecordingTeamPort::default());
        let service = Arc::new(TeamApplications {
            pool: pool.clone(),
            port: port.clone(),
            guild_id,
            panel_text: PanelText::default(),
            pate_text: PateText::default(),
            publish_enabled: true,
        });

        let interaction = BridgeInteraction {
            guild_id,
            channel_id: TEAM_APPLICATION_NOTIFY_CHANNEL_ID,
            user_id: 777,
            message_id: Some(moderator_message_id),
            author_can_manage_messages: true,
            ..BridgeInteraction::default()
        };
        let reply = service.change_status(&interaction, 1, "accept", "").await;
        assert!(reply.content.is_some());

        let roles = port.added_roles.lock().expect("added_roles lock");
        assert_eq!(roles.len(), 1);
        assert_eq!(
            roles[0],
            (guild_id, applicant, crate::concierge::PATE_ROLE_ID)
        );
        drop(roles);

        let dms = port.sent_dms.lock().expect("sent_dms lock");
        assert_eq!(dms.len(), 1);
        let raw = serde_json::to_string(&dms[0].1).expect("dm json");
        assert!(
            raw.contains("Paten-Zentrale") || raw.contains("1524083665838276860"),
            "{raw}"
        );
    }
}
