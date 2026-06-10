//! /api/public/me* — persönliche Statistiken (Session-Cookie nötig).

use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Duration;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::public::resolve_display_names;
use crate::timeutil::{isoformat, now_local, parse_iso, to_iso};
use crate::{
    internal_error, is_missing_table, parse_positive_int, require_session, safe_int,
    session_user_id, SharedApp,
};

type Params = Query<HashMap<String, String>>;

fn db_error(err: impl std::fmt::Display) -> Response {
    tracing::error!(%err, "PublicStats(me): DB-Fehler");
    internal_error()
}

fn avatar_url(user_id: &Value, avatar: Option<&Value>) -> Value {
    let uid = match user_id {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Value::Null,
    };
    let hash = avatar
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    match hash {
        Some(hash) => {
            let ext = if hash.starts_with("a_") { "gif" } else { "png" };
            json!(format!(
                "https://cdn.discordapp.com/avatars/{uid}/{hash}.{ext}?size=256"
            ))
        }
        None => Value::Null,
    }
}

/// `GET /api/public/me`.
pub async fn handle_me(State(app): State<SharedApp>, headers: HeaderMap) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let user_id = session.get("user_id").cloned().unwrap_or(Value::Null);
    let name = session
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Json(json!({
        "user_id": match &user_id { Value::String(s) => s.clone(), Value::Number(n) => n.to_string(), _ => String::new() },
        "name": name,
        "avatar_url": avatar_url(&user_id, session.get("avatar")),
    }))
    .into_response()
}

/// `_rank_for_points`: Platz = Anzahl mit mehr Punkten + 1; fehlende Tabelle → null.
fn rank_for_points(conn: &Connection, table: &str, user_id: i64) -> rusqlite::Result<Option<i64>> {
    let points: Option<SqlValue> = match conn.query_row(
        &format!("SELECT total_points FROM {table} WHERE user_id = ?1"),
        [user_id],
        |row| row.get(0),
    ) {
        Ok(v) => Some(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(err) if err.to_string().to_lowercase().contains("no such table") => None,
        Err(err) => return Err(err),
    };
    let Some(points) = points else {
        return Ok(None);
    };
    let better: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE total_points > ?1"),
        [safe_int(&points)],
        |row| row.get(0),
    )?;
    Ok(Some(better + 1))
}

/// `GET /api/public/me/stats`.
pub async fn handle_me_stats(State(app): State<SharedApp>, headers: HeaderMap) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let user_id = match session_user_id(&session) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let result = app
        .db
        .read(move |conn| {
            let voice: Option<(SqlValue, SqlValue)> = conn
                .query_row(
                    "SELECT total_seconds, total_points FROM voice_stats WHERE user_id = ?1",
                    [user_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let text: Option<(SqlValue, SqlValue)> = match conn.query_row(
                "SELECT total_messages, total_points FROM text_stats WHERE user_id = ?1",
                [user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(err) if err.to_string().to_lowercase().contains("no such table") => None,
                Err(err) => return Err(err),
            };
            let voice_rank = rank_for_points(conn, "voice_stats", user_id)?;
            let text_rank = rank_for_points(conn, "text_stats", user_id)?;
            Ok(json!({
                "voice": {
                    "lifetime_seconds": voice.as_ref().map(|(s, _)| safe_int(s)).unwrap_or(0),
                    "lifetime_points": voice.as_ref().map(|(_, p)| safe_int(p)).unwrap_or(0),
                    "rank": voice_rank,
                },
                "text": {
                    "lifetime_messages": text.as_ref().map(|(m, _)| safe_int(m)).unwrap_or(0),
                    "lifetime_points": text.as_ref().map(|(_, p)| safe_int(p)).unwrap_or(0),
                    "rank": text_rank,
                },
            }))
        })
        .await;

    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => db_error(err),
    }
}

const WEEKDAYS_DE: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];

/// Co-Player-IDs aus JSON-Array: >0, != user, dedupliziert in Reihenfolge.
fn parse_co_ids(raw: Option<&str>, user_id: i64) -> Vec<i64> {
    let Ok(decoded) = serde_json::from_str::<Value>(raw.unwrap_or("[]")) else {
        return Vec::new();
    };
    let Some(list) = decoded.as_array() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in list {
        let id = match entry {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse::<i64>().ok(),
            _ => None,
        };
        let Some(id) = id else { continue };
        if id <= 0 || id == user_id || !seen.insert(id) {
            continue;
        }
        out.push(id);
    }
    out
}

