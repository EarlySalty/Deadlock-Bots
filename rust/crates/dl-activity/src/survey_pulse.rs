use std::sync::Arc;

use chrono::{DateTime, Utc};
use dl_central_db::{is_proactive_dm_allowed, ProactiveDmDenialReason, ProactiveDmEligibility};
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

use crate::outbox::{HandlerOutcome, OutboxError, OutboxHandler, OutboxRow};

const VOICE_ACTIVE_DAYS: i64 = 14;
const SURVEY_COOLDOWN_DAYS: i64 = 60;
const EVENT_VALUES: [&str; 5] = [
    "community_abend",
    "turnier",
    "coaching",
    "workshop",
    "casual",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurveyPulseConfig {
    pub enabled: bool,
    pub interval_days: u64,
}

impl SurveyPulseConfig {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let enabled = lookup("SURVEY_PULSE_ENABLED").is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
        let interval_days = lookup("SURVEY_PULSE_INTERVAL_DAYS")
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|days| *days > 0)
            .unwrap_or(75);
        Self {
            enabled,
            interval_days,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WaveRunSummary {
    pub wave_id: i64,
    pub invited: u64,
    pub skipped_inactive: u64,
    pub skipped_opt_out: u64,
    pub skipped_budget: u64,
    pub skipped_cooldown: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SurveyPulseError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("ungültige Umfrage-Antwort")]
    InvalidAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurveyAnswer {
    Satisfaction(i16),
    Events(Vec<String>),
    Freitext(String),
}

pub async fn run_wave(
    pool: &PgPool,
    guild_id: i64,
    config: SurveyPulseConfig,
    now: DateTime<Utc>,
) -> Result<WaveRunSummary, SurveyPulseError> {
    let mut tx = pool.begin().await?;
    let snapshot = json!({
        "guild_id": guild_id,
        "interval_days": config.interval_days,
        "voice_active_days": VOICE_ACTIVE_DAYS,
        "survey_cooldown_days": SURVEY_COOLDOWN_DAYS,
    });
    let wave_id: i64 = sqlx::query_scalar(
        "INSERT INTO bot.survey_waves(started_at, config_snapshot)
         VALUES ($1, $2) RETURNING id",
    )
    .bind(now)
    .bind(snapshot)
    .fetch_one(&mut *tx)
    .await?;
    let rows = sqlx::query(
        "SELECT
            retention.user_id,
            (retention.opted_out OR COALESCE(privacy.opted_out, FALSE)) AS opted_out,
            patterns.last_pinged_at,
            COALESCE(patterns.ping_count_30d, 0)::INTEGER AS ping_count_30d,
            EXISTS (
                SELECT 1 FROM activity.voice_session_log AS voice
                 WHERE voice.user_id = retention.user_id
                   AND voice.guild_id = retention.guild_id
                   AND voice.ended_at >= $2 - INTERVAL '14 days'
            ) AS voice_active,
            EXISTS (
                SELECT 1 FROM bot.action_outbox AS previous
                 WHERE previous.action_type = 'survey_pulse'
                   AND previous.user_id = retention.user_id
                   AND previous.created_at >= $2 - INTERVAL '60 days'
            ) AS survey_cooldown
         FROM activity.user_retention_tracking AS retention
         LEFT JOIN activity.user_activity_patterns AS patterns
           ON patterns.user_id = retention.user_id
         LEFT JOIN core.user_privacy AS privacy
           ON privacy.user_id = retention.user_id
         WHERE retention.guild_id = $1
         ORDER BY retention.user_id",
    )
    .bind(guild_id)
    .bind(now)
    .fetch_all(&mut *tx)
    .await?;

    let mut summary = WaveRunSummary {
        wave_id,
        ..WaveRunSummary::default()
    };
    for row in rows {
        let user_id: i64 = row.try_get("user_id")?;
        let voice_active: bool = row.try_get("voice_active")?;
        if !voice_active {
            summary.skipped_inactive += 1;
            tracing::debug!(
                wave_id,
                user_id,
                decision = "übersprungen_inaktiv",
                "Umfragen-Puls-Auswahl"
            );
            continue;
        }
        let eligibility = ProactiveDmEligibility {
            user_id,
            opted_out: row.try_get("opted_out")?,
            last_pinged_at: row.try_get("last_pinged_at")?,
            ping_count_30d: row.try_get("ping_count_30d")?,
        };
        match is_proactive_dm_allowed(eligibility, now) {
            Err(ProactiveDmDenialReason::OptedOut) => {
                summary.skipped_opt_out += 1;
                tracing::debug!(
                    wave_id,
                    user_id,
                    decision = "übersprungen_opt_out",
                    "Umfragen-Puls-Auswahl"
                );
                continue;
            }
            Err(ProactiveDmDenialReason::MonthlyBudgetExhausted) => {
                summary.skipped_budget += 1;
                tracing::debug!(
                    wave_id,
                    user_id,
                    decision = "übersprungen_budget",
                    "Umfragen-Puls-Auswahl"
                );
                continue;
            }
            Err(ProactiveDmDenialReason::CooldownActive { .. }) => {
                summary.skipped_cooldown += 1;
                tracing::debug!(
                    wave_id,
                    user_id,
                    decision = "übersprungen_cooldown",
                    "Umfragen-Puls-Auswahl"
                );
                continue;
            }
            Ok(()) => {}
        }
        if row.try_get::<bool, _>("survey_cooldown")? {
            summary.skipped_cooldown += 1;
            tracing::debug!(
                wave_id,
                user_id,
                decision = "übersprungen_cooldown",
                "Umfragen-Puls-Auswahl"
            );
            continue;
        }

        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id).await? {
            summary.skipped_opt_out += 1;
            tracing::info!(
                writer = "bot.action_outbox.survey_pulse",
                user_id,
                "übersprungen wegen Opt-out"
            );
            continue;
        }
        let inserted = sqlx::query(
            "INSERT INTO bot.action_outbox(
                action_type, user_id, guild_id, payload, anchor, idempotency_key
             ) VALUES ('survey_pulse', $1, $2, jsonb_build_object('wave_id', $3), $4, $5)
             ON CONFLICT (idempotency_key) DO NOTHING",
        )
        .bind(user_id)
        .bind(guild_id)
        .bind(wave_id)
        .bind(format!("survey_pulse:{wave_id}"))
        .bind(format!("survey_pulse:{wave_id}:{user_id}"))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 1 {
            summary.invited += 1;
            tracing::debug!(
                wave_id,
                user_id,
                decision = "eingeladen",
                "Umfragen-Puls-Auswahl"
            );
        }
    }
    tx.commit().await?;
    tracing::info!(
        wave_id,
        invited = summary.invited,
        skipped_inactive = summary.skipped_inactive,
        skipped_opt_out = summary.skipped_opt_out,
        skipped_budget = summary.skipped_budget,
        skipped_cooldown = summary.skipped_cooldown,
        "Umfragen-Puls-Welle abgeschlossen"
    );
    Ok(summary)
}

