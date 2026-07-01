//! Leave-Survey — Port von `cogs/leave_survey.py`.
//!
//! Wer den Server verlässt (nicht gebannt, kein Opt-out, max. alle 30 Tage),
//! bekommt eine Exit-Umfrage per DM — mit Fragen je Nutzer-Typ:
//! **A** (< 2 Tage, nie im Voice, < 3 Nachrichten), **B** (≥ 14 Tage und
//! aktiv: ≥ 0,5 Sessions/Woche ODER ≥ 30 Nachrichten ODER > 1 h Voice),
//! sonst **C**. Grund-Auswahl (Select, custom_ids
//! `leave_survey:reason:{bucket}` — bestehende DMs überleben den Umstieg)
//! → Nachfrage-Modal → alles in `member_leave_surveys` (+ Web-Link mit
//! Einmal-Token für ausführliches Feedback).

use std::sync::Arc;

use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::db::{advisory_lock, i64_to_i32, u64_to_i64};

pub const LOGS_CHANNEL_ID: u64 = 1374364800817303632;
pub const SURVEY_BASE_URL: &str = "https://deutsche-deadlock-community.de/survey";
pub const MIN_DAYS_BETWEEN_SURVEYS: i64 = 30;
pub const BUCKET_A_MAX_DAYS: i64 = 2;
pub const BUCKET_A_MAX_MESSAGES: i64 = 3;
pub const BUCKET_B_MIN_DAYS: i64 = 14;
pub const BUCKET_B_MIN_WEEKLY_SESSIONS: f64 = 0.5;
pub const BUCKET_B_MIN_MESSAGES: i64 = 30;
pub const BUCKET_B_MIN_VOICE_SECONDS: i64 = 3600;
const MEMBER_LEAVE_SURVEYS_ID_LOCK: i64 = 0x4451_0008_0010_0001;

pub fn reason_options(bucket: &str) -> &'static [(&'static str, &'static str)] {
    match bucket {
        "A" => &[
            (
                "Verifizierung/Onboarding hat nicht geklappt",
                "onboarding_failed",
            ),
            ("Server war unübersichtlich", "confusing"),
            ("Hab nicht gefunden wonach ich gesucht hab", "not_found"),
            ("War aus Versehen / falscher Server", "wrong_server"),
            ("Technisches Problem (Bot, Links, Channels)", "technical"),
            ("Anderer Grund", "other"),
        ],
        "B" => &[
            ("Stimmung/Community hat sich verändert", "mood_changed"),
            ("Konflikt oder Ärger mit jemandem", "conflict"),
            ("Zu wenig los / keine Mitspieler mehr", "inactive_server"),
            ("Spiele Deadlock kaum/nicht mehr", "stopped_playing"),
            ("Moderation / Regeln", "moderation"),
            ("Persönliche Gründe / keine Zeit", "personal"),
            ("Anderer Grund", "other"),
        ],
        _ => &[
            ("War nie richtig warm geworden", "never_warmed_up"),
            ("Zu wenig Aktivität / Mitspieler", "low_activity"),
            ("Spiele Deadlock nicht mehr", "stopped_playing"),
            ("Keine Zeit / Discord aufgeräumt", "no_time"),
            ("Hat mir nicht gefallen", "disliked"),
            ("Anderer Grund", "other"),
        ],
    }
}

pub fn follow_up_question(reason_code: &str) -> &'static str {
    match reason_code {
        "onboarding_failed" => "An welchem Schritt hing es genau?",
        "confusing" => "Was hast du gesucht und nicht gefunden?",
        "technical" => "Welcher Bot/Link/Channel und was ist passiert?",
        "conflict" => "Magst du sagen was vorgefallen ist? Bleibt vertraulich.",
        "mood_changed" => "Was hat sich verändert und seit wann fühlte es sich anders an?",
        "moderation" => "Welche Entscheidung oder Regel war das Problem?",
        "stopped_playing" => "Was müsste passieren damit du wieder Deadlock spielst?",
        "inactive_server" | "low_activity" => "Was hätte mehr los gemacht für dich?",
        "personal" | "no_time" => {
            "Alles gut - magst du trotzdem kurz sagen ob etwas am Server lag?"
        }
        "never_warmed_up" => "Was hätte dir geholfen anzukommen?",
        "not_found" => "Wonach hast du gesucht?",
        "wrong_server" => "Kein Problem - alles gut.",
        "disliked" => "Was genau hat dir nicht gefallen?",
        _ => "Erzähl gern in eigenen Worten.",
    }
}