/// `GET /api/public/me/voice-history`.
pub async fn handle_me_voice_history(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Query(params): Params,
) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let user_id = match session_user_id(&session) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let days = match parse_positive_int(params.get("range"), 30, 1, 365, "range") {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let recent_limit = match parse_positive_int(params.get("sessions"), 12, 1, 50, "sessions") {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let mode = match normalize_mode(params.get("mode")) {
        Ok(m) => m,
        Err(resp) => return resp,
    };
    let cutoff = format!("-{days} day");
    let mode_for_db = mode.clone();

    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT date(started_at) AS day, SUM(duration_seconds) AS total_seconds,
                        COUNT(*) AS sessions, COUNT(DISTINCT user_id) AS users
                   FROM voice_session_log
                  WHERE started_at >= datetime('now', ?1) AND user_id = ?2
                  GROUP BY date(started_at) ORDER BY day DESC",
            )?;
            let daily: Vec<Value> = stmt
                .query_map(rusqlite::params![&cutoff, user_id], |row| {
                    Ok(json!({
                        "day": row.get::<_, Option<String>>(0)?,
                        "total_seconds": row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        "sessions": row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        "users": row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    }))
                })?
                .collect::<Result<_, _>>()?;

            let top_user: Option<(Option<String>, i64, i64, i64)> = conn
                .query_row(
                    "SELECT MAX(display_name), SUM(duration_seconds), SUM(points), COUNT(*)
                       FROM voice_session_log
                      WHERE started_at >= datetime('now', ?1) AND user_id = ?2
                      GROUP BY user_id",
                    rusqlite::params![&cutoff, user_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            row.get(3)?,
                        ))
                    },
                )
                .optional()?;

            let mut stmt = conn.prepare(
                "WITH grouped AS (
                    SELECT CASE
                        WHEN ?1 = 'hour' THEN strftime('%H', started_at)
                        WHEN ?1 = 'day' THEN strftime('%w', started_at)
                        WHEN ?1 = 'week' THEN strftime('%Y-%W', started_at)
                        ELSE strftime('%Y-%m', started_at)
                    END AS bucket, duration_seconds, COALESCE(peak_users, 0) AS peak_users
                    FROM voice_session_log
                    WHERE started_at >= datetime('now', ?2) AND user_id = ?3
                 )
                 SELECT bucket, SUM(duration_seconds), COUNT(*), SUM(peak_users)
                   FROM grouped GROUP BY bucket ORDER BY bucket",
            )?;
            let bucket_rows: Vec<(Option<String>, i64, i64, i64)> = stmt
                .query_map(rusqlite::params![&mode_for_db, &cutoff, user_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    ))
                })?
                .collect::<Result<_, _>>()?;

            let range_stats: Option<(i64, i64, i64, i64, i64, SqlValue)> = conn
                .query_row(
                    "SELECT SUM(duration_seconds), SUM(points), COUNT(*),
                            SUM(COALESCE(peak_users, 0)), COUNT(DISTINCT date(started_at)),
                            MAX(ended_at)
                       FROM voice_session_log
                      WHERE started_at >= datetime('now', ?1) AND user_id = ?2",
                    rusqlite::params![&cutoff, user_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                            row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            row.get(2)?,
                            row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .optional()?;

            let lifetime: Option<(i64, i64, SqlValue)> = conn
                .query_row(
                    "SELECT total_seconds, total_points, last_update FROM voice_stats
                      WHERE user_id = ?1",
                    [user_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                            row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            row.get(2)?,
                        ))
                    },
                )
                .optional()?;

            let (lifetime_sessions, lifetime_last): (i64, SqlValue) = conn.query_row(
                "SELECT COUNT(*), MAX(ended_at) FROM voice_session_log WHERE user_id = ?1",
                [user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            struct RecentRow {
                id: i64,
                guild_id: Option<SqlValue>,
                channel_id: Option<SqlValue>,
                channel_name: Option<String>,
                started_at: SqlValue,
                ended_at: SqlValue,
                duration_seconds: i64,
                points: i64,
                peak_users: i64,
                co_ids: Vec<i64>,
            }
            let mut stmt = conn.prepare(
                "SELECT id, guild_id, channel_id, channel_name, started_at, ended_at,
                        duration_seconds, points, peak_users, co_player_ids
                   FROM voice_session_log WHERE user_id = ?1
                  ORDER BY datetime(ended_at) DESC, id DESC LIMIT ?2",
            )?;
            let recent: Vec<RecentRow> = stmt
                .query_map(rusqlite::params![user_id, recent_limit], |row| {
                    let guild: SqlValue = row.get(1)?;
                    let channel: SqlValue = row.get(2)?;
                    let co_raw: Option<String> = row.get(9)?;
                    Ok(RecentRow {
                        id: row.get(0)?,
                        guild_id: (guild != SqlValue::Null).then_some(guild),
                        channel_id: (channel != SqlValue::Null).then_some(channel),
                        channel_name: row.get(3)?,
                        started_at: row.get(4)?,
                        ended_at: row.get(5)?,
                        duration_seconds: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                        points: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                        peak_users: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
                        co_ids: parse_co_ids(co_raw.as_deref(), user_id),
                    })
                })?
                .collect::<Result<_, _>>()?;

            let name_map = resolve_display_names(conn, &[user_id])?;
            let all_co: Vec<i64> = recent.iter().flat_map(|r| r.co_ids.iter().copied()).collect();
            let co_names = if all_co.is_empty() {
                HashMap::new()
            } else {
                resolve_display_names(conn, &all_co)?
            };

            let own_name = name_map
                .get(&user_id)
                .cloned()
                .unwrap_or_else(|| format!("User {user_id}"));

            let buckets = normalize_buckets_voice(&mode_for_db, &bucket_rows);

            let recent_sessions: Vec<Value> = recent
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "guild_id": r.guild_id.as_ref().map(|v| safe_int(v).to_string()),
                        "channel_id": r.channel_id.as_ref().map(|v| safe_int(v).to_string()),
                        "channel_name": r.channel_name.as_deref().filter(|s| !s.is_empty()),
                        "started_at": to_iso(r.started_at.clone()),
                        "ended_at": to_iso(r.ended_at.clone()),
                        "duration_seconds": r.duration_seconds,
                        "points": r.points,
                        "peak_users": r.peak_users,
                        "co_player_count": r.co_ids.len(),
                        "co_players": r.co_ids.iter().map(|id| json!({
                            "user_id": id.to_string(),
                            "display_name": co_names.get(id).cloned()
                                .unwrap_or_else(|| format!("User {id}")),
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();

            let (range_seconds, range_points, range_sessions, sum_peak, active_days, range_last) =
                range_stats.unwrap_or((0, 0, 0, 0, 0, SqlValue::Null));
            let last_session = if range_last == SqlValue::Null {
                lifetime_last
            } else {
                range_last
            };
            let (lifetime_seconds, lifetime_points, lifetime_update) =
                lifetime.unwrap_or((0, 0, SqlValue::Null));

            Ok(json!({
                "range_days": days,
                "mode": mode_for_db,
                "user": { "user_id": user_id.to_string(), "display_name": own_name },
                "daily": daily,
                "top_users": top_user.map(|(display_name, seconds, points, sessions)| vec![json!({
                    "user_id": user_id.to_string(),
                    "display_name": display_name.filter(|s| !s.is_empty()).unwrap_or_else(|| own_name.clone()),
                    "total_seconds": seconds,
                    "total_points": points,
                    "sessions": sessions,
                })]).unwrap_or_default(),
                "buckets": buckets,
                "user_summary": {
                    "user_id": user_id.to_string(),
                    "display_name": own_name,
                    "range_seconds": range_seconds,
                    "range_points": range_points,
                    "range_sessions": range_sessions,
                    "range_days": active_days,
                    "range_avg_session_seconds": if range_sessions > 0 {
                        json!(range_seconds as f64 / range_sessions as f64)
                    } else { json!(0) },
                    "range_avg_peak": if range_sessions > 0 {
                        json!(sum_peak as f64 / range_sessions as f64)
                    } else { json!(0) },
                    "lifetime_seconds": lifetime_seconds,
                    "lifetime_points": lifetime_points,
                    "lifetime_sessions": lifetime_sessions,
                    "lifetime_last_update": to_iso(lifetime_update),
                    "last_session": to_iso(last_session),
                },
                "recent_sessions_limit": recent_limit,
                "recent_sessions": recent_sessions,
            }))
        })
        .await;

    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => db_error(err),
    }
}

/// Bucket-Normalisierung (voice): hour → 24 Slots "00".."23",
/// day → deutsche Wochentage (SQLite %w: 0=Sonntag).
fn normalize_buckets_voice(mode: &str, rows: &[(Option<String>, i64, i64, i64)]) -> Vec<Value> {
    let entry = |label: &str, total: i64, sessions: i64, sum_peak: i64| {
        json!({
            "label": label,
            "total_seconds": total,
            "sessions": sessions,
            "avg_peak": if sessions > 0 {
                json!(sum_peak as f64 / sessions as f64)
            } else { json!(0) },
        })
    };
    let existing: HashMap<&str, &(Option<String>, i64, i64, i64)> = rows
        .iter()
        .filter_map(|r| r.0.as_deref().map(|label| (label, r)))
        .collect();
    match mode {
        "hour" => (0..24)
            .map(|h| {
                let label = format!("{h:02}");
                match existing.get(label.as_str()) {
                    Some((_, total, sessions, peak)) => entry(&label, *total, *sessions, *peak),
                    None => entry(&label, 0, 0, 0),
                }
            })
            .collect(),
        "day" => (0..7)
            .map(|d| {
                let key = d.to_string();
                match existing.get(key.as_str()) {
                    Some((_, total, sessions, peak)) => {
                        entry(WEEKDAYS_DE[d], *total, *sessions, *peak)
                    }
                    None => entry(WEEKDAYS_DE[d], 0, 0, 0),
                }
            })
            .collect(),
        _ => rows
            .iter()
            .map(|(label, total, sessions, peak)| {
                entry(label.as_deref().unwrap_or(""), *total, *sessions, *peak)
            })
            .collect(),
    }
}

fn normalize_mode(raw: Option<&String>) -> Result<String, Response> {
    let mode = raw
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "day".to_string());
    if ["hour", "day", "week", "month"].contains(&mode.as_str()) {
        Ok(mode)
    } else {
        Err(dl_webcore::envelope::error_code(
            axum::http::StatusCode::BAD_REQUEST,
            "invalid_mode",
        ))
    }
}

