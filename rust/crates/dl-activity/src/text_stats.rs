//! Text-Gamification — Port der `text_stats`/`text_conversation_log`-Logik aus
//! `user_activity_analyzer.on_message`.
//!
//! Pro `(user_id, channel_id)` läuft eine Konversations-Session, die
//! Nachrichten und Punkte sammelt. Nach `SESSION_WINDOW_SECS` ohne neue
//! Nachricht (oder beim periodischen Flush) wird sie nach `text_conversation_log`
//! geschrieben und in `text_stats` aufaddiert. `text_stats` speist das
//! öffentliche Text-Leaderboard (`!tleaderboard` / `!messagestats`) — ohne
//! diesen Writer veraltet die Quelle nach dem Cutover.
//!
//! Diese Scheibe enthält Schema + Zustandslogik + Flush; die Anbindung an den
//! `MessageEvent`-Strom (inkl. Reply-Erkennung) + der 60-s-Flush-Loop folgen.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dl_db::{Db, DbError};
use rusqlite::params;

/// Idle-Fenster: eine Session wird nach so vielen Sekunden ohne neue Nachricht
/// abgeschlossen (Python `TEXT_SESSION_WINDOW_SECONDS`).
pub const SESSION_WINDOW_SECS: i64 = 600;

#[derive(Debug, Clone)]
struct Session {
    started_at: i64,
    last_message_at: i64,
    guild_id: u64,
    message_count: i64,
    points_accum: f64,
    co_participants: HashSet<u64>,
    had_interaction: bool,
    last_reply_credited: bool,
}

impl Session {
    fn new(now: i64, guild_id: u64) -> Self {
        Self {
            started_at: now,
            last_message_at: now,
            guild_id,
            message_count: 0,
            points_accum: 0.0,
            co_participants: HashSet::new(),
            had_interaction: false,
            last_reply_credited: false,
        }
    }

    /// Endpunkte wie Python `_flush_text_session`: Interaktion gibt 50 % Bonus.
    fn final_points(&self) -> i64 {
        let raw = if self.had_interaction {
            self.points_accum * 1.5
        } else {
            self.points_accum
        };
        raw.round() as i64
    }
}

pub struct TextSessions {
    db: Db,
    open: Mutex<HashMap<(u64, u64), Session>>,
}

