//! Aktivitäts-Analyzer — Port des Kerns von `cogs/user_activity_analyzer.py`.
//!
//! Zwei Loops:
//! - alle 6 h: Aktivitätsmuster der letzten 14 Tage aus `voice_session_log`
//!   → `user_activity_patterns` (typische Stunden/Wochentage, Score) —
//!   idempotent überschreibend.
//! - alle 10 min: wer sitzt gerade mit wem im Voice → `user_co_players`
//!   (+1 Session, +10 Minuten, bidirektional, Namen werden mitgepflegt).
//!
//! Bewusst NICHT portiert: der 6-h-Co-Spieler-Backfill des Originals — er
//! addierte alle 6 Stunden die kompletten 2-Wochen-Aggregate ERNEUT auf
//! `user_co_players` (Doppelzählung zusätzlich zum 10-min-Tracker, stiller
//! Akkumulations-Bug). Der 10-min-Pfad ist der korrekte Schreiber.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, NaiveDateTime, Timelike, Utc};
use dl_db::Db;

pub const ANALYZE_INTERVAL: Duration = Duration::from_secs(6 * 3600);
pub const CO_PLAYER_INTERVAL: Duration = Duration::from_secs(600);
pub const CO_PLAYER_MINUTES_PER_TICK: i64 = 10;
pub const WINDOW_DAYS: i64 = 14;

// ── Pure Muster-Berechnung (Referenzwerte aus CPython im Test) ─────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityPattern {
    /// Top-3-Stunden (0–23), nach Häufigkeit, Insertion-Order als Tie-Break.
    pub typical_hours: Vec<u32>,
    /// Top-3-Wochentage (0=Mo … 6=So).
    pub typical_days: Vec<u32>,
    pub sessions_count: i64,
    pub total_minutes: i64,
    pub last_active: Option<NaiveDateTime>,
}

/// Wie `_analyze_single_user`: Python-dict-Insertion-Order + stabile
/// Sortierung nach Häufigkeit bestimmen die Tie-Breaks.
pub fn analyze_sessions(sessions: &[(Option<NaiveDateTime>, i64)]) -> ActivityPattern {
    let mut hour_order: Vec<(u32, i64)> = Vec::new();
    let mut day_order: Vec<(u32, i64)> = Vec::new();
    let mut total_minutes = 0i64;
    let mut last_active: Option<NaiveDateTime> = None;

    for (started_at, duration_seconds) in sessions {
        total_minutes += duration_seconds / 60;
        let Some(started_at) = started_at else {
            continue;
        };
        let hour = started_at.hour();
        match hour_order.iter_mut().find(|(h, _)| *h == hour) {
            Some((_, count)) => *count += 1,
            None => hour_order.push((hour, 1)),
        }
        let day = started_at.weekday().num_days_from_monday();
        match day_order.iter_mut().find(|(d, _)| *d == day) {
            Some((_, count)) => *count += 1,
            None => day_order.push((day, 1)),
        }
        if last_active.map(|last| *started_at > last).unwrap_or(true) {
            last_active = Some(*started_at);
        }
    }
    // Stabile Sortierung absteigend nach Häufigkeit (wie Pythons sorted)
    hour_order.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    day_order.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

    ActivityPattern {
        typical_hours: hour_order.iter().take(3).map(|(h, _)| *h).collect(),
        typical_days: day_order.iter().take(3).map(|(d, _)| *d).collect(),
        sessions_count: sessions.len() as i64,
        total_minutes,
        last_active,
    }
}

// ── Analyzer ───────────────────────────────────────────────────────────────

/// Voice-Sicht für den Co-Spieler-Tracker: Gruppen von (user_id, display_name)
/// pro Kanal mit ≥ 2 Nicht-Bots.
#[async_trait::async_trait]
pub trait VoiceGroups: Send + Sync {
    async fn channel_groups(&self) -> Vec<Vec<(u64, String)>>;
}

pub struct ActivityAnalyzer {
    pub db: Db,
    pub voice: Arc<dyn VoiceGroups>,
}

impl ActivityAnalyzer {
    pub fn new(db: Db, voice: Arc<dyn VoiceGroups>) -> Arc<Self> {
        Arc::new(Self { db, voice })
    }

