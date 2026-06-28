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

use dl_db::Db;
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

pub const LOGS_CHANNEL_ID: u64 = 1374364800817303632;
pub const SURVEY_BASE_URL: &str = "https://deutsche-deadlock-community.de/survey";
pub const MIN_DAYS_BETWEEN_SURVEYS: i64 = 30;
pub const BUCKET_A_MAX_DAYS: i64 = 2;
pub const BUCKET_A_MAX_MESSAGES: i64 = 3;
pub const BUCKET_B_MIN_DAYS: i64 = 14;
pub const BUCKET_B_MIN_WEEKLY_SESSIONS: f64 = 0.5;
pub const BUCKET_B_MIN_MESSAGES: i64 = 30;
pub const BUCKET_B_MIN_VOICE_SECONDS: i64 = 3600;

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
    pub db: Db,
    pub port: Arc<dyn SurveyPort>,
    pub guild_name: String,
}

impl LeaveSurvey {
    pub fn new(db: Db, port: Arc<dyn SurveyPort>, guild_name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            db,
            port,
            guild_name: guild_name.into(),
        })
    }

    pub async fn on_member_remove(self: &Arc<Self>, guild_id: u64, user_id: u64) {
        // Opt-out + frischer Ban (15 s) + Survey-Sperre (30 Tage)
        let skip: bool = self
            .db
            .read(move |conn| {
                let opted_out: Option<i64> = conn
                    .query_row(
                        "SELECT opted_out FROM user_privacy WHERE user_id = ?1",
                        [user_id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .flatten_into();
                if opted_out.unwrap_or(0) != 0 {
                    return Ok(true);
                }
                let recent_ban: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM member_events
                          WHERE user_id = ?1 AND guild_id = ?2 AND event_type = 'ban'
                            AND timestamp >= datetime('now', '-15 seconds') LIMIT 1",
                        rusqlite::params![user_id, guild_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if recent_ban.is_some() {
                    return Ok(true);
                }
                let min_created =
                    chrono::Utc::now().timestamp() - MIN_DAYS_BETWEEN_SURVEYS * 24 * 3600;
                let recent_survey: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM member_leave_surveys
                          WHERE user_id = ?1 AND strftime('%s', created_at) >= ?2 LIMIT 1",
                        rusqlite::params![user_id, min_created.to_string()],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(recent_survey.is_some())
            })
            .await
            .unwrap_or(true);
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

        let (token_clone, display_clone, bucket_owned) =
            (token.clone(), display.clone(), bucket.to_string());
        let survey_id: Option<i64> = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO member_leave_surveys(
                       user_id, guild_id, left_at, display_name, user_bucket,
                       days_on_server, survey_token, dm_status
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'failed')",
                    rusqlite::params![
                        user_id,
                        guild_id,
                        chrono::Utc::now().timestamp(),
                        display_clone,
                        bucket_owned,
                        days,
                        token_clone,
                    ],
                )?;
                Ok(Some(conn.last_insert_rowid()))
            })
            .await
            .unwrap_or(None);
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
        let status_clone = status.clone();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE member_leave_surveys SET dm_status = ?1 WHERE id = ?2",
                    rusqlite::params![status_clone, survey_id],
                )
                .map(|_| ())
            })
            .await;
        self.port
            .post_log(format!(
                "👋 Leave-Survey für <@{user_id}> (Bucket {bucket}, {days} Tage, DM: {status})"
            ))
            .await;
    }

    async fn classify_user(&self, guild_id: u64, user_id: u64) -> (i64, &'static str) {
        let stats = self
            .db
            .read(move |conn| {
                let join_ts: Option<i64> = conn
                    .query_row(
                        "SELECT MIN(strftime('%s', timestamp)) FROM member_events
                          WHERE user_id = ?1 AND guild_id = ?2 AND event_type = 'join'",
                        rusqlite::params![user_id, guild_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten()
                    .and_then(|s| s.parse().ok());
                let voice_sessions: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM voice_session_log WHERE user_id = ?1",
                    [user_id],
                    |row| row.get(0),
                )?;
                let message_count: i64 = conn
                    .query_row(
                        "SELECT message_count FROM message_activity
                          WHERE user_id = ?1 AND guild_id = ?2",
                        rusqlite::params![user_id, guild_id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                let avg_weekly: f64 = conn
                    .query_row(
                        "SELECT avg_weekly_sessions FROM user_retention_tracking WHERE user_id = ?1",
                        [user_id],
                        |row| row.get::<_, Option<f64>>(0),
                    )
                    .optional()?
                    .flatten()
                    .unwrap_or(0.0);
                let voice_seconds: i64 = conn
                    .query_row(
                        "SELECT total_seconds FROM voice_stats WHERE user_id = ?1",
                        [user_id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                Ok((join_ts, voice_sessions, message_count, avg_weekly, voice_seconds))
            })
            .await
            .unwrap_or((None, 0, 0, 0.0, 0));
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

/// Hilfs-Trait: Option<Option<T>> → Option<T> (rusqlite-Komfort).
trait FlattenInto<T> {
    fn flatten_into(self) -> Option<T>;
}
impl<T> FlattenInto<T> for Option<T> {
    fn flatten_into(self) -> Option<T> {
        self
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
            let user_id = interaction.user_id;
            let survey_id: Option<i64> = self
                .survey
                .db
                .read(move |conn| {
                    conn.query_row(
                        "SELECT id FROM member_leave_surveys
                          WHERE user_id = ?1 AND responded_at IS NULL
                          ORDER BY id DESC LIMIT 1",
                        [user_id],
                        |row| row.get(0),
                    )
                    .optional()
                })
                .await
                .ok()
                .flatten();
            let Some(survey_id) = survey_id else {
                return BridgeReply::ephemeral_text(
                    "Zu dieser Auswahl wurde kein offener Survey gefunden.",
                );
            };
            let reason_clone = reason_code.clone();
            let _ = self
                .survey
                .db
                .write(move |conn| {
                    conn.execute(
                        "UPDATE member_leave_surveys SET reason_code = ?1
                          WHERE id = ?2 AND user_id = ?3",
                        rusqlite::params![reason_clone, survey_id, user_id],
                    )
                    .map(|_| ())
                })
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
        let user_id = interaction.user_id;
        let (follow_clone, extra_clone) = (follow_up.clone(), extra.clone());
        let _ = self
            .survey
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE member_leave_surveys
                        SET follow_up_question = ?1, follow_up_text = ?2,
                            extra_text = ?3, responded_at = ?4
                      WHERE id = ?5 AND user_id = ?6",
                    rusqlite::params![
                        question,
                        follow_clone,
                        extra_clone,
                        chrono::Utc::now().timestamp(),
                        survey_id,
                        user_id
                    ],
                )
                .map(|_| ())
            })
            .await;
        self.survey
            .port
            .post_log(format!(
                "📝 Leave-Survey-Antwort von <@{user_id}> (Survey {survey_id}, Grund {reason_code}):\n{}\n{}",
                follow_up.unwrap_or_else(|| "—".to_string()),
                extra.unwrap_or_else(|| "—".to_string()),
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