pub async fn upsert_response(
    pool: &PgPool,
    wave_id: i64,
    user_id: i64,
    answer: SurveyAnswer,
    answered_at: DateTime<Utc>,
) -> Result<bool, SurveyPulseError> {
    let (satisfaction, events, freitext) = match answer {
        SurveyAnswer::Satisfaction(value) if (1..=5).contains(&value) => (Some(value), None, None),
        SurveyAnswer::Events(values)
            if !values.is_empty()
                && values.len() <= EVENT_VALUES.len()
                && values
                    .iter()
                    .all(|value| EVENT_VALUES.contains(&value.as_str())) =>
        {
            (None, Some(values), None)
        }
        SurveyAnswer::Freitext(value)
            if !value.trim().is_empty() && value.chars().count() <= 1_000 =>
        {
            (None, None, Some(value.trim().to_string()))
        }
        _ => return Err(SurveyPulseError::InvalidAnswer),
    };
    let stored = sqlx::query_scalar::<_, bool>(
        "INSERT INTO bot.survey_responses(
            wave_id, user_id, satisfaction, events, freitext, answered_at
         )
         SELECT $1, $2, $3, $4, $5, $6
          WHERE EXISTS (
              SELECT 1 FROM bot.action_outbox AS invitation
               WHERE invitation.action_type = 'survey_pulse'
                 AND invitation.user_id = $2
                 AND invitation.status = 'sent'
                 AND invitation.payload ->> 'wave_id' = $1::TEXT
          )
         ON CONFLICT (wave_id, user_id) DO UPDATE SET
            satisfaction = COALESCE(EXCLUDED.satisfaction, bot.survey_responses.satisfaction),
            events = COALESCE(EXCLUDED.events, bot.survey_responses.events),
            freitext = COALESCE(EXCLUDED.freitext, bot.survey_responses.freitext),
            answered_at = EXCLUDED.answered_at
         RETURNING TRUE",
    )
    .bind(wave_id)
    .bind(user_id)
    .bind(satisfaction)
    .bind(events)
    .bind(freitext)
    .bind(answered_at)
    .fetch_optional(pool)
    .await?;
    Ok(stored.unwrap_or(false))
}

fn event_label(value: &str) -> &'static str {
    match value {
        "community_abend" => "Community-Abende",
        "turnier" => "Turniere",
        "coaching" => "Coaching-Sessions",
        "workshop" => "Workshops und Guides",
        _ => "Casual-Runden",
    }
}