    /// 6-h-Lauf: Muster aller aktiven User der letzten 14 Tage.
    pub async fn analyze_all(&self) -> usize {
        let cutoff = (Utc::now() - chrono::Duration::days(WINDOW_DAYS))
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let sessions: Vec<(u64, Option<String>, i64)> = self
            .db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT user_id, started_at, duration_seconds
                       FROM voice_session_log
                      WHERE started_at >= ?1
                      ORDER BY user_id, started_at",
                )?;
                let rows = stmt.query_map([cutoff], |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    ))
                })?;
                rows.collect()
            })
            .await
            .unwrap_or_default();

        let mut grouped: HashMap<u64, Vec<(Option<NaiveDateTime>, i64)>> = HashMap::new();
        for (user_id, started_at, duration) in sessions {
            let parsed = started_at
                .as_deref()
                .and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok());
            grouped.entry(user_id).or_default().push((parsed, duration));
        }

        let analyzed = grouped.len();
        for (user_id, user_sessions) in grouped {
            let pattern = analyze_sessions(&user_sessions);
            self.persist_pattern(user_id, &pattern).await;
        }
        tracing::info!(analyzed, "Aktivitätsmuster aktualisiert");
        analyzed
    }

    async fn persist_pattern(&self, user_id: u64, pattern: &ActivityPattern) {
        let typical_hours = serde_json::to_string(&pattern.typical_hours).unwrap_or_default();
        let typical_days = serde_json::to_string(&pattern.typical_days).unwrap_or_default();
        let last_active = pattern
            .last_active
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let (sessions_count, total_minutes) = (pattern.sessions_count, pattern.total_minutes);
        let result = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO user_activity_patterns(
                       user_id, typical_hours, typical_days, activity_score_2w,
                       sessions_count_2w, total_minutes_2w, last_active_at, last_analyzed_at
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,CURRENT_TIMESTAMP)
                     ON CONFLICT(user_id) DO UPDATE SET
                       typical_hours = excluded.typical_hours,
                       typical_days = excluded.typical_days,
                       activity_score_2w = excluded.activity_score_2w,
                       sessions_count_2w = excluded.sessions_count_2w,
                       total_minutes_2w = excluded.total_minutes_2w,
                       last_active_at = excluded.last_active_at,
                       last_analyzed_at = CURRENT_TIMESTAMP",
                    rusqlite::params![
                        user_id,
                        typical_hours,
                        typical_days,
                        sessions_count,
                        sessions_count,
                        total_minutes,
                        last_active,
                    ],
                )
                .map(|_| ())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, user_id, "Pattern-Persist fehlgeschlagen");
        }
    }

    /// 10-min-Lauf: aktuelle Voice-Paarungen → user_co_players (+1/+10 min).
    pub async fn track_co_players(&self) {
        let groups = self.voice.channel_groups().await;
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let (a_id, a_name) = group[i].clone();
                    let (b_id, b_name) = group[j].clone();
                    self.record_pair(a_id, &a_name, b_id, &b_name).await;
                }
            }
        }
    }

    async fn record_pair(&self, user_id: u64, user_name: &str, co_id: u64, co_name: &str) {
        if user_id == co_id {
            return;
        }
        let pairs = [
            (user_id, co_id, user_name.to_string(), co_name.to_string()),
            (co_id, user_id, co_name.to_string(), user_name.to_string()),
        ];
        let result = self
            .db
            .write(move |conn| {
                for (uid, co_uid, uid_name, co_name) in &pairs {
                    conn.execute(
                        "INSERT INTO user_co_players(
                           user_id, co_player_id, sessions_together, total_minutes_together,
                           last_played_together, user_display_name, co_player_display_name
                         ) VALUES(?1, ?2, 1, ?3, CURRENT_TIMESTAMP, ?4, ?5)
                         ON CONFLICT(user_id, co_player_id) DO UPDATE SET
                           sessions_together = sessions_together + 1,
                           total_minutes_together = total_minutes_together + excluded.total_minutes_together,
                           last_played_together = CURRENT_TIMESTAMP,
                           user_display_name = COALESCE(excluded.user_display_name, user_display_name),
                           co_player_display_name = COALESCE(excluded.co_player_display_name, co_player_display_name)",
                        rusqlite::params![uid, co_uid, CO_PLAYER_MINUTES_PER_TICK, uid_name, co_name],
                    )?;
                }
                Ok(())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, user_id, co_id, "Co-Player-Persist fehlgeschlagen");
        }
    }

    /// Top-Mitspieler (für LFG/Empfehlungen).
    pub async fn top_co_players(&self, user_id: u64, limit: i64) -> Vec<(u64, i64)> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT co_player_id, sessions_together FROM user_co_players
                      WHERE user_id = ?1 ORDER BY sessions_together DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![user_id, limit], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }
}