pub fn bucket_description(bucket: &str, display_name: &str) -> String {
    match bucket {
        "A" => "Hey, du warst nur kurz bei uns - schade! Woran lag es?".to_string(),
        "B" => format!(
            "Hey {display_name}, du warst eine Weile aktiv dabei - schade dass du gegangen bist. Ehrliches Feedback hilft uns wirklich."
        ),
        _ => "Schade dass du gegangen bist. Magst du kurz sagen warum?".to_string(),
    }
}

pub fn truncate_label(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit - 3).collect();
    format!("{}...", cut.trim_end())
}

/// Bucket-Klassifikation (pure, wie `_classify`).
pub fn classify(
    days_on_server: i64,
    voice_sessions: i64,
    message_count: i64,
    avg_weekly_sessions: f64,
    voice_total_seconds: i64,
) -> &'static str {
    if days_on_server < BUCKET_A_MAX_DAYS
        && voice_sessions == 0
        && message_count < BUCKET_A_MAX_MESSAGES
    {
        return "A";
    }
    if days_on_server >= BUCKET_B_MIN_DAYS
        && (avg_weekly_sessions >= BUCKET_B_MIN_WEEKLY_SESSIONS
            || message_count >= BUCKET_B_MIN_MESSAGES
            || voice_total_seconds > BUCKET_B_MIN_VOICE_SECONDS)
    {
        return "B";
    }
    "C"
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait SurveyPort: Send + Sync {
    /// DM mit Embed + Bucket-Select → "sent" | "blocked" | "failed".
    async fn send_survey_dm(&self, user_id: u64, embed: Value, components: Value) -> String;
    async fn post_log(&self, text: String);
    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String>;
}

pub struct LeaveSurvey {
    pub pool: PgPool,
    pub port: Arc<dyn SurveyPort>,
    pub guild_name: String,
}

