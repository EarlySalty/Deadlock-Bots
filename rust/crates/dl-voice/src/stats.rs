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

use crate::tracker::{calculate_points, VoiceTracker};

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
    port: Arc<dyn StatsPort>,
    limiter: RateLimiter,
}

impl VoiceStatsCommands {
    pub fn new(db: Db, tracker: Arc<VoiceTracker>, port: Arc<dyn StatsPort>) -> Arc<Self> {
        Arc::new(Self {
            store: VoiceStatsStore::new(db),
            tracker,
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
    ) -> Option<StatsReply> {
        let root = content.split_whitespace().next()?.to_lowercase();
        match root.as_str() {
            "!vstats" => Some(self.vstats(content, guild_id, author_id, author_name).await),
            "!vleaderboard" | "!vlb" | "!voicetop" => {
                Some(self.vleaderboard(guild_id, author_id).await)
            }
            _ => None,
        }
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
                        .reply_for(content, guild_id, event.author_id, &event.author_display_name)
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
}
