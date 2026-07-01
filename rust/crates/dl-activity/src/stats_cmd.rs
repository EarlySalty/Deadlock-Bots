//! Aktivitäts-/Text-Statistik-Befehle (Prefix), 1:1-Port aus
//! `cogs/user_activity_analyzer.py`:
//! `!useranalysis`/`!ua`/`!analyze`, `!myactivity`,
//! `!tleaderboard`/`!tlb`/`!texttop`, `!messagestats`/`!msgstats`,
//! `!serverstats` (Letzteres admin-gegated).
//!
//! Zahlenformatierung ist im Original bewusst uneinheitlich und wird hier
//! beibehalten: Leaderboards nutzen den deutschen Punkt als Tausender-Trenner,
//! `!messagestats`/`!serverstats` das Python-Komma.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Datelike, NaiveDateTime, Timelike};
use dl_discord::{ChannelSender, Dispatcher};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::db::{discord_id_to_i64, i64_to_u64, ActivityDbResult};

const DAY_NAMES: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
pub const SMARTPING_PING_MESSAGE_PLACEHOLDER: &str =
    "deine Leute sind gerade aktiv – komm doch in den Voice und zock ne Runde mit! 🎮";
pub const SMARTPING_USAGE_PLACEHOLDER: &str = "Nutzung: `!smartping @User [grund]`";
pub const SMARTPING_BLOCKED_PLACEHOLDER: &str =
    "❌ Dieser User kann gerade nicht gepingt werden (Cooldown oder Opt-out).";

/// Cache-Zugriffe der Befehle (Namen + Guild-Name). Implementiert von der
/// `StatsNames`-Glue.
#[async_trait::async_trait]
pub trait NamePort: Send + Sync {
    async fn resolve_names(&self, user_ids: &[u64]) -> HashMap<u64, String>;
    async fn guild_name(&self, guild_id: u64) -> Option<String>;
}

// ── Helfer ──────────────────────────────────────────────────────────────────

/// `<@123>` / `<@!123>` oder rohe ID → user_id.
fn parse_target(token: &str) -> Option<u64> {
    if let Some(inner) = token.strip_prefix("<@").and_then(|s| s.strip_suffix('>')) {
        return inner.strip_prefix('!').unwrap_or(inner).parse::<u64>().ok();
    }
    token.parse::<u64>().ok()
}

/// Erstes Ziel nach dem Befehlswort, sonst None.
fn first_target(content: &str) -> Option<u64> {
    content.split_whitespace().skip(1).find_map(parse_target)
}

/// Tausender-Trennung mit `sep` (Punkt = deutsch, Komma = Python-Default).
fn group(value: i64, sep: char) -> String {
    let digits = value.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(*b as char);
    }
    if value < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// `_format_leaderboard_number`: deutscher Punkt-Trenner.
fn format_de(value: i64) -> String {
    group(value, '.')
}

/// Python-Default `f"{n:,}"`: Komma-Trenner.
fn format_comma(value: i64) -> String {
    group(value, ',')
}

/// Erste `n` Zeichen (entspricht Pythons `s[:n]` für ASCII-Timestamps).
fn left(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `s[:n] if s else 'Unbekannt'`.
fn left_or_unknown(value: &Option<String>, n: usize) -> String {
    match value {
        Some(s) if !s.is_empty() => left(s, n),
        _ => "Unbekannt".to_string(),
    }
}

fn day_name(d: i64) -> Option<&'static str> {
    usize::try_from(d)
        .ok()
        .and_then(|i| DAY_NAMES.get(i).copied())
}

