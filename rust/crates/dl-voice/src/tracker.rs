//! Voice-Session-Tracking — Port von `cogs/voice_activity_tracker.py` (Kern).
//!
//! Semantik wie das Original:
//! - Sessions starten, sobald ≥ `min_users_for_tracking` AKTIVE (ungemutete
//!   oder in Grace befindliche) Nicht-Bots im Kanal sind.
//! - Mute/Deaf beendet die Aktivität; Mitglieder mit der Grace-Rolle bekommen
//!   eine Schonfrist (Standard 3 min), bevor die Session endet.
//! - Beim Ende: Punkte (1/Minute + Peak-Bonus), Upsert in `voice_stats`,
//!   History-Insert in `voice_session_log` (UTC-naive Timestamps,
//!   `%Y-%m-%d %H:%M:%S` — Vertrag mit Public-Stats/LFG).
//! - Privacy: Opt-outs (`user_privacy.opted_out`) werden nie getrackt.
//!
//! Bewusste 4a-Lücke (kommt mit 4b, vor dem Voice-Cutover): das
//! Feedback-DM-System nach der ersten Session sowie die Statistik-Commands.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDateTime, Utc};
use dl_db::Db;
use dl_discord::{Dispatcher, VoiceEvent};
use serde_json::{json, Value};

pub const KV_NAMESPACE: &str = "voice_cfg";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerConfig {
    pub min_users_for_tracking: i64,
    pub grace_period_duration: i64,
    pub session_timeout: i64,
    pub afk_timeout: i64,
    pub special_role_id: u64,
    pub max_sessions_per_user: i64,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            min_users_for_tracking: 2,
            grace_period_duration: 180,
            session_timeout: 300,
            afk_timeout: 1800,
            special_role_id: 1313624729466441769,
            max_sessions_per_user: 100,
        }
    }
}

impl TrackerConfig {
    fn from_json(raw: &str, defaults: &Self) -> Self {
        let data: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
        let int = |key: &str, default: i64| {
            data.get(key)
                .and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(default)
        };
        Self {
            min_users_for_tracking: int("min_users_for_tracking", defaults.min_users_for_tracking),
            grace_period_duration: int("grace_period_duration", defaults.grace_period_duration),
            session_timeout: int("session_timeout", defaults.session_timeout),
            afk_timeout: int("afk_timeout", defaults.afk_timeout),
            special_role_id: int("special_role_id", defaults.special_role_id as i64) as u64,
            max_sessions_per_user: int("max_sessions_per_user", defaults.max_sessions_per_user),
        }
    }

    fn to_json(&self) -> String {
        json!({
            "min_users_for_tracking": self.min_users_for_tracking,
            "grace_period_duration": self.grace_period_duration,
            "session_timeout": self.session_timeout,
            "afk_timeout": self.afk_timeout,
            "special_role_id": self.special_role_id,
            "max_sessions_per_user": self.max_sessions_per_user,
        })
        .to_string()
    }
}

/// Punkte: 1 pro Minute + Peak-Bonus (Referenzwerte aus CPython im Test).
pub fn calculate_points(seconds: i64, peak_users: i64) -> i64 {
    if seconds <= 0 {
        return 0;
    }
    let mut points = seconds / 60;
    if points > 0 {
        if peak_users >= 5 {
            points += (points / 10).max(1);
        } else if peak_users >= 3 {
            points += (points / 20).max(1);
        }
    }
    points.max(0)
}

/// Zustand eines Mitglieds im Voice-Kanal (vom Gateway-Cache geliefert).
#[derive(Debug, Clone, Default)]
pub struct VoiceMemberState {
    pub user_id: u64,
    pub display_name: String,
    pub is_bot: bool,
    pub is_afk_channel: bool,
    pub muted_or_deaf: bool,
    pub has_grace_role: bool,
}