pub fn survey_dm_body(wave_id: i64) -> Value {
    let scores: Vec<Value> = (1..=5)
        .map(|score| {
            json!({
                "type": 2,
                "style": 2,
                "label": format!("{score} ⭐"),
                "custom_id": format!("survey_pulse:satisfaction:{wave_id}:{score}"),
            })
        })
        .collect();
    let options: Vec<Value> = EVENT_VALUES
        .iter()
        .map(|value| {
            json!({
                "label": event_label(value),
                "value": value,
            })
        })
        .collect();
    json!({
        "content": "Hey! Kurzer Community-Puls von uns, drei schnelle Fragen, dauert keine 30 Sekunden.\n\n**1. Wie zufrieden bist du gerade mit dem Server?** Klick auf 1 bis 5 Sterne.\n**2. Welche Events wünschst du dir öfter?** Wähl unten aus.\n**3. Was fehlt dir?** Schreib es uns über den Button.\n\nWir werten die Antworten gesammelt aus, damit wir wissen, was als Nächstes dran ist. Antworten kannst du ändern, solange die Umfrage läuft.",
        "components": [
            { "type": 1, "components": scores },
            { "type": 1, "components": [{
                "type": 3,
                "custom_id": format!("survey_pulse:events:{wave_id}"),
                "placeholder": "Welche Events willst du öfter sehen?",
                "options": options,
                "min_values": 1,
                "max_values": EVENT_VALUES.len(),
            }]},
            { "type": 1, "components": [{
                "type": 2,
                "style": 2,
                "label": "Was fehlt dir? Schreib es uns",
                "custom_id": format!("survey_pulse:freetext:{wave_id}"),
            }]}
        ],
        "allowed_mentions": { "parse": [] },
    })
}

pub struct SurveyPulseHandler {
    pool: PgPool,
}

impl SurveyPulseHandler {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

fn parse_wave_id(parts: &[&str], index: usize) -> Option<i64> {
    parts
        .get(index)?
        .parse()
        .ok()
        .filter(|wave_id| *wave_id > 0)
}

fn response_reply(result: Result<bool, SurveyPulseError>) -> BridgeReply {
    match result {
        Ok(true) => BridgeReply::ephemeral_text(
            "Danke, ist notiert! Du kannst deine Antwort ändern, solange die Umfrage läuft.",
        ),
        Ok(false) | Err(SurveyPulseError::InvalidAnswer) => BridgeReply::ephemeral_text(
            "Diese Umfrage läuft leider nicht mehr, die Antwort wurde nicht gespeichert.",
        ),
        Err(error) => {
            tracing::warn!(%error, "Umfragen-Puls-Antwort konnte nicht gespeichert werden");
            BridgeReply::ephemeral_text(
                "Das hat gerade nicht geklappt. Versuch es in ein paar Minuten nochmal.",
            )
        }
    }
}

#[async_trait::async_trait]
impl InteractionHandler for SurveyPulseHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let parts: Vec<&str> = interaction.custom_id.split(':').collect();
        if parts.first() != Some(&"survey_pulse") {
            return BridgeReply::ephemeral_text(
                "Diese Aktion kennen wir nicht, die Umfrage-Nachricht ist vermutlich veraltet.",
            );
        }
        let Ok(user_id) = i64::try_from(interaction.user_id) else {
            return BridgeReply::ephemeral_text(
                "Deine Discord-ID konnten wir nicht verarbeiten, bitte meld das dem Team.",
            );
        };
        match parts.get(1).copied() {
            Some("satisfaction") => {
                let Some(wave_id) = parse_wave_id(&parts, 2) else {
                    return BridgeReply::ephemeral_text("Diese Umfrage gibt es nicht mehr.");
                };
                let Some(score) = parts.get(3).and_then(|value| value.parse::<i16>().ok()) else {
                    return BridgeReply::ephemeral_text(
                        "Diese Bewertung konnten wir nicht lesen, bitte nutz die Buttons 1 bis 5.",
                    );
                };
                response_reply(
                    upsert_response(
                        &self.pool,
                        wave_id,
                        user_id,
                        SurveyAnswer::Satisfaction(score),
                        Utc::now(),
                    )
                    .await,
                )
            }
            Some("events") => {
                let Some(wave_id) = parse_wave_id(&parts, 2) else {
                    return BridgeReply::ephemeral_text("Diese Umfrage gibt es nicht mehr.");
                };
                response_reply(
                    upsert_response(
                        &self.pool,
                        wave_id,
                        user_id,
                        SurveyAnswer::Events(interaction.values),
                        Utc::now(),
                    )
                    .await,
                )
            }
            Some("freetext") => {
                let Some(wave_id) = parse_wave_id(&parts, 2) else {
                    return BridgeReply::ephemeral_text("Diese Umfrage gibt es nicht mehr.");
                };
                BridgeReply {
                    modal: Some(ModalSpec {
                        custom_id: format!("survey_pulse:freetext_submit:{wave_id}"),
                        title: "Was fehlt dir auf dem Server?".to_string(),
                        fields: vec![ModalField {
                            custom_id: "freetext".to_string(),
                            label: "Dein Feedback".to_string(),
                            placeholder: "Was fehlt, was nervt, was wünschst du dir?".to_string(),
                            value: None,
                            required: true,
                            min_length: 1,
                            max_length: 1_000,
                            paragraph: true,
                        }],
                    }),
                    ..BridgeReply::default()
                }
            }
            Some("freetext_submit") => {
                let Some(wave_id) = parse_wave_id(&parts, 2) else {
                    return BridgeReply::ephemeral_text("Diese Umfrage gibt es nicht mehr.");
                };
                let value = interaction
                    .options
                    .get("freetext")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                response_reply(
                    upsert_response(
                        &self.pool,
                        wave_id,
                        user_id,
                        SurveyAnswer::Freitext(value),
                        Utc::now(),
                    )
                    .await,
                )
            }
            _ => BridgeReply::ephemeral_text(
                "Diese Aktion kennen wir nicht, die Umfrage-Nachricht ist vermutlich veraltet.",
            ),
        }
    }
}

