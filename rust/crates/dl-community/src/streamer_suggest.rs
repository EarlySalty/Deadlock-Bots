//! Streamer vorschlagen (Community-Streamer-Brücke, Paket F, Discord-Seite).
//!
//! Button "Streamer vorschlagen" am Clip-Panel (`clips.rs`, Kanal
//! [`PANEL_CHANNEL_ID`]) → Modal (Twitch-Kanal oder Link, warum) → Speichern in
//! `community.streamer_suggestions` → Weitergabe an den Twitch-Bot
//! (`POST /internal/twitch/v1/scout/community-suggestion`, Idempotency-Key
//! `discord-suggest-<id>`). Kommt keine Antwort, bleibt `forwarded_at` NULL und
//! [`spawn`] versucht es mit wachsendem Abstand erneut, bis der Twitch-Bot
//! antwortet. Punkte vergibt Paket C aus dem Outcomes-Endpunkt, nicht dieses Modul.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};
use sqlx::{PgPool, Row as _};

use crate::db::u64_to_i64;

/// Kanal mit dem Button: das Clip-Panel im Community-Bereich. Ein eigener
/// Vorschlags-Kanal existiert nicht; das Clip-Panel wird ohnehin alle 5 Minuten
/// aktualisiert und braucht deshalb keinen eigenen Publisher.
pub const PANEL_CHANNEL_ID: u64 = crate::clips::SUBMIT_CHANNEL_ID;
pub const OPEN_CUSTOM_ID: &str = "streamer_suggest_btn_v1";
pub const MODAL_CUSTOM_ID: &str = "streamer_suggest_modal_v1";
pub const FIELD_CHANNEL: &str = "twitch_channel";
pub const FIELD_REASON: &str = "reason";
pub const BUTTON_LABEL: &str = "Streamer vorschlagen";
/// Höchstens so viele neue Vorschläge je Mitglied in 24 Stunden.
pub const DAILY_LIMIT: i64 = 3;
pub const MIN_REASON_CHARS: usize = 3;
pub const MAX_REASON_CHARS: usize = 300;
/// Erster Neuversuch nach 5 Minuten, dann doppelt so lange, höchstens 6 Stunden.
pub const RETRY_BASE_MINUTES: i32 = 5;
pub const RETRY_MAX_MINUTES: i32 = 360;
pub const RETRY_BATCH: i64 = 20;
const RETRY_TICK: Duration = Duration::from_secs(120);
/// Die Interaction wird nach 2 Sekunden automatisch verzögert; länger als das
/// soll das Mitglied trotzdem nicht warten.
const INLINE_FORWARD_TIMEOUT: Duration = Duration::from_secs(8);

/// Kanal, in dem Partner-Streams angekündigt werden.
const PARTNER_STREAMS_CHANNEL_ID: u64 = 1_304_169_815_505_637_458;

/// Hinweis im Clip-Panel unter den Clip-Regeln.
pub const PANEL_HINT: &str = "**Streamer vorschlagen:** Du kennst einen Kanal, der Deadlock streamt und zu uns passt? Schlag ihn mit dem zweiten Knopf vor.";

pub const TEXT_INVALID_CHANNEL: &str = "Das sieht nicht nach einem Twitch-Kanal aus. Gib bitte den Kanalnamen oder den Link ein, zum Beispiel twitch.tv/name.";
pub const TEXT_REASON_MISSING: &str = "Schreib bitte kurz dazu, warum der Kanal gut zu uns passt.";
pub const TEXT_RATE_LIMITED: &str = "Du hast in den letzten 24 Stunden schon 3 Kanäle vorgeschlagen. Danke dafür! Morgen kannst du wieder welche vorschlagen.";
pub const TEXT_OPTED_OUT: &str = "Du hast der Verarbeitung deiner Community-Daten widersprochen. Deshalb können wir deinen Vorschlag nicht speichern.";
pub const TEXT_FAILED: &str = "Das hat gerade nicht geklappt. Versuch es bitte gleich noch einmal.";
pub const TEXT_BLOCKED: &str =
    "Diesen Kanal können wir leider nicht aufnehmen. Danke trotzdem für deinen Vorschlag.";