/// Lesezugriff auf den Voice-Zustand — implementiert vom Gateway-Cache,
/// in Tests gemockt.
#[async_trait::async_trait]
pub trait VoiceSnapshot: Send + Sync {
    async fn channel_states(
        &self,
        guild_id: u64,
        channel_id: u64,
        grace_role_id: u64,
    ) -> Vec<VoiceMemberState>;

    /// Kanal-Name für die Session-Historie (None → Platzhalter).
    async fn channel_name(&self, _guild_id: u64, _channel_id: u64) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone)]
struct Session {
    user_id: u64,
    display_name: String,
    guild_id: u64,
    channel_id: u64,
    channel_name: String,
    start_time: NaiveDateTime,
    last_update: NaiveDateTime,
    peak_users: i64,
    user_counts: Vec<i64>,
    co_player_ids: BTreeSet<u64>,
}

#[derive(Debug, Clone)]
struct GracePeriod {
    started: NaiveDateTime,
}

#[derive(Default)]
struct TrackerState {
    sessions: HashMap<(u64, u64), Session>,
    grace: HashMap<(u64, u64), GracePeriod>,
    config_cache: HashMap<u64, TrackerConfig>,
}

pub struct VoiceTracker {
    db: Db,
    snapshot: Arc<dyn VoiceSnapshot>,
    state: tokio::sync::Mutex<TrackerState>,
}

impl VoiceTracker {
    pub fn new(db: Db, snapshot: Arc<dyn VoiceSnapshot>) -> Arc<Self> {
        Arc::new(Self {
            db,
            snapshot,
            state: tokio::sync::Mutex::new(TrackerState::default()),
        })
    }

    /// Konfiguration pro Guild aus kv_store (ns voice_cfg) — Default wird
    /// beim ersten Zugriff zurückgeschrieben (wie ConfigManager.get).
    pub async fn config(&self, guild_id: u64) -> TrackerConfig {
        {
            let state = self.state.lock().await;
            if let Some(cfg) = state.config_cache.get(&guild_id) {
                return cfg.clone();
            }
        }
        let defaults = TrackerConfig::default();
        let stored = self
            .db
            .kv_get(KV_NAMESPACE, guild_id.to_string())
            .await
            .ok()
            .flatten();
        let cfg = match stored {
            Some(raw) => TrackerConfig::from_json(&raw, &defaults),
            None => {
                let _ = self
                    .db
                    .kv_set(KV_NAMESPACE, guild_id.to_string(), defaults.to_json())
                    .await;
                defaults
            }
        };
        self.state
            .lock()
            .await
            .config_cache
            .insert(guild_id, cfg.clone());
        cfg
    }