/// Python-`repr` einer Integer-Liste: `[20, 21, 19]`.
fn py_list_ints(v: &[i64]) -> String {
    let inner = v
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Python-`repr` der Wochentag-Namensliste: `['Mo', 'Di']` (einfache Quotes).
fn py_list_day_names(v: &[i64]) -> String {
    let inner = v
        .iter()
        .filter_map(|d| day_name(*d))
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// JSON-Array (`"[20, 21]"`) → `Vec<i64>` (leer bei NULL/Parsefehler).
fn parse_json_ints(raw: &Option<String>) -> Vec<i64> {
    raw.as_deref()
        .and_then(|s| serde_json::from_str::<Vec<i64>>(s).ok())
        .unwrap_or_default()
}

fn parse_dt(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
}

/// `(days_ago, hours_ago)` seit `last_active` (Format `%Y-%m-%d %H:%M:%S`).
fn time_ago(s: &str) -> Option<(i64, i64)> {
    let parsed = parse_dt(s)?;
    let now = chrono::Utc::now().naive_utc();
    let secs = now.signed_duration_since(parsed).num_seconds().max(0);
    Some((secs / 86400, (secs % 86400) / 3600))
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Datenzugriff ─────────────────────────────────────────────────────────────

struct MemberEventRow {
    event_type: String,
    timestamp: Option<String>,
    account_created_at: Option<String>,
    join_position: Option<i64>,
}

struct MsgRow {
    count: i64,
    last: Option<String>,
    first: Option<String>,
    channel_id: Option<i64>,
}

struct PatternFull {
    hours: Option<String>,
    days: Option<String>,
    score: i64,
    total_minutes: i64,
    last_active: Option<String>,
    ping_count: i64,
    last_pinged: Option<String>,
}

struct PingPatternRow {
    hours: Option<String>,
    days: Option<String>,
    score: i64,
    last_pinged: Option<String>,
    ping_count: i64,
}

/// Handgeschriebene Reads gegen die geteilten SQLite-Tabellen — es gibt keine
/// importierbare API dafür (die Leaderboard-SQL liegt nur im dl-stats-Handler).
struct ActivityStatsStore {
    pool: PgPool,
}

impl ActivityStatsStore {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn member_history(&self, user_id: u64, guild_id: u64) -> Vec<MemberEventRow> {
        let (Ok(user_id), Ok(guild_id)) = (
            discord_id_to_i64(user_id, "user_id"),
            discord_id_to_i64(guild_id, "guild_id"),
        ) else {
            return Vec::new();
        };
        sqlx::query!(
            r#"
            SELECT event_type AS "event_type!",
                   to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "timestamp?",
                   to_char(account_created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "account_created_at?",
                   join_position
            FROM activity.member_events
            WHERE user_id = $1
              AND guild_id = $2
            ORDER BY occurred_at ASC NULLS FIRST, id ASC
            "#,
            user_id,
            guild_id,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| MemberEventRow {
                    event_type: row.event_type,
                    timestamp: row.timestamp,
                    account_created_at: row.account_created_at,
                    join_position: row.join_position.map(i64::from),
                })
                .collect()
        })
        .unwrap_or_default()
    }

    async fn voice_totals(&self, user_id: u64) -> (i64, i64) {
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return (0, 0);
        };
        sqlx::query!(
            r#"
            SELECT total_seconds AS "total_seconds!",
                   total_points AS "total_points!"
            FROM voice.voice_stats
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| (row.total_seconds, row.total_points))
        .unwrap_or((0, 0))
    }

    /// Neueste Session: `(channel_name, duration_seconds)`.
    async fn last_voice_session(&self, user_id: u64, guild_id: u64) -> Option<(String, i64)> {
        let (Ok(user_id), Ok(guild_id)) = (
            discord_id_to_i64(user_id, "user_id"),
            discord_id_to_i64(guild_id, "guild_id"),
        ) else {
            return None;
        };
        sqlx::query!(
            r#"
            SELECT COALESCE(channel_name, '') AS "channel_name!",
                   duration_seconds AS "duration_seconds!"
            FROM activity.voice_session_log
            WHERE user_id = $1
              AND guild_id = $2
            ORDER BY ended_at DESC
            LIMIT 1
            "#,
            user_id,
            guild_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| (row.channel_name, row.duration_seconds))
    }

    async fn display_name_from_db(&self, user_id: u64) -> Option<String> {
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return None;
        };
        sqlx::query!(
            r#"
            SELECT name AS "name!"
            FROM (
                SELECT user_display_name AS name, last_played_together AS ts
                FROM activity.user_co_players
                WHERE user_id = $1
                  AND user_display_name IS NOT NULL
                UNION ALL
                SELECT co_player_display_name AS name, last_played_together AS ts
                FROM activity.user_co_players
                WHERE co_player_id = $1
                  AND co_player_display_name IS NOT NULL
                UNION ALL
                SELECT display_name AS name, ended_at AS ts
                FROM activity.voice_session_log
                WHERE user_id = $1
                  AND display_name IS NOT NULL
            ) names
            WHERE name IS NOT NULL
              AND name != ''
            ORDER BY ts DESC NULLS LAST
            LIMIT 1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| row.name)
    }

    async fn message_activity(&self, user_id: u64, guild_id: u64) -> Option<MsgRow> {
        let (Ok(user_id), Ok(guild_id)) = (
            discord_id_to_i64(user_id, "user_id"),
            discord_id_to_i64(guild_id, "guild_id"),
        ) else {
            return None;
        };
        sqlx::query!(
            r#"
            SELECT COALESCE(message_count, 0)::BIGINT AS "count!",
                   to_char(last_message_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last?",
                   to_char(first_message_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "first?",
                   channel_id
            FROM activity.message_activity
            WHERE user_id = $1
              AND guild_id = $2
            "#,
            user_id,
            guild_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| MsgRow {
            count: row.count,
            last: row.last,
            first: row.first,
            channel_id: row.channel_id,
        })
    }

    async fn co_players(&self, user_id: u64, limit: i64) -> Vec<(u64, i64)> {
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return Vec::new();
        };
        sqlx::query!(
            r#"
            SELECT co_player_id,
                   COALESCE(sessions_together, 0)::INT AS "sessions_together!"
            FROM activity.user_co_players
            WHERE user_id = $1
            ORDER BY COALESCE(sessions_together, 0) DESC,
                     COALESCE(total_minutes_together, 0) DESC
            LIMIT $2
            "#,
            user_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .filter_map(|row| {
                    i64_to_u64(row.co_player_id, "co_player_id")
                        .map(|id| (id, i64::from(row.sessions_together)))
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// `(typical_hours_json, typical_days_json, activity_score_2w)`.
    async fn pattern_basic(&self, user_id: u64) -> Option<(Option<String>, Option<String>, i64)> {
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return None;
        };
        sqlx::query!(
            r#"
            SELECT typical_hours::text AS "hours?",
                   typical_days::text AS "days?",
                   COALESCE(activity_score_2w, 0)::INT AS "score!"
            FROM activity.user_activity_patterns
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| (row.hours, row.days, i64::from(row.score)))
    }

    async fn pattern_full(&self, user_id: u64) -> Option<PatternFull> {
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return None;
        };
        sqlx::query!(
            r#"
            SELECT typical_hours::text AS "hours?",
                   typical_days::text AS "days?",
                   COALESCE(activity_score_2w, 0)::INT AS "score!",
                   COALESCE(total_minutes_2w, 0)::INT AS "total_minutes!",
                   to_char(last_active_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_active?",
                   COALESCE(ping_count_30d, 0)::INT AS "ping_count!",
                   to_char(last_pinged_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_pinged?"
            FROM activity.user_activity_patterns
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| PatternFull {
            hours: row.hours,
            days: row.days,
            score: i64::from(row.score),
            total_minutes: i64::from(row.total_minutes),
            last_active: row.last_active,
            ping_count: i64::from(row.ping_count),
            last_pinged: row.last_pinged,
        })
    }

    /// Top-`limit` Text: `(user_id, total_messages, total_points)`.
    async fn text_top(&self, limit: i64) -> Vec<(u64, i64, i64)> {
        sqlx::query!(
            r#"
            SELECT user_id,
                   total_messages AS "total_messages!",
                   total_points AS "total_points!"
            FROM activity.text_stats
            ORDER BY total_points DESC, total_messages DESC
            LIMIT $1
            "#,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .filter_map(|row| {
                    i64_to_u64(row.user_id, "user_id")
                        .map(|id| (id, row.total_messages, row.total_points))
                })
                .collect()
        })
        .unwrap_or_default()
    }

    async fn text_rank(&self, user_id: u64) -> Option<(i64, i64)> {
        let user_id = discord_id_to_i64(user_id, "user_id").ok()?;
        let row = sqlx::query!(
            r#"
            SELECT total_points AS "points!",
                   total_messages AS "messages!"
            FROM activity.text_stats
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()?;

        let rank = sqlx::query!(
            r#"
            SELECT COUNT(*) + 1 AS "rank!"
            FROM activity.text_stats
            WHERE total_points > $1
               OR (total_points = $1 AND total_messages > $2)
            "#,
            row.points,
            row.messages,
        )
        .fetch_one(&self.pool)
        .await
        .ok()?
        .rank;
        Some((rank, row.points))
    }

    /// Letzte Events eines Users (neueste zuerst): `(event_type, timestamp)`.
    /// `display_name` wird mitselektiert (Query-Form wie im Original), aber im
    /// Output nicht genutzt. `limit` geht roh in `LIMIT` (SQLite: 0 = keine
    /// Zeilen, negativ = kein Limit) — der Aufrufer klemmt vorher auf 50.
    async fn member_events_recent(
        &self,
        user_id: u64,
        guild_id: u64,
        limit: i64,
    ) -> Vec<(String, Option<String>)> {
        let (Ok(user_id), Ok(guild_id)) = (
            discord_id_to_i64(user_id, "user_id"),
            discord_id_to_i64(guild_id, "guild_id"),
        ) else {
            return Vec::new();
        };
        sqlx::query!(
            r#"
            SELECT event_type AS "event_type!",
                   to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "timestamp?",
                   display_name
            FROM activity.member_events
            WHERE user_id = $1
              AND guild_id = $2
            ORDER BY occurred_at DESC NULLS LAST, id DESC
            LIMIT CASE WHEN $3::BIGINT < 0 THEN NULL ELSE $3::BIGINT END
            "#,
            user_id,
            guild_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| {
                    let _ = row.display_name;
                    (row.event_type, row.timestamp)
                })
                .collect()
        })
        .unwrap_or_default()
    }

    async fn ping_pattern(&self, user_id: u64) -> Option<PingPatternRow> {
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return None;
        };
        sqlx::query!(
            r#"
            SELECT typical_hours::text AS "hours?",
                   typical_days::text AS "days?",
                   COALESCE(activity_score_2w, 0)::INT AS "score!",
                   to_char(last_pinged_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_pinged?",
                   COALESCE(ping_count_30d, 0)::INT AS "ping_count!"
            FROM activity.user_activity_patterns
            WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| PingPatternRow {
            hours: row.hours,
            days: row.days,
            score: i64::from(row.score),
            last_pinged: row.last_pinged,
            ping_count: i64::from(row.ping_count),
        })
    }

    async fn record_ping(&self, user_id: u64) -> ActivityDbResult<()> {
        let user_id = discord_id_to_i64(user_id, "user_id")?;
        sqlx::query!(
            r#"
            UPDATE activity.user_activity_patterns
            SET last_pinged_at = now(),
                ping_count_30d = COALESCE(ping_count_30d, 0) + 1
            WHERE user_id = $1
            "#,
            user_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn server_event_counts(&self, guild_id: u64) -> Vec<(String, i64)> {
        let Ok(guild_id) = discord_id_to_i64(guild_id, "guild_id") else {
            return Vec::new();
        };
        sqlx::query!(
            r#"
            SELECT event_type AS "event_type!",
                   COUNT(*) AS "count!"
            FROM activity.member_events
            WHERE guild_id = $1
            GROUP BY event_type
            ORDER BY COUNT(*) DESC
            "#,
            guild_id,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.event_type, row.count))
                .collect()
        })
        .unwrap_or_default()
    }

    async fn server_total_messages(&self, guild_id: u64) -> i64 {
        let Ok(guild_id) = discord_id_to_i64(guild_id, "guild_id") else {
            return 0;
        };
        sqlx::query!(
            r#"
            SELECT COALESCE(SUM(message_count), 0)::BIGINT AS "total!"
            FROM activity.message_activity
            WHERE guild_id = $1
            "#,
            guild_id,
        )
        .fetch_one(&self.pool)
        .await
        .map(|row| row.total)
        .unwrap_or(0)
    }

    async fn server_total_voice(&self, guild_id: u64) -> i64 {
        let Ok(guild_id) = discord_id_to_i64(guild_id, "guild_id") else {
            return 0;
        };
        sqlx::query!(
            r#"
            SELECT COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total!"
            FROM activity.voice_session_log
            WHERE guild_id = $1
            "#,
            guild_id,
        )
        .fetch_one(&self.pool)
        .await
        .map(|row| row.total)
        .unwrap_or(0)
    }

    async fn server_top_users(&self, guild_id: u64, limit: i64) -> Vec<(u64, i64)> {
        let Ok(guild_id) = discord_id_to_i64(guild_id, "guild_id") else {
            return Vec::new();
        };
        sqlx::query!(
            r#"
            SELECT user_id,
                   COALESCE(message_count, 0)::BIGINT AS "message_count!"
            FROM activity.message_activity
            WHERE guild_id = $1
            ORDER BY COALESCE(message_count, 0) DESC
            LIMIT $2
            "#,
            guild_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .filter_map(|row| {
                    i64_to_u64(row.user_id, "user_id").map(|id| (id, row.message_count))
                })
                .collect()
        })
        .unwrap_or_default()
    }
}

