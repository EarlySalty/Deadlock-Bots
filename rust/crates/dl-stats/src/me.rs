//! /api/public/me* — persönliche Statistiken (Session-Cookie nötig).

use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration, Timelike, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::public::resolve_display_names;
use crate::timeutil::{hour, to_iso, utc_naive, weekday_mon0};
use crate::{internal_error, parse_positive_int, require_session, session_user_id, SharedApp};

type Params = Query<HashMap<String, String>>;

fn db_error(err: impl std::fmt::Display) -> Response {
    tracing::error!(%err, "PublicStats(me): DB-Fehler");
    internal_error()
}

pub(crate) fn avatar_url(user_id: &Value, avatar: Option<&str>) -> Value {
    let uid = match user_id {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Value::Null,
    };
    let hash = avatar.map(str::trim).filter(|s| !s.is_empty());
    match hash {
        Some(url) if url.starts_with("https://cdn.discordapp.com/avatars/") => json!(url),
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
        "avatar_url": avatar_url(&user_id, session.get("avatar").and_then(Value::as_str)),
    }))
    .into_response()
}

async fn voice_rank_for_points(pool: &PgPool, user_id: i64) -> Result<Option<i64>, sqlx::Error> {
    let Some(row) = sqlx::query!(
        r#"
        SELECT total_points
        FROM voice.voice_stats
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let better = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "better!"
        FROM voice.voice_stats
        WHERE total_points > $1
        "#,
        row.total_points,
    )
    .fetch_one(pool)
    .await?;
    Ok(Some(better.better + 1))
}

async fn text_rank_for_points(pool: &PgPool, user_id: i64) -> Result<Option<i64>, sqlx::Error> {
    let Some(row) = sqlx::query!(
        r#"
        SELECT total_points
        FROM activity.text_stats
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let better = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "better!"
        FROM activity.text_stats
        WHERE total_points > $1
        "#,
        row.total_points,
    )
    .fetch_one(pool)
    .await?;
    Ok(Some(better.better + 1))
}

