//! Voice-Feedback-DMs — Port des Feedback-Systems aus
//! `cogs/voice_activity_tracker.py` (schließt die letzte 4a-Lücke).
//!
//! Erste Session ≥ 5 min mit Mitspielern → freundliche Feedback-DM mit
//! Button (`voice_feedback:start`, custom_id unverändert — alte DMs
//! funktionieren weiter) → 4-Fragen-Modal → Antwort wird gespeichert und
//! an den Owner weitergeleitet. Zweit-Feedback nach ≥ 4 verschiedenen
//! Voice-Tagen, einmalig.

use std::sync::Arc;

use dl_db::Db;
use dl_discord::interactions::{ChannelSender, ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler, InteractionRouter, MessageEvent,
};
use rusqlite::OptionalExtension;

pub const MIN_SECONDS: i64 = 300;
pub const MAX_NAMES: usize = 10;
pub const SECOND_MIN_DAYS: i64 = 4;
pub const FORWARD_USER_ID: u64 = 662995601738170389;
pub const START_CUSTOM_ID: &str = "voice_feedback:start";
pub const MODAL_CUSTOM_ID: &str = "voice_feedback:modal";
pub const RESPONSE_WINDOW_SECONDS: i64 = 72 * 3600;
pub const ACK_TEXT: &str = "Danke für dein Feedback! 🙌\n\n\
Wenn sonst irgendwas sein sollte, kannst du dich jederzeit an unser Team wenden – hier beißt keiner und jeder hilft gerne! :) \
Falls es doch mal ein Problem geben sollte, wende dich bitte direkt an einen Community Moderator (bei kleineren Dingen), einen Moderator oder an den Owner. ❤️";

struct FeedbackRequestResponse {
    id: i64,
    sent_at_ts: i64,
    status: String,
    request_type: String,
    co_player_names: String,
    channel_name: String,
    duration_seconds: i64,
}

/// DM-Text wie das Original (first/second).
pub fn build_message(display_name: &str, request_type: &str, co_player_names: &[String]) -> String {
    let mut names: Vec<String> = co_player_names.to_vec();
    let extra = names.len().saturating_sub(MAX_NAMES);
    names.truncate(MAX_NAMES);
    let mut co_text = names.join(", ");
    if extra > 0 {
        co_text = format!("{co_text} (+{extra} weitere)");
    }
    let mut lines: Vec<String> = if request_type == "second" {
        vec![
            format!("Hey {display_name}, danke für deine Voice-Runden."),
            "Kurzes Update: Was läuft gut, was nervt, was sollen wir fixen?".to_string(),
            "Button drücken und in 1-2 Sätzen Feedback dalassen.".to_string(),
        ]
    } else {
        vec![
            format!("Hey {display_name}!"),
            "Wie waren deine ersten Runden bei uns? Wir würden mega gern wissen, wie's dir gefallen hat :)".to_string(),
            "Hau einfach kurz auf den Button und lass uns wissen, was gut lief oder auch nicht und was vielleicht noch besser gehen könnte. Dauert nur ne Minute und wir freuen uns echt über deine Meinung ❤️".to_string(),
        ]
    };
    if !co_player_names.is_empty() {
        lines.push(format!("Mit im Call waren u.a.: {co_text}"));
    }
    lines.join("\n\n")
}

/// Das 4-Fragen-Modal (Labels/Placeholder wortgleich).
pub fn feedback_modal(request_id: i64) -> ModalSpec {
    let field =
        |custom_id: &str, label: &str, placeholder: &str, required: bool, max: u16| ModalField {
            custom_id: custom_id.to_string(),
            label: label.to_string(),
            placeholder: placeholder.to_string(),
            required,
            min_length: 0,
            max_length: max,
            paragraph: true,
        };
    ModalSpec {
        custom_id: format!("{MODAL_CUSTOM_ID}:{request_id}"),
        title: "Kurzes Voice-Feedback".to_string(),
        fields: vec![
            field(
                "q1",
                "Wie war dein Eindruck?",
                "Kurz bewerten (1-10) und warum: Stimmung, Ablauf, Technik",
                true,
                500,
            ),
            field(
                "q2",
                "Was sollen wir verbessern?",
                "1-2 klare Punkte: Moderation, Themen, Ablauf, Technik, Verhalten",
                false,
                900,
            ),
            field(
                "q3",
                "Wie lief es mit den anderen?",
                "Highlights oder Probleme im Miteinander (gern mit Namen, falls relevant)",
                false,
                900,
            ),
            field(
                "q4",
                "Noch etwas, das wir wissen sollten?",
                "Wünsche, Probleme, Ideen, was dir aufgefallen ist",
                false,
                900,
            ),
        ],
    }
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait FeedbackPort: Send + Sync {
    /// DM mit Feedback-Button → (status, message_id):
    /// "sent" | "forbidden" (DMs zu) | "error".
    async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>);
    async fn forward_to_owner(&self, owner_id: u64, text: String);
    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String>;
}

