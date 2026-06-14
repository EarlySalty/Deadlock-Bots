//! Retention-Tracking — Daten-Layer (Port von `cogs/user_retention.py`).
//!
//! Schreibt `user_retention_tracking`: bei Voice-Join `last_active_at` +
//! `total_active_days` (nur ein neuer Tag zählt), und alle 30 min
//! `avg_weekly_sessions` aus `voice_session_log`. **Cutover-kritisch:** die
//! Leave-Survey-Einstufung (Bucket A/B/C, [`crate::leave_survey`]) LIEST
//! `avg_weekly_sessions` hier — ohne diesen Schreiber veraltet die Quelle.
//!
//! Die „Wir-vermissen-dich"-DM (`daily_retention_check`) ist user-facing
//! (Embed + Feedback-Button) und folgt im gebündelten UI-Pass.

use std::time::Duration;

use dl_db::{Db, DbError};
use rusqlite::{params, OptionalExtension};

const SYNC_INTERVAL: Duration = Duration::from_secs(30 * 60);
const LOOKBACK_DAYS: i64 = 60;

#[derive(Clone)]
pub struct RetentionTracker {
    db: Db,
}

impl RetentionTracker {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Legt die Tabelle an (idempotent), Schema wie `rust/docs/db-schema.sql`.
    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS user_retention_tracking(
                       user_id INTEGER PRIMARY KEY,
                       guild_id INTEGER NOT NULL,
                       first_seen_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       last_active_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       total_active_days INTEGER NOT NULL DEFAULT 0,
                       avg_weekly_sessions REAL DEFAULT 0,
                       last_miss_you_sent_at INTEGER,
                       miss_you_count INTEGER NOT NULL DEFAULT 0,
                       opted_out INTEGER NOT NULL DEFAULT 0,
                       updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                     );
                     CREATE TABLE IF NOT EXISTS user_retention_messages(
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       user_id INTEGER NOT NULL,
                       guild_id INTEGER NOT NULL,
                       message_type TEXT NOT NULL,
                       sent_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                       delivery_status TEXT NOT NULL DEFAULT 'sent',
                       error_message TEXT
                     );",
                )?;
                Ok(())
            })
            .await
    }

    /// Voice-Join/-Wechsel: `last_active_at` + `total_active_days` (nur ein
    /// neuer UTC-Tag erhöht den Zähler). Gated auf `user_privacy`-Opt-out.
    pub async fn update_user_activity(
        &self,
        user_id: u64,
        guild_id: u64,
        now: i64,
        today: String,
    ) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                // Opt-out-Gate (user_privacy, wertbasiert; fail-open bei Fehler).
                let opted = conn
                    .query_row(
                        "SELECT opted_out FROM user_privacy WHERE user_id=?1",
                        params![user_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
                    .filter(|v| *v != 0);
                if opted.is_some() {
                    return Ok(());
                }

                let row: Option<(i64, i64)> = conn
                    .query_row(
                        "SELECT last_active_at, total_active_days FROM user_retention_tracking WHERE user_id=?1",
                        params![user_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                match row {
                    Some((last_active_ts, total_days)) => {
                        let last_date = chrono::DateTime::from_timestamp(last_active_ts, 0)
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_default();
                        let new_total = if last_date != today {
                            total_days + 1
                        } else {
                            total_days
                        };
                        conn.execute(
                            "UPDATE user_retention_tracking
                               SET last_active_at=?1, total_active_days=?2, updated_at=?1
                             WHERE user_id=?3",
                            params![now, new_total, user_id],
                        )?;
                    }
                    None => {
                        conn.execute(
                            "INSERT INTO user_retention_tracking
                               (user_id, guild_id, first_seen_at, last_active_at, total_active_days, updated_at)
                             VALUES(?1, ?2, ?3, ?3, 1, ?3)",
                            params![user_id, guild_id, now],
                        )?;
                    }
                }
                Ok(())
            })
            .await
    }

    /// 30-min-Lauf: `avg_weekly_sessions` aus `voice_session_log` (letzte 60
    /// Tage). `weeks_active = max(1, (now-first_session)/Woche)`. Gibt die Zahl
    /// aktualisierter User zurück.
    pub async fn sync_activity_data(&self, now: i64) -> Result<usize, DbError> {
        let cutoff = now - LOOKBACK_DAYS * 86400;
        self.db
            .write(move |conn| {
                let agg: Vec<(i64, i64, i64, i64, i64, i64)> = {
                    let mut stmt = conn.prepare(
                        "SELECT user_id, guild_id,
                                COUNT(DISTINCT date(started_at)) AS active_days,
                                COUNT(*) AS total_sessions,
                                CAST(MIN(strftime('%s', started_at)) AS INTEGER) AS first_s,
                                CAST(MAX(strftime('%s', started_at)) AS INTEGER) AS last_s
                           FROM voice_session_log
                          WHERE CAST(strftime('%s', started_at) AS INTEGER) > ?1
                          GROUP BY user_id",
                    )?;
                    let rows = stmt.query_map(params![cutoff], |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get::<_, Option<i64>>(4)?.unwrap_or(now),
                            r.get::<_, Option<i64>>(5)?.unwrap_or(now),
                        ))
                    })?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                let count = agg.len();
                for (user_id, guild_id, active_days, total_sessions, first_s, last_s) in agg {
                    let weeks = ((now - first_s) as f64 / (7.0 * 86400.0)).max(1.0);
                    let avg_weekly = total_sessions as f64 / weeks;
                    conn.execute(
                        "INSERT INTO user_retention_tracking
                           (user_id, guild_id, first_seen_at, last_active_at, total_active_days, avg_weekly_sessions, updated_at)
                         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT(user_id) DO UPDATE SET
                           last_active_at=MAX(user_retention_tracking.last_active_at, excluded.last_active_at),
                           total_active_days=MAX(user_retention_tracking.total_active_days, excluded.total_active_days),
                           avg_weekly_sessions=excluded.avg_weekly_sessions,
                           updated_at=excluded.updated_at",
                        params![user_id, guild_id, first_s, last_s, active_days, avg_weekly, now],
                    )?;
                }
                Ok(count)
            })
            .await
    }
}