// ── Befehlsschicht ───────────────────────────────────────────────────────────

/// Antwort: Klartext oder genau ein Embed.
pub struct StatsReply {
    pub content: Option<String>,
    pub embeds: Vec<Value>,
    post_send: Option<PostSendAction>,
}

#[derive(Debug, Clone, Copy)]
enum PostSendAction {
    RecordPing { user_id: u64 },
}

impl StatsReply {
    fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            embeds: vec![],
            post_send: None,
        }
    }
    fn embed(embed: Value) -> Self {
        Self {
            content: None,
            embeds: vec![embed],
            post_send: None,
        }
    }
    fn with_post_send(mut self, action: PostSendAction) -> Self {
        self.post_send = Some(action);
        self
    }
}

pub struct ActivityStatsCommands {
    store: ActivityStatsStore,
    port: Arc<dyn NamePort>,
}

impl ActivityStatsCommands {
    pub fn new(pool: PgPool, port: Arc<dyn NamePort>) -> Arc<Self> {
        Arc::new(Self {
            store: ActivityStatsStore::new(pool),
            port,
        })
    }

    pub async fn reply_for(
        &self,
        content: &str,
        guild_id: u64,
        author_id: u64,
        is_admin: bool,
        can_manage_messages: bool,
        can_manage_guild: bool,
    ) -> Option<StatsReply> {
        let root = content.split_whitespace().next()?.to_lowercase();
        match root.as_str() {
            "!useranalysis" | "!ua" | "!analyze" => {
                Some(self.useranalysis(content, guild_id, author_id).await)
            }
            "!myactivity" => Some(self.myactivity(content, guild_id, author_id).await),
            "!tleaderboard" | "!tlb" | "!texttop" => {
                Some(self.tleaderboard(guild_id, author_id).await)
            }
            "!messagestats" | "!msgstats" => {
                Some(self.messagestats(content, guild_id, author_id).await)
            }
            "!memberevents" | "!mevents" => {
                Some(self.memberevents(content, guild_id, author_id).await)
            }
            "!checkping" => Some(self.checkping(content, author_id).await),
            "!smartping" if can_manage_messages => Some(self.smartping(content).await),
            // !serverstats braucht im Original Manage-Server.
            // Nicht-Admins werden still ignoriert (kein Reply, wie bei rank/nudge).
            "!serverstats" if is_admin || can_manage_guild => {
                Some(self.serverstats(guild_id).await)
            }
            _ => None,
        }
    }