pub struct VoiceFeedback {
    pub db: Db,
    pub port: Arc<dyn FeedbackPort>,
}

impl VoiceFeedback {
    pub fn new(db: Db, port: Arc<dyn FeedbackPort>) -> Arc<Self> {
        Arc::new(Self { db, port })
    }

    /// Vom Tracker nach jeder finalisierten Session gerufen.
    #[allow(clippy::too_many_arguments)]
    pub async fn on_session_end(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        channel_name: String,
        co_player_ids: Vec<u64>,
        seconds: i64,
        was_first_session: bool,
    ) {
        if was_first_session && seconds >= MIN_SECONDS && !co_player_ids.is_empty() {
            self.send_request(
                guild_id,
                user_id,
                channel_id,
                &channel_name,
                &co_player_ids,
                seconds,
                "first",
            )
            .await;
            return;
        }
        // Zweit-Feedback: hatte first, noch kein second, ≥ 4 Voice-Tage
        if !self.had_request(user_id, "first").await || self.had_request(user_id, "second").await {
            return;
        }
        if self.distinct_voice_days(user_id).await < SECOND_MIN_DAYS {
            return;
        }
        self.send_request(
            guild_id,
            user_id,
            channel_id,
            &channel_name,
            &co_player_ids,
            seconds,
            "second",
        )
        .await;
    }

    async fn had_request(&self, user_id: u64, request_type: &str) -> bool {
        let request_type = request_type.to_string();
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT 1 FROM voice_feedback_requests
                      WHERE user_id = ?1 AND request_type = ?2 LIMIT 1",
                    rusqlite::params![user_id, request_type],
                    |_| Ok(()),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .is_some()
    }