/// Spawnt den Voice-Join-Subscriber + den 30-min-Sync-Loop.
pub fn spawn(tracker: RetentionTracker, dispatcher: &dl_discord::Dispatcher) {
    {
        let tracker = tracker.clone();
        tokio::spawn(async move {
            let _ = tracker.ensure_schema().await;
            loop {
                let now = chrono::Utc::now().timestamp();
                if let Err(e) = tracker.sync_activity_data(now).await {
                    tracing::warn!(%e, "retention-sync fehlgeschlagen");
                }
                tokio::time::sleep(SYNC_INTERVAL).await;
            }
        });
    }

    let mut events = dispatcher.subscribe_voice();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                // Join (aus dem Nichts) ODER Kanalwechsel zählt als Aktivität.
                Ok(dl_discord::VoiceEvent::Join {
                    guild_id, user_id, ..
                })
                | Ok(dl_discord::VoiceEvent::Move {
                    guild_id, user_id, ..
                }) => {
                    let now = chrono::Utc::now();
                    let _ = tracker
                        .update_user_activity(
                            user_id,
                            guild_id,
                            now.timestamp(),
                            now.format("%Y-%m-%d").to_string(),
                        )
                        .await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk() -> (tempfile::TempDir, RetentionTracker) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("r.sqlite3")).expect("db");
        let t = RetentionTracker::new(db.clone());
        t.ensure_schema().await.expect("schema");
        db.write(|c| {
            c.execute_batch(
                "CREATE TABLE user_privacy(user_id INTEGER PRIMARY KEY, opted_out INTEGER DEFAULT 0);
                 CREATE TABLE voice_session_log(user_id INTEGER, guild_id INTEGER, started_at DATETIME, duration_seconds INTEGER);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        (dir, t)
    }

    fn day(ts: i64) -> String {
        chrono::DateTime::from_timestamp(ts, 0)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    }

    #[tokio::test]
    async fn update_zaehlt_nur_neuen_tag() {
        let (_d, t) = mk().await;
        // now+today konsistent (last_date wird aus dem gespeicherten ts abgeleitet).
        let d1a = 1_780_000_000_i64; // 20:26 UTC an Tag X
        let d1b = d1a + 3600; // selber Tag X
        let d2 = d1a + 86400; // Tag X+1
        t.update_user_activity(5, 1, d1a, day(d1a)).await.unwrap(); // → 1
        t.update_user_activity(5, 1, d1b, day(d1b)).await.unwrap(); // selber Tag → 1
        t.update_user_activity(5, 1, d2, day(d2)).await.unwrap(); // neuer Tag → 2
        let total: i64 =
            t.db.read(|c| {
                c.query_row(
                    "SELECT total_active_days FROM user_retention_tracking WHERE user_id=5",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn update_respektiert_opt_out() {
        let (_d, t) = mk().await;
        t.db.write(|c| {
            c.execute(
                "INSERT INTO user_privacy(user_id, opted_out) VALUES(9, 1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        t.update_user_activity(9, 1, 1000, "2026-06-01".into())
            .await
            .unwrap();
        let n: i64 =
            t.db.read(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM user_retention_tracking WHERE user_id=9",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(n, 0, "Opt-out-User wird nicht getrackt");
    }

    #[tokio::test]
    async fn sync_berechnet_avg_weekly() {
        let (_d, t) = mk().await;
        // 2 Sessions am selben Start; first=last → weeks_active=max(1,…)=1 → avg=2.
        t.db.write(|c| {
            c.execute_batch(
                "INSERT INTO voice_session_log(user_id, guild_id, started_at, duration_seconds)
                 VALUES (7, 1, '2026-06-01 10:00:00', 600),
                        (7, 1, '2026-06-01 12:00:00', 600);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        // now nahe an started_at → weeks ~1.
        let now =
            chrono::DateTime::parse_from_str("2026-06-01 13:00:00 +0000", "%Y-%m-%d %H:%M:%S %z")
                .unwrap()
                .timestamp();
        let updated = t.sync_activity_data(now).await.unwrap();
        assert_eq!(updated, 1);
        let avg: f64 =
            t.db.read(|c| {
                c.query_row(
                    "SELECT avg_weekly_sessions FROM user_retention_tracking WHERE user_id=7",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert!((avg - 2.0).abs() < 0.01, "avg_weekly = {avg}");
    }
}
