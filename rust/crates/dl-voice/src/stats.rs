//! Voice-Statistik-Befehle (`!vstats`, `!vleaderboard`/`!vlb`/`!voicetop`).
//!
//! 1:1-Port der Prefix-Commands aus `cogs/voice_activity_tracker.py`. Bewusst
//! OHNE Admin-Gate — die Befehle stehen allen offen. Die Aggregat-Daten in
//! `voice_stats` sind global (kein Guild-Filter, wie im Original); der Live-
//! Zuschlag stammt aus der laufenden Session der aktuellen Guild.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dl_db::Db;
use dl_discord::{ChannelSender, Dispatcher};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::feedback::VoiceFeedback;
use crate::tracker::{calculate_points, VoiceTracker};

pub const VOICE_ADMIN_TEST_TITLE: &str = "🔧 Voice-System-Test (zentrale DB)";
pub const VOICE_ADMIN_STATUS_TITLE: &str = "🔧 Voice-System Admin-Status (zentrale DB)";
pub const VOICE_ADMIN_CONFIG_TITLE: &str = "⚙️ Voice-Tracker-Konfiguration (zentrale DB)";
pub const VOICE_ADMIN_LIVE_SESSIONS_LABEL: &str = "🔴 Live-Sessions";
pub const VOICE_ADMIN_GRACE_DURATION_LABEL: &str = "⏱️ Schonzeit";
pub const VOICE_ADMIN_MIN_USERS_LABEL: &str = "👥 Min. User";
pub const VOICE_ADMIN_GRACE_PERIODS_LABEL: &str = "🛡️ Schonzeiten";
pub const VOICE_ADMIN_ROLE_ID_LABEL: &str = "🎖️ Rollen-ID";
pub const VOICE_ADMIN_SESSION_TIMEOUT_LABEL: &str = "🔄 Session-Timeout";
pub const VOICE_ADMIN_MAX_SESSIONS_LABEL: &str = "📊 Max. Sessions";
pub const VOICE_ADMIN_CONFIG_UPDATED_TEXT: &str =
    "✅ Einstellung aktualisiert (zentral gespeichert).";
pub const VOICE_ADMIN_CONFIG_INVALID_TEXT: &str = "❌ Ungültige Eingabe. Optionen: grace_duration (60–600), grace_role (Rollen-ID), min_users (2–10), session_timeout (60–3600), max_sessions (10–10000).";
pub const VOICE_FEEDBACK_TEST_SENT_TEXT: &str = "Feedback-Test verschickt. Bitte DMs prüfen.";
pub const VOICE_FEEDBACK_TEST_UNAVAILABLE_TEXT: &str =
    "Voice-Feedback ist aktuell nicht verfügbar.";

/// Cache-Zugriffe, die die Befehle über den Gateway-Cache brauchen
/// (Namensauflösung, Rollencheck, Guild-Name). Implementiert von der
/// `CacheSnapshot`-Glue.
#[async_trait::async_trait]
pub trait StatsPort: Send + Sync {
    /// Anzeigenamen für mehrere User aus dem Cache (erste Fundgilde). Nicht
    /// gefundene IDs fehlen in der Map; der Aufrufer füllt sie selbst auf.
    async fn resolve_names(&self, user_ids: &[u64]) -> HashMap<u64, String>;
    /// Rollen-IDs eines Mitglieds in einer Guild (Cache, sonst leer).
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    /// Guild-Name aus dem Cache.
    async fn guild_name(&self, guild_id: u64) -> Option<String>;
}

/// `<@123>` / `<@!123>` oder rohe ID → user_id (wie `parse_mention` in
/// nudge/rank, plus der ID-Pfad von Pythons MemberConverter).
fn parse_target(token: &str) -> Option<u64> {
    if let Some(inner) = token.strip_prefix("<@").and_then(|s| s.strip_suffix('>')) {
        return inner.strip_prefix('!').unwrap_or(inner).parse::<u64>().ok();
    }
    token.parse::<u64>().ok()
}