    async fn distinct_voice_days(&self, user_id: u64) -> i64 {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT COUNT(DISTINCT date(started_at)) FROM voice_session_log
                      WHERE user_id = ?1",
                    [user_id],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_request(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        channel_name: &str,
        co_player_ids: &[u64],
        seconds: i64,
        request_type: &str,
    ) {
        let mut names: Vec<String> = Vec::new();
        for co_id in co_player_ids.iter().take(MAX_NAMES + 5) {
            if let Some(name) = self.port.display_name(guild_id, *co_id).await {
                names.push(name);
            }
        }
        let display = self
            .port
            .display_name(guild_id, user_id)
            .await
            .unwrap_or_else(|| format!("User {user_id}"));
        let text = build_message(&display, request_type, &names);
        let co_text = if names.is_empty() {
            "-".to_string()
        } else {
            names.join(", ")
        };

        // Request anlegen (alte pending-Requests des Users vorher räumen)
        let (channel_name_owned, request_type_owned, co_text_owned) =
            (channel_name.to_string(), request_type.to_string(), co_text);
        let request_id: Option<i64> = self
            .db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM voice_feedback_requests
                      WHERE user_id = ?1 AND status = 'pending'",
                    [user_id],
                )?;
                conn.execute(
                    "INSERT INTO voice_feedback_requests(
                       user_id, guild_id, channel_id, channel_name, co_player_names,
                       duration_seconds, request_type, status
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending')",
                    rusqlite::params![
                        user_id,
                        guild_id,
                        channel_id,
                        channel_name_owned,
                        co_text_owned,
                        seconds,
                        request_type_owned,
                    ],
                )?;
                Ok(Some(conn.last_insert_rowid()))
            })
            .await
            .unwrap_or(None);
        let Some(request_id) = request_id else { return };

        let (status, message_id) = self.port.send_feedback_dm(user_id, text).await;
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE voice_feedback_requests
                        SET status = ?1, prompt_message_id = ?2 WHERE id = ?3",
                    rusqlite::params![status, message_id, request_id],
                )
                .map(|_| ())
            })
            .await;
    }

    /// Freitext-Antworten auf die Feedback-DM (Python `on_message` in DMs).
    /// Gibt den Dankestext zurück, wenn der User noch keine Antwort bestätigt
    /// bekommen hat.
    pub async fn handle_dm_message(self: &Arc<Self>, event: MessageEvent) -> Option<&'static str> {
        if event.guild_id.is_some() {
            return None;
        }
        let content = event.content.trim().to_string();
        if content.is_empty()
            || dl_community::privacy::is_opted_out(&self.db, event.author_id as i64).await
        {
            return None;
        }

        let user_id = event.author_id;
        let row: Option<FeedbackRequestResponse> = self
            .db
            .read(move |conn| {
                conn.query_row(
                    "SELECT id, sent_at_ts, status, request_type, co_player_names, channel_name, duration_seconds
                       FROM voice_feedback_requests
                      WHERE user_id = ?1
                      ORDER BY sent_at_ts DESC
                      LIMIT 1",
                    [user_id],
                    |row| {
                        Ok(FeedbackRequestResponse {
                            id: row.get(0)?,
                            sent_at_ts: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            status: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                            request_type: row
                                .get::<_, Option<String>>(3)?
                                .unwrap_or_else(|| "first".to_string()),
                            co_player_names: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                            channel_name: row
                                .get::<_, Option<String>>(5)?
                                .unwrap_or_else(|| "Voice".to_string()),
                            duration_seconds: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                        })
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten();
        let row = row?;

        let now = chrono::Utc::now().timestamp();
        if row.sent_at_ts > 0 && now.saturating_sub(row.sent_at_ts) > RESPONSE_WINDOW_SECONDS {
            return None;
        }

        let should_ack = row.status != "responded";
        let request_id = row.id;
        let message_id = event.message_id;
        let content_for_db = content.clone();
        let stored = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO voice_feedback_responses(request_id, user_id, message_id, content)
                     VALUES(?1, ?2, ?3, ?4)",
                    rusqlite::params![request_id, user_id, message_id, content_for_db],
                )?;
                conn.execute(
                    "UPDATE voice_feedback_requests SET status='responded' WHERE id = ?1",
                    [request_id],
                )
                .map(|_| ())
            })
            .await;
        if let Err(err) = stored {
            tracing::warn!(%err, request_id, user_id, "VoiceFeedback: Freitext-Antwort speichern fehlgeschlagen");
            return None;
        }

        let duration_min = if row.duration_seconds > 0 {
            std::cmp::max(1, row.duration_seconds / 60).to_string()
        } else {
            "?".to_string()
        };
        let co_players = if row.co_player_names.trim().is_empty() {
            "—".to_string()
        } else {
            row.co_player_names
        };
        self.port
            .forward_to_owner(
                FORWARD_USER_ID,
                format!(
                    "📩 Neues Voice-Feedback (Req #{}, Typ: {})\nVon: {} ({})\nKanal: {}\nDauer: {duration_min} Min\nMit im Call: {co_players}\n\nAntwort:\n{content}",
                    row.id, row.request_type, event.author_display_name, event.author_id, row.channel_name
                ),
            )
            .await;

        should_ack.then_some(ACK_TEXT)
    }
}

/// Button + Modal-Submit (custom_ids: voice_feedback:start / :modal:{id}).
struct FeedbackHandler {
    feedback: Arc<VoiceFeedback>,
}