/// Stand eines Vorschlags; entspricht der Spalte `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionStatus {
    /// Gespeichert, Twitch-Bot hat noch nicht geantwortet.
    Pending,
    Created,
    AlreadyKnown,
    AlreadyPartner,
    Blocked,
    NotFound,
    /// Twitch-Bot hat die Anfrage endgültig abgelehnt (Formfehler, Schlüsselkonflikt).
    Rejected,
}

impl SuggestionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Created => "created",
            Self::AlreadyKnown => "already_known",
            Self::AlreadyPartner => "already_partner",
            Self::Blocked => "blocked",
            Self::NotFound => "not_found",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "created" => Self::Created,
            "already_known" => Self::AlreadyKnown,
            "already_partner" => Self::AlreadyPartner,
            "blocked" => Self::Blocked,
            "not_found" => Self::NotFound,
            "rejected" => Self::Rejected,
            _ => return None,
        })
    }

    /// Antwortstatus des Twitch-Bots (nur die fünf aus dem Vertrag).
    pub fn from_remote(value: &str) -> Option<Self> {
        Self::parse(value).filter(|status| !matches!(status, Self::Pending | Self::Rejected))
    }
}

// ── Reine Logik ────────────────────────────────────────────────────────────

/// Pfade auf twitch.tv, die kein Kanal sind.
const RESERVED_TWITCH_PATHS: [&str; 14] = [
    "directory",
    "downloads",
    "drops",
    "friends",
    "inventory",
    "jobs",
    "login",
    "messages",
    "p",
    "search",
    "settings",
    "signup",
    "subscriptions",
    "videos",
];

/// Twitch-Login aus der Eingabe: `name`, `@name` oder ein Kanal-Link
/// (`twitch.tv/name`, mit oder ohne `https://`, `www.` oder `m.`). Ergebnis klein,
/// nur `a-z`, `0-9` und `_`, 1 bis 25 Zeichen; dieselbe Regel wie im Twitch-Bot.
pub fn parse_twitch_login(raw: &str) -> Option<String> {
    let mut value = raw.trim();
    for prefix in ["https://", "http://"] {
        if value
            .get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
        {
            value = &value[prefix.len()..];
        }
    }
    let mut from_link = false;
    for host in ["www.twitch.tv/", "m.twitch.tv/", "twitch.tv/"] {
        if value
            .get(..host.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(host))
        {
            value = &value[host.len()..];
            value = value.split(['/', '?', '#']).next().unwrap_or_default();
            from_link = true;
            break;
        }
    }
    if !from_link && value.contains(['/', '.', ':']) {
        // Fremde Links (YouTube, clips.twitch.tv, ...) sind kein Kanal.
        return None;
    }
    let login = value.trim_start_matches('@').to_ascii_lowercase();
    let valid = (1..=25).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !RESERVED_TWITCH_PATHS.contains(&login.as_str());
    valid.then_some(login)
}

/// Grund: Steuerzeichen werden Leerzeichen, getrimmt, höchstens
/// [`MAX_REASON_CHARS`] Zeichen. Zu kurz oder leer → `None`.
pub fn normalize_reason(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed: String = cleaned.trim().chars().take(MAX_REASON_CHARS).collect();
    let trimmed = trimmed.trim_end().to_string();
    (trimmed.chars().count() >= MIN_REASON_CHARS).then_some(trimmed)
}

pub fn idempotency_key(suggestion_id: i64) -> String {
    format!("discord-suggest-{suggestion_id}")
}

/// Login fett, Unterstriche maskiert (sonst kursiv in Discord).
fn shown(login: &str) -> String {
    format!("**{}**", login.replace('_', "\\_"))
}