    async fn is_opted_out(&self, user_id: u64) -> bool {
        self.db
            .read(move |conn| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    "SELECT opted_out FROM user_privacy WHERE user_id = ?1",
                    [user_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    /// Zentraler Event-Einstieg (Subscriber des Voice-Dispatchers).
    pub async fn handle_event(self: &Arc<Self>, event: VoiceEvent) {
        match event {
            VoiceEvent::Join {
                guild_id,
                user_id,
                channel_id,
            } => {
                if self.is_opted_out(user_id).await {
                    self.drop_runtime_state(user_id).await;
                    return;
                }
                self.update_channel(guild_id, channel_id).await;
            }
            VoiceEvent::Leave {
                guild_id,
                user_id,
                channel_id,
            } => {
                if self.is_opted_out(user_id).await {
                    self.drop_runtime_state(user_id).await;
                    return;
                }
                self.end_session(user_id, guild_id, Utc::now().naive_utc())
                    .await;
                self.update_channel(guild_id, channel_id).await;
            }
            VoiceEvent::Move {
                guild_id,
                user_id,
                from_channel_id,
                to_channel_id,
            } => {
                if self.is_opted_out(user_id).await {
                    self.drop_runtime_state(user_id).await;
                    return;
                }
                self.end_session(user_id, guild_id, Utc::now().naive_utc())
                    .await;
                self.update_channel(guild_id, from_channel_id).await;
                self.update_channel(guild_id, to_channel_id).await;
            }
            VoiceEvent::Update {
                guild_id,
                user_id,
                channel_id,
                was_muted,
                is_muted,
            } => {
                if self.is_opted_out(user_id).await {
                    self.drop_runtime_state(user_id).await;
                    return;
                }
                // Grace-Logik bei Mute-Wechsel (nur mit Grace-Rolle)
                if !was_muted && is_muted {
                    let cfg = self.config(guild_id).await;
                    let states = self
                        .snapshot
                        .channel_states(guild_id, channel_id, cfg.special_role_id)
                        .await;
                    if states
                        .iter()
                        .any(|s| s.user_id == user_id && s.has_grace_role)
                    {
                        self.start_grace(user_id, guild_id).await;
                    }
                } else if was_muted && !is_muted {
                    self.end_grace(user_id, guild_id).await;
                }
                self.update_channel(guild_id, channel_id).await;
            }
        }
    }

    async fn drop_runtime_state(&self, user_id: u64) {
        let mut state = self.state.lock().await;
        state.sessions.retain(|(uid, _), _| *uid != user_id);
        state.grace.retain(|(uid, _), _| *uid != user_id);
    }

    async fn start_grace(&self, user_id: u64, guild_id: u64) {
        let mut state = self.state.lock().await;
        state
            .grace
            .entry((user_id, guild_id))
            .or_insert(GracePeriod {
                started: Utc::now().naive_utc(),
            });
    }

    async fn end_grace(&self, user_id: u64, guild_id: u64) {
        self.state.lock().await.grace.remove(&(user_id, guild_id));
    }

    /// Kern: Sessions im Kanal starten/aktualisieren/beenden
    /// (wie `update_channel_sessions`).
    pub async fn update_channel(self: &Arc<Self>, guild_id: u64, channel_id: u64) {
        let cfg = self.config(guild_id).await;
        let states = self
            .snapshot
            .channel_states(guild_id, channel_id, cfg.special_role_id)
            .await;
        if states.is_empty() {
            return;
        }
        let now = Utc::now().naive_utc();
        let channel_name = self
            .snapshot
            .channel_name(guild_id, channel_id)
            .await
            .unwrap_or_else(|| format!("Channel {channel_id}"));

        // Opt-outs vorab filtern (nie tracken)
        let mut filtered = Vec::new();
        for member in states {
            if member.is_bot || member.is_afk_channel {
                continue;
            }
            if self.is_opted_out(member.user_id).await {
                continue;
            }
            filtered.push(member);
        }

        let mut to_finalize: Vec<Session> = Vec::new();
        {
            let mut state = self.state.lock().await;

            // aktiv = ungemutet ODER (Grace-Rolle && Grace läuft noch)
            let active: Vec<&VoiceMemberState> = filtered
                .iter()
                .filter(|m| {
                    if !m.muted_or_deaf {
                        return true;
                    }
                    if !m.has_grace_role {
                        return false;
                    }
                    state
                        .grace
                        .get(&(m.user_id, guild_id))
                        .map(|g| (now - g.started).num_seconds() <= cfg.grace_period_duration)
                        .unwrap_or(false)
                })
                .collect();
            let user_count = active.len() as i64;
            let active_ids: BTreeSet<u64> = active.iter().map(|m| m.user_id).collect();

            // Peak/Co-Player-Update für laufende Sessions der Aktiven
            for member in &active {
                if let Some(session) = state.sessions.get_mut(&(member.user_id, guild_id)) {
                    session.user_counts.push(user_count);
                    session.peak_users = session.peak_users.max(user_count);
                    session
                        .co_player_ids
                        .extend(active_ids.iter().filter(|id| **id != member.user_id));
                }
            }

            // Sessions starten, wenn genug aktive User da sind
            if user_count >= cfg.min_users_for_tracking {
                for member in &active {
                    let key = (member.user_id, guild_id);
                    state.sessions.entry(key).or_insert_with(|| Session {
                        user_id: member.user_id,
                        display_name: member.display_name.clone(),
                        guild_id,
                        channel_id,
                        channel_name: channel_name.clone(),
                        start_time: now,
                        last_update: now,
                        peak_users: 1,
                        user_counts: Vec::new(),
                        co_player_ids: BTreeSet::new(),
                    });
                    if let Some(session) = state.sessions.get_mut(&key) {
                        session.last_update = now;
                    }
                }
            }

            // Nicht (mehr) aktive Kanal-Mitglieder: Session beenden
            for member in &filtered {
                let key = (member.user_id, guild_id);
                if !active_ids.contains(&member.user_id) {
                    if let Some(session) = state.sessions.remove(&key) {
                        to_finalize.push(session);
                    }
                    state.grace.remove(&key);
                }
            }
        }
        for session in to_finalize {
            self.finalize(session, now).await;
        }
    }

    async fn end_session(self: &Arc<Self>, user_id: u64, guild_id: u64, end_time: NaiveDateTime) {
        let session = {
            let mut state = self.state.lock().await;
            state.grace.remove(&(user_id, guild_id));
            state.sessions.remove(&(user_id, guild_id))
        };
        if let Some(session) = session {
            self.finalize(session, end_time).await;
        }
    }

    /// Persistiert eine abgeschlossene Session (wie `_finalize_session`).
    async fn finalize(&self, session: Session, end_time: NaiveDateTime) {
        let seconds = (end_time - session.start_time).num_seconds().max(0);
        if seconds <= 0 {
            return;
        }
        let points = calculate_points(seconds, session.peak_users.max(1));
        let started_iso = session.start_time.format("%Y-%m-%d %H:%M:%S").to_string();
        let ended_iso = end_time.format("%Y-%m-%d %H:%M:%S").to_string();
        let user_counts_json = serde_json::to_string(&session.user_counts).unwrap_or_default();
        let co_player_ids_json =
            serde_json::to_string(&session.co_player_ids.iter().collect::<Vec<_>>())
                .unwrap_or_default();

        let result = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO voice_stats(user_id, total_seconds, total_points, last_update)
                     VALUES(?1, ?2, ?3, CURRENT_TIMESTAMP)
                     ON CONFLICT(user_id) DO UPDATE SET
                       total_seconds = total_seconds + excluded.total_seconds,
                       total_points  = total_points  + excluded.total_points,
                       last_update   = CURRENT_TIMESTAMP",
                    rusqlite::params![session.user_id, seconds, points],
                )?;
                conn.execute(
                    "INSERT INTO voice_session_log(
                       user_id, display_name, guild_id, channel_id, channel_name,
                       started_at, ended_at, duration_seconds, points, peak_users,
                       user_counts_json, co_player_ids
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    rusqlite::params![
                        session.user_id,
                        session.display_name,
                        session.guild_id,
                        session.channel_id,
                        session.channel_name,
                        started_iso,
                        ended_iso,
                        seconds,
                        points,
                        session.peak_users,
                        user_counts_json,
                        co_player_ids_json,
                    ],
                )?;
                Ok(())
            })
            .await;
        if let Err(err) = result {
            tracing::error!(%err, user_id = session.user_id, "Voice-Session-Persistierung fehlgeschlagen");
        }
    }