async fn me_stats_payload(pool: &PgPool, user_id: i64) -> Result<Value, sqlx::Error> {
    let voice = sqlx::query!(
        r#"
        SELECT total_seconds, total_points
        FROM voice.voice_stats
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?;
    let text = sqlx::query!(
        r#"
        SELECT total_messages, total_points
        FROM activity.text_stats
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?;
    let voice_rank = voice_rank_for_points(pool, user_id).await?;
    let text_rank = text_rank_for_points(pool, user_id).await?;

    Ok(json!({
        "voice": {
            "lifetime_seconds": voice.as_ref().map(|row| row.total_seconds).unwrap_or(0),
            "lifetime_points": voice.as_ref().map(|row| row.total_points).unwrap_or(0),
            "rank": voice_rank,
        },
        "text": {
            "lifetime_messages": text.as_ref().map(|row| row.total_messages).unwrap_or(0),
            "lifetime_points": text.as_ref().map(|row| row.total_points).unwrap_or(0),
            "rank": text_rank,
        },
    }))
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

    match me_stats_payload(&app.pool, user_id).await {
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
    let cutoff = Utc::now() - Duration::days(days);

    match voice_history_payload(&app.pool, user_id, days, recent_limit, &mode, cutoff).await {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => db_error(err),
    }
}

async fn voice_history_payload(
    pool: &PgPool,
    user_id: i64,
    days: i64,
    recent_limit: i64,
    mode: &str,
    cutoff: DateTime<Utc>,
) -> Result<Value, sqlx::Error> {
    let daily_rows = sqlx::query!(
        r#"
        SELECT (started_at AT TIME ZONE 'UTC')::date::text AS "day!",
               COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
               COUNT(*) AS "sessions!",
               COUNT(DISTINCT user_id) AS "users!"
        FROM activity.voice_session_log
        WHERE started_at >= $1
          AND user_id = $2
        GROUP BY (started_at AT TIME ZONE 'UTC')::date
        ORDER BY (started_at AT TIME ZONE 'UTC')::date DESC
        "#,
        cutoff,
        user_id,
    )
    .fetch_all(pool)
    .await?;
    let daily: Vec<Value> = daily_rows
        .into_iter()
        .map(|row| {
            json!({
                "day": row.day,
                "total_seconds": row.total_seconds,
                "sessions": row.sessions,
                "users": row.users,
            })
        })
        .collect();

    let top_user = sqlx::query!(
        r#"
        SELECT MAX(display_name) AS "display_name?",
               COALESCE(SUM(duration_seconds), 0)::BIGINT AS "seconds!",
               COALESCE(SUM(points), 0)::BIGINT AS "points!",
               COUNT(*) AS "sessions!"
        FROM activity.voice_session_log
        WHERE started_at >= $1
          AND user_id = $2
        GROUP BY user_id
        "#,
        cutoff,
        user_id,
    )
    .fetch_optional(pool)
    .await?;

    let bucket_rows = sqlx::query!(
        r#"
        WITH source AS (
            SELECT started_at AT TIME ZONE 'UTC' AS utc_started,
                   duration_seconds,
                   COALESCE(peak_users, 0) AS peak_users
            FROM activity.voice_session_log
            WHERE started_at >= $2
              AND user_id = $3
        ),
        grouped AS (
            SELECT CASE
                    WHEN $1 = 'hour' THEN to_char(utc_started, 'HH24')
                    WHEN $1 = 'day' THEN EXTRACT(DOW FROM utc_started)::INT::TEXT
                    WHEN $1 = 'week' THEN to_char(utc_started, 'YYYY-') ||
                        lpad(((EXTRACT(DOY FROM utc_started)::INT - 1 -
                            ((8 - EXTRACT(ISODOW FROM date_trunc('year', utc_started))::INT) % 7) + 7) / 7)::TEXT, 2, '0')
                    ELSE to_char(utc_started, 'YYYY-MM')
                   END AS bucket,
                   duration_seconds,
                   peak_users
            FROM source
        )
        SELECT bucket AS "bucket!",
               COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
               COUNT(*) AS "sessions!",
               COALESCE(SUM(peak_users), 0)::BIGINT AS "sum_peak!"
        FROM grouped
        GROUP BY bucket
        ORDER BY bucket
        "#,
        mode,
        cutoff,
        user_id,
    )
    .fetch_all(pool)
    .await?;
    let bucket_rows: Vec<(Option<String>, i64, i64, i64)> = bucket_rows
        .into_iter()
        .map(|row| {
            (
                Some(row.bucket),
                row.total_seconds,
                row.sessions,
                row.sum_peak,
            )
        })
        .collect();

    let range_stats = sqlx::query!(
        r#"
        SELECT COALESCE(SUM(duration_seconds), 0)::BIGINT AS "range_seconds!",
               COALESCE(SUM(points), 0)::BIGINT AS "range_points!",
               COUNT(*) AS "range_sessions!",
               COALESCE(SUM(COALESCE(peak_users, 0)), 0)::BIGINT AS "sum_peak!",
               COUNT(DISTINCT (started_at AT TIME ZONE 'UTC')::date) AS "active_days!",
               MAX(ended_at) AS "range_last?"
        FROM activity.voice_session_log
        WHERE started_at >= $1
          AND user_id = $2
        "#,
        cutoff,
        user_id,
    )
    .fetch_one(pool)
    .await?;

    let lifetime = sqlx::query!(
        r#"
        SELECT total_seconds, total_points, last_update
        FROM voice.voice_stats
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?;

    let lifetime_sessions = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "sessions!", MAX(ended_at) AS "last?"
        FROM activity.voice_session_log
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_one(pool)
    .await?;

    let recent_rows = sqlx::query!(
        r#"
        SELECT id, guild_id, channel_id, channel_name, started_at, ended_at,
               duration_seconds, points, peak_users, co_player_ids::text AS "co_player_ids?"
        FROM activity.voice_session_log
        WHERE user_id = $1
        ORDER BY ended_at DESC, id DESC
        LIMIT $2
        "#,
        user_id,
        recent_limit,
    )
    .fetch_all(pool)
    .await?;
    struct RecentRow {
        id: i64,
        guild_id: Option<i64>,
        channel_id: Option<i64>,
        channel_name: Option<String>,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
        duration_seconds: i64,
        points: i64,
        peak_users: i64,
        co_ids: Vec<i64>,
    }
    let recent: Vec<RecentRow> = recent_rows
        .into_iter()
        .map(|row| RecentRow {
            id: row.id,
            guild_id: row.guild_id,
            channel_id: row.channel_id,
            channel_name: row.channel_name,
            started_at: row.started_at,
            ended_at: row.ended_at,
            duration_seconds: row.duration_seconds,
            points: i64::from(row.points),
            peak_users: i64::from(row.peak_users.unwrap_or(0)),
            co_ids: parse_co_ids(row.co_player_ids.as_deref(), user_id),
        })
        .collect();

    let name_map = resolve_display_names(pool, &[user_id]).await?;
    let all_co: Vec<i64> = recent
        .iter()
        .flat_map(|r| r.co_ids.iter().copied())
        .collect();
    let co_names = if all_co.is_empty() {
        HashMap::new()
    } else {
        resolve_display_names(pool, &all_co).await?
    };

    let own_name = name_map
        .get(&user_id)
        .cloned()
        .unwrap_or_else(|| format!("User {user_id}"));

    let buckets = normalize_buckets_voice(mode, &bucket_rows);
    let last_session = range_stats.range_last.or(lifetime_sessions.last);
    let (lifetime_seconds, lifetime_points, lifetime_update) = lifetime
        .as_ref()
        .map(|row| (row.total_seconds, row.total_points, row.last_update))
        .unwrap_or((0, 0, None));

    let recent_sessions: Vec<Value> = recent
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "guild_id": r.guild_id.map(|v| v.to_string()),
                "channel_id": r.channel_id.map(|v| v.to_string()),
                "channel_name": r.channel_name.as_deref().filter(|s| !s.is_empty()),
                "started_at": to_iso(Some(r.started_at)),
                "ended_at": to_iso(Some(r.ended_at)),
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

    Ok(json!({
        "range_days": days,
        "mode": mode,
        "user": { "user_id": user_id.to_string(), "display_name": own_name },
        "daily": daily,
        "top_users": top_user.map(|row| vec![json!({
            "user_id": user_id.to_string(),
            "display_name": row.display_name.filter(|s| !s.is_empty()).unwrap_or_else(|| own_name.clone()),
            "total_seconds": row.seconds,
            "total_points": row.points,
            "sessions": row.sessions,
        })]).unwrap_or_default(),
        "buckets": buckets,
        "user_summary": {
            "user_id": user_id.to_string(),
            "display_name": own_name,
            "range_seconds": range_stats.range_seconds,
            "range_points": range_stats.range_points,
            "range_sessions": range_stats.range_sessions,
            "range_days": range_stats.active_days,
            "range_avg_session_seconds": if range_stats.range_sessions > 0 {
                json!(range_stats.range_seconds as f64 / range_stats.range_sessions as f64)
            } else { json!(0) },
            "range_avg_peak": if range_stats.range_sessions > 0 {
                json!(range_stats.sum_peak as f64 / range_stats.range_sessions as f64)
            } else { json!(0) },
            "lifetime_seconds": lifetime_seconds,
            "lifetime_points": lifetime_points,
            "lifetime_sessions": lifetime_sessions.sessions,
            "lifetime_last_update": to_iso(lifetime_update),
            "last_session": to_iso(last_session),
        },
        "recent_sessions_limit": recent_limit,
        "recent_sessions": recent_sessions,
    }))
}

/// Bucket-Normalisierung (voice): hour -> 24 Slots "00".."23",
/// day -> deutsche Wochentage (Postgres DOW: 0=Sonntag).
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

/// `GET /api/public/me/text-history`.
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
    let cutoff = Utc::now() - Duration::days(days);

    match text_history_payload(&app.pool, user_id, days, recent_limit, &mode, cutoff).await {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => db_error(err),
    }
}

async fn text_history_payload(
    pool: &PgPool,
    user_id: i64,
    days: i64,
    recent_limit: i64,
    mode: &str,
    cutoff: DateTime<Utc>,
) -> Result<Value, sqlx::Error> {
    let daily_rows = sqlx::query!(
        r#"
        SELECT (started_at AT TIME ZONE 'UTC')::date::text AS "day!",
               COALESCE(SUM(message_count), 0)::BIGINT AS "total_messages!",
               COALESCE(SUM(points), 0)::BIGINT AS "total_points!",
               COUNT(*) AS "sessions!"
        FROM activity.text_conversation_log
        WHERE started_at >= $1
          AND user_id = $2
        GROUP BY (started_at AT TIME ZONE 'UTC')::date
        ORDER BY (started_at AT TIME ZONE 'UTC')::date DESC
        "#,
        cutoff,
        user_id,
    )
    .fetch_all(pool)
    .await?;
    let daily: Vec<Value> = daily_rows
        .into_iter()
        .map(|row| {
            json!({
                "day": row.day,
                "total_messages": row.total_messages,
                "total_points": row.total_points,
                "sessions": row.sessions,
            })
        })
        .collect();

    let bucket_rows = sqlx::query!(
        r#"
        WITH source AS (
            SELECT started_at AT TIME ZONE 'UTC' AS utc_started,
                   message_count,
                   points
            FROM activity.text_conversation_log
            WHERE started_at >= $2
              AND user_id = $3
        ),
        grouped AS (
            SELECT CASE
                    WHEN $1 = 'hour' THEN to_char(utc_started, 'HH24')
                    WHEN $1 = 'day' THEN EXTRACT(DOW FROM utc_started)::INT::TEXT
                    WHEN $1 = 'week' THEN to_char(utc_started, 'YYYY-') ||
                        lpad(((EXTRACT(DOY FROM utc_started)::INT - 1 -
                            ((8 - EXTRACT(ISODOW FROM date_trunc('year', utc_started))::INT) % 7) + 7) / 7)::TEXT, 2, '0')
                    ELSE to_char(utc_started, 'YYYY-MM')
                   END AS bucket,
                   message_count,
                   points
            FROM source
        )
        SELECT bucket AS "bucket!",
               COALESCE(SUM(message_count), 0)::BIGINT AS "messages!",
               COALESCE(SUM(points), 0)::BIGINT AS "points!",
               COUNT(*) AS "sessions!"
        FROM grouped
        GROUP BY bucket
        ORDER BY bucket
        "#,
        mode,
        cutoff,
        user_id,
    )
    .fetch_all(pool)
    .await?;
    let bucket_rows: Vec<(Option<String>, i64, i64, i64)> = bucket_rows
        .into_iter()
        .map(|row| (Some(row.bucket), row.messages, row.points, row.sessions))
        .collect();

    let range_stats = sqlx::query!(
        r#"
        SELECT COALESCE(SUM(message_count), 0)::BIGINT AS "range_messages!",
               COALESCE(SUM(points), 0)::BIGINT AS "range_points!",
               COUNT(*) AS "range_sessions!",
               MAX(ended_at) AS "range_last?"
        FROM activity.text_conversation_log
        WHERE started_at >= $1
          AND user_id = $2
        "#,
        cutoff,
        user_id,
    )
    .fetch_one(pool)
    .await?;

    let lifetime = sqlx::query!(
        r#"
        SELECT total_messages, total_points, last_update
        FROM activity.text_stats
        WHERE user_id = $1
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?;

    let recent_rows = sqlx::query!(
        r#"
        SELECT id, channel_id, started_at, ended_at,
               message_count, points, co_participant_ids::text AS "co_participant_ids?",
               had_interaction
        FROM activity.text_conversation_log
        WHERE user_id = $1
        ORDER BY ended_at DESC, id DESC
        LIMIT $2
        "#,
        user_id,
        recent_limit,
    )
    .fetch_all(pool)
    .await?;

    struct TextRow {
        id: i64,
        channel_id: Option<i64>,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
        message_count: i64,
        points: i64,
        had_interaction: i64,
        co_participants: usize,
    }
    let recent: Vec<TextRow> = recent_rows
        .into_iter()
        .map(|row| TextRow {
            id: row.id,
            channel_id: row.channel_id,
            started_at: row.started_at,
            ended_at: row.ended_at,
            message_count: i64::from(row.message_count),
            points: i64::from(row.points),
            had_interaction: if row.had_interaction { 1 } else { 0 },
            co_participants: parse_co_ids(row.co_participant_ids.as_deref(), user_id).len(),
        })
        .collect();

    let buckets = normalize_buckets_text(mode, &bucket_rows);
    let (lifetime_messages, lifetime_points, lifetime_update) = lifetime
        .as_ref()
        .map(|row| (row.total_messages, row.total_points, row.last_update))
        .unwrap_or((0, 0, None));
    let last_session = range_stats.range_last.or(lifetime_update);

    Ok(json!({
        "range_days": days,
        "mode": mode,
        "user_summary": {
            "user_id": user_id.to_string(),
            "lifetime_messages": lifetime_messages,
            "lifetime_points": lifetime_points,
            "range_messages": range_stats.range_messages,
            "range_points": range_stats.range_points,
            "range_sessions": range_stats.range_sessions,
            "last_session": to_iso(last_session),
        },
        "daily": daily,
        "buckets": buckets,
        "recent_sessions_limit": recent_limit,
        "recent_sessions": recent.iter().map(|r| json!({
            "id": r.id,
            "channel_id": r.channel_id.map(|v| v.to_string()),
            "started_at": to_iso(Some(r.started_at)),
            "ended_at": to_iso(Some(r.ended_at)),
            "message_count": r.message_count,
            "points": r.points,
            "had_interaction": r.had_interaction,
            "co_participants": r.co_participants,
        })).collect::<Vec<_>>(),
    }))
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

/// `GET /api/public/me/heatmap` — 7x24-Sekunden-Matrix.
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
    let cutoff = Utc::now() - Duration::days(days);

    match me_heatmap_payload(&app.pool, user_id, cutoff).await {
        Ok((matrix, total_seconds)) => {
            Json(json!({ "matrix": matrix, "total_seconds": total_seconds })).into_response()
        }
        Err(err) => db_error(err),
    }
}

async fn me_heatmap_payload(
    pool: &PgPool,
    user_id: i64,
    cutoff: DateTime<Utc>,
) -> Result<([[i64; 24]; 7], i64), sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT started_at, ended_at, duration_seconds
        FROM activity.voice_session_log
        WHERE user_id = $1
          AND started_at >= $2
        ORDER BY started_at
        "#,
        user_id,
        cutoff,
    )
    .fetch_all(pool)
    .await?;

    let mut matrix = [[0i64; 24]; 7];
    let mut total_seconds = 0i64;
    for row in rows {
        if row.ended_at <= row.started_at {
            continue;
        }
        let mut cursor = row.started_at;
        while cursor < row.ended_at {
            let next_hour_naive = cursor
                .date_naive()
                .and_hms_opt(cursor.hour(), 0, 0)
                .expect("gültige Stunde")
                + Duration::hours(1);
            let next_hour = DateTime::<Utc>::from_naive_utc_and_offset(next_hour_naive, Utc);
            let segment_end = next_hour.min(row.ended_at);
            let seconds = (segment_end - cursor).num_seconds();
            if seconds > 0 {
                let naive = utc_naive(&cursor);
                matrix[weekday_mon0(&naive)][hour(&naive)] += seconds;
                total_seconds += seconds;
            }
            cursor = segment_end;
        }
    }

    Ok((matrix, total_seconds))
}

/// `GET /api/public/me/co-players`.
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

    let rows = match sqlx::query!(
        r#"
        SELECT co_player_id, sessions_together, total_minutes_together,
               last_played_together, co_player_display_name
        FROM activity.user_co_players
        WHERE user_id = $1
        ORDER BY sessions_together DESC NULLS LAST,
                 total_minutes_together DESC NULLS LAST,
                 last_played_together DESC NULLS LAST
        LIMIT $2
        "#,
        user_id,
        limit,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let ids: Vec<i64> = rows.iter().map(|row| row.co_player_id).collect();
    let fallback = match resolve_display_names(&app.pool, &ids).await {
        Ok(names) => names,
        Err(err) => return db_error(err),
    };
    let entries: Vec<Value> = rows
        .iter()
        .map(|row| {
            let uid = row.co_player_id;
            let name = row
                .co_player_display_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| fallback.get(&uid).cloned())
                .unwrap_or_else(|| format!("User {uid}"));
            json!({
                "user_id": uid.to_string(),
                "name": name,
                "sessions_together": i64::from(row.sessions_together.unwrap_or(0)),
                "total_minutes_together": i64::from(row.total_minutes_together.unwrap_or(0)),
                "last_played": to_iso(row.last_played_together),
            })
        })
        .collect();
    Json(json!({ "entries": entries })).into_response()
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("gültiger Test-Zeitpunkt")
            .with_timezone(&Utc)
    }

    async fn insert_voice_session(
        pool: &PgPool,
        id: i64,
        user_id: i64,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO activity.voice_session_log
                (id, user_id, started_at, ended_at, duration_seconds, points)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(started_at)
        .bind(ended_at)
        .bind((ended_at - started_at).num_seconds())
        .bind(1_i32)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_text_session(
        pool: &PgPool,
        id: i64,
        user_id: i64,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO activity.text_conversation_log
                (id, user_id, started_at, ended_at, message_count, points)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(started_at)
        .bind(ended_at)
        .bind(1_i32)
        .bind(1_i32)
        .execute(pool)
        .await?;
        Ok(())
    }

    fn field_for<'a>(rows: &'a [Value], key: &str, expected: &str, field: &str) -> &'a Value {
        rows.iter()
            .find(|row| row[key] == json!(expected))
            .unwrap_or_else(|| panic!("Zeile mit {key}={expected} fehlt"))
            .get(field)
            .unwrap_or_else(|| panic!("Feld {field} fehlt"))
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn me_stats_roundtrip_and_missing_user() -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let now = Utc::now();

        sqlx::query!(
            r#"
            INSERT INTO voice.voice_stats (user_id, total_seconds, total_points, last_update)
            VALUES ($1, $2, $3, $4), ($5, $6, $7, $8)
            "#,
            1001_i64,
            3600_i64,
            42_i64,
            now,
            1002_i64,
            7200_i64,
            100_i64,
            now,
        )
        .execute(db.pool())
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO activity.text_stats (user_id, total_messages, total_points, last_update)
            VALUES ($1, $2, $3, $4)
            "#,
            1001_i64,
            25_i64,
            7_i64,
            now,
        )
        .execute(db.pool())
        .await?;

        let payload = me_stats_payload(db.pool(), 1001).await?;
        assert_eq!(payload["voice"]["lifetime_seconds"], json!(3600));
        assert_eq!(payload["voice"]["lifetime_points"], json!(42));
        assert_eq!(payload["voice"]["rank"], json!(2));
        assert_eq!(payload["text"]["lifetime_messages"], json!(25));
        assert_eq!(payload["text"]["rank"], json!(1));

        let missing = me_stats_payload(db.pool(), 9999).await?;
        assert_eq!(missing["voice"]["lifetime_seconds"], json!(0));
        assert_eq!(missing["voice"]["rank"], Value::Null);
        assert_eq!(missing["text"]["lifetime_messages"], json!(0));
        assert_eq!(missing["text"]["rank"], Value::Null);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn me_history_and_heatmap_bucket_utc_wall_time_edges(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let user_id = 4242_i64;
        let cases = [
            (ts("2026-06-30T23:30:00Z"), ts("2026-06-30T23:40:00Z")),
            (ts("2026-07-01T00:30:00Z"), ts("2026-07-01T00:40:00Z")),
            (ts("2026-01-04T23:30:00Z"), ts("2026-01-04T23:40:00Z")),
            (ts("2026-01-05T00:30:00Z"), ts("2026-01-05T00:40:00Z")),
            (ts("2026-03-29T01:30:00Z"), ts("2026-03-29T01:45:00Z")),
        ];

        for (idx, (started_at, ended_at)) in cases.iter().copied().enumerate() {
            insert_voice_session(
                db.pool(),
                10_000 + idx as i64,
                user_id,
                started_at,
                ended_at,
            )
            .await?;
            insert_text_session(
                db.pool(),
                20_000 + idx as i64,
                user_id,
                started_at,
                ended_at,
            )
            .await?;
        }

        let cutoff = ts("2025-01-01T00:00:00Z");
        let voice_week = voice_history_payload(db.pool(), user_id, 365, 10, "week", cutoff).await?;
        let voice_daily = voice_week["daily"].as_array().expect("voice daily array");
        assert_eq!(
            field_for(voice_daily, "day", "2026-06-30", "sessions"),
            &json!(1)
        );
        assert_eq!(
            field_for(voice_daily, "day", "2026-07-01", "sessions"),
            &json!(1)
        );
        let voice_buckets = voice_week["buckets"]
            .as_array()
            .expect("voice bucket array");
        assert_eq!(
            field_for(voice_buckets, "label", "2026-00", "sessions"),
            &json!(1)
        );
        assert_eq!(
            field_for(voice_buckets, "label", "2026-01", "sessions"),
            &json!(1)
        );

        let voice_hour = voice_history_payload(db.pool(), user_id, 365, 10, "hour", cutoff).await?;
        let voice_hours = voice_hour["buckets"].as_array().expect("voice hours array");
        assert_eq!(field_for(voice_hours, "label", "01", "sessions"), &json!(1));
        assert_eq!(field_for(voice_hours, "label", "03", "sessions"), &json!(0));

        let text_week = text_history_payload(db.pool(), user_id, 365, 10, "week", cutoff).await?;
        let text_daily = text_week["daily"].as_array().expect("text daily array");
        assert_eq!(
            field_for(text_daily, "day", "2026-06-30", "sessions"),
            &json!(1)
        );
        assert_eq!(
            field_for(text_daily, "day", "2026-07-01", "sessions"),
            &json!(1)
        );
        let text_buckets = text_week["buckets"].as_array().expect("text bucket array");
        assert_eq!(
            field_for(text_buckets, "label", "2026-00", "sessions"),
            &json!(1)
        );
        assert_eq!(
            field_for(text_buckets, "label", "2026-01", "sessions"),
            &json!(1)
        );

        let text_hour = text_history_payload(db.pool(), user_id, 365, 10, "hour", cutoff).await?;
        let text_hours = text_hour["buckets"].as_array().expect("text hours array");
        assert_eq!(field_for(text_hours, "label", "01", "sessions"), &json!(1));
        assert_eq!(field_for(text_hours, "label", "03", "sessions"), &json!(0));

        let (matrix, total_seconds) = me_heatmap_payload(db.pool(), user_id, cutoff).await?;
        assert_eq!(matrix[6][1], 900);
        assert_eq!(matrix[6][3], 0);
        assert_eq!(total_seconds, 3_300);
        Ok(())
    }
}