/// member_events-Basis-Writer: join/remove aus dem Dispatcher persistieren.
/// Bewusste Interim-Lücke (dokumentiert in docs/06): keine Invite-
/// Attribution — `metadata` bleibt leer, die Join-Quellen-Auswertung zählt
/// diese Joins als „Unbekannt", bis das Invite-Snapshot-Diffing portiert ist.
pub fn spawn_member_events(
    db: dl_db::Db,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_members();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let (guild_id, user_id, event_type) = match event {
                        dl_discord::MemberEvent::Join { guild_id, user_id } => {
                            (guild_id, user_id, "join")
                        }
                        dl_discord::MemberEvent::Remove { guild_id, user_id } => {
                            (guild_id, user_id, "leave")
                        }
                    };
                    let result = db
                        .write(move |conn| {
                            conn.execute(
                                "INSERT INTO member_events(user_id, guild_id, event_type, metadata)
                                 VALUES(?1, ?2, ?3, NULL)",
                                rusqlite::params![user_id, guild_id, event_type],
                            )
                            .map(|_| ())
                        })
                        .await;
                    if let Err(err) = result {
                        tracing::warn!(%err, "member_events-Insert fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// message_activity-Writer: Nachrichten-Zähler je User×Guild (wie der
/// on_message-Tracker des Originals; Privacy-Opt-out wird respektiert).
/// Grundlage u. a. für die Leave-Survey-Einstufung (Bucket A/B/C).
pub fn spawn_message_activity(
    db: dl_db::Db,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let Some(guild_id) = event.guild_id else {
                        continue;
                    };
                    let (user_id, channel_id) = (event.author_id, event.channel_id);
                    let result = db
                        .write(move |conn| {
                            use rusqlite::OptionalExtension;
                            let opted_out: Option<i64> = conn
                                .query_row(
                                    "SELECT opted_out FROM user_privacy WHERE user_id = ?1",
                                    [user_id],
                                    |row| row.get(0),
                                )
                                .optional()?
                                .filter(|v| *v != 0);
                            if opted_out.is_some() {
                                return Ok(());
                            }
                            conn.execute(
                                "INSERT INTO message_activity(
                                   user_id, guild_id, channel_id, message_count,
                                   last_message_at, first_message_at
                                 ) VALUES (?1, ?2, ?3, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
                                 ON CONFLICT(user_id, guild_id) DO UPDATE SET
                                   message_count = message_count + 1,
                                   last_message_at = CURRENT_TIMESTAMP,
                                   channel_id = excluded.channel_id",
                                rusqlite::params![user_id, guild_id, channel_id],
                            )
                            .map(|_| ())
                        })
                        .await;
                    if let Err(err) = result {
                        tracing::warn!(%err, "message_activity-Upsert fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub fn spawn(analyzer: Arc<ActivityAnalyzer>) -> Vec<tokio::task::JoinHandle<()>> {
    let pattern = analyzer.clone();
    let pattern_task = tokio::spawn(async move {
        loop {
            pattern.analyze_all().await;
            tokio::time::sleep(ANALYZE_INTERVAL).await;
        }
    });
    let co_players = analyzer;
    let co_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(CO_PLAYER_INTERVAL).await;
            co_players.track_co_players().await;
        }
    });
    vec![pattern_task, co_task]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn dt(s: &str) -> Option<NaiveDateTime> {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
    }

    #[test]
    fn muster_wie_python() {
        // Referenzwerte aus CPython: [18, 21, 20] / [0, 1, 2] / 6 / 240
        let sessions = vec![
            (dt("2026-06-01 18:30:00"), 3600),
            (dt("2026-06-01 20:10:00"), 1800),
            (dt("2026-06-02 18:05:00"), 900),
            (dt("2026-06-03 21:00:00"), 7200),
            (dt("2026-06-08 18:00:00"), 600),
            (dt("2026-06-08 21:30:00"), 300),
        ];
        let pattern = analyze_sessions(&sessions);
        assert_eq!(pattern.typical_hours, vec![18, 21, 20]);
        assert_eq!(pattern.typical_days, vec![0, 1, 2]);
        assert_eq!(pattern.sessions_count, 6);
        assert_eq!(pattern.total_minutes, 240);
        assert_eq!(pattern.last_active, dt("2026-06-08 21:30:00"));
    }

    struct MockVoice {
        groups: StdMutex<Vec<Vec<(u64, String)>>>,
    }

    #[async_trait::async_trait]
    impl VoiceGroups for MockVoice {
        async fn channel_groups(&self) -> Vec<Vec<(u64, String)>> {
            self.groups.lock().expect("lock").clone()
        }
    }

    const DDLS: [&str; 3] = [
        "CREATE TABLE voice_session_log(id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, guild_id INTEGER, channel_id INTEGER, channel_name TEXT, started_at DATETIME NOT NULL, ended_at DATETIME NOT NULL, duration_seconds INTEGER NOT NULL DEFAULT 0, points INTEGER NOT NULL DEFAULT 0, peak_users INTEGER, user_counts_json TEXT, display_name TEXT, co_player_ids TEXT)",
        "CREATE TABLE user_activity_patterns(user_id INTEGER PRIMARY KEY, typical_hours TEXT, typical_days TEXT, activity_score_2w INTEGER DEFAULT 0, sessions_count_2w INTEGER DEFAULT 0, total_minutes_2w INTEGER DEFAULT 0, last_active_at DATETIME, last_analyzed_at DATETIME DEFAULT CURRENT_TIMESTAMP, last_pinged_at DATETIME, ping_count_30d INTEGER DEFAULT 0)",
        "CREATE TABLE user_co_players(user_id INTEGER NOT NULL, co_player_id INTEGER NOT NULL, sessions_together INTEGER DEFAULT 1, total_minutes_together INTEGER DEFAULT 0, last_played_together DATETIME DEFAULT CURRENT_TIMESTAMP, user_display_name TEXT, co_player_display_name TEXT, PRIMARY KEY(user_id, co_player_id))",
    ];

    async fn setup() -> (tempfile::TempDir, Arc<ActivityAnalyzer>, Arc<MockVoice>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        let voice = Arc::new(MockVoice {
            groups: StdMutex::new(Vec::new()),
        });
        (dir, ActivityAnalyzer::new(db, voice.clone()), voice)
    }

    #[tokio::test]
    async fn analyze_schreibt_patterns() {
        let (_dir, analyzer, _voice) = setup().await;
        let recent = (Utc::now() - chrono::Duration::days(2))
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        analyzer
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO voice_session_log(user_id, started_at, ended_at, duration_seconds)
                     VALUES(100, ?1, ?1, 1800)",
                    [recent],
                )
                .map(|_| ())
            })
            .await
            .expect("seed");
        assert_eq!(analyzer.analyze_all().await, 1);
        let (count, minutes): (i64, i64) = analyzer
            .db
            .read(|conn| {
                conn.query_row(
                    "SELECT sessions_count_2w, total_minutes_2w FROM user_activity_patterns WHERE user_id = 100",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .await
            .expect("pattern");
        assert_eq!((count, minutes), (1, 30));
    }

    #[tokio::test]
    async fn co_player_tracking_bidirektional() {
        let (_dir, analyzer, voice) = setup().await;
        *voice.groups.lock().expect("lock") =
            vec![vec![(100, "Anna".to_string()), (200, "Ben".to_string())]];
        analyzer.track_co_players().await;
        analyzer.track_co_players().await; // zweiter Tick akkumuliert

        let (sessions, minutes, name): (i64, i64, String) = analyzer
            .db
            .read(|conn| {
                conn.query_row(
                    "SELECT sessions_together, total_minutes_together, co_player_display_name
                       FROM user_co_players WHERE user_id = 100 AND co_player_id = 200",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .await
            .expect("row");
        assert_eq!((sessions, minutes), (2, 20));
        assert_eq!(name, "Ben");
        // Gegenrichtung existiert ebenfalls
        let top = analyzer.top_co_players(200, 5).await;
        assert_eq!(top, vec![(100, 2)]);
    }
}