/// Antworttext für einen gespeicherten Vorschlag je Stand.
pub fn status_text(status: SuggestionStatus, login: &str) -> String {
    let channel = shown(login);
    match status {
        SuggestionStatus::Created => format!(
            "Danke für deinen Vorschlag! Wir schauen uns den Kanal {channel} an. Ob wir ihn ansprechen, entscheidet unser Team. Wird der Kanal Partner in unserem Streamer-Programm, bekommst du 150 Punkte."
        ),
        SuggestionStatus::Pending | SuggestionStatus::Rejected => format!(
            "Danke für deinen Vorschlag! Wir schauen uns den Kanal {channel} an. Ob wir ihn ansprechen, entscheidet unser Team."
        ),
        SuggestionStatus::AlreadyKnown => format!(
            "Danke! Den Kanal {channel} kennen wir schon, er steht bereits auf unserer Liste."
        ),
        SuggestionStatus::AlreadyPartner => format!(
            "{channel} ist schon Partner in unserem Streamer-Programm. Schau gern in <#{PARTNER_STREAMS_CHANNEL_ID}> vorbei, da siehst du, wer gerade live ist."
        ),
        SuggestionStatus::NotFound => format!(
            "Einen Twitch-Kanal {channel} finden wir nicht. Prüf bitte die Schreibweise. Am einfachsten kopierst du den Link aus der Adresszeile, zum Beispiel twitch.tv/name."
        ),
        SuggestionStatus::Blocked => TEXT_BLOCKED.to_string(),
    }
}

/// Ergebnis eines Klicks auf "Absenden".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    InvalidChannel,
    ReasonMissing,
    OptedOut,
    RateLimited,
    /// Dieses Mitglied hat den Kanal schon vorgeschlagen.
    AlreadyByYou {
        login: String,
        status: SuggestionStatus,
    },
    Saved {
        id: i64,
        login: String,
        status: SuggestionStatus,
    },
    Failed,
}

pub fn outcome_text(outcome: &SubmitOutcome) -> String {
    match outcome {
        SubmitOutcome::InvalidChannel => TEXT_INVALID_CHANNEL.to_string(),
        SubmitOutcome::ReasonMissing => TEXT_REASON_MISSING.to_string(),
        SubmitOutcome::OptedOut => TEXT_OPTED_OUT.to_string(),
        SubmitOutcome::RateLimited => TEXT_RATE_LIMITED.to_string(),
        SubmitOutcome::Failed => TEXT_FAILED.to_string(),
        SubmitOutcome::Saved { login, status, .. } => status_text(*status, login),
        SubmitOutcome::AlreadyByYou { login, status } => match status {
            SuggestionStatus::AlreadyPartner
            | SuggestionStatus::NotFound
            | SuggestionStatus::Blocked => status_text(*status, login),
            _ => format!(
                "Den Kanal {} hast du schon vorgeschlagen, danke! Wir schauen ihn uns an.",
                shown(login)
            ),
        },
    }
}

// ── Weitergabe an den Twitch-Bot ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardRequest {
    pub id: i64,
    pub discord_id: i64,
    pub twitch_login: String,
    pub reason: String,
}