/// Erstes Ziel nach dem Befehlswort (Mention oder rohe ID), sonst None.
fn first_target(content: &str) -> Option<u64> {
    content.split_whitespace().skip(1).find_map(parse_target)
}

/// Deutsche Tausender-Trennung mit Punkt — Port von `_format_leaderboard_number`
/// (`f"{int(value):,}".replace(",", ".")`).
fn format_de(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push('.');
        }
        out.push(*b as char);
    }
    if value < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Gleitendes Fenster wie der Cog-eigene `RateLimiter` (max Anfragen pro
/// Fenster, pro User). `check` registriert bei Erlaubnis den Treffer; bei Limit
/// liefert es die Restsekunden, bis der älteste Treffer aus dem Fenster fällt.
struct RateLimiter {
    max: usize,
    window: Duration,
    hits: Mutex<HashMap<u64, VecDeque<Instant>>>,
}

impl RateLimiter {
    fn new(max: usize, window_secs: u64) -> Self {
        Self {
            max,
            window: Duration::from_secs(window_secs),
            hits: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, user_id: u64) -> Result<(), i64> {
        let now = Instant::now();
        let mut map = self.hits.lock().expect("rate-limit mutex");
        let dq = map.entry(user_id).or_default();
        while let Some(&front) = dq.front() {
            if now.duration_since(front) >= self.window {
                dq.pop_front();
            } else {
                break;
            }
        }
        if dq.len() >= self.max {
            let oldest = *dq.front().expect("queue not empty");
            // Wie Python `int(window - elapsed)`: Differenz als Float bilden und
            // erst das Ergebnis zur Null hin abschneiden (nicht elapsed vorrunden).
            let remaining = self.window.as_secs_f64() - now.duration_since(oldest).as_secs_f64();
            return Err(remaining.max(0.0) as i64);
        }
        dq.push_back(now);
        Ok(())
    }
}

/// Antwort eines Befehls: Klartext oder genau ein Embed.
pub struct StatsReply {
    pub content: Option<String>,
    pub embeds: Vec<Value>,
}

impl StatsReply {
    fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            embeds: vec![],
        }
    }
    fn embed(embed: Value) -> Self {
        Self {
            content: None,
            embeds: vec![embed],
        }
    }
}

/// Lesezugriff auf `voice_stats` (es gibt keine vorhandene öffentliche API
/// dafür; das öffentliche Web-Leaderboard liegt im nicht importierbaren
/// dl-stats-HTTP-Handler).
struct VoiceStatsStore {
    db: Db,
}

impl VoiceStatsStore {
    fn new(db: Db) -> Self {
        Self { db }
    }