pub fn register(router: &mut InteractionRouter, handler: Arc<SurveyPulseHandler>) {
    router.on_prefix("survey_pulse:", handler);
}

#[async_trait::async_trait]
pub trait SurveyPulsePort: Send + Sync {
    async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String>;
}

#[async_trait::async_trait]
impl SurveyPulsePort for dl_discord::DiscordAdapter {
    async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String> {
        let body = body
            .as_object()
            .ok_or_else(|| "survey DM body is not an object".to_string())?;
        let channel = self
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|error| error.to_string())?;
        self.send_raw_public(channel.id.get(), body)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// `action_type` der Umfragen-Puls-Zeilen in `bot.action_outbox`.
pub const SURVEY_PULSE_ACTION_TYPE: &str = "survey_pulse";

/// Zustellung einer Umfragen-Puls-Einladung als Outbox-Handler.
///
/// Der Handler prüft das Proaktiv-DM-Gate direkt vor der Wirkung erneut,
/// verschickt die DM und bucht den Ping. Den Statuswechsel der Outbox-Zeile
/// schreibt der Dispatcher aus dem zurückgegebenen Urteil.
pub struct SurveyPulseOutboxHandler {
    port: Arc<dyn SurveyPulsePort>,
}

impl SurveyPulseOutboxHandler {
    pub fn new(port: Arc<dyn SurveyPulsePort>) -> Arc<Self> {
        Arc::new(Self { port })
    }
}

#[async_trait::async_trait]
impl OutboxHandler for SurveyPulseOutboxHandler {
    async fn handle(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        row: &OutboxRow,
        now: DateTime<Utc>,
    ) -> Result<HandlerOutcome, OutboxError> {
        let id = row.id;
        let user_id = row.user_id;
        let eligibility = sqlx::query(
            "SELECT
                COALESCE((
                    SELECT retention.opted_out
                      FROM activity.user_retention_tracking AS retention
                     WHERE retention.user_id = $1 AND retention.guild_id = $2
                ), TRUE) OR COALESCE((
                    SELECT privacy.opted_out FROM core.user_privacy AS privacy
                     WHERE privacy.user_id = $1
                ), FALSE) AS opted_out,
                (SELECT patterns.last_pinged_at
                   FROM activity.user_activity_patterns AS patterns
                  WHERE patterns.user_id = $1) AS last_pinged_at,
                COALESCE((SELECT patterns.ping_count_30d
                   FROM activity.user_activity_patterns AS patterns
                  WHERE patterns.user_id = $1), 0)::INTEGER AS ping_count_30d",
        )
        .bind(user_id)
        .bind(row.guild_id)
        .fetch_one(&mut **tx)
        .await?;
        let gate = is_proactive_dm_allowed(
            ProactiveDmEligibility {
                user_id,
                opted_out: eligibility.try_get("opted_out")?,
                last_pinged_at: eligibility.try_get("last_pinged_at")?,
                ping_count_30d: eligibility.try_get("ping_count_30d")?,
            },
            now,
        );
        if let Err(reason) = gate {
            let reason = match reason {
                ProactiveDmDenialReason::OptedOut => "opted_out",
                ProactiveDmDenialReason::CooldownActive { .. } => "cooldown",
                ProactiveDmDenialReason::MonthlyBudgetExhausted => "budget",
            };
            tracing::debug!(
                id,
                user_id,
                decision = "übersprungen",
                reason,
                "Umfragen-Puls-Zustellung"
            );
            return Ok(HandlerOutcome::Suppressed {
                reason: reason.to_string(),
            });
        }
        let Some(wave_id) = row.payload.get("wave_id").and_then(Value::as_i64) else {
            tracing::warn!(
                id,
                user_id,
                decision = "fehlgeschlagen",
                grund = "invalid wave_id",
                "Umfragen-Puls-Zustellung"
            );
            return Ok(HandlerOutcome::Failed {
                error: "invalid wave_id".to_string(),
            });
        };
        let Ok(discord_user_id) = u64::try_from(user_id) else {
            tracing::warn!(
                id,
                user_id,
                decision = "fehlgeschlagen",
                grund = "invalid user_id",
                "Umfragen-Puls-Zustellung"
            );
            return Ok(HandlerOutcome::Failed {
                error: "invalid user_id".to_string(),
            });
        };
        if let Err(error) = self
            .port
            .send_dm(discord_user_id, survey_dm_body(wave_id))
            .await
        {
            tracing::warn!(id, user_id, grund = %error, "Umfragen-Puls-DM fehlgeschlagen");
            return Ok(HandlerOutcome::Failed { error });
        }
        sqlx::query(
            "INSERT INTO activity.user_activity_patterns(
                user_id, last_pinged_at, ping_count_30d
             ) VALUES ($1, $2, 1)
             ON CONFLICT (user_id) DO UPDATE SET
                ping_count_30d = CASE
                    WHEN activity.user_activity_patterns.last_pinged_at < $2 - INTERVAL '30 days'
                        THEN 1
                    ELSE COALESCE(activity.user_activity_patterns.ping_count_30d, 0) + 1
                END,
                last_pinged_at = $2",
        )
        .bind(user_id)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        tracing::debug!(
            id,
            wave_id,
            user_id,
            decision = "gesendet",
            "Umfragen-Puls-Zustellung"
        );
        Ok(HandlerOutcome::Sent)
    }
}