/// `GET /api/public/me/text-history` — text_conversation_log/text_stats optional.
pub async fn handle_me_text_history(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Query(params): Params,
) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let user_id = match session_user_id(&session) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let days = match parse_positive_int(params.get("range"), 30, 1, 365, "range") {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let recent_limit = match parse_positive_int(params.get("sessions"), 12, 1, 50, "sessions") {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let mode = match normalize_mode(params.get("mode")) {
        Ok(m) => m,
        Err(resp) => return resp,
    };
    let cutoff = format!("-{days} day");
    let mode_for_db = mode.clone();

    let result = app
        .db
        .read(move |conn| {
            let missing =
                |e: &rusqlite::Error| e.to_string().to_lowercase().contains("no such table");

            let daily: Vec<Value> = match conn.prepare(
                "SELECT date(started_at) AS day, SUM(message_count), SUM(points), COUNT(*)
                   FROM text_conversation_log
                  WHERE started_at >= datetime('now', ?1) AND user_id = ?2
                  GROUP BY date(started_at) ORDER BY day DESC",
            ) {
                Ok(mut stmt) => stmt
                    .query_map(rusqlite::params![&cutoff, user_id], |row| {
                        Ok(json!({
                            "day": row.get::<_, Option<String>>(0)?,
                            "total_messages": row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            "total_points": row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            "sessions": row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        }))
                    })?
                    .collect::<Result<_, _>>()?,
                Err(e) if missing(&e) => Vec::new(),
                Err(e) => return Err(e),
            };

            let bucket_rows: Vec<(Option<String>, i64, i64, i64)> = match conn.prepare(
                "WITH grouped AS (
                    SELECT CASE
                        WHEN ?1 = 'hour' THEN strftime('%H', started_at)
                        WHEN ?1 = 'day' THEN strftime('%w', started_at)
                        WHEN ?1 = 'week' THEN strftime('%Y-%W', started_at)
                        ELSE strftime('%Y-%m', started_at)
                    END AS bucket, message_count, points
                    FROM text_conversation_log
                    WHERE started_at >= datetime('now', ?2) AND user_id = ?3
                 )
                 SELECT bucket, SUM(message_count), SUM(points), COUNT(*)
                   FROM grouped GROUP BY bucket ORDER BY bucket",
            ) {
                Ok(mut stmt) => stmt
                    .query_map(rusqlite::params![&mode_for_db, &cutoff, user_id], |row| {
                        Ok((
                            row.get(0)?,
                            row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        ))
                    })?
                    .collect::<Result<_, _>>()?,
                Err(e) if missing(&e) => Vec::new(),
                Err(e) => return Err(e),
            };

            let range_stats: Option<(i64, i64, i64, SqlValue)> = match conn.query_row(
                "SELECT SUM(message_count), SUM(points), COUNT(*), MAX(ended_at)
                   FROM text_conversation_log
                  WHERE started_at >= datetime('now', ?1) AND user_id = ?2",
                rusqlite::params![&cutoff, user_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                        row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) if missing(&e) => None,
                Err(e) => return Err(e),
            };

            let lifetime: Option<(i64, i64, SqlValue)> = match conn.query_row(
                "SELECT total_messages, total_points, last_update FROM text_stats
                  WHERE user_id = ?1",
                [user_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                        row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        row.get(2)?,
                    ))
                },
            ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) if missing(&e) => None,
                Err(e) => return Err(e),
            };

            struct TextRow {
                id: i64,
                channel_id: Option<SqlValue>,
                started_at: SqlValue,
                ended_at: SqlValue,
                message_count: i64,
                points: i64,
                had_interaction: i64,
                co_participants: usize,
            }
            let recent: Vec<TextRow> = match conn.prepare(
                "SELECT id, guild_id, channel_id, started_at, ended_at,
                        message_count, points, co_participant_ids, had_interaction
                   FROM text_conversation_log WHERE user_id = ?1
                  ORDER BY datetime(ended_at) DESC, id DESC LIMIT ?2",
            ) {
                Ok(mut stmt) => stmt
                    .query_map(rusqlite::params![user_id, recent_limit], |row| {
                        let channel: SqlValue = row.get(2)?;
                        let co_raw: Option<String> = row.get(7)?;
                        let had: SqlValue = row.get(8)?;
                        Ok(TextRow {
                            id: row.get(0)?,
                            channel_id: (channel != SqlValue::Null).then_some(channel),
                            started_at: row.get(3)?,
                            ended_at: row.get(4)?,
                            message_count: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                            points: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                            had_interaction: safe_int(&had),
                            co_participants: parse_co_ids(co_raw.as_deref(), user_id).len(),
                        })
                    })?
                    .collect::<Result<_, _>>()?,
                Err(e) if missing(&e) => Vec::new(),
                Err(e) => return Err(e),
            };

            let buckets = normalize_buckets_text(&mode_for_db, &bucket_rows);
            let (range_messages, range_points, range_sessions, range_last) =
                range_stats.unwrap_or((0, 0, 0, SqlValue::Null));
            let (lifetime_messages, lifetime_points, lifetime_update) =
                lifetime.unwrap_or((0, 0, SqlValue::Null));
            let last_session = if range_last == SqlValue::Null {
                lifetime_update
            } else {
                range_last
            };

            Ok(json!({
                "range_days": days,
                "mode": mode_for_db,
                "user_summary": {
                    "user_id": user_id.to_string(),
                    "lifetime_messages": lifetime_messages,
                    "lifetime_points": lifetime_points,
                    "range_messages": range_messages,
                    "range_points": range_points,
                    "range_sessions": range_sessions,
                    "last_session": to_iso(last_session),
                },
                "daily": daily,
                "buckets": buckets,
                "recent_sessions_limit": recent_limit,
                "recent_sessions": recent.iter().map(|r| json!({
                    "id": r.id,
                    "channel_id": r.channel_id.as_ref().map(|v| safe_int(v).to_string()),
                    "started_at": to_iso(r.started_at.clone()),
                    "ended_at": to_iso(r.ended_at.clone()),
                    "message_count": r.message_count,
                    "points": r.points,
                    "had_interaction": r.had_interaction,
                    "co_participants": r.co_participants,
                })).collect::<Vec<_>>(),
            }))
        })
        .await;

    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => db_error(err),
    }
}