    async fn resolve_one(&self, user_id: u64) -> String {
        if let Some(name) = self.port.resolve_names(&[user_id]).await.remove(&user_id) {
            return name;
        }
        self.store
            .display_name_from_db(user_id)
            .await
            .unwrap_or_else(|| format!("User {user_id}"))
    }

    async fn resolve_map(&self, ids: &[u64]) -> HashMap<u64, String> {
        let mut names = self.port.resolve_names(ids).await;
        for user_id in ids {
            if names.contains_key(user_id) {
                continue;
            }
            if let Some(name) = self.store.display_name_from_db(*user_id).await {
                names.insert(*user_id, name);
            }
        }
        names
    }

    async fn useranalysis(&self, content: &str, guild_id: u64, author_id: u64) -> StatsReply {
        let target = first_target(content).unwrap_or(author_id);
        let name = self.resolve_one(target).await;
        let mut fields: Vec<Value> = Vec::new();

        // 📅 Server-History (nur bei vorhandenen Events).
        let events = self.store.member_history(target, guild_id).await;
        if let Some(first_join) = events.first() {
            let joins = events.iter().filter(|e| e.event_type == "join").count();
            let leaves = events.iter().filter(|e| e.event_type == "leave").count();
            let bans = events.iter().filter(|e| e.event_type == "ban").count();
            let mut value = format!(
                "**Erstes Join:** {}\n**Joins:** {joins} | **Leaves:** {leaves}",
                left_or_unknown(&first_join.timestamp, 16)
            );
            if bans > 0 {
                value.push_str(&format!(" | **Bans:** {bans}"));
            }
            if let Some(acc) = first_join
                .account_created_at
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                value.push_str(&format!("\n**Account erstellt:** {}", left(acc, 10)));
            }
            if let Some(pos) = first_join.join_position.filter(|p| *p != 0) {
                value.push_str(&format!("\n**Join-Position:** #{pos}"));
            }
            fields.push(json!({ "name": "📅 Server-History", "value": value, "inline": false }));
        }

        // 🎙️ Voice-Aktivität (immer ein Feld).
        let (vsecs, vpoints) = self.store.voice_totals(target).await;
        let voice_value = if vsecs != 0 {
            let mut v = format!(
                "**Gesamtzeit:** {}h {}m\n**Punkte:** {vpoints}",
                vsecs / 3600,
                (vsecs % 3600) / 60
            );
            if let Some((channel_name, dur)) = self.store.last_voice_session(target, guild_id).await
            {
                v.push_str(&format!(
                    "\n**Letzter Voice:** {channel_name} ({}min)",
                    dur / 60
                ));
            }
            v
        } else {
            "Keine Voice-Aktivität".to_string()
        };
        fields.push(json!({ "name": "🎙️ Voice-Aktivität", "value": voice_value, "inline": true }));

        // 💬 Nachrichten (immer ein Feld).
        let msg = self.store.message_activity(target, guild_id).await;
        let msg_value = match msg.as_ref().filter(|m| m.count != 0) {
            Some(m) => format!(
                "**Nachrichten:** {}\n**Erste Nachricht:** {}\n**Letzte Nachricht:** {}",
                m.count,
                left_or_unknown(&m.first, 16),
                left_or_unknown(&m.last, 16)
            ),
            None => "Keine Nachrichten".to_string(),
        };
        fields.push(json!({ "name": "💬 Nachrichten", "value": msg_value, "inline": true }));

        // 👥 Top Mitspieler (nur bei vorhandenen).
        let co = self.store.co_players(target, 3).await;
        if !co.is_empty() {
            let names = self
                .resolve_map(&co.iter().map(|(id, _)| *id).collect::<Vec<_>>())
                .await;
            let mut v = String::new();
            for (cid, sessions) in &co {
                let cname = names
                    .get(cid)
                    .cloned()
                    .unwrap_or_else(|| format!("User {cid}"));
                v.push_str(&format!("**{cname}** ({sessions}x)\n"));
            }
            fields.push(json!({ "name": "👥 Top Mitspieler", "value": v, "inline": false }));
        }