pub async fn run_due_wave(
    pool: &PgPool,
    guild_id: i64,
    config: SurveyPulseConfig,
    now: DateTime<Utc>,
) -> Result<Option<WaveRunSummary>, SurveyPulseError> {
    if !config.enabled {
        return Ok(None);
    }
    let interval_days = i32::try_from(config.interval_days).unwrap_or(i32::MAX);
    let due: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (
            SELECT 1 FROM bot.survey_waves AS waves
             WHERE waves.config_snapshot ->> 'guild_id' = $1::TEXT
               AND waves.started_at > $2 - make_interval(days => $3)
        )",
    )
    .bind(guild_id)
    .bind(now)
    .bind(interval_days)
    .fetch_one(pool)
    .await?;
    if !due {
        return Ok(None);
    }
    run_wave(pool, guild_id, config, now).await.map(Some)
}

/// Startet nur die Wellen-Planung.
///
/// Die Zustellung der erzeugten Zeilen macht der Outbox-Dispatcher, der einen
/// eigenen Lebenszyklus hat. Dieser Task erzeugt lediglich neue Einladungen und
/// hängt deshalb weiter an `SURVEY_PULSE_ENABLED`.
pub fn spawn_scheduler(
    pool: PgPool,
    guild_id: i64,
    config: SurveyPulseConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            if let Err(error) = run_due_wave(&pool, guild_id, config, Utc::now()).await {
                tracing::warn!(%error, "Umfragen-Puls-Scheduler fehlgeschlagen");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "testing")]
    use std::sync::{Arc, Mutex};

    #[cfg(feature = "testing")]
    use chrono::{Duration, TimeZone, Utc};
    #[cfg(feature = "testing")]
    use dl_discord::{BridgeInteraction, InteractionHandler, InteractionRouter};
    #[cfg(feature = "testing")]
    use serde_json::json;

    use super::*;

    #[cfg(feature = "testing")]
    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    #[cfg(feature = "testing")]
    async fn seed_user(
        pool: &sqlx::PgPool,
        user_id: i64,
        guild_id: i64,
        voice_ended_at: chrono::DateTime<Utc>,
        opted_out: bool,
        last_pinged_at: Option<chrono::DateTime<Utc>>,
        ping_count_30d: i32,
    ) {
        sqlx::query(
            "INSERT INTO activity.user_retention_tracking(
                user_id, guild_id, first_seen_at, last_active_at,
                total_active_days, opted_out, updated_at
             ) VALUES ($1, $2, $3, $3, 1, $4, $3)",
        )
        .bind(user_id)
        .bind(guild_id)
        .bind(now())
        .bind(opted_out)
        .execute(pool)
        .await
        .expect("retention row");
        sqlx::query(
            "INSERT INTO activity.user_activity_patterns(
                user_id, last_active_at, last_pinged_at, ping_count_30d
             ) VALUES ($1, $2, $3, $4)",
        )
        .bind(user_id)
        .bind(voice_ended_at)
        .bind(last_pinged_at)
        .bind(ping_count_30d)
        .execute(pool)
        .await
        .expect("activity pattern");
        sqlx::query(
            "INSERT INTO activity.voice_session_log(
                id, user_id, guild_id, channel_id, started_at, ended_at,
                duration_seconds, points
             ) VALUES ($1, $1, $2, 10, $3 - INTERVAL '30 minutes', $3, 1800, 1)",
        )
        .bind(user_id)
        .bind(guild_id)
        .bind(voice_ended_at)
        .execute(pool)
        .await
        .expect("voice session");
    }

    #[test]
    fn config_is_disabled_by_default_and_uses_75_days() {
        let config = SurveyPulseConfig::from_lookup(|_| None);
        assert!(!config.enabled);
        assert_eq!(config.interval_days, 75);
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn wave_selects_recent_voice_users_and_exposes_every_skip_reason() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool();
        let guild_id = 99;
        seed_user(pool, 1, guild_id, now() - Duration::days(1), false, None, 0).await;
        seed_user(
            pool,
            2,
            guild_id,
            now() - Duration::days(15),
            false,
            None,
            0,
        )
        .await;
        seed_user(pool, 3, guild_id, now() - Duration::days(1), true, None, 0).await;
        seed_user(
            pool,
            4,
            guild_id,
            now() - Duration::days(1),
            false,
            Some(now() - Duration::days(3)),
            0,
        )
        .await;
        // Monthly budget requires a real ping inside 30 days, outside the 14-day cooldown.
        seed_user(
            pool,
            5,
            guild_id,
            now() - Duration::days(1),
            false,
            Some(now() - Duration::days(20)),
            2,
        )
        .await;
        seed_user(pool, 6, guild_id, now() - Duration::days(1), false, None, 0).await;
        sqlx::query(
            "INSERT INTO bot.action_outbox(
                action_type, user_id, guild_id, payload, anchor, idempotency_key, created_at
             ) VALUES ('survey_pulse', 6, $1, '{\"wave_id\":999}', 'old', 'old-survey', $2)",
        )
        .bind(guild_id)
        .bind(now() - Duration::days(30))
        .execute(pool)
        .await
        .expect("old survey invite");

        let summary = run_wave(
            pool,
            guild_id,
            SurveyPulseConfig {
                enabled: true,
                interval_days: 75,
            },
            now(),
        )
        .await
        .expect("wave");

        assert_eq!(summary.invited, 1);
        assert_eq!(summary.skipped_inactive, 1);
        assert_eq!(summary.skipped_opt_out, 1);
        assert_eq!(summary.skipped_budget, 1);
        assert_eq!(summary.skipped_cooldown, 2);
        let invited: Vec<i64> = sqlx::query_scalar(
            "SELECT user_id FROM bot.action_outbox
              WHERE action_type = 'survey_pulse' AND payload ->> 'wave_id' = $1",
        )
        .bind(summary.wave_id.to_string())
        .fetch_all(pool)
        .await
        .expect("outbox users");
        assert_eq!(invited, vec![1]);
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn privacy_lock_blockiert_survey_einladung_nach_loeschung() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool().clone();
        let guild_id = 99;
        seed_user(
            &pool,
            42,
            guild_id,
            now() - Duration::days(1),
            false,
            None,
            0,
        )
        .await;

        let mut erase_tx = pool.begin().await.expect("erase tx");
        dl_central_db::lock_user_privacy(&mut erase_tx, 42)
            .await
            .expect("privacy lock");
        sqlx::query(
            "INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
             VALUES(42, TRUE, now())",
        )
        .execute(&mut *erase_tx)
        .await
        .expect("privacy tombstone");

        let write_pool = pool.clone();
        let write = tokio::spawn(async move {
            run_wave(
                &write_pool,
                guild_id,
                SurveyPulseConfig {
                    enabled: true,
                    interval_days: 75,
                },
                now(),
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        erase_tx.commit().await.expect("erase commit");
        let summary = write.await.expect("writer task").expect("writer result");

        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM bot.action_outbox
              WHERE action_type = 'survey_pulse' AND user_id = 42",
        )
        .fetch_one(&pool)
        .await
        .expect("survey refill count");
        assert_eq!(summary.invited, 0);
        assert_eq!(count, 0, "Löschung darf Survey-Einladung nicht neu anlegen");
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn response_upsert_overwrites_instead_of_duplicating() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool();
        let wave_id: i64 = sqlx::query_scalar(
            "INSERT INTO bot.survey_waves(config_snapshot) VALUES ('{}') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("wave");
        sqlx::query(
            "INSERT INTO bot.action_outbox(
                action_type, user_id, guild_id, payload, anchor, idempotency_key, status
             ) VALUES ('survey_pulse', 42, 99, jsonb_build_object('wave_id', $1),
                       'survey', 'survey-response-test', 'sent')",
        )
        .bind(wave_id)
        .execute(pool)
        .await
        .expect("invite");

        assert!(
            upsert_response(pool, wave_id, 42, SurveyAnswer::Satisfaction(2), now(),)
                .await
                .expect("first answer")
        );
        assert!(upsert_response(
            pool,
            wave_id,
            42,
            SurveyAnswer::Satisfaction(5),
            now() + Duration::minutes(1),
        )
        .await
        .expect("replacement answer"));

        let row: (i64, Option<i16>) = sqlx::query_as(
            "SELECT COUNT(*), MAX(satisfaction) FROM bot.survey_responses
              WHERE wave_id = $1 AND user_id = 42",
        )
        .bind(wave_id)
        .fetch_one(pool)
        .await
        .expect("response");
        assert_eq!(row, (1, Some(5)));
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn summary_view_aggregates_rate_satisfaction_and_event_counts() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool();
        let wave_id: i64 = sqlx::query_scalar(
            "INSERT INTO bot.survey_waves(config_snapshot) VALUES ('{}') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("wave");
        for user_id in 1_i64..=4 {
            sqlx::query(
                "INSERT INTO bot.action_outbox(
                    action_type, user_id, guild_id, payload, anchor, idempotency_key
                 ) VALUES ('survey_pulse', $1, 99, jsonb_build_object('wave_id', $2),
                           'survey', $3)",
            )
            .bind(user_id)
            .bind(wave_id)
            .bind(format!("summary-{user_id}"))
            .execute(pool)
            .await
            .expect("invite");
        }
        sqlx::query(
            "INSERT INTO bot.survey_responses(wave_id, user_id, satisfaction, events) VALUES
                ($1, 1, 4, ARRAY['turnier', 'workshop']),
                ($1, 2, 2, ARRAY['turnier'])",
        )
        .bind(wave_id)
        .execute(pool)
        .await
        .expect("responses");

        let row: (i64, i64, f64, Option<f64>, serde_json::Value) = sqlx::query_as(
            "SELECT invited_count, response_count,
                        response_rate::DOUBLE PRECISION,
                        average_satisfaction::DOUBLE PRECISION,
                        event_counts
                   FROM bot.survey_wave_summary WHERE wave_id = $1",
        )
        .bind(wave_id)
        .fetch_one(pool)
        .await
        .expect("summary");
        assert_eq!(row.0, 4);
        assert_eq!(row.1, 2);
        assert_eq!(row.2, 0.5);
        assert_eq!(row.3, Some(3.0));
        assert_eq!(row.4, json!({ "turnier": 2, "workshop": 1 }));
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn interaction_handler_persists_buttons_select_and_modal_upserts() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool();
        let wave_id: i64 = sqlx::query_scalar(
            "INSERT INTO bot.survey_waves(config_snapshot) VALUES ('{}') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("wave");
        sqlx::query(
            "INSERT INTO bot.action_outbox(
                action_type, user_id, guild_id, payload, anchor, idempotency_key, status
             ) VALUES ('survey_pulse', 42, 99, jsonb_build_object('wave_id', $1),
                       'survey', 'interaction-test', 'sent')",
        )
        .bind(wave_id)
        .execute(pool)
        .await
        .expect("invite");
        let handler = SurveyPulseHandler::new(pool.clone());

        handler
            .handle(BridgeInteraction {
                custom_id: format!("survey_pulse:satisfaction:{wave_id}:4"),
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;
        handler
            .handle(BridgeInteraction {
                custom_id: format!("survey_pulse:events:{wave_id}"),
                values: vec!["turnier".to_string(), "workshop".to_string()],
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;
        let modal = handler
            .handle(BridgeInteraction {
                custom_id: format!("survey_pulse:freetext:{wave_id}"),
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(
            modal.modal.as_ref().map(|spec| spec.custom_id.as_str()),
            Some(format!("survey_pulse:freetext_submit:{wave_id}").as_str())
        );
        let mut options = std::collections::HashMap::new();
        options.insert("freetext".to_string(), json!("Mehr Turniere"));
        handler
            .handle(BridgeInteraction {
                custom_id: format!("survey_pulse:freetext_submit:{wave_id}"),
                options,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        let row: (Option<i16>, Option<Vec<String>>, Option<String>) = sqlx::query_as(
            "SELECT satisfaction, events, freitext FROM bot.survey_responses
              WHERE wave_id = $1 AND user_id = 42",
        )
        .bind(wave_id)
        .fetch_one(pool)
        .await
        .expect("response");
        assert_eq!(row.0, Some(4));
        assert_eq!(
            row.1,
            Some(vec!["turnier".to_string(), "workshop".to_string()])
        );
        assert_eq!(row.2.as_deref(), Some("Mehr Turniere"));

        let mut router = InteractionRouter::new();
        register(&mut router, handler);
        assert!(router
            .resolve_component("survey_pulse:satisfaction:1:5")
            .is_some());
    }

    #[cfg(feature = "testing")]
    #[derive(Default)]
    struct MockPort {
        sent: Mutex<Vec<(u64, Value)>>,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl SurveyPulsePort for MockPort {
        async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String> {
            self.sent.lock().expect("sent").push((user_id, body));
            Ok(())
        }
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn outbox_dispatch_rechecks_gate_and_accounts_only_successful_dm() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool();
        seed_user(pool, 1, 99, now() - Duration::days(1), false, None, 0).await;
        seed_user(pool, 2, 99, now() - Duration::days(1), false, None, 0).await;
        let wave_id: i64 = sqlx::query_scalar(
            "INSERT INTO bot.survey_waves(config_snapshot) VALUES ('{}') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("wave");
        for user_id in [1_i64, 2] {
            sqlx::query(
                "INSERT INTO bot.action_outbox(
                    action_type, user_id, guild_id, payload, anchor, idempotency_key, scheduled_for
                 ) VALUES ('survey_pulse', $1, 99, jsonb_build_object('wave_id', $2),
                           'survey', $3, $4)",
            )
            .bind(user_id)
            .bind(wave_id)
            .bind(format!("dispatch-{user_id}"))
            .bind(now())
            .execute(pool)
            .await
            .expect("outbox");
        }
        sqlx::query(
            "UPDATE activity.user_retention_tracking SET opted_out = TRUE WHERE user_id = 2",
        )
        .execute(pool)
        .await
        .expect("opt out");
        let port = Arc::new(MockPort::default());
        let dispatcher = crate::outbox::OutboxDispatcher::new(pool.clone()).register(
            SURVEY_PULSE_ACTION_TYPE,
            SurveyPulseOutboxHandler::new(port.clone()),
        );

        assert_eq!(
            dispatcher.dispatch_one(now()).await.expect("first"),
            crate::outbox::DispatchOutcome::Sent
        );
        assert_eq!(
            dispatcher.dispatch_one(now()).await.expect("second"),
            crate::outbox::DispatchOutcome::Suppressed
        );
        assert_eq!(port.sent.lock().expect("sent").len(), 1);
        let statuses: Vec<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT user_id, status, suppress_reason FROM bot.action_outbox
              WHERE idempotency_key LIKE 'dispatch-%' ORDER BY user_id",
        )
        .fetch_all(pool)
        .await
        .expect("statuses");
        assert_eq!(statuses[0], (1, "sent".to_string(), None));
        assert_eq!(
            statuses[1],
            (2, "suppressed".to_string(), Some("opted_out".to_string()))
        );
        let pattern: (Option<chrono::DateTime<Utc>>, Option<i32>) = sqlx::query_as(
            "SELECT last_pinged_at, ping_count_30d
               FROM activity.user_activity_patterns WHERE user_id = 1",
        )
        .fetch_one(pool)
        .await
        .expect("accounting");
        assert_eq!(pattern, (Some(now()), Some(1)));
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn due_check_creates_at_most_one_wave_per_interval() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let config = SurveyPulseConfig {
            enabled: true,
            interval_days: 75,
        };
        assert!(run_due_wave(db.pool(), 99, config, now())
            .await
            .expect("first due check")
            .is_some());
        assert!(
            run_due_wave(db.pool(), 99, config, now() + Duration::days(74))
                .await
                .expect("second due check")
                .is_none()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM bot.survey_waves")
                .fetch_one(db.pool())
                .await
                .expect("wave count"),
            1
        );
    }

    #[test]
    fn survey_dm_contains_five_scores_multiselect_and_freetext_button() {
        let body = survey_dm_body(17);
        let rows = body["components"].as_array().expect("component rows");
        let score_count = rows[0]["components"]
            .as_array()
            .expect("score buttons")
            .len();
        assert_eq!(score_count, 5);
        assert_eq!(rows[1]["components"][0]["type"], 3);
        assert_eq!(rows[1]["components"][0]["min_values"], 1);
        assert!(rows[1]["components"][0]["max_values"]
            .as_u64()
            .is_some_and(|value| value > 1));
        assert_eq!(
            rows[2]["components"][0]["custom_id"],
            "survey_pulse:freetext:17"
        );
    }
}