    /// Anzahl laufender Sessions (Diagnose/Tests).
    pub async fn active_sessions(&self) -> usize {
        self.state.lock().await.sessions.len()
    }

    /// Grace-Monitor-Tick: abgelaufene Schonfristen beenden (60s-Loop).
    pub async fn expire_grace_periods(self: &Arc<Self>) {
        let now = Utc::now().naive_utc();
        let expired: Vec<(u64, u64)> = {
            let state = self.state.lock().await;
            let mut expired = Vec::new();
            for ((user_id, guild_id), grace) in &state.grace {
                let cfg = state
                    .config_cache
                    .get(guild_id)
                    .cloned()
                    .unwrap_or_default();
                if (now - grace.started).num_seconds() >= cfg.grace_period_duration {
                    expired.push((*user_id, *guild_id));
                }
            }
            expired
        };
        for (user_id, guild_id) in expired {
            self.end_grace(user_id, guild_id).await;
        }
    }

    /// Cleanup-Tick: verwaiste Sessions finalisieren (5min-Loop).
    pub async fn cleanup_stale_sessions(self: &Arc<Self>) {
        let now = Utc::now().naive_utc();
        let stale: Vec<Session> = {
            let mut state = self.state.lock().await;
            let mut stale_keys = Vec::new();
            for (key, session) in &state.sessions {
                let timeout = state
                    .config_cache
                    .get(&session.guild_id)
                    .map(|c| c.session_timeout)
                    .unwrap_or_else(|| TrackerConfig::default().session_timeout);
                if (now - session.last_update).num_seconds() >= timeout {
                    stale_keys.push(*key);
                }
            }
            stale_keys
                .iter()
                .filter_map(|key| state.sessions.remove(key))
                .collect()
        };
        for session in stale {
            let end_time = session.last_update;
            self.finalize(session, end_time).await;
        }
    }