#[async_trait::async_trait]
impl InteractionHandler for FeedbackHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.custom_id == START_CUSTOM_ID {
            // Neuester gesendeter Request des klickenden Users
            let user_id = interaction.user_id;
            let request_id: Option<i64> = self
                .feedback
                .db
                .read(move |conn| {
                    conn.query_row(
                        "SELECT id FROM voice_feedback_requests
                          WHERE user_id = ?1 AND status IN ('sent', 'pending')
                          ORDER BY id DESC LIMIT 1",
                        [user_id],
                        |row| row.get(0),
                    )
                    .optional()
                })
                .await
                .ok()
                .flatten();
            let Some(request_id) = request_id else {
                return BridgeReply::ephemeral_text(
                    "Diese Feedback-Anfrage ist nicht mehr offen — trotzdem danke!",
                );
            };
            return BridgeReply {
                modal: Some(feedback_modal(request_id)),
                ..BridgeReply::default()
            };
        }

        // Modal-Submit: voice_feedback:modal:{request_id}
        let request_id: i64 = interaction
            .custom_id
            .rsplit(':')
            .next()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0);
        let answers: Vec<String> = ["q1", "q2", "q3", "q4"]
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let value = interaction
                    .options
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if value.is_empty() {
                    format!("{}) —", index + 1)
                } else {
                    format!("{}) {value}", index + 1)
                }
            })
            .collect();
        let combined = answers.join("\n");
        let user_id = interaction.user_id;
        let combined_clone = combined.clone();
        let _ = self
            .feedback
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO voice_feedback_responses(request_id, user_id, content)
                     VALUES(?1, ?2, ?3)",
                    rusqlite::params![request_id, user_id, combined_clone],
                )?;
                conn.execute(
                    "UPDATE voice_feedback_requests SET status='responded' WHERE id = ?1",
                    [request_id],
                )
                .map(|_| ())
            })
            .await;

        // An den Owner weiterleiten (best-effort)
        self.feedback
            .port
            .forward_to_owner(
                FORWARD_USER_ID,
                format!("📝 Voice-Feedback von <@{user_id}> (Request {request_id}):\n{combined}"),
            )
            .await;

        BridgeReply::ephemeral_text(ACK_TEXT)
    }
}

pub fn register(router: &mut InteractionRouter, feedback: Arc<VoiceFeedback>) {
    let handler = Arc::new(FeedbackHandler { feedback });
    router.on_custom_id(START_CUSTOM_ID, handler.clone());
    router.on_prefix(format!("{MODAL_CUSTOM_ID}:"), handler);
}