        // 📈 Aktivitätsmuster (nur bei vorhandenem, truthy Score).
        if let Some((hours_json, days_json, score)) = self.store.pattern_basic(target).await {
            if score != 0 {
                let mut v = format!("**Activity Score:** {score}\n");
                let hours = parse_json_ints(&hours_json);
                if !hours.is_empty() {
                    let hs = hours
                        .iter()
                        .take(3)
                        .map(|h| format!("{h}:00"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    v.push_str(&format!("**Typische Zeiten:** {hs}\n"));
                }
                let days = parse_json_ints(&days_json);
                if !days.is_empty() {
                    let ds = days
                        .iter()
                        .take(3)
                        .filter_map(|d| day_name(*d))
                        .collect::<Vec<_>>()
                        .join(", ");
                    v.push_str(&format!("**Typische Tage:** {ds}"));
                }
                fields.push(json!({ "name": "📈 Aktivitätsmuster", "value": v, "inline": false }));
            }
        }

        StatsReply::embed(json!({
            "title": format!("📊 User-Aktivitäts-Analyse - {name}"),
            "color": 0xE67E22,
            "footer": { "text": format!("User ID: {target}") },
            "timestamp": now_iso(),
            "fields": fields
        }))
    }

    async fn myactivity(&self, content: &str, guild_id: u64, author_id: u64) -> StatsReply {
        let _ = guild_id;
        let target = first_target(content).unwrap_or(author_id);
        let name = self.resolve_one(target).await;
        let Some(p) = self.store.pattern_full(target).await else {
            return StatsReply::text(format!("❌ Keine Aktivitätsdaten für {name} vorhanden."));
        };
        let mut fields: Vec<Value> = Vec::new();

        let hours = parse_json_ints(&p.hours);
        if !hours.is_empty() {
            let hs = hours
                .iter()
                .map(|h| format!("{h}:00"))
                .collect::<Vec<_>>()
                .join(", ");
            fields
                .push(json!({ "name": "🕐 Typische Online-Zeiten", "value": hs, "inline": false }));
        }
        let days = parse_json_ints(&p.days);
        if !days.is_empty() {
            let ds = days
                .iter()
                .filter_map(|d| day_name(*d))
                .collect::<Vec<_>>()
                .join(", ");
            fields.push(json!({ "name": "📅 Typische Wochentage", "value": ds, "inline": false }));
        }
        fields.push(
            json!({ "name": "⭐ Activity Score (2W)", "value": format!("{} Sessions", p.score), "inline": true }),
        );
        fields.push(json!({
            "name": "⏱️ Gesamtzeit (2W)",
            "value": format!("{}h {}m", p.total_minutes / 60, p.total_minutes % 60),
            "inline": true
        }));
        if let Some(last_active) = p.last_active.as_deref().filter(|s| !s.is_empty()) {
            if let Some((days_ago, hours_ago)) = time_ago(last_active) {
                fields.push(json!({
                    "name": "🔴 Zuletzt aktiv",
                    "value": format!("vor {days_ago}d {hours_ago}h"),
                    "inline": true
                }));
            }
        }
        fields.push(
            json!({ "name": "📬 Pings (30d)", "value": format!("{}/3", p.ping_count), "inline": true }),
        );
        if let Some(lp) = p.last_pinged.as_deref().filter(|s| !s.is_empty()) {
            fields.push(json!({ "name": "📬 Zuletzt gepingt", "value": lp, "inline": true }));
        }

        let co = self.store.co_players(target, 5).await;
        if !co.is_empty() {
            let names = self
                .resolve_map(&co.iter().map(|(id, _)| *id).collect::<Vec<_>>())
                .await;
            let lines: Vec<String> = co
                .iter()
                .take(3)
                .map(|(cid, sessions)| {
                    let cname = names
                        .get(cid)
                        .cloned()
                        .unwrap_or_else(|| format!("User {cid}"));
                    format!("**{cname}** ({sessions}x zusammen)")
                })
                .collect();
            fields.push(
                json!({ "name": "👥 Top Mitspieler", "value": lines.join("\n"), "inline": false }),
            );
        }

        StatsReply::embed(json!({
            "title": format!("📊 Aktivitätsmuster - {name}"),
            "color": 0x3498DB,
            "footer": { "text": "Daten der letzten 2 Wochen" },
            "fields": fields
        }))
    }

    async fn tleaderboard(&self, guild_id: u64, author_id: u64) -> StatsReply {
        let rows = self.store.text_top(10).await;
        let guild_name = self.port.guild_name(guild_id).await.unwrap_or_default();
        let description = if rows.is_empty() {
            "📊 Noch keine Text-Aktivität aufgezeichnet.".to_string()
        } else {
            let names = self
                .resolve_map(&rows.iter().map(|(id, _, _)| *id).collect::<Vec<_>>())
                .await;
            let lines: Vec<String> = rows
                .iter()
                .enumerate()
                .map(|(idx, (uid, msgs, points))| {
                    let medal = match idx + 1 {
                        1 => "🥇".to_string(),
                        2 => "🥈".to_string(),
                        3 => "🥉".to_string(),
                        n => format!("{n}."),
                    };
                    // tleaderboard-Fallback ist "Unbekannt" (nicht "User {id}").
                    let name = names
                        .get(uid)
                        .cloned()
                        .unwrap_or_else(|| "Unbekannt".to_string());
                    format!(
                        "{medal} **{name}** — {} Msgs · {} Punkte",
                        format_de(*msgs),
                        format_de(*points)
                    )
                })
                .collect();
            lines.join("\n")
        };
        let footer = match self.store.text_rank(author_id).await {
            None => "Noch keine Punkte".to_string(),
            Some((rank, points)) => {
                format!("Du bist auf Platz {rank} · {} Punkte", format_de(points))
            }
        };
        StatsReply::embed(json!({
            "title": format!("🏆 Text-Leaderboard - {guild_name}"),
            "color": 0xF1C40F,
            "description": description,
            "footer": { "text": footer }
        }))
    }

    async fn messagestats(&self, content: &str, guild_id: u64, author_id: u64) -> StatsReply {
        let target = first_target(content).unwrap_or(author_id);
        let name = self.resolve_one(target).await;
        let Some(m) = self
            .store
            .message_activity(target, guild_id)
            .await
            .filter(|m| m.count != 0)
        else {
            return StatsReply::text(format!("❌ Keine Message-Aktivität für {name} gefunden."));
        };
        let mut fields = vec![
            json!({ "name": "📊 Nachrichten", "value": format_comma(m.count), "inline": true }),
            json!({ "name": "📅 Erste Nachricht", "value": left_or_unknown(&m.first, 16), "inline": true }),
            json!({ "name": "🕐 Letzte Nachricht", "value": left_or_unknown(&m.last, 16), "inline": true }),
        ];
        if let Some(cid) = m.channel_id.filter(|c| *c != 0) {
            fields.push(
                json!({ "name": "📍 Letzter Channel", "value": format!("<#{cid}>"), "inline": true }),
            );
        }
        if let (Some(first), Some(last)) = (m.first.as_deref(), m.last.as_deref()) {
            if let (Some(fd), Some(ld)) = (parse_dt(first), parse_dt(last)) {
                let days = ld.signed_duration_since(fd).num_days().max(1);
                let avg = m.count as f64 / days as f64;
                fields.push(
                    json!({ "name": "📈 Durchschnitt/Tag", "value": format!("{avg:.1}"), "inline": true }),
                );
            }
        }
        StatsReply::embed(json!({
            "title": format!("💬 Message-Statistiken - {name}"),
            "color": 0x2ECC71,
            "fields": fields
        }))
    }

    async fn memberevents(&self, content: &str, guild_id: u64, author_id: u64) -> StatsReply {
        // Args: optionale Mention (Ziel) + optionaler Integer (limit, Default 10).
        // Eine reine Zahl ist immer das Limit (nicht als User-ID gedeutet),
        // damit `!memberevents 25` nicht fälschlich als Ziel-ID zählt.
        let mut target: Option<u64> = None;
        let mut limit: i64 = 10;
        for tok in content.split_whitespace().skip(1) {
            if tok.starts_with("<@") {
                if let Some(uid) = parse_target(tok) {
                    target = Some(uid);
                }
            } else if let Ok(n) = tok.parse::<i64>() {
                limit = n;
            }
        }
        let target = target.unwrap_or(author_id);
        let name = self.resolve_one(target).await;

        // Query klemmt auf 50; der Footer zeigt das ROHE limit (Original-Diskrepanz).
        let events = self
            .store
            .member_events_recent(target, guild_id, limit.min(50))
            .await;
        if events.is_empty() {
            return StatsReply::text(format!("❌ Keine Events für {name} gefunden."));
        }
        let mut description = String::new();
        for (event_type, timestamp) in &events {
            let icon = match event_type.as_str() {
                "join" => "➕",
                "leave" => "➖",
                "ban" => "🔨",
                "unban" => "✅",
                _ => "•",
            };
            description.push_str(&format!(
                "{icon} **{}** - {}\n",
                event_type.to_uppercase(),
                left_or_unknown(timestamp, 16)
            ));
        }
        StatsReply::embed(json!({
            "title": format!("📋 Member-Events - {name}"),
            "color": 0x3498DB,
            "description": description,
            "footer": { "text": format!("Zeige {} von max {} Events", events.len(), limit) }
        }))
    }

    /// Ping-Eligibility-Check (Port von `should_ping_user` +
    /// `check_ping_command`).
    async fn checkping(&self, content: &str, author_id: u64) -> StatsReply {
        let target = first_target(content).unwrap_or(author_id);
        let name = self.resolve_one(target).await;
        let (can_ping, reason) = self.ping_eligibility(target).await;
        StatsReply::embed(json!({
            "title": format!("🔔 Ping-Check - {name}"),
            "color": if can_ping { 0x2ECC71 } else { 0xE74C3C },
            "fields": [
                { "name": "Status", "value": if can_ping { "✅ Kann gepingt werden" } else { "❌ Kann nicht gepingt werden" }, "inline": false },
                { "name": "Grund", "value": reason, "inline": false }
            ]
        }))
    }

    async fn smartping(&self, content: &str) -> StatsReply {
        let Some(target) = first_target(content) else {
            return StatsReply::text(SMARTPING_USAGE_PLACEHOLDER);
        };
        let (can_ping, _reason) = self.ping_eligibility(target).await;
        if !can_ping {
            return StatsReply::text(SMARTPING_BLOCKED_PLACEHOLDER);
        }
        StatsReply::text(format!("<@{target}> {SMARTPING_PING_MESSAGE_PLACEHOLDER}"))
            .with_post_send(PostSendAction::RecordPing { user_id: target })
    }

    async fn run_post_send(&self, reply: &StatsReply) {
        match reply.post_send {
            Some(PostSendAction::RecordPing { user_id }) => {
                if let Err(err) = self.store.record_ping(user_id).await {
                    tracing::warn!(%err, user_id, "Smartping-Zaehler konnte nicht aktualisiert werden");
                }
            }
            None => {}
        }
    }

    /// `(can_ping, reason)` — byte-genauer Port der Prüfreihenfolge aus
    /// `should_ping_user` (max 3/30d, ≥24h seit letztem Ping, ≥5 Sessions/2W,
    /// ±2h-Zeitfenster mit Wrap, optionaler Wochentag).
    async fn ping_eligibility(&self, user_id: u64) -> (bool, String) {
        let Some(p) = self.store.ping_pattern(user_id).await else {
            return (false, "Keine Aktivitätsdaten vorhanden".to_string());
        };
        let max_pings_30d = 3;
        if p.ping_count >= max_pings_30d {
            return (
                false,
                format!(
                    "Rate-Limit erreicht ({}/{} in 30d)",
                    p.ping_count, max_pings_30d
                ),
            );
        }
        if let Some(last) = p.last_pinged.as_deref().filter(|s| !s.is_empty()) {
            if let Some(parsed) = parse_dt(last) {
                let since = chrono::Utc::now()
                    .naive_utc()
                    .signed_duration_since(parsed)
                    .num_seconds();
                if since < 86400 {
                    let hours_remaining = (86400 - since) as f64 / 3600.0;
                    return (
                        false,
                        format!("Zu früh (noch {hours_remaining:.1}h bis nächster Ping)"),
                    );
                }
            }
        }
        if p.score < 5 {
            return (
                false,
                format!("User zu inaktiv (nur {} Sessions in 2W)", p.score),
            );
        }
        let now = chrono::Utc::now().naive_utc();
        let current_hour = now.hour() as i64;
        let current_day = now.weekday().num_days_from_monday() as i64;
        let typical_hours = parse_json_ints(&p.hours);
        let typical_days = parse_json_ints(&p.days);
        let hour_match = typical_hours.iter().any(|h| {
            let diff = (current_hour - h).abs();
            diff <= 2 || diff >= 22
        });
        if !hour_match {
            return (
                false,
                format!(
                    "Außerhalb typischer Online-Zeiten (typisch: {}h)",
                    py_list_ints(&typical_hours)
                ),
            );
        }
        if !typical_days.is_empty() && !typical_days.contains(&current_day) {
            return (
                false,
                format!(
                    "Unpassender Wochentag (typisch: {})",
                    py_list_day_names(&typical_days)
                ),
            );
        }
        (true, "OK - User kann gepingt werden".to_string())
    }

    async fn serverstats(&self, guild_id: u64) -> StatsReply {
        let mut fields: Vec<Value> = Vec::new();

        let events = self.store.server_event_counts(guild_id).await;
        if !events.is_empty() {
            let mut v = String::new();
            for (etype, count) in &events {
                v.push_str(&format!("**{}:** {count}\n", etype.to_uppercase()));
            }
            fields.push(json!({ "name": "📋 Member Events", "value": v, "inline": true }));
        }

        let total_messages = self.store.server_total_messages(guild_id).await;
        if total_messages != 0 {
            fields.push(json!({
                "name": "💬 Nachrichten (gesamt)",
                "value": format_comma(total_messages),
                "inline": true
            }));
        }

        let total_voice = self.store.server_total_voice(guild_id).await;
        if total_voice != 0 {
            fields.push(json!({
                "name": "🎙️ Voice-Zeit (gesamt)",
                "value": format!("{}h", format_comma(total_voice / 3600)),
                "inline": true
            }));
        }

        let top = self.store.server_top_users(guild_id, 5).await;
        if !top.is_empty() {
            let names = self
                .resolve_map(&top.iter().map(|(id, _)| *id).collect::<Vec<_>>())
                .await;
            let mut v = String::new();
            for (i, (uid, count)) in top.iter().enumerate() {
                let uname = names
                    .get(uid)
                    .cloned()
                    .unwrap_or_else(|| format!("User {uid}"));
                v.push_str(&format!(
                    "{}. **{uname}** - {} Messages\n",
                    i + 1,
                    format_comma(*count)
                ));
            }
            fields.push(json!({ "name": "🏆 Top 5 Aktivste User", "value": v, "inline": false }));
        }

        StatsReply::embed(json!({
            "title": format!("📊 Server-Statistiken - {}", self.port.guild_name(guild_id).await.unwrap_or_default()),
            "color": 0xF1C40F,
            "footer": { "text": format!("Server ID: {guild_id}") },
            "timestamp": now_iso(),
            "fields": fields
        }))
    }
}

/// `MessageEvent`-Subscriber für die Aktivitäts-/Text-Stats-Befehle.
/// Kein generelles Admin-Gate — nur `!serverstats` prüft `author_is_admin`.
pub fn spawn_command(
    commands: Arc<ActivityStatsCommands>,
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
                            event.author_is_admin,
                            event.author_can_manage_messages,
                            event.author_can_manage_guild,
                        )
                        .await
                    else {
                        continue;
                    };
                    let sent = sender
                        .send_to_channel(event.channel_id, reply.content.as_deref(), &reply.embeds)
                        .await;
                    if sent.is_ok() {
                        commands.run_post_send(&reply).await;
                    }
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
    #[cfg(feature = "testing")]
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[cfg(feature = "testing")]
    use std::sync::Mutex as StdMutex;