fn normalize_buckets_text(mode: &str, rows: &[(Option<String>, i64, i64, i64)]) -> Vec<Value> {
    let entry = |label: &str, messages: i64, points: i64, sessions: i64| {
        json!({
            "label": label,
            "total_messages": messages,
            "total_points": points,
            "sessions": sessions,
        })
    };
    let existing: HashMap<&str, &(Option<String>, i64, i64, i64)> = rows
        .iter()
        .filter_map(|r| r.0.as_deref().map(|label| (label, r)))
        .collect();
    match mode {
        "hour" => (0..24)
            .map(|h| {
                let label = format!("{h:02}");
                match existing.get(label.as_str()) {
                    Some((_, m, p, s)) => entry(&label, *m, *p, *s),
                    None => entry(&label, 0, 0, 0),
                }
            })
            .collect(),
        "day" => (0..7)
            .map(|d| {
                let key = d.to_string();
                match existing.get(key.as_str()) {
                    Some((_, m, p, s)) => entry(WEEKDAYS_DE[d], *m, *p, *s),
                    None => entry(WEEKDAYS_DE[d], 0, 0, 0),
                }
            })
            .collect(),
        _ => rows
            .iter()
            .map(|(label, m, p, s)| entry(label.as_deref().unwrap_or(""), *m, *p, *s))
            .collect(),
    }
}