impl ForwardRequest {
    pub fn payload(&self) -> Value {
        json!({
            "twitch_login": self.twitch_login,
            "suggested_by_discord_id": self.discord_id.to_string(),
            "reason": (!self.reason.is_empty()).then_some(self.reason.as_str()),
            "idempotency_key": idempotency_key(self.id),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardResult {
    Answered {
        status: SuggestionStatus,
        twitch_user_id: Option<String>,
    },
    /// Endgültig abgelehnt, ein Neuversuch ändert nichts.
    Permanent(String),
    /// Später erneut versuchen.
    Retry(String),
}

fn valid_twitch_user_id(id: &str) -> bool {
    (1..=20).contains(&id.len()) && !id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit())
}

/// Ordnet die Antwort des Twitch-Bots ein (reine Regel).
pub fn classify_response(http_status: u16, body: &Value) -> ForwardResult {
    match http_status {
        200..=299 => {
            let Some(status) = body
                .get("status")
                .and_then(Value::as_str)
                .and_then(SuggestionStatus::from_remote)
            else {
                return ForwardResult::Retry(format!("Antwort ohne bekannten Status: {body}"));
            };
            let twitch_user_id = body
                .get("twitch_user_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| valid_twitch_user_id(id))
                .map(str::to_string);
            ForwardResult::Answered {
                status,
                twitch_user_id,
            }
        }
        400 | 409 | 422 => ForwardResult::Permanent(format!(
            "HTTP {http_status}: {}",
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("ohne Code")
        )),
        other => ForwardResult::Retry(format!("HTTP {other}")),
    }
}

#[async_trait]
pub trait SuggestionForwarder: Send + Sync {
    async fn forward(&self, request: &ForwardRequest) -> ForwardResult;
}

#[async_trait]
impl SuggestionForwarder for dl_bridges::twitch::TwitchApiClient {
    async fn forward(&self, request: &ForwardRequest) -> ForwardResult {
        match self
            .post_community_suggestion(&request.payload(), &idempotency_key(request.id))
            .await
        {
            Ok((status, body)) => classify_response(status, &body),
            Err(error) => ForwardResult::Retry(error.to_string()),
        }
    }
}

// ── Speicher und Ablauf ────────────────────────────────────────────────────

pub struct StreamerSuggestions {
    pool: PgPool,
    forwarder: Option<Arc<dyn SuggestionForwarder>>,
}

impl StreamerSuggestions {
    /// Ohne `forwarder` (kein Twitch-Token) wird gespeichert und nichts weitergegeben;
    /// sobald ein Start mit Token kommt, holt der Retry-Loop alles nach.
    pub fn new(pool: PgPool, forwarder: Option<Arc<dyn SuggestionForwarder>>) -> Arc<Self> {
        Arc::new(Self { pool, forwarder })
    }

    /// Prüft, speichert und gibt sofort weiter.
    pub async fn submit(
        &self,
        discord_id: u64,
        raw_channel: &str,
        raw_reason: &str,
    ) -> SubmitOutcome {
        let Some(login) = parse_twitch_login(raw_channel) else {
            return SubmitOutcome::InvalidChannel;
        };
        let Some(reason) = normalize_reason(raw_reason) else {
            return SubmitOutcome::ReasonMissing;
        };
        let Ok(discord_id) = u64_to_i64(discord_id, "discord_id") else {
            return SubmitOutcome::Failed;
        };
        let outcome = match self.store(discord_id, &login, &reason).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(%error, discord_id, "Streamer-Vorschlag konnte nicht gespeichert werden");
                return SubmitOutcome::Failed;
            }
        };
        let SubmitOutcome::Saved { id, .. } = outcome else {
            return outcome;
        };
        tracing::info!(
            discord_id,
            suggestion_id = id,
            "Streamer-Vorschlag gespeichert"
        );
        let request = ForwardRequest {
            id,
            discord_id,
            twitch_login: login.clone(),
            reason,
        };
        let status =
            match tokio::time::timeout(INLINE_FORWARD_TIMEOUT, self.forward_one(&request)).await {
                Ok(status) => status,
                Err(_) => {
                    tracing::warn!(
                        suggestion_id = id,
                        "Streamer-Vorschlag: Weitergabe dauert zu lange, Retry-Loop übernimmt"
                    );
                    SuggestionStatus::Pending
                }
            };
        SubmitOutcome::Saved { id, login, status }
    }

    /// Speichert in einer Transaktion unter der Datenschutz-Sperre des Mitglieds;
    /// die Sperre serialisiert auch das Tageslimit.
    async fn store(
        &self,
        discord_id: i64,
        login: &str,
        reason: &str,
    ) -> Result<SubmitOutcome, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, discord_id).await? {
            return Ok(SubmitOutcome::OptedOut);
        }
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT status FROM community.streamer_suggestions
              WHERE discord_id = $1 AND twitch_login = $2",
        )
        .bind(discord_id)
        .bind(login)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(status) = existing {
            return Ok(SubmitOutcome::AlreadyByYou {
                login: login.to_string(),
                status: SuggestionStatus::parse(&status).unwrap_or(SuggestionStatus::Pending),
            });
        }
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community.streamer_suggestions
              WHERE discord_id = $1 AND created_at > now() - interval '24 hours'",
        )
        .bind(discord_id)
        .fetch_one(&mut *tx)
        .await?;
        if recent >= DAILY_LIMIT {
            return Ok(SubmitOutcome::RateLimited);
        }
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO community.streamer_suggestions
                 (discord_id, twitch_login, reason, last_attempt_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (discord_id, twitch_login) DO NOTHING
             RETURNING id",
        )
        .bind(discord_id)
        .bind(login)
        .bind(reason)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(match id {
            Some(id) => SubmitOutcome::Saved {
                id,
                login: login.to_string(),
                status: SuggestionStatus::Pending,
            },
            None => SubmitOutcome::AlreadyByYou {
                login: login.to_string(),
                status: SuggestionStatus::Pending,
            },
        })
    }

    /// Eine Weitergabe; schreibt das Ergebnis und liefert den neuen Stand.
    async fn forward_one(&self, request: &ForwardRequest) -> SuggestionStatus {
        let Some(forwarder) = &self.forwarder else {
            return SuggestionStatus::Pending;
        };
        let result = forwarder.forward(request).await;
        let (status, twitch_user_id) = match &result {
            ForwardResult::Answered {
                status,
                twitch_user_id,
            } => (*status, twitch_user_id.clone()),
            ForwardResult::Permanent(error) => {
                tracing::warn!(suggestion_id = request.id, %error, "Streamer-Vorschlag vom Twitch-Bot abgelehnt");
                (SuggestionStatus::Rejected, None)
            }
            ForwardResult::Retry(error) => {
                tracing::warn!(suggestion_id = request.id, %error, "Streamer-Vorschlag: Weitergabe fehlgeschlagen, neuer Versuch folgt");
                (SuggestionStatus::Pending, None)
            }
        };
        let written = if status == SuggestionStatus::Pending {
            sqlx::query(
                "UPDATE community.streamer_suggestions
                    SET forward_attempts = forward_attempts + 1, last_attempt_at = now()
                  WHERE id = $1 AND forwarded_at IS NULL",
            )
            .bind(request.id)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                "UPDATE community.streamer_suggestions
                    SET status = $2,
                        twitch_user_id = COALESCE($3, twitch_user_id),
                        forward_attempts = forward_attempts + 1,
                        last_attempt_at = now(),
                        forwarded_at = now()
                  WHERE id = $1 AND forwarded_at IS NULL",
            )
            .bind(request.id)
            .bind(status.as_str())
            .bind(twitch_user_id)
            .execute(&self.pool)
            .await
        };
        if let Err(error) = written {
            tracing::warn!(suggestion_id = request.id, %error, "Streamer-Vorschlag: Ergebnis konnte nicht gespeichert werden");
        } else if status != SuggestionStatus::Pending {
            tracing::info!(
                suggestion_id = request.id,
                status = status.as_str(),
                "Streamer-Vorschlag weitergegeben"
            );
        }
        status
    }

    /// Offene Weitergaben nachholen. Fällig ist ein Vorschlag, wenn der letzte
    /// Versuch länger als 5 Minuten mal 2 hoch Versuche (höchstens 6 Stunden) her
    /// ist. Liefert die Zahl der jetzt abgeschlossenen Vorschläge.
    pub async fn forward_pending(&self) -> Result<usize, sqlx::Error> {
        if self.forwarder.is_none() {
            return Ok(0);
        }
        let rows = sqlx::query(
            "UPDATE community.streamer_suggestions s
                SET last_attempt_at = now()
              WHERE s.id IN (
                    SELECT id FROM community.streamer_suggestions
                     WHERE forwarded_at IS NULL
                       AND NOT EXISTS (
                           SELECT 1 FROM core.user_privacy p
                            WHERE p.user_id = discord_id
                              AND (p.opted_out OR p.deleted_at IS NOT NULL))
                       AND (last_attempt_at IS NULL
                            OR last_attempt_at <= now() - make_interval(mins => LEAST(
                                   $1 * power(2, LEAST(forward_attempts, 10))::int, $2)))
                     ORDER BY last_attempt_at NULLS FIRST, id
                     LIMIT $3
                     FOR UPDATE SKIP LOCKED)
            RETURNING s.id, s.discord_id, s.twitch_login, s.reason",
        )
        .bind(RETRY_BASE_MINUTES)
        .bind(RETRY_MAX_MINUTES)
        .bind(RETRY_BATCH)
        .fetch_all(&self.pool)
        .await?;
        let mut done = 0;
        for row in rows {
            let request = ForwardRequest {
                id: row.try_get("id")?,
                discord_id: row.try_get("discord_id")?,
                twitch_login: row.try_get("twitch_login")?,
                reason: row.try_get("reason")?,
            };
            if self.forward_one(&request).await != SuggestionStatus::Pending {
                done += 1;
            }
        }
        Ok(done)
    }
}