    /// Keep-Alive-Tick (2min-Loop): laufende Sessions als lebendig markieren.
    pub async fn touch_sessions(&self) {
        let now = Utc::now().naive_utc();
        let mut state = self.state.lock().await;
        for session in state.sessions.values_mut() {
            session.last_update = now;
        }
    }
}

/// Subscriber + Wartungs-Loops starten.
pub fn spawn(
    tracker: Arc<VoiceTracker>,
    dispatcher: &Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();

    let mut events = dispatcher.subscribe_voice();
    let event_tracker = tracker.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => event_tracker.handle_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "VoiceTracker: Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    let keepalive = tracker.clone();
    handles.push(tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(120)).await;
            keepalive.touch_sessions().await;
        }
    }));

    let grace = tracker.clone();
    handles.push(tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            grace.expire_grace_periods().await;
        }
    }));

    let cleanup = tracker;
    handles.push(tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            cleanup.cleanup_stale_sessions().await;
        }
    }));

    handles
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn punkte_wie_python() {
        // Referenzwerte aus CPython (calculate_points)
        assert_eq!(calculate_points(600, 5), 11);
        assert_eq!(calculate_points(600, 3), 11);
        assert_eq!(calculate_points(600, 2), 10);
        assert_eq!(calculate_points(59, 5), 0);
        assert_eq!(calculate_points(3600, 5), 66);
        assert_eq!(calculate_points(3600, 4), 63);
        assert_eq!(calculate_points(0, 5), 0);
    }

    /// Mock-Snapshot mit setzbarem Kanal-Zustand.
    struct MockSnapshot {
        states: StdMutex<HashMap<(u64, u64), Vec<VoiceMemberState>>>,
        names: StdMutex<HashMap<u64, String>>,
    }

    #[async_trait::async_trait]
    impl VoiceSnapshot for MockSnapshot {
        async fn channel_states(
            &self,
            guild_id: u64,
            channel_id: u64,
            _grace_role_id: u64,
        ) -> Vec<VoiceMemberState> {
            self.states
                .lock()
                .expect("mock lock")
                .get(&(guild_id, channel_id))
                .cloned()
                .unwrap_or_default()
        }

        async fn channel_name(&self, _guild_id: u64, channel_id: u64) -> Option<String> {
            self.names
                .lock()
                .expect("mock lock")
                .get(&channel_id)
                .cloned()
        }
    }

    fn member(user_id: u64, name: &str) -> VoiceMemberState {
        VoiceMemberState {
            user_id,
            display_name: name.to_string(),
            ..VoiceMemberState::default()
        }
    }

    const DDLS: [&str; 4] = [
        "CREATE TABLE kv_store(ns TEXT NOT NULL, k TEXT NOT NULL, v TEXT NOT NULL, PRIMARY KEY(ns, k))",
        "CREATE TABLE user_privacy(user_id INTEGER PRIMARY KEY, opted_out INTEGER NOT NULL DEFAULT 0, deleted_at INTEGER, reason TEXT, updated_at INTEGER)",
        "CREATE TABLE voice_stats(user_id INTEGER PRIMARY KEY, total_seconds INTEGER NOT NULL DEFAULT 0, last_update DATETIME DEFAULT CURRENT_TIMESTAMP, total_points INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE voice_session_log(id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, guild_id INTEGER, channel_id INTEGER, channel_name TEXT, started_at DATETIME NOT NULL, ended_at DATETIME NOT NULL, duration_seconds INTEGER NOT NULL DEFAULT 0, points INTEGER NOT NULL DEFAULT 0, peak_users INTEGER, user_counts_json TEXT, display_name TEXT, co_player_ids TEXT)",
    ];

    async fn setup() -> (tempfile::TempDir, Arc<VoiceTracker>, Arc<MockSnapshot>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("test.sqlite3")).expect("db");
        for ddl in DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        let snapshot = Arc::new(MockSnapshot {
            states: StdMutex::new(HashMap::new()),
            names: StdMutex::new(HashMap::new()),
        });
        let tracker = VoiceTracker::new(db, snapshot.clone());
        (dir, tracker, snapshot)
    }

    #[tokio::test]
    async fn session_lebenszyklus_schreibt_stats_und_log() {
        let (_dir, tracker, snapshot) = setup().await;
        snapshot
            .names
            .lock()
            .expect("lock")
            .insert(10, "Lobby 1".to_string());

        // Zwei aktive User betreten den Kanal → Sessions starten
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![member(100, "Anna"), member(200, "Ben")]);
        tracker
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 10,
            })
            .await;
        // Zweites Join-Event (wie real: jeder Beitritt feuert) → Co-Player-Update
        tracker
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 200,
                channel_id: 10,
            })
            .await;
        assert_eq!(tracker.active_sessions().await, 2);

        // Startzeit künstlich zurückdatieren, damit Sekunden > 0 entstehen
        {
            let mut state = tracker.state.lock().await;
            for session in state.sessions.values_mut() {
                session.start_time -= chrono::Duration::seconds(600);
            }
        }

        // Anna verlässt den Kanal → ihre Session wird finalisiert
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![member(200, "Ben")]);
        tracker
            .handle_event(VoiceEvent::Leave {
                guild_id: 1,
                user_id: 100,
                channel_id: 10,
            })
            .await;
        assert_eq!(tracker.active_sessions().await, 1);

        let (seconds, points, co_players, channel_name): (i64, i64, String, String) = tracker
            .db
            .read(|conn| {
                conn.query_row(
                    "SELECT duration_seconds, points, co_player_ids, channel_name
                     FROM voice_session_log WHERE user_id = 100",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
            })
            .await
            .expect("log row");
        assert!(seconds >= 600);
        assert_eq!(points, calculate_points(seconds, 2));
        assert_eq!(co_players, "[200]");
        assert_eq!(channel_name, "Lobby 1");

        let total: i64 = tracker
            .db
            .read(|conn| {
                conn.query_row(
                    "SELECT total_seconds FROM voice_stats WHERE user_id = 100",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("stats row");
        assert_eq!(total, seconds);
    }

    #[tokio::test]
    async fn allein_im_kanal_startet_keine_session() {
        let (_dir, tracker, snapshot) = setup().await;
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![member(100, "Solo")]);
        tracker
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 10,
            })
            .await;
        assert_eq!(tracker.active_sessions().await, 0);
    }

    #[tokio::test]
    async fn mute_ohne_grace_rolle_beendet_session() {
        let (_dir, tracker, snapshot) = setup().await;
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![member(100, "Anna"), member(200, "Ben")]);
        tracker
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 10,
            })
            .await;
        assert_eq!(tracker.active_sessions().await, 2);

        // Ben mutet sich (keine Grace-Rolle) → seine Session endet
        let mut muted = member(200, "Ben");
        muted.muted_or_deaf = true;
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![member(100, "Anna"), muted]);
        tracker
            .handle_event(VoiceEvent::Update {
                guild_id: 1,
                user_id: 200,
                channel_id: 10,
                was_muted: false,
                is_muted: true,
            })
            .await;
        assert_eq!(tracker.active_sessions().await, 1);
    }

    #[tokio::test]
    async fn grace_rolle_haelt_session_bei_mute() {
        let (_dir, tracker, snapshot) = setup().await;
        let mut anna = member(100, "Anna");
        anna.has_grace_role = true;
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![anna.clone(), member(200, "Ben")]);
        tracker
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 10,
            })
            .await;
        assert_eq!(tracker.active_sessions().await, 2);

        // Anna mutet sich — Grace-Rolle → Session bleibt
        anna.muted_or_deaf = true;
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![anna, member(200, "Ben")]);
        tracker
            .handle_event(VoiceEvent::Update {
                guild_id: 1,
                user_id: 100,
                channel_id: 10,
                was_muted: false,
                is_muted: true,
            })
            .await;
        assert_eq!(tracker.active_sessions().await, 2);
    }

    #[tokio::test]
    async fn opt_out_wird_nie_getrackt() {
        let (_dir, tracker, snapshot) = setup().await;
        tracker
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO user_privacy(user_id, opted_out) VALUES(100, 1)",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("optout");
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![member(100, "Anna"), member(200, "Ben")]);
        tracker
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 200,
                channel_id: 10,
            })
            .await;
        // Anna (Opt-out) bekommt keine Session; Ben allein reicht nicht für min_users=2
        assert_eq!(tracker.active_sessions().await, 0);
    }

    #[tokio::test]
    async fn config_default_wird_persistiert_und_gelesen() {
        let (_dir, tracker, _snapshot) = setup().await;
        let cfg = tracker.config(1).await;
        assert_eq!(cfg, TrackerConfig::default());
        let raw = tracker
            .db
            .kv_get(KV_NAMESPACE, "1")
            .await
            .expect("kv")
            .expect("persistiert");
        assert!(raw.contains("\"min_users_for_tracking\":2"));

        // Überschreiben + Cache invalidieren prüfen (frischer Tracker)
        tracker
            .db
            .kv_set(
                KV_NAMESPACE,
                "1",
                json!({"min_users_for_tracking": 3}).to_string(),
            )
            .await
            .expect("set");
        let fresh = VoiceTracker::new(tracker.db.clone(), tracker.snapshot.clone());
        let cfg = fresh.config(1).await;
        assert_eq!(cfg.min_users_for_tracking, 3);
        assert_eq!(cfg.grace_period_duration, 180); // Default bleibt
    }

    #[tokio::test]
    async fn cleanup_finalisiert_verwaiste_sessions() {
        let (_dir, tracker, snapshot) = setup().await;
        snapshot
            .states
            .lock()
            .expect("lock")
            .insert((1, 10), vec![member(100, "Anna"), member(200, "Ben")]);
        tracker
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 10,
            })
            .await;
        // last_update künstlich altern lassen (> session_timeout 300s)
        {
            let mut state = tracker.state.lock().await;
            for session in state.sessions.values_mut() {
                session.start_time -= chrono::Duration::seconds(900);
                session.last_update -= chrono::Duration::seconds(600);
            }
        }
        tracker.cleanup_stale_sessions().await;
        assert_eq!(tracker.active_sessions().await, 0);
        let rows: i64 = tracker
            .db
            .read(|c| c.query_row("SELECT COUNT(*) FROM voice_session_log", [], |r| r.get(0)))
            .await
            .expect("count");
        assert_eq!(rows, 2);
    }
}