    #[test]
    fn format_trenner() {
        assert_eq!(format_de(1234567), "1.234.567");
        assert_eq!(format_comma(1234567), "1,234,567");
        assert_eq!(format_de(0), "0");
    }

    #[test]
    fn left_or_unknown_faellt_zurueck() {
        assert_eq!(
            left_or_unknown(&Some("2026-06-14 20:14:57".into()), 16),
            "2026-06-14 20:14"
        );
        assert_eq!(left_or_unknown(&None, 16), "Unbekannt");
        assert_eq!(left_or_unknown(&Some(String::new()), 16), "Unbekannt");
    }

    #[test]
    fn day_name_grenzen() {
        assert_eq!(day_name(0), Some("Mo"));
        assert_eq!(day_name(6), Some("So"));
        assert_eq!(day_name(7), None);
        assert_eq!(day_name(-1), None);
    }

    #[test]
    fn py_list_repr_byte_genau() {
        assert_eq!(py_list_ints(&[20, 21, 19]), "[20, 21, 19]");
        assert_eq!(py_list_ints(&[]), "[]");
        assert_eq!(py_list_day_names(&[0, 1]), "['Mo', 'Di']");
        assert_eq!(py_list_day_names(&[6]), "['So']");
        assert_eq!(py_list_day_names(&[]), "[]");
    }