impl TextSessions {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Legt die Tabellen an (idempotent), Schema wie `service/db.py:1000/1007`.
    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS text_stats(
                       user_id        INTEGER PRIMARY KEY,
                       total_messages INTEGER NOT NULL DEFAULT 0,
                       total_points   INTEGER NOT NULL DEFAULT 0,
                       last_update    DATETIME DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE TABLE IF NOT EXISTS text_conversation_log(
                       id                 INTEGER PRIMARY KEY AUTOINCREMENT,
                       user_id            INTEGER NOT NULL,
                       guild_id           INTEGER,
                       channel_id         INTEGER,
                       started_at         DATETIME NOT NULL,
                       ended_at           DATETIME NOT NULL,
                       message_count      INTEGER NOT NULL DEFAULT 0,
                       points             INTEGER NOT NULL DEFAULT 0,
                       co_participant_ids TEXT,
                       had_interaction    INTEGER NOT NULL DEFAULT 0
                     );",
                )?;
                Ok(())
            })
            .await
    }

    /// Verarbeitet eine Nachricht: flusht zuerst abgelaufene Sessions, dann
    /// aktualisiert/öffnet die Session des Autors (Punkte, Co-Teilnehmer).
    pub async fn on_message(
        &self,
        user_id: u64,
        channel_id: u64,
        guild_id: u64,
        is_reply: bool,
        now: i64,
    ) {
        self.flush_expired(now).await;

        let mut open = self.open.lock().expect("text-session lock");
        // Andere offene Sessions im selben Channel → gegenseitige Interaktion.
        let others: Vec<u64> = open
            .iter()
            .filter(|((uid, cid), _)| *cid == channel_id && *uid != user_id)
            .map(|((uid, _), _)| *uid)
            .collect();
        let interaction = !others.is_empty();
        for other_uid in &others {
            if let Some(other) = open.get_mut(&(*other_uid, channel_id)) {
                other.had_interaction = true;
                other.co_participants.insert(user_id);
            }
        }

        let session = open
            .entry((user_id, channel_id))
            .or_insert_with(|| Session::new(now, guild_id));
        if interaction {
            session.had_interaction = true;
            session.co_participants.extend(others);
        }
        let msg_index = session.message_count + 1;
        let mut raw_points = 2.0 * (msg_index as f64).sqrt();
        if is_reply && !session.last_reply_credited {
            raw_points += 1.0;
            session.last_reply_credited = true;
        }
        session.message_count = msg_index;
        session.points_accum += raw_points;
        session.last_message_at = now;
        session.guild_id = guild_id;
    }

    /// Schließt alle Sessions, die seit `SESSION_WINDOW_SECS` keine Nachricht
    /// mehr hatten.
    pub async fn flush_expired(&self, now: i64) {
        let expired = {
            let mut open = self.open.lock().expect("text-session lock");
            let keys: Vec<(u64, u64)> = open
                .iter()
                .filter(|(_, s)| now - s.last_message_at >= SESSION_WINDOW_SECS)
                .map(|(k, _)| *k)
                .collect();
            keys.into_iter()
                .filter_map(|k| open.remove(&k).map(|s| (k, s)))
                .collect::<Vec<_>>()
        };
        for ((user_id, channel_id), session) in expired {
            self.write_session(user_id, channel_id, &session).await;
        }
    }

    /// Schreibt alle offenen Sessions weg (Shutdown / periodischer Voll-Flush).
    pub async fn flush_all(&self) {
        let all = {
            let mut open = self.open.lock().expect("text-session lock");
            open.drain().collect::<Vec<_>>()
        };
        for ((user_id, channel_id), session) in all {
            self.write_session(user_id, channel_id, &session).await;
        }
    }

    /// Privacy-Opt-out wie der message_activity-Writer (fail-open bei Fehler).
    async fn is_opted_out(&self, user_id: u64) -> bool {
        self.db
            .read(move |conn| {
                use rusqlite::OptionalExtension;
                Ok(conn
                    .query_row(
                        "SELECT opted_out FROM user_privacy WHERE user_id = ?1",
                        [user_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                    .filter(|v| *v != 0)
                    .is_some())
            })
            .await
            .unwrap_or(false)
    }

    async fn write_session(&self, user_id: u64, channel_id: u64, session: &Session) {
        if session.message_count <= 0 {
            return;
        }
        let final_points = session.final_points();
        let mut co: Vec<u64> = session.co_participants.iter().copied().collect();
        co.sort_unstable();
        let co_ids: Option<String> =
            (!co.is_empty()).then(|| co.iter().map(u64::to_string).collect::<Vec<_>>().join(","));
        let started = fmt_ts(session.started_at);
        let ended = fmt_ts(session.last_message_at);
        let guild_id = session.guild_id;
        let message_count = session.message_count;
        let had_interaction = i64::from(session.had_interaction);

        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO text_conversation_log(
                       user_id, guild_id, channel_id, started_at, ended_at,
                       message_count, points, co_participant_ids, had_interaction)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        user_id,
                        guild_id,
                        channel_id,
                        started,
                        ended,
                        message_count,
                        final_points,
                        co_ids,
                        had_interaction
                    ],
                )?;
                conn.execute(
                    "INSERT INTO text_stats(user_id, total_messages, total_points, last_update)
                     VALUES(?1, ?2, ?3, CURRENT_TIMESTAMP)
                     ON CONFLICT(user_id) DO UPDATE SET
                       total_messages = text_stats.total_messages + excluded.total_messages,
                       total_points   = text_stats.total_points + excluded.total_points,
                       last_update    = CURRENT_TIMESTAMP",
                    params![user_id, message_count, final_points],
                )?;
                Ok(())
            })
            .await;
    }
}