impl LeaveSurvey {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn SurveyPort>,
        guild_name: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            guild_name: guild_name.into(),
        })
    }

    pub async fn on_member_remove(self: &Arc<Self>, guild_id: u64, user_id: u64) {
        let (Ok(guild_id_i64), Ok(user_id_i64)) = (
            u64_to_i64(guild_id, "guild_id"),
            u64_to_i64(user_id, "user_id"),
        ) else {
            return;
        };
        // Opt-out + frischer Ban (15 s) + Survey-Sperre (30 Tage)
        let opted_out = crate::privacy::is_opted_out(&self.pool, user_id_i64).await;
        let recent_ban = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                  FROM activity.member_events
                 WHERE user_id = $1
                   AND guild_id = $2
                   AND event_type = 'ban'
                   AND occurred_at >= now() - INTERVAL '15 seconds'
            ) AS "exists!"
            "#,
            user_id_i64,
            guild_id_i64,
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(true);
        let min_created = chrono::Utc::now() - chrono::Duration::days(MIN_DAYS_BETWEEN_SURVEYS);
        let recent_survey = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                  FROM activity.member_leave_surveys
                 WHERE user_id = $1
                   AND created_at >= $2
            ) AS "exists!"
            "#,
            user_id_i64,
            min_created,
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(true);
        let skip = opted_out || recent_ban || recent_survey;
        if skip {
            return;
        }

        let (days, bucket) = self.classify_user(guild_id, user_id).await;
        let token = {
            // urlsafe base64 ohne Padding aus 16 Zufallsbytes (wie token_urlsafe(16))
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>())
        };
        let display = self
            .port
            .display_name(guild_id, user_id)
            .await
            .unwrap_or_else(|| format!("User {user_id}"));

        let survey_id = self
            .insert_survey(
                user_id_i64,
                guild_id_i64,
                display.clone(),
                bucket.to_string(),
                days,
                token.clone(),
            )
            .await;
        let Some(survey_id) = survey_id else { return };

        let description = bucket_description(bucket, &display);
        let survey_url = format!("{SURVEY_BASE_URL}?t={token}");
        let embed = json!({
            "title": format!("Feedback zu {}", self.guild_name),
            "description": format!(
                "{description}\n\nBitte wähle unten den passendsten Grund aus.\n\n\
        Wenn du ausführlicher Feedback geben magst (auch mit Bildern): {survey_url}"
            ),
            "color": 0x5865F2,
        });
        let options: Vec<Value> = reason_options(bucket)
            .iter()
            .map(|(label, code)| json!({ "label": truncate_label(label, 45), "value": code }))
            .collect();
        let components = json!([{ "type": 1, "components": [{
            "type": 3, "custom_id": format!("leave_survey:reason:{bucket}"),
            "placeholder": "Grund auswählen …",
            "options": options, "min_values": 1, "max_values": 1,
        }]}]);

        let status = self.port.send_survey_dm(user_id, embed, components).await;
        let _ = sqlx::query!(
            r#"
            UPDATE activity.member_leave_surveys
               SET dm_status = $1
             WHERE id = $2
            "#,
            status.clone(),
            survey_id,
        )
        .execute(&self.pool)
        .await;
        self.port
            .post_log(format!(
                "👋 Leave-Survey für <@{user_id}> (Bucket {bucket}, {days} Tage, DM: {status})"
            ))
            .await;
    }

    async fn insert_survey(
        &self,
        user_id: i64,
        guild_id: i64,
        display_name: String,
        user_bucket: String,
        days_on_server: i64,
        survey_token: String,
    ) -> Option<i64> {
        let days_on_server = i64_to_i32(days_on_server, "days_on_server").ok()?;
        let mut tx = self.pool.begin().await.ok()?;
        advisory_lock(&mut tx, MEMBER_LEAVE_SURVEYS_ID_LOCK)
            .await
            .ok()?;
        let duplicate = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM activity.member_leave_surveys WHERE survey_token = $1
            ) AS "exists!"
            "#,
            survey_token,
        )
        .fetch_one(&mut *tx)
        .await
        .ok()?;
        if duplicate {
            return None;
        }
        let next_id = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(MAX(id), 0) + 1 AS "next_id!: i64"
              FROM activity.member_leave_surveys
            "#
        )
        .fetch_one(&mut *tx)
        .await
        .ok()?;
        sqlx::query!(
            r#"
            INSERT INTO activity.member_leave_surveys(
                id, user_id, guild_id, left_at, display_name, user_bucket,
                days_on_server, survey_token, dm_status, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'failed', $4)
            "#,
            next_id,
            user_id,
            guild_id,
            chrono::Utc::now(),
            display_name,
            user_bucket,
            days_on_server,
            survey_token,
        )
        .execute(&mut *tx)
        .await
        .ok()?;
        tx.commit().await.ok()?;
        Some(next_id)
    }

    async fn classify_user(&self, guild_id: u64, user_id: u64) -> (i64, &'static str) {
        let (Ok(guild_id), Ok(user_id)) = (
            u64_to_i64(guild_id, "guild_id"),
            u64_to_i64(user_id, "user_id"),
        ) else {
            return (0, "C");
        };
        let join_at = sqlx::query_scalar!(
            r#"
            SELECT MIN(occurred_at) AS "join_at?"
              FROM activity.member_events
             WHERE user_id = $1 AND guild_id = $2 AND event_type = 'join'
            "#,
            user_id,
            guild_id,
        )
        .fetch_one(&self.pool)
        .await
        .ok()
        .flatten();
        let voice_sessions = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.voice_session_log
             WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        let message_count = sqlx::query_scalar!(
            r#"
            SELECT message_count
              FROM activity.message_activity
             WHERE user_id = $1 AND guild_id = $2
            "#,
            user_id,
            guild_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .unwrap_or(0);
        let avg_weekly = sqlx::query_scalar!(
            r#"
            SELECT avg_weekly_sessions
              FROM activity.user_retention_tracking
             WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .unwrap_or(0.0);
        let voice_seconds = sqlx::query_scalar!(
            r#"
            SELECT total_seconds
              FROM voice.voice_stats
             WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(0);
        let stats = (
            join_at.map(|dt| dt.timestamp()),
            voice_sessions,
            message_count,
            avg_weekly,
            voice_seconds,
        );
        let (join_ts, voice_sessions, message_count, avg_weekly, voice_seconds) = stats;
        let days = join_ts
            .map(|ts| ((chrono::Utc::now().timestamp() - ts) / 86400).max(0))
            .unwrap_or(0);
        (
            days,
            classify(
                days,
                voice_sessions,
                message_count,
                avg_weekly,
                voice_seconds,
            ),
        )
    }
}

// ── Interaction-Handler ────────────────────────────────────────────────────

struct SurveyHandler {
    survey: Arc<LeaveSurvey>,
}

#[async_trait::async_trait]
impl InteractionHandler for SurveyHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        // Grund gewählt → reason persistieren + Nachfrage-Modal
        if interaction.custom_id.starts_with("leave_survey:reason:") {
            let Some(reason_code) = interaction.values.first().cloned() else {
                return BridgeReply::ephemeral_text("Keine Auswahl.");
            };
            let Ok(user_id) = u64_to_i64(interaction.user_id, "interaction.user_id") else {
                return BridgeReply::ephemeral_text(
                    "Diese Discord-ID kann nicht verarbeitet werden.",
                );
            };
            let survey_id = sqlx::query_scalar!(
                r#"
                SELECT id
                  FROM activity.member_leave_surveys
                 WHERE user_id = $1 AND responded_at IS NULL
                 ORDER BY id DESC
                 LIMIT 1
                "#,
                user_id,
            )
            .fetch_optional(&self.survey.pool)
            .await
            .ok()
            .flatten();
            let Some(survey_id) = survey_id else {
                return BridgeReply::ephemeral_text(
                    "Zu dieser Auswahl wurde kein offener Survey gefunden.",
                );
            };
            let _ = sqlx::query!(
                r#"
                UPDATE activity.member_leave_surveys
                   SET reason_code = $1
                 WHERE id = $2 AND user_id = $3
                "#,
                &reason_code,
                survey_id,
                user_id,
            )
            .execute(&self.survey.pool)
            .await;
            let question = follow_up_question(&reason_code);
            return BridgeReply {
                modal: Some(ModalSpec {
                    custom_id: format!("leave_survey:modal:{survey_id}:{reason_code}"),
                    title: "Kurzes Feedback".to_string(),
                    fields: vec![
                        ModalField {
                            custom_id: "follow_up".to_string(),
                            label: truncate_label(question, 45),
                            placeholder: String::new(),
                            required: false,
                            min_length: 0,
                            max_length: 1000,
                            paragraph: true,
                        },
                        ModalField {
                            custom_id: "extra".to_string(),
                            label: "Möchtest du noch etwas loswerden?".to_string(),
                            placeholder: String::new(),
                            required: false,
                            min_length: 0,
                            max_length: 1000,
                            paragraph: true,
                        },
                    ],
                }),
                ..BridgeReply::default()
            };
        }

        // Modal-Submit: leave_survey:modal:{id}:{reason}
        let parts: Vec<&str> = interaction.custom_id.split(':').collect();
        let survey_id: i64 = parts.get(2).and_then(|raw| raw.parse().ok()).unwrap_or(0);
        let reason_code = parts.get(3).copied().unwrap_or("other").to_string();
        let question = follow_up_question(&reason_code).to_string();
        let get_field = |key: &str| {
            interaction
                .options
                .get(key)
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let follow_up = get_field("follow_up");
        let extra = get_field("extra");
        let Ok(user_id) = u64_to_i64(interaction.user_id, "interaction.user_id") else {
            return BridgeReply::ephemeral_text("Diese Discord-ID kann nicht verarbeitet werden.");
        };
        let _ = sqlx::query!(
            r#"
            UPDATE activity.member_leave_surveys
               SET follow_up_question = $1,
                   follow_up_text = $2,
                   extra_text = $3,
                   responded_at = $4
             WHERE id = $5 AND user_id = $6
            "#,
            question,
            follow_up,
            extra,
            chrono::Utc::now(),
            survey_id,
            user_id,
        )
        .execute(&self.survey.pool)
        .await;
        self.survey
            .port
            .post_log(format!(
                "📝 Leave-Survey-Antwort von <@{user_id}> (Survey {survey_id}, Grund {reason_code}):\n{}\n{}",
                get_field("follow_up").unwrap_or_else(|| "—".to_string()),
                get_field("extra").unwrap_or_else(|| "—".to_string()),
            ))
            .await;
        BridgeReply::ephemeral_text("Danke für dein ehrliches Feedback.")
    }
}

pub fn register(router: &mut InteractionRouter, survey: Arc<LeaveSurvey>) {
    let handler = Arc::new(SurveyHandler { survey });
    router.on_prefix("leave_survey:reason:", handler.clone());
    router.on_prefix("leave_survey:modal:", handler);
}

/// Member-Remove-Subscriber.
pub fn spawn(
    survey: Arc<LeaveSurvey>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_members();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::MemberEvent::Remove {
                    guild_id, user_id, ..
                }) => {
                    survey.on_member_remove(guild_id, user_id).await;
                }
                Ok(_) => {}
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
    fn klassifikation_wie_python() {
        // A: < 2 Tage, nie Voice, < 3 Nachrichten
        assert_eq!(classify(1, 0, 2, 0.0, 0), "A");
        assert_eq!(classify(1, 1, 2, 0.0, 0), "C"); // war im Voice → kein A
        assert_eq!(classify(1, 0, 5, 0.0, 0), "C"); // zu viele Nachrichten
                                                    // B: ≥ 14 Tage UND aktiv (eine der drei Bedingungen)
        assert_eq!(classify(20, 0, 0, 0.6, 0), "B");
        assert_eq!(classify(20, 0, 35, 0.0, 0), "B");
        assert_eq!(classify(20, 0, 0, 0.0, 4000), "B");
        assert_eq!(classify(20, 0, 0, 0.0, 0), "C"); // alt aber inaktiv
        assert_eq!(classify(5, 3, 50, 1.0, 9999), "C"); // aktiv aber zu kurz
    }

    #[test]
    fn texte_und_labels() {
        assert!(bucket_description("B", "Anna").contains("Hey Anna,"));
        assert!(!bucket_description("A", "Anna").contains("Anna"));
        assert_eq!(
            follow_up_question("inactive_server"),
            "Was hätte mehr los gemacht für dich?"
        );
        assert_eq!(
            follow_up_question("unbekannt"),
            "Erzähl gern in eigenen Worten."
        );
        assert_eq!(truncate_label("kurz", 45), "kurz");
        let long = "Dies ist eine sehr lange Beschriftung die gekuerzt werden muss";
        let cut = truncate_label(long, 45);
        assert!(cut.chars().count() <= 45);
        assert!(cut.ends_with("..."));
        assert_eq!(reason_options("A").len(), 6);
        assert_eq!(reason_options("B").len(), 7);
        assert_eq!(reason_options("C").len(), 6);
    }
}