    /// `(total_seconds, total_points)`; Default `(0, 0)` ohne Datensatz.
    async fn totals(&self, user_id: u64) -> (i64, i64) {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT total_seconds, total_points FROM voice_stats WHERE user_id = ?1",
                    [user_id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .unwrap_or((0, 0))
    }

    /// Top-`limit` nach Punkten DESC, Sekunden DESC: `(user_id, secs, points)`.
    async fn top(&self, limit: i64) -> Vec<(u64, i64, i64)> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT user_id, total_seconds, total_points FROM voice_stats \
                     ORDER BY total_points DESC, total_seconds DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map([limit], |r| {
                    Ok((
                        r.get::<_, i64>(0)? as u64,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap_or_default()
    }

    /// Footer-Platzierung: `None` ohne Datensatz, sonst `(rank, points)` mit
    /// identischem Tiebreak wie das Leaderboard (Punkte, dann Sekunden).
    async fn rank(&self, user_id: u64) -> Option<(i64, i64)> {
        self.db
            .read(move |conn| {
                let row: Option<(i64, i64)> = conn
                    .query_row(
                        "SELECT total_points, total_seconds FROM voice_stats WHERE user_id = ?1",
                        [user_id],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .optional()?;
                let Some((points, seconds)) = row else {
                    return Ok(None);
                };
                let rank: i64 = conn.query_row(
                    "SELECT COUNT(*) + 1 FROM voice_stats \
                     WHERE total_points > ?1 OR (total_points = ?1 AND total_seconds > ?2)",
                    rusqlite::params![points, seconds],
                    |r| r.get(0),
                )?;
                Ok(Some((rank, points)))
            })
            .await
            .ok()
            .flatten()
    }
}

/// Befehlsschicht für `!vstats` / `!vleaderboard`.
pub struct VoiceStatsCommands {
    store: VoiceStatsStore,
    tracker: Arc<VoiceTracker>,
    feedback: Option<Arc<VoiceFeedback>>,
    port: Arc<dyn StatsPort>,
    limiter: RateLimiter,
}

impl VoiceStatsCommands {
    pub fn new(
        db: Db,
        tracker: Arc<VoiceTracker>,
        port: Arc<dyn StatsPort>,
        feedback: Option<Arc<VoiceFeedback>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: VoiceStatsStore::new(db),
            tracker,
            feedback,
            port,
            // Cog-Original: max 5 Anfragen pro 30-s-Fenster pro User.
            limiter: RateLimiter::new(5, 30),
        })
    }

    pub async fn reply_for(
        &self,
        content: &str,
        guild_id: u64,
        author_id: u64,
        author_name: &str,
        author_is_admin: bool,
    ) -> Option<StatsReply> {
        let root = content.split_whitespace().next()?.to_lowercase();
        match root.as_str() {
            "!vstats" => Some(self.vstats(content, guild_id, author_id, author_name).await),
            "!vleaderboard" | "!vlb" | "!voicetop" => {
                Some(self.vleaderboard(guild_id, author_id).await)
            }
            "!vtest" => Some(self.vtest(guild_id).await),
            "!vf1" if author_is_admin => Some(
                self.feedback_test(content, guild_id, author_id, "first")
                    .await,
            ),
            "!vf4" if author_is_admin => Some(
                self.feedback_test(content, guild_id, author_id, "second")
                    .await,
            ),
            "!voice_status" if author_is_admin => Some(self.voice_status(guild_id).await),
            "!voice_config" if author_is_admin => Some(self.voice_config(content, guild_id).await),
            "!vf1" | "!vf4" | "!voice_status" | "!voice_config" => None,
            _ => None,
        }
    }

    async fn vtest(&self, guild_id: u64) -> StatsReply {
        let cfg = self.tracker.config(guild_id).await;
        StatsReply::embed(json!({
            "title": VOICE_ADMIN_TEST_TITLE,
            "color": 0x00FF99,
            "fields": [
                { "name": VOICE_ADMIN_LIVE_SESSIONS_LABEL, "value": self.tracker.active_session_count().await.to_string(), "inline": true },
                { "name": VOICE_ADMIN_GRACE_DURATION_LABEL, "value": cfg.grace_period_duration.to_string(), "inline": true },
                { "name": VOICE_ADMIN_MIN_USERS_LABEL, "value": cfg.min_users_for_tracking.to_string(), "inline": true },
            ],
        }))
    }

    async fn voice_status(&self, guild_id: u64) -> StatsReply {
        let cfg = self.tracker.config(guild_id).await;
        StatsReply::embed(json!({
            "title": VOICE_ADMIN_STATUS_TITLE,
            "color": 0x00FF99,
            "fields": [
                { "name": VOICE_ADMIN_LIVE_SESSIONS_LABEL, "value": self.tracker.active_session_count().await.to_string(), "inline": true },
                { "name": VOICE_ADMIN_GRACE_PERIODS_LABEL, "value": self.tracker.grace_period_count().await.to_string(), "inline": true },
                { "name": VOICE_ADMIN_MIN_USERS_LABEL, "value": cfg.min_users_for_tracking.to_string(), "inline": true },
                { "name": VOICE_ADMIN_GRACE_DURATION_LABEL, "value": cfg.grace_period_duration.to_string(), "inline": true },
                { "name": VOICE_ADMIN_ROLE_ID_LABEL, "value": cfg.special_role_id.to_string(), "inline": true },
            ],
        }))
    }

    async fn voice_config(&self, content: &str, guild_id: u64) -> StatsReply {
        let mut parts = content.split_whitespace();
        let _root = parts.next();
        let Some(setting) = parts.next() else {
            let cfg = self.tracker.config(guild_id).await;
            return StatsReply::embed(json!({
                "title": VOICE_ADMIN_CONFIG_TITLE,
                "color": 0x0099FF,
                "fields": [
                    { "name": VOICE_ADMIN_MIN_USERS_LABEL, "value": cfg.min_users_for_tracking.to_string(), "inline": true },
                    { "name": VOICE_ADMIN_GRACE_DURATION_LABEL, "value": cfg.grace_period_duration.to_string(), "inline": true },
                    { "name": VOICE_ADMIN_ROLE_ID_LABEL, "value": cfg.special_role_id.to_string(), "inline": true },
                    { "name": VOICE_ADMIN_SESSION_TIMEOUT_LABEL, "value": cfg.session_timeout.to_string(), "inline": true },
                    { "name": VOICE_ADMIN_MAX_SESSIONS_LABEL, "value": cfg.max_sessions_per_user.to_string(), "inline": true },
                ],
            }));
        };
        let Some(value) = parts.next() else {
            return StatsReply::text(VOICE_ADMIN_CONFIG_INVALID_TEXT);
        };
        let Ok(parsed) = value.parse::<i64>() else {
            return StatsReply::text(VOICE_ADMIN_CONFIG_INVALID_TEXT);
        };
        let mut cfg = self.tracker.config(guild_id).await;
        let valid = match setting.to_ascii_lowercase().as_str() {
            "grace_duration" if (60..=600).contains(&parsed) => {
                cfg.grace_period_duration = parsed;
                true
            }
            "grace_role" if parsed > 0 => {
                cfg.special_role_id = parsed as u64;
                true
            }
            "min_users" if (2..=10).contains(&parsed) => {
                cfg.min_users_for_tracking = parsed;
                true
            }
            "session_timeout" if (60..=3600).contains(&parsed) => {
                cfg.session_timeout = parsed;
                true
            }
            "max_sessions" if (10..=10000).contains(&parsed) => {
                cfg.max_sessions_per_user = parsed;
                true
            }
            _ => false,
        };
        if !valid {
            return StatsReply::text(VOICE_ADMIN_CONFIG_INVALID_TEXT);
        }
        match self.tracker.store_config(guild_id, cfg).await {
            Ok(()) => StatsReply::text(VOICE_ADMIN_CONFIG_UPDATED_TEXT),
            Err(err) => {
                tracing::warn!(%err, guild_id, "Voice config update failed");
                StatsReply::text(VOICE_ADMIN_CONFIG_INVALID_TEXT)
            }
        }
    }

    async fn feedback_test(
        &self,
        content: &str,
        guild_id: u64,
        author_id: u64,
        request_type: &str,
    ) -> StatsReply {
        let Some(feedback) = &self.feedback else {
            return StatsReply::text(VOICE_FEEDBACK_TEST_UNAVAILABLE_TEXT);
        };
        let target = first_target(content).unwrap_or(author_id);
        let co_players = if target == author_id {
            Vec::new()
        } else {
            vec![author_id]
        };
        feedback
            .send_test_request(
                guild_id,
                target,
                0,
                "Admin Test",
                &co_players,
                300,
                request_type,
            )
            .await;
        StatsReply::text(VOICE_FEEDBACK_TEST_SENT_TEXT)
    }

    async fn vstats(
        &self,
        content: &str,
        guild_id: u64,
        author_id: u64,
        author_name: &str,
    ) -> StatsReply {
        if let Err(remaining) = self.limiter.check(author_id) {
            return StatsReply::text(format!(
                "⏰ Rate limit reached. Try again in {remaining} seconds."
            ));
        }
        let target = first_target(content).unwrap_or(author_id);
        let (total_seconds, total_points) = self.store.totals(target).await;
        let live = self.tracker.live_session(target, guild_id).await;
        let (live_add, live_points) = match live {
            Some((secs, peak)) => (secs, calculate_points(secs, peak)),
            None => (0, 0),
        };
        let total = total_seconds + live_add;
        let hours = total / 3600;
        let minutes = (total % 3600) / 60;
        let name = self.resolve_one(target).await;

        let mut fields = vec![
            json!({ "name": "⏱️ Gesamtzeit", "value": format!("{hours}h {minutes}m"), "inline": true }),
            json!({ "name": "⭐ Punkte", "value": (total_points + live_points).to_string(), "inline": true }),
        ];
        if live.is_some() {
            fields.push(json!({
                "name": "Status",
                "value": format!("🔴 Live: +{}m / +{}pts", live_add / 60, live_points),
                "inline": true
            }));
        }
        let special = self.tracker.special_role_id(guild_id).await;
        if self
            .port
            .member_role_ids(guild_id, target)
            .await
            .contains(&special)
        {
            fields.push(json!({
                "name": "🎖️ Spezielle Rolle",
                "value": "Grace Period berechtigt (3 Min Schutz)",
                "inline": false
            }));
        }
        StatsReply::embed(json!({
            "title": format!("📊 Voice-Statistiken - {name}"),
            "color": 0x3498DB,
            "fields": fields,
            "footer": { "text": format!("Angefragt von {author_name}") }
        }))
    }

    async fn vleaderboard(&self, guild_id: u64, author_id: u64) -> StatsReply {
        if let Err(remaining) = self.limiter.check(author_id) {
            return StatsReply::text(format!(
                "⏰ Rate limit reached. Try again in {remaining} seconds."
            ));
        }
        let rows = self.store.top(10).await;
        let guild_name = self.port.guild_name(guild_id).await.unwrap_or_default();
        let description = if rows.is_empty() {
            "📊 Noch keine Voice-Aktivität aufgezeichnet.".to_string()
        } else {
            let ids: Vec<u64> = rows.iter().map(|(uid, _, _)| *uid).collect();
            let names = self.port.resolve_names(&ids).await;
            let mut out = String::new();
            for (idx, (uid, secs, points)) in rows.iter().enumerate() {
                let medal = match idx + 1 {
                    1 => "🥇".to_string(),
                    2 => "🥈".to_string(),
                    3 => "🥉".to_string(),
                    n => format!("{n}."),
                };
                let name = names
                    .get(uid)
                    .cloned()
                    .unwrap_or_else(|| format!("User {uid}"));
                let hours = secs / 3600;
                let minutes = (secs % 3600) / 60;
                out.push_str(&format!(
                    "{medal} **{name}** — {hours}h {minutes}m · {} Punkte\n",
                    format_de(*points)
                ));
            }
            out
        };
        let footer = match self.store.rank(author_id).await {
            None => "Noch keine Punkte".to_string(),
            Some((rank, points)) => {
                format!("Du bist auf Platz {rank} · {} Punkte", format_de(points))
            }
        };
        StatsReply::embed(json!({
            "title": format!("🏆 Voice-Leaderboard - {guild_name}"),
            "color": 0xF1C40F,
            "description": description,
            "footer": { "text": footer }
        }))
    }

    async fn resolve_one(&self, user_id: u64) -> String {
        self.port
            .resolve_names(&[user_id])
            .await
            .remove(&user_id)
            .unwrap_or_else(|| format!("User {user_id}"))
    }
}

/// `MessageEvent`-Subscriber für `!vstats` / `!vleaderboard` (+ Aliasse).
/// Bewusst OHNE Admin-Gate — die Befehle stehen allen offen.
pub fn spawn_command(
    commands: Arc<VoiceStatsCommands>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let Some(guild_id) = event.guild_id else {
                        continue;
                    };
                    let content = event.content.trim();
                    let Some(reply) = commands
                        .reply_for(
                            content,
                            guild_id,
                            event.author_id,
                            &event.author_display_name,
                            event.author_is_admin,
                        )
                        .await
                    else {
                        continue;
                    };
                    let _ = sender
                        .send_to_channel(event.channel_id, reply.content.as_deref(), &reply.embeds)
                        .await;
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
    fn format_de_setzt_punkte() {
        assert_eq!(format_de(0), "0");
        assert_eq!(format_de(999), "999");
        assert_eq!(format_de(1234), "1.234");
        assert_eq!(format_de(1234567), "1.234.567");
    }

    #[test]
    fn parse_target_akzeptiert_mention_und_id() {
        assert_eq!(parse_target("<@123>"), Some(123));
        assert_eq!(parse_target("<@!123>"), Some(123));
        assert_eq!(parse_target("456"), Some(456));
        assert_eq!(parse_target("kein-ziel"), None);
    }

    #[test]
    fn first_target_ueberspringt_befehlswort() {
        assert_eq!(first_target("!vstats <@9>"), Some(9));
        assert_eq!(first_target("!vstats"), None);
        // Das Befehlswort selbst wird nie als Ziel gelesen.
        assert_eq!(first_target("!vstats text <@7>"), Some(7));
    }

    #[test]
    fn rate_limiter_blockt_nach_max() {
        let rl = RateLimiter::new(2, 30);
        assert!(rl.check(1).is_ok());
        assert!(rl.check(1).is_ok());
        assert!(rl.check(1).is_err());
        // Anderer User unberührt.
        assert!(rl.check(2).is_ok());
    }

    struct MockStatsPort;

    #[async_trait::async_trait]
    impl StatsPort for MockStatsPort {
        async fn resolve_names(&self, user_ids: &[u64]) -> HashMap<u64, String> {
            user_ids
                .iter()
                .map(|id| (*id, format!("User {id}")))
                .collect()
        }

        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            Vec::new()
        }

        async fn guild_name(&self, guild_id: u64) -> Option<String> {
            Some(format!("Guild {guild_id}"))
        }
    }

    struct MockSnapshot;

    #[async_trait::async_trait]
    impl crate::tracker::VoiceSnapshot for MockSnapshot {
        async fn channel_states(
            &self,
            _guild_id: u64,
            _channel_id: u64,
            _grace_role_id: u64,
        ) -> Vec<crate::tracker::VoiceMemberState> {
            Vec::new()
        }
    }

    struct MockFeedbackPort {
        dms: StdMutex<Vec<(u64, String)>>,
        forwards: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl crate::feedback::FeedbackPort for MockFeedbackPort {
        async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>) {
            self.dms.lock().expect("lock").push((user_id, text));
            ("sent".to_string(), Some(555))
        }

        async fn forward_to_owner(&self, _owner_id: u64, text: String) {
            self.forwards.lock().expect("lock").push(text);
        }

        async fn delete_feedback_prompt(&self, _user_id: u64, _message_id: u64) {}

        async fn display_name(&self, _guild_id: u64, user_id: u64) -> Option<String> {
            Some(format!("User {user_id}"))
        }
    }

    const ADMIN_DDLS: [&str; 5] = [
        "CREATE TABLE kv_store(ns TEXT NOT NULL, k TEXT NOT NULL, v TEXT NOT NULL, PRIMARY KEY(ns, k))",
        "CREATE TABLE voice_stats(user_id INTEGER PRIMARY KEY, total_seconds INTEGER NOT NULL DEFAULT 0, total_points INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE voice_session_log(id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, guild_id INTEGER, channel_id INTEGER, channel_name TEXT, started_at DATETIME, ended_at DATETIME, duration_seconds INTEGER NOT NULL DEFAULT 0, points INTEGER NOT NULL DEFAULT 0, peak_users INTEGER, user_counts_json TEXT, display_name TEXT, co_player_ids TEXT)",
        "CREATE TABLE voice_feedback_requests(id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, guild_id INTEGER, channel_id INTEGER, channel_name TEXT, co_player_names TEXT, duration_seconds INTEGER, request_type TEXT DEFAULT 'first', status TEXT, error_message TEXT, prompt_message_id INTEGER, sent_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now')))",
        "CREATE TABLE voice_feedback_responses(id INTEGER PRIMARY KEY AUTOINCREMENT, request_id INTEGER, user_id INTEGER NOT NULL, message_id INTEGER, content TEXT, received_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now')))",
    ];

    async fn admin_setup() -> (
        tempfile::TempDir,
        Arc<VoiceStatsCommands>,
        Arc<crate::tracker::VoiceTracker>,
        Arc<MockFeedbackPort>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in ADMIN_DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        let tracker = crate::tracker::VoiceTracker::new(db.clone(), Arc::new(MockSnapshot));
        let feedback_port = Arc::new(MockFeedbackPort {
            dms: StdMutex::new(Vec::new()),
            forwards: StdMutex::new(Vec::new()),
        });
        let feedback = crate::feedback::VoiceFeedback::new(db.clone(), feedback_port.clone());
        let commands =
            VoiceStatsCommands::new(db, tracker.clone(), Arc::new(MockStatsPort), Some(feedback));
        (dir, commands, tracker, feedback_port)
    }

    #[tokio::test]
    async fn admin_commands_existieren_und_sind_admin_gated() {
        let (_dir, commands, _tracker, _feedback_port) = admin_setup().await;

        assert!(commands
            .reply_for("!vtest", 1, 9, "Admin", false)
            .await
            .is_some());
        assert!(commands
            .reply_for("!voice_status", 1, 9, "Admin", true)
            .await
            .is_some());
        assert!(commands
            .reply_for("!voice_config", 1, 9, "Admin", true)
            .await
            .is_some());
        assert!(commands
            .reply_for("!voice_config", 1, 9, "Admin", false)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn voice_config_schreibt_kv_store() {
        let (_dir, commands, tracker, _feedback_port) = admin_setup().await;
        let reply = commands
            .reply_for("!voice_config min_users 4", 1, 9, "Admin", true)
            .await
            .expect("reply");
        assert_eq!(
            reply.content.as_deref(),
            Some(VOICE_ADMIN_CONFIG_UPDATED_TEXT)
        );
        assert_eq!(tracker.config(1).await.min_users_for_tracking, 4);
    }

    #[tokio::test]
    async fn vf1_und_vf4_senden_feedback_testprompts() {
        let (_dir, commands, _tracker, feedback_port) = admin_setup().await;
        let first = commands
            .reply_for("!vf1 <@200>", 1, 9, "Admin", true)
            .await
            .expect("first reply");
        assert_eq!(
            first.content.as_deref(),
            Some(VOICE_FEEDBACK_TEST_SENT_TEXT)
        );

        let second = commands
            .reply_for("!vf4 <@300>", 1, 9, "Admin", true)
            .await
            .expect("second reply");
        assert_eq!(
            second.content.as_deref(),
            Some(VOICE_FEEDBACK_TEST_SENT_TEXT)
        );

        let dms = feedback_port.dms.lock().expect("lock");
        assert_eq!(
            dms.iter().map(|(user_id, _)| *user_id).collect::<Vec<_>>(),
            vec![200, 300]
        );
    }
}