pub fn spawn_dm_responses(
    feedback: Arc<VoiceFeedback>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let channel_id = event.channel_id;
                    let Some(ack) = feedback.handle_dm_message(event).await else {
                        continue;
                    };
                    let _ = sender.send_to_channel(channel_id, Some(ack), &[]).await;
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
    use std::sync::Mutex as StdMutex;

    #[test]
    fn texte_wie_python() {
        let names: Vec<String> = (1..=12).map(|i| format!("Spieler{i}")).collect();
        let text = build_message("Anna", "first", &names);
        assert!(text.starts_with("Hey Anna!"));
        assert!(text.contains("Wie waren deine ersten Runden bei uns?"));
        assert!(text.contains("(+2 weitere)")); // 12 Namen → 10 + 2
        let text = build_message("Ben", "second", &[]);
        assert!(text.contains("danke für deine Voice-Runden"));
        assert!(!text.contains("Mit im Call"));
    }

    #[test]
    fn modal_aufbau() {
        let modal = feedback_modal(42);
        assert_eq!(modal.custom_id, "voice_feedback:modal:42");
        assert_eq!(modal.fields.len(), 4);
        assert!(modal.fields[0].required);
        assert!(!modal.fields[1].required);
        assert!(modal.fields.iter().all(|f| f.paragraph));
    }

    struct MockPort {
        dms: StdMutex<Vec<(u64, String)>>,
        forwards: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl FeedbackPort for MockPort {
        async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>) {
            self.dms.lock().expect("lock").push((user_id, text));
            ("sent".to_string(), Some(777))
        }
        async fn forward_to_owner(&self, _owner_id: u64, text: String) {
            self.forwards.lock().expect("lock").push(text);
        }
        async fn display_name(&self, _guild_id: u64, user_id: u64) -> Option<String> {
            Some(format!("User {user_id}"))
        }
    }

    const DDLS: [&str; 3] = [
        "CREATE TABLE voice_feedback_requests(id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, guild_id INTEGER, channel_id INTEGER, channel_name TEXT, co_player_names TEXT, duration_seconds INTEGER, request_type TEXT DEFAULT 'first', status TEXT, error_message TEXT, prompt_message_id INTEGER, sent_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now')))",
        "CREATE TABLE voice_feedback_responses(id INTEGER PRIMARY KEY AUTOINCREMENT, request_id INTEGER, user_id INTEGER NOT NULL, message_id INTEGER, content TEXT, received_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now')))",
        "CREATE TABLE voice_session_log(id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, started_at DATETIME, ended_at DATETIME, duration_seconds INTEGER)",
    ];

    async fn setup() -> (tempfile::TempDir, Arc<VoiceFeedback>, Arc<MockPort>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        let port = Arc::new(MockPort {
            dms: StdMutex::new(Vec::new()),
            forwards: StdMutex::new(Vec::new()),
        });
        (dir, VoiceFeedback::new(db, port.clone()), port)
    }

    fn dm_event(user_id: u64, message_id: u64, content: &str) -> MessageEvent {
        MessageEvent {
            guild_id: None,
            channel_id: 900,
            message_id,
            author_id: user_id,
            author_display_name: "FeedbackUser".to_string(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_is_staff: false,
            content: content.to_string(),
            message_created_at: chrono::Utc::now().timestamp(),
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            author_created_at: 0,
            author_joined_at: None,
        }
    }

    #[tokio::test]
    async fn erste_session_sendet_dm_und_persistiert() {
        let (_dir, feedback, port) = setup().await;
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200, 300], 600, true)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);
        let (status, rtype): (String, String) = feedback
            .db
            .read(|conn| {
                conn.query_row(
                    "SELECT status, request_type FROM voice_feedback_requests WHERE user_id = 100",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .await
            .expect("request");
        assert_eq!((status.as_str(), rtype.as_str()), ("sent", "first"));

        // Kurze Session / allein → kein weiterer Request
        feedback
            .on_session_end(1, 400, 10, "Lane 1".into(), vec![], 600, true)
            .await;
        feedback
            .on_session_end(1, 500, 10, "Lane 1".into(), vec![200], 100, true)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn zweit_feedback_erst_nach_vier_tagen() {
        let (_dir, feedback, port) = setup().await;
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, true)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);

        // Nur 2 verschiedene Tage → noch kein second
        feedback
            .db
            .write(|conn| {
                conn.execute_batch(
                    "INSERT INTO voice_session_log(user_id, started_at) VALUES
                       (100, '2026-06-01 18:00:00'), (100, '2026-06-02 18:00:00');",
                )
            })
            .await
            .expect("seed");
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, false)
            .await;
        assert_eq!(port.dms.lock().expect("lock").len(), 1);

        // 4 Tage → second kommt genau einmal
        feedback
            .db
            .write(|conn| {
                conn.execute_batch(
                    "INSERT INTO voice_session_log(user_id, started_at) VALUES
                       (100, '2026-06-03 18:00:00'), (100, '2026-06-04 18:00:00');",
                )
            })
            .await
            .expect("seed");
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, false)
            .await;
        feedback
            .on_session_end(1, 100, 10, "Lane 1".into(), vec![200], 600, false)
            .await;
        let dms = port.dms.lock().expect("lock");
        assert_eq!(dms.len(), 2);
        assert!(dms[1].1.contains("danke für deine Voice-Runden"));
    }

    #[tokio::test]
    async fn dm_freitext_antwort_wie_python() {
        let (_dir, feedback, port) = setup().await;
        let now = chrono::Utc::now().timestamp();
        feedback
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO voice_feedback_requests(
                       user_id, guild_id, channel_id, channel_name, co_player_names,
                       duration_seconds, request_type, status, sent_at_ts
                     ) VALUES(100, 1, 10, 'Lane 1', 'Alice, Bob', 601, 'first', 'sent', ?1)",
                    [now],
                )
                .map(|_| ())
            })
            .await
            .expect("seed");

        let ack = feedback
            .handle_dm_message(dm_event(100, 700, "War gut, aber bitte mehr Moderation."))
            .await;
        assert_eq!(ack, Some(ACK_TEXT));
        let (status, message_id, content): (String, i64, String) = feedback
            .db
            .read(|conn| {
                conn.query_row(
                    "SELECT r.status, a.message_id, a.content
                       FROM voice_feedback_requests r
                       JOIN voice_feedback_responses a ON a.request_id = r.id
                      WHERE r.user_id = 100",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .await
            .expect("response");
        assert_eq!(status, "responded");
        assert_eq!(message_id, 700);
        assert_eq!(content, "War gut, aber bitte mehr Moderation.");
        let forwards = port.forwards.lock().expect("lock");
        assert_eq!(forwards.len(), 1);
        assert!(forwards[0].contains("📩 Neues Voice-Feedback (Req #"));
        assert!(forwards[0].contains("Mit im Call: Alice, Bob"));

        drop(forwards);
        let ack = feedback
            .handle_dm_message(dm_event(100, 701, "Nachtrag"))
            .await;
        assert_eq!(ack, None);
    }
}