/// `GET /api/public/me/heatmap` — 7×24-Sekunden-Matrix.
pub async fn handle_me_heatmap(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Query(params): Params,
) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let user_id = match session_user_id(&session) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let days = match parse_positive_int(params.get("days"), 90, 7, 365, "days") {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let cutoff = isoformat(now_local() - Duration::days(days));

    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT started_at, ended_at, duration_seconds FROM voice_session_log
                  WHERE user_id = ?1 AND started_at >= ?2 ORDER BY started_at",
            )?;
            let rows: Vec<(Option<String>, Option<String>, i64)> = stmt
                .query_map(rusqlite::params![user_id, &cutoff], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    ))
                })?
                .collect::<Result<_, _>>()?;
            Ok(rows)
        })
        .await;

    let rows = match result {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

    // _build_voice_matrix: Sessions stundenweise aufteilen
    let mut matrix = [[0i64; 24]; 7];
    let mut total_seconds = 0i64;
    for (started_raw, ended_raw, duration) in &rows {
        let Some(started) = started_raw.as_deref().and_then(parse_iso) else {
            continue;
        };
        let ended = match ended_raw.as_deref().and_then(parse_iso) {
            Some(e) => e,
            None => {
                if *duration <= 0 {
                    continue;
                }
                started + Duration::seconds(*duration)
            }
        };
        if ended <= started {
            continue;
        }
        let mut cursor = started;
        while cursor < ended {
            let next_hour = cursor
                .date()
                .and_hms_opt(chrono::Timelike::hour(&cursor.time()), 0, 0)
                .expect("gültige Stunde")
                + Duration::hours(1);
            let segment_end = next_hour.min(ended);
            let seconds = (segment_end - cursor).num_seconds();
            if seconds > 0 {
                matrix[crate::timeutil::weekday_mon0(&cursor)][crate::timeutil::hour(&cursor)] +=
                    seconds;
                total_seconds += seconds;
            }
            cursor = segment_end;
        }
    }

    Json(json!({ "matrix": matrix, "total_seconds": total_seconds })).into_response()
}