/// Unix-Sekunden → `YYYY-MM-DD HH:MM:SS` (UTC), wie Python `_text_session_ts`.
fn fmt_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Subscriber auf den Nachrichten-Strom + 60-s-Flush-Loop (Python:
/// `on_message` + `@tasks.loop(seconds=60) flush_text_sessions`).
/// Opt-out-User werden übersprungen; der Flush-Loop schließt verwaiste Sessions.
pub fn spawn_text_stats(
    sessions: Arc<TextSessions>,
    dispatcher: &dl_discord::Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut messages = dispatcher.subscribe_messages();
    let on_msg = {
        let sessions = sessions.clone();
        tokio::spawn(async move {
            loop {
                match messages.recv().await {
                    Ok(event) => {
                        let Some(guild_id) = event.guild_id else {
                            continue;
                        };
                        if sessions.is_opted_out(event.author_id).await {
                            continue;
                        }
                        let now = chrono::Utc::now().timestamp();
                        sessions
                            .on_message(
                                event.author_id,
                                event.channel_id,
                                guild_id,
                                event.is_reply,
                                now,
                            )
                            .await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };
    let flush = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let now = chrono::Utc::now().timestamp();
            sessions.flush_expired(now).await;
        }
    });
    vec![on_msg, flush]
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk() -> (tempfile::TempDir, TextSessions) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let store = TextSessions::new(db);
        store.ensure_schema().await.expect("schema");
        (dir, store)
    }

    async fn stats(store: &TextSessions, user_id: u64) -> (i64, i64) {
        store
            .db
            .read(move |conn| {
                conn.query_row(
                    "SELECT total_messages, total_points FROM text_stats WHERE user_id=?1",
                    params![user_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .or(Ok((0, 0)))
            })
            .await
            .unwrap()
    }

    async fn log_count(store: &TextSessions) -> i64 {
        store
            .db
            .read(|conn| {
                conn.query_row("SELECT COUNT(*) FROM text_conversation_log", [], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn einzelne_nachricht_zwei_punkte() {
        let (_d, store) = mk().await;
        store.on_message(1, 10, 100, false, 1000).await;
        store.flush_all().await;
        // round(2*sqrt(1)) = 2, keine Interaktion.
        assert_eq!(stats(&store, 1).await, (1, 2));
        assert_eq!(log_count(&store).await, 1);
    }

    #[tokio::test]
    async fn reply_gibt_bonuspunkt() {
        let (_d, store) = mk().await;
        store.on_message(1, 10, 100, true, 1000).await;
        store.flush_all().await;
        // 2*sqrt(1) + 1 = 3, einmaliger Reply-Bonus.
        assert_eq!(stats(&store, 1).await, (1, 3));
    }

    #[tokio::test]
    async fn interaktion_gibt_50_prozent_bonus() {
        let (_d, store) = mk().await;
        // Zwei User im selben Channel → beide had_interaction.
        store.on_message(1, 10, 100, false, 1000).await;
        store.on_message(2, 10, 100, false, 1001).await;
        store.flush_all().await;
        // beide: round(2 * 1.5) = 3
        assert_eq!(stats(&store, 1).await, (1, 3));
        assert_eq!(stats(&store, 2).await, (1, 3));
    }

    #[tokio::test]
    async fn getrennte_channel_keine_interaktion() {
        let (_d, store) = mk().await;
        store.on_message(1, 10, 100, false, 1000).await;
        store.on_message(2, 20, 100, false, 1001).await;
        store.flush_all().await;
        assert_eq!(stats(&store, 1).await, (1, 2));
        assert_eq!(stats(&store, 2).await, (1, 2));
    }

    #[tokio::test]
    async fn idle_flusht_und_startet_neu() {
        let (_d, store) = mk().await;
        store.on_message(1, 10, 100, false, 1000).await;
        // 1 s nach Fensterablauf → erste Session wird geflusht, neue beginnt.
        store
            .on_message(1, 10, 100, false, 1000 + SESSION_WINDOW_SECS + 1)
            .await;
        assert_eq!(
            stats(&store, 1).await,
            (1, 2),
            "nur die erste Session ist geflusht"
        );
        store.flush_all().await;
        assert_eq!(stats(&store, 1).await, (2, 4), "beide Sessions summiert");
        assert_eq!(log_count(&store).await, 2);
    }

    #[tokio::test]
    async fn leere_session_wird_nicht_geschrieben() {
        let (_d, store) = mk().await;
        // Ohne on_message gibt es keine Session; flush_all schreibt nichts.
        store.flush_all().await;
        assert_eq!(log_count(&store).await, 0);
    }
}
