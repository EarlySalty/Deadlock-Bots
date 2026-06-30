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

use sqlx::PgPool;

use crate::db::{
    discord_id_to_i64, i64_to_i32, next_text_conversation_id, utc_from_unix_seconds,
    ActivityDbResult,
};

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
    pool: PgPool,
    open: Mutex<HashMap<(u64, u64), Session>>,
}

impl TextSessions {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Prüft, dass die zentrale Migration die Tabellen bereitgestellt hat.
    pub async fn ensure_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            SELECT 1 AS "ok!"
            FROM activity.text_stats
            LIMIT 0
            "#
        )
        .fetch_optional(&self.pool)
        .await?;

        sqlx::query!(
            r#"
            SELECT 1 AS "ok!"
            FROM activity.text_conversation_log
            LIMIT 0
            "#
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(())
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
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return false;
        };
        sqlx::query!(
            r#"
            SELECT opted_out AS "opted_out!"
            FROM core.user_privacy
            WHERE user_id = $1
              AND opted_out = TRUE
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map(|row| row.is_some())
        .unwrap_or(false)
    }

    async fn write_session(&self, user_id: u64, channel_id: u64, session: &Session) {
        if session.message_count <= 0 {
            return;
        }
        let result = self.write_session_inner(user_id, channel_id, session).await;
        if let Err(err) = result {
            tracing::warn!(%err, user_id, channel_id, "Text-Session konnte nicht persistiert werden");
        }
    }

    async fn write_session_inner(
        &self,
        user_id: u64,
        channel_id: u64,
        session: &Session,
    ) -> ActivityDbResult<()> {
        let final_points = session.final_points();
        let mut co: Vec<u64> = session.co_participants.iter().copied().collect();
        co.sort_unstable();
        let co_ids_json = if co.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&co).unwrap_or_else(|_| "[]".to_string()))
        };
        let user_id = discord_id_to_i64(user_id, "user_id")?;
        let guild_id = discord_id_to_i64(session.guild_id, "guild_id")?;
        let channel_id = discord_id_to_i64(channel_id, "channel_id")?;
        let started = utc_from_unix_seconds(session.started_at)?;
        let ended = utc_from_unix_seconds(session.last_message_at)?;
        let message_count_i32 = i64_to_i32(session.message_count, "message_count")?;
        let final_points_i32 = i64_to_i32(final_points, "points")?;
        let message_count = session.message_count;
        let had_interaction = session.had_interaction;

        let mut tx = self.pool.begin().await?;
        let id = next_text_conversation_id(&mut tx).await?;
        sqlx::query!(
            r#"
            INSERT INTO activity.text_conversation_log(
                id, user_id, guild_id, channel_id, started_at, ended_at,
                message_count, points, co_participant_ids, had_interaction
            )
            VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9::text::jsonb, $10)
            "#,
            id,
            user_id,
            guild_id,
            channel_id,
            started,
            ended,
            message_count_i32,
            final_points_i32,
            co_ids_json,
            had_interaction,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO activity.text_stats(user_id, total_messages, total_points, last_update)
            VALUES($1, $2, $3, now())
            ON CONFLICT(user_id) DO UPDATE SET
                total_messages = activity.text_stats.total_messages + EXCLUDED.total_messages,
                total_points = activity.text_stats.total_points + EXCLUDED.total_points,
                last_update = now()
            "#,
            user_id,
            message_count,
            final_points,
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }
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

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;

    async fn mk() -> Result<(dl_central_db::TestDb, TextSessions), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let store = TextSessions::new(db.pool().clone());
        store.ensure_schema().await.expect("schema");
        Ok((db, store))
    }

    async fn stats(
        store: &TextSessions,
        user_id: u64,
    ) -> Result<(i64, i64), Box<dyn std::error::Error>> {
        let user_id = discord_id_to_i64(user_id, "user_id")?;
        let row = sqlx::query!(
            r#"
            SELECT total_messages AS "total_messages!",
                   total_points AS "total_points!"
            FROM activity.text_stats
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&store.pool)
        .await?;
        Ok(row
            .map(|row| (row.total_messages, row.total_points))
            .unwrap_or((0, 0)))
    }

    async fn log_count(store: &TextSessions) -> Result<i64, Box<dyn std::error::Error>> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM activity.text_conversation_log
            "#
        )
        .fetch_one(&store.pool)
        .await?;
        Ok(row.count)
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn einzelne_nachricht_zwei_punkte() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        store.on_message(1, 10, 100, false, 1000).await;
        store.flush_all().await;
        // round(2*sqrt(1)) = 2, keine Interaktion.
        assert_eq!(stats(&store, 1).await?, (1, 2));
        assert_eq!(log_count(&store).await?, 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn reply_gibt_bonuspunkt() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        store.on_message(1, 10, 100, true, 1000).await;
        store.flush_all().await;
        // 2*sqrt(1) + 1 = 3, einmaliger Reply-Bonus.
        assert_eq!(stats(&store, 1).await?, (1, 3));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn interaktion_gibt_50_prozent_bonus() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        // Zwei User im selben Channel → beide had_interaction.
        store.on_message(1, 10, 100, false, 1000).await;
        store.on_message(2, 10, 100, false, 1001).await;
        store.flush_all().await;
        // beide: round(2 * 1.5) = 3
        assert_eq!(stats(&store, 1).await?, (1, 3));
        assert_eq!(stats(&store, 2).await?, (1, 3));
        let row = sqlx::query!(
            r#"
            SELECT co_participant_ids::text AS "co_participant_ids?"
            FROM activity.text_conversation_log
            WHERE user_id = $1
            "#,
            1_i64,
        )
        .fetch_one(&store.pool)
        .await?;
        assert_eq!(row.co_participant_ids.as_deref(), Some("[2]"));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn co_participant_ids_snowflakes_bleiben_json_integer(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        let user_id = 9_223_372_036_854_775_000_u64;
        let expected_co_participants = vec![
            9_223_372_036_854_775_001_u64,
            9_223_372_036_854_775_002_u64,
            9_223_372_036_854_775_003_u64,
        ];

        store.on_message(user_id, 10, 100, false, 1000).await;
        for (offset, co_participant) in expected_co_participants.iter().copied().enumerate() {
            store
                .on_message(co_participant, 10, 100, false, 1001 + offset as i64)
                .await;
        }
        store.flush_all().await;

        let user_id = discord_id_to_i64(user_id, "user_id")?;
        let row = sqlx::query!(
            r#"
            SELECT co_participant_ids::text AS "co_participant_ids!"
            FROM activity.text_conversation_log
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_one(&store.pool)
        .await?;

        let values: Vec<serde_json::Value> = serde_json::from_str(&row.co_participant_ids)?;
        let actual_co_participants = values
            .iter()
            .map(|value| match value {
                serde_json::Value::Number(number) => number
                    .as_u64()
                    .ok_or_else(|| format!("co_participant_id is not a u64 integer: {number}")),
                other => Err(format!("co_participant_id is not a JSON number: {other}")),
            })
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(actual_co_participants, expected_co_participants);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn getrennte_channel_keine_interaktion() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        store.on_message(1, 10, 100, false, 1000).await;
        store.on_message(2, 20, 100, false, 1001).await;
        store.flush_all().await;
        assert_eq!(stats(&store, 1).await?, (1, 2));
        assert_eq!(stats(&store, 2).await?, (1, 2));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn idle_flusht_und_startet_neu() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        store.on_message(1, 10, 100, false, 1000).await;
        // 1 s nach Fensterablauf → erste Session wird geflusht, neue beginnt.
        store
            .on_message(1, 10, 100, false, 1000 + SESSION_WINDOW_SECS + 1)
            .await;
        assert_eq!(
            stats(&store, 1).await?,
            (1, 2),
            "nur die erste Session ist geflusht"
        );
        store.flush_all().await;
        assert_eq!(stats(&store, 1).await?, (2, 4), "beide Sessions summiert");
        assert_eq!(log_count(&store).await?, 2);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn leere_session_wird_nicht_geschrieben() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        // Ohne on_message gibt es keine Session; flush_all schreibt nichts.
        store.flush_all().await;
        assert_eq!(log_count(&store).await?, 0);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn zu_grosse_discord_id_schreibt_nichts() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        store
            .on_message(i64::MAX as u64 + 1, 10, 100, false, 1000)
            .await;
        store.flush_all().await;
        assert_eq!(log_count(&store).await?, 0);
        Ok(())
    }
}