#[cfg(all(test, feature = "testing"))]
mod pg_tests {
    use super::*;
    use dl_central_db::testing::test_pool;
    use std::sync::Arc;

    struct MockSurveyPort;

    #[async_trait::async_trait]
    impl SurveyPort for MockSurveyPort {
        async fn send_survey_dm(&self, _user_id: u64, _embed: Value, _components: Value) -> String {
            "sent".to_string()
        }

        async fn post_log(&self, _text: String) {}

        async fn display_name(&self, _guild_id: u64, _user_id: u64) -> Option<String> {
            Some("Tester".to_string())
        }
    }

    #[tokio::test]
    async fn insert_survey_blockiert_doppelten_survey_token() {
        let db = test_pool().await.expect("test_pool");
        let survey = LeaveSurvey::new(db.pool().clone(), Arc::new(MockSurveyPort), "Test Guild");

        let token = "leave-survey-dupe-token".to_string();
        let first = survey
            .insert_survey(
                7001,
                1,
                "Tester One".to_string(),
                "A".to_string(),
                1,
                token.clone(),
            )
            .await;
        let second = survey
            .insert_survey(
                7002,
                1,
                "Tester Two".to_string(),
                "B".to_string(),
                14,
                token.clone(),
            )
            .await;

        assert_eq!(first, Some(1));
        assert_eq!(second, None);
        let rows = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.member_leave_surveys
             WHERE survey_token = $1
            "#,
            token,
        )
        .fetch_one(db.pool())
        .await
        .expect("survey token row count");
        assert_eq!(rows, 1);
    }
}