    #[test]
    fn parse_json_ints_robust() {
        assert_eq!(
            parse_json_ints(&Some("[20, 21, 19]".into())),
            vec![20, 21, 19]
        );
        assert_eq!(parse_json_ints(&None), Vec::<i64>::new());
        assert_eq!(parse_json_ints(&Some("kaputt".into())), Vec::<i64>::new());
    }

    #[cfg(feature = "testing")]
    struct MockNames {
        names: StdMutex<HashMap<u64, String>>,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl NamePort for MockNames {
        async fn resolve_names(&self, user_ids: &[u64]) -> HashMap<u64, String> {
            let names = self.names.lock().expect("names");
            user_ids
                .iter()
                .filter_map(|id| names.get(id).map(|name| (*id, name.clone())))
                .collect()
        }

        async fn guild_name(&self, _guild_id: u64) -> Option<String> {
            Some("Guild".to_string())
        }
    }

    #[cfg(feature = "testing")]
    async fn stats_test_db() -> Result<(dl_central_db::TestDb, PgPool), Box<dyn std::error::Error>>
    {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool().clone();
        Ok((db, pool))
    }

    #[cfg(feature = "testing")]
    fn commands(pool: PgPool) -> Arc<ActivityStatsCommands> {
        ActivityStatsCommands::new(
            pool,
            Arc::new(MockNames {
                names: StdMutex::new(HashMap::new()),
            }),
        )
    }