/// `GET /api/public/me/co-players` — user_co_players ist optional.
pub async fn handle_me_co_players(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Query(params): Params,
) -> Response {
    let session = match require_session(&app, &headers) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let user_id = match session_user_id(&session) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let limit = match parse_positive_int(params.get("limit"), 15, 1, 50, "limit") {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT co_player_id, sessions_together, total_minutes_together,
                        last_played_together, co_player_display_name
                   FROM user_co_players WHERE user_id = ?1
                  ORDER BY sessions_together DESC, total_minutes_together DESC,
                           last_played_together DESC
                  LIMIT ?2",
            )?;
            let rows: Vec<(SqlValue, i64, i64, SqlValue, Option<String>)> = stmt
                .query_map(rusqlite::params![user_id, limit], |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?
                .collect::<Result<_, _>>()?;
            let ids: Vec<i64> = rows.iter().map(|(id, ..)| safe_int(id)).collect();
            let fallback = resolve_display_names(conn, &ids)?;
            let entries: Vec<Value> = rows
                .iter()
                .map(|(id, sessions, minutes, last, display)| {
                    let uid = safe_int(id);
                    let name = display
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .or_else(|| fallback.get(&uid).cloned())
                        .unwrap_or_else(|| format!("User {uid}"));
                    json!({
                        "user_id": uid.to_string(),
                        "name": name,
                        "sessions_together": sessions,
                        "total_minutes_together": minutes,
                        "last_played": to_iso(last.clone()),
                    })
                })
                .collect();
            Ok(json!({ "entries": entries }))
        })
        .await;

    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(ref err) if is_missing_table(err, &["user_co_players"]) => {
            Json(json!({ "entries": [] })).into_response()
        }
        Err(err) => db_error(err),
    }
}