// ── Discord ────────────────────────────────────────────────────────────────

/// Button für das Clip-Panel.
pub fn panel_button() -> Value {
    json!({ "type": 2, "style": 2, "label": BUTTON_LABEL, "custom_id": OPEN_CUSTOM_ID })
}

pub fn modal() -> ModalSpec {
    ModalSpec {
        custom_id: MODAL_CUSTOM_ID.to_string(),
        title: "Streamer vorschlagen".to_string(),
        fields: vec![
            ModalField {
                custom_id: FIELD_CHANNEL.to_string(),
                label: "Twitch-Kanal oder Link".to_string(),
                placeholder: "twitch.tv/kanalname".to_string(),
                value: None,
                required: true,
                min_length: 1,
                max_length: 100,
                paragraph: false,
            },
            ModalField {
                custom_id: FIELD_REASON.to_string(),
                label: "Warum passt der Kanal zu uns?".to_string(),
                placeholder: "Kurz in ein, zwei Sätzen".to_string(),
                value: None,
                required: true,
                min_length: MIN_REASON_CHARS as u16,
                max_length: MAX_REASON_CHARS as u16,
                paragraph: true,
            },
        ],
    }
}

struct SuggestHandler {
    service: Arc<StreamerSuggestions>,
}

#[async_trait]
impl InteractionHandler for SuggestHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        match interaction.custom_id.as_str() {
            OPEN_CUSTOM_ID => BridgeReply {
                modal: Some(modal()),
                ..BridgeReply::default()
            },
            MODAL_CUSTOM_ID => {
                let field = |key: &str| {
                    interaction
                        .options
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                let outcome = self
                    .service
                    .submit(
                        interaction.user_id,
                        &field(FIELD_CHANNEL),
                        &field(FIELD_REASON),
                    )
                    .await;
                BridgeReply {
                    content: Some(outcome_text(&outcome)),
                    ephemeral: true,
                    allowed_mentions: Some(json!({ "parse": [] })),
                    ..BridgeReply::default()
                }
            }
            _ => BridgeReply::ephemeral_text(TEXT_FAILED),
        }
    }
}

pub fn register(router: &mut InteractionRouter, service: Arc<StreamerSuggestions>) {
    let handler = Arc::new(SuggestHandler { service });
    router.on_custom_id(OPEN_CUSTOM_ID, handler.clone());
    router.on_custom_id(MODAL_CUSTOM_ID, handler);
}

/// Retry-Loop für offene Weitergaben (alle 2 Minuten, Abstand je Vorschlag wächst).
pub fn spawn(service: Arc<StreamerSuggestions>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(RETRY_TICK).await;
            match service.forward_pending().await {
                Ok(0) => {}
                Ok(done) => tracing::info!(done, "Streamer-Vorschläge nachträglich weitergegeben"),
                Err(error) => {
                    tracing::warn!(%error, "Streamer-Vorschläge: Retry-Loop fehlgeschlagen")
                }
            }
        }
    })
}

#[cfg(test)]
#[path = "streamer_suggest_tests.rs"]
mod tests;