    #[cfg(feature = "testing")]
    struct MockChannelSender {
        fail: bool,
        sends: AtomicUsize,
    }

    #[cfg(feature = "testing")]
    impl MockChannelSender {
        fn new(fail: bool) -> Self {
            Self {
                fail,
                sends: AtomicUsize::new(0),
            }
        }
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl ChannelSender for MockChannelSender {
        async fn send_to_channel(
            &self,
            _channel_id: u64,
            _content: Option<&str>,
            _embeds: &[Value],
        ) -> Result<u64, String> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err("send failed".to_string())
            } else {
                Ok(123)
            }
        }
    }

    #[cfg(feature = "testing")]
    fn message_event(content: &str) -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id: Some(1),
            channel_id: 555,
            message_id: 777,
            author_id: 99,
            author_display_name: "Admin".to_string(),
            author_is_admin: false,
            author_can_manage_messages: true,
            author_can_manage_guild: false,
            author_is_staff: true,
            author_staff_status_known: true,
            content: content.to_string(),
            message_created_at: 0,
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 0,
            author_joined_at: None,
        }
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn useranalysis_letzte_voice_session_ist_guild_scoped(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, pool) = stats_test_db().await?;
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_session_log(
                id, user_id, guild_id, channel_name, started_at, ended_at,
                duration_seconds, points
            )
            VALUES
                ($1, 42, 2, 'Falsche Guild', $2, $3, 3600, 0),
                ($4, 42, 1, 'Richtige Guild', $5, $6, 1800, 0)
            "#,
            1_i64,
            test_dt("2026-01-01 10:00:00"),
            test_dt("2026-01-01 11:00:00"),
            2_i64,
            test_dt("2026-01-01 09:00:00"),
            test_dt("2026-01-01 09:30:00"),
        )
        .execute(&pool)
        .await?;

        let store = ActivityStatsStore::new(pool);
        assert_eq!(
            store.last_voice_session(42, 1).await,
            Some(("Richtige Guild".to_string(), 1800))
        );
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn namensfallback_nutzt_db_wenn_cache_leer_ist() -> Result<(), Box<dyn std::error::Error>>
    {
        let (_db, pool) = stats_test_db().await?;
        sqlx::query!(
            r#"
            INSERT INTO activity.user_co_players(
                user_id, co_player_id, sessions_together, total_minutes_together,
                user_display_name, co_player_display_name, last_played_together
            )
            VALUES(42, 77, 1, 10, 'DB Name', 'Andere', $1)
            "#,
            test_dt("2026-01-01 10:00:00"),
        )
        .execute(&pool)
        .await?;

        assert_eq!(commands(pool).resolve_one(42).await, "DB Name");
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn serverstats_erlaubt_manage_guild_ohne_admin() -> Result<(), Box<dyn std::error::Error>>
    {
        let (_db, pool) = stats_test_db().await?;
        let reply = commands(pool)
            .reply_for("!serverstats", 1, 42, false, false, true)
            .await;
        assert!(reply.is_some());
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn smartping_baut_reply_ohne_vorzeitig_zu_zaehlen(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, pool) = stats_test_db().await?;
        let now = chrono::Utc::now().naive_utc();
        let hour = now.hour();
        let day = now.weekday().num_days_from_monday();
        seed_ping_pattern(&pool, hour, day).await?;

        let reply = commands(pool.clone())
            .reply_for("!smartping <@42>", 1, 99, false, true, false)
            .await
            .expect("reply");
        let expected = format!("<@42> {SMARTPING_PING_MESSAGE_PLACEHOLDER}");
        assert_eq!(reply.content.as_deref(), Some(expected.as_str()));
        let row = ping_row(&pool).await?;
        assert_eq!(row.0, 0);
        assert!(row.1.is_none());
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn smartping_sendfehler_verbraucht_rate_limit_nicht(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_db, pool) = stats_test_db().await?;
        let now = chrono::Utc::now().naive_utc();
        let hour = now.hour();
        let day = now.weekday().num_days_from_monday();
        seed_ping_pattern(&pool, hour, day).await?;

        let dispatcher = Dispatcher::new();
        let sender = Arc::new(MockChannelSender::new(true));
        let task = spawn_command(commands(pool.clone()), &dispatcher, sender.clone());
        dispatcher.publish_message(message_event("!smartping <@42>"));
        for _ in 0..20 {
            if sender.sends.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        task.abort();

        let (count, last) = ping_row(&pool).await?;
        assert_eq!(sender.sends.load(Ordering::SeqCst), 1);
        assert_eq!(count, 0);
        assert!(last.is_none());
        Ok(())
    }

    #[cfg(feature = "testing")]
    fn test_dt(raw: &str) -> chrono::DateTime<chrono::Utc> {
        NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
            .expect("test timestamp")
            .and_utc()
    }

    #[cfg(feature = "testing")]
    async fn seed_ping_pattern(
        pool: &PgPool,
        hour: u32,
        day: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let hours = format!("[{hour}]");
        let days = format!("[{day}]");
        sqlx::query!(
            r#"
            INSERT INTO activity.user_activity_patterns(
                user_id, typical_hours, typical_days, activity_score_2w, ping_count_30d
            )
            VALUES($1, $2::text::jsonb, $3::text::jsonb, 5, 0)
            "#,
            42_i64,
            hours,
            days,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    #[cfg(feature = "testing")]
    async fn ping_row(
        pool: &PgPool,
    ) -> Result<(i64, Option<chrono::DateTime<chrono::Utc>>), Box<dyn std::error::Error>> {
        let row = sqlx::query!(
            r#"
            SELECT COALESCE(ping_count_30d, 0)::INT AS "ping_count!",
                   last_pinged_at
            FROM activity.user_activity_patterns
            WHERE user_id = 42
            "#
        )
        .fetch_one(pool)
        .await?;
        Ok((i64::from(row.ping_count), row.last_pinged_at))
    }
}
