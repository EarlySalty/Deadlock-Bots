//! Öffentliche Analytics-Endpunkte — Logik 1:1 aus public_stats.py.

use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use dl_core::pyfloat::py_round;
use serde_json::{json, Map, Value};
use sqlx::PgPool;

use crate::ranks::{detect_lane, RankResolver, LANES, RANK_COLORS, RANK_ORDER};
use crate::timeutil::{hour, isoformat, now_local, to_iso, utc_naive, weekday_mon0};
use crate::{internal_error, parse_positive_int, SharedApp};

type Params = Query<HashMap<String, String>>;

fn db_error(err: impl std::fmt::Display) -> Response {
    tracing::error!(%err, "PublicStats: DB-Fehler");
    internal_error()
}

pub async fn handle_health() -> Response {
    Json(json!({ "ok": true, "ts": chrono::Local::now().timestamp() })).into_response()
}

pub async fn handle_rank_colors() -> Response {
    let mut colors = Map::new();
    for (rank, color) in RANK_COLORS {
        colors.insert(rank.to_string(), json!(color));
    }
    Json(json!({ "colors": colors, "generated_at": isoformat(now_local()) })).into_response()
}

/// `GET /api/activity-heatmap` — 7x24-Buckets pro Rang, letzte 14 Tage.
pub async fn handle_activity_heatmap(State(app): State<SharedApp>) -> Response {
    let now = now_local();
    let cutoff = Utc::now() - Duration::days(14);
    let rows = match sqlx::query!(
        r#"
        SELECT started_at, user_id
        FROM activity.voice_session_log
        WHERE started_at >= $1
        ORDER BY started_at
        "#,
        cutoff,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let mut resolver = RankResolver::new(&app.pool);
    let mut counts: HashMap<&str, [[i64; 24]; 7]> =
        RANK_ORDER.iter().map(|r| (*r, [[0; 24]; 7])).collect();
    for row in rows {
        let started = utc_naive(&row.started_at);
        let rank = match resolver.rank_of(row.user_id).await {
            Ok(rank) => rank,
            Err(err) => return db_error(err),
        };
        let Some(rank) = rank else {
            continue;
        };
        if let Some(grid) = counts.get_mut(rank) {
            grid[weekday_mon0(&started)][hour(&started)] += 1;
        }
    }

    let mut heatmap = Map::new();
    for rank in RANK_ORDER {
        let grid = counts.get(rank).expect("alle Ränge initialisiert");
        let mut entries = Vec::with_capacity(7 * 24);
        for (day, row) in grid.iter().enumerate() {
            for (h, count) in row.iter().enumerate() {
                entries.push(json!({ "day": day, "hour": h, "count": count }));
            }
        }
        heatmap.insert(rank.to_string(), Value::Array(entries));
    }

    Json(json!({
        "heatmap": heatmap,
        "rank_order": RANK_ORDER,
        "generated_at": isoformat(now),
    }))
    .into_response()
}

/// `GET /api/rank-distribution`.
pub async fn handle_rank_distribution(
    State(app): State<SharedApp>,
    Query(params): Params,
) -> Response {
    let now = now_local();
    let now_utc = Utc::now();
    let weeks = match parse_positive_int(params.get("weeks"), 4, 1, 52, "weeks") {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let rows = match sqlx::query!(
        r#"
        SELECT lower(deadlock_rank_name) AS "rank!", COUNT(*) AS "cnt!"
        FROM core.steam_links
        WHERE verified = TRUE
          AND deadlock_rank_name IS NOT NULL
        GROUP BY deadlock_rank_name
        "#
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let mut distribution: Map<String, Value> = RANK_ORDER
        .iter()
        .map(|r| (r.to_string(), json!(0)))
        .collect();
    for row in rows {
        if distribution.contains_key(&row.rank) {
            distribution.insert(row.rank, json!(row.cnt));
        }
    }

    let mut resolver = RankResolver::new(&app.pool);
    let mut weekly = Vec::new();
    for week in 0..weeks {
        let week_start = now_utc - Duration::weeks(week + 1);
        let week_end = now_utc - Duration::weeks(week);
        let user_rows = match sqlx::query!(
            r#"
            SELECT user_id
            FROM activity.voice_session_log
            WHERE started_at >= $1
              AND started_at < $2
            "#,
            week_start,
            week_end,
        )
        .fetch_all(&app.pool)
        .await
        {
            Ok(rows) => rows,
            Err(err) => return db_error(err),
        };

        let mut week_data: HashMap<&str, i64> = RANK_ORDER.iter().map(|r| (*r, 0)).collect();
        let mut seen: HashSet<i64> = HashSet::new();
        for row in user_rows {
            if seen.contains(&row.user_id) {
                continue;
            }
            let rank = match resolver.rank_of(row.user_id).await {
                Ok(rank) => rank,
                Err(err) => return db_error(err),
            };
            if let Some(rank) = rank {
                *week_data.get_mut(rank).expect("init") += 1;
                seen.insert(row.user_id);
            }
        }
        let data: Map<String, Value> = RANK_ORDER
            .iter()
            .map(|r| (r.to_string(), json!(week_data[*r])))
            .collect();
        weekly.push(json!({ "week": week + 1, "data": data }));
    }
    weekly.reverse();

    Json(json!({
        "distribution": distribution,
        "rank_order": RANK_ORDER,
        "weekly_trend": weekly,
        "generated_at": isoformat(now),
    }))
    .into_response()
}

/// `GET /api/lane-preferences`.
pub async fn handle_lane_preferences(State(app): State<SharedApp>) -> Response {
    let now = now_local();
    let cutoff = Utc::now() - Duration::days(30);
    let rows = match sqlx::query!(
        r#"
        SELECT user_id, channel_name
        FROM activity.voice_session_log
        WHERE started_at >= $1
        "#,
        cutoff,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let mut resolver = RankResolver::new(&app.pool);
    let mut data: HashMap<&str, HashMap<&str, i64>> = RANK_ORDER
        .iter()
        .map(|r| (*r, LANES.iter().map(|l| (*l, 0)).collect()))
        .collect();
    let mut seen_per_lane: HashMap<&str, HashSet<i64>> =
        LANES.iter().map(|l| (*l, HashSet::new())).collect();

    for row in rows {
        let lane = detect_lane(row.channel_name.as_deref()).unwrap_or("unknown");
        let seen = seen_per_lane.get_mut(lane).expect("init");
        if seen.contains(&row.user_id) {
            continue;
        }
        let rank = match resolver.rank_of(row.user_id).await {
            Ok(rank) => rank,
            Err(err) => return db_error(err),
        };
        if let Some(rank) = rank {
            *data
                .get_mut(rank)
                .expect("init")
                .get_mut(lane)
                .expect("init") += 1;
            seen.insert(row.user_id);
        }
    }

    let preferences: Map<String, Value> = RANK_ORDER
        .iter()
        .map(|r| {
            let lanes: Map<String, Value> = LANES
                .iter()
                .map(|l| (l.to_string(), json!(data[*r][*l])))
                .collect();
            (r.to_string(), Value::Object(lanes))
        })
        .collect();

    Json(json!({
        "preferences": preferences,
        "rank_order": RANK_ORDER,
        "lanes": LANES,
        "generated_at": isoformat(now),
    }))
    .into_response()
}

/// `GET /api/new-player-windows`.
pub async fn handle_new_player_windows(State(app): State<SharedApp>) -> Response {
    let cutoff = Utc::now() - Duration::days(30);
    let rows = match sqlx::query!(
        r#"
        SELECT started_at, channel_name
        FROM activity.voice_session_log
        WHERE started_at >= $1
        "#,
        cutoff,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let mut hour_counts = [0i64; 24];
    let mut day_hour = [[0i64; 24]; 7];
    for row in rows {
        if detect_lane(row.channel_name.as_deref()) == Some("new_player") {
            let started = utc_naive(&row.started_at);
            let (d, h) = (weekday_mon0(&started), hour(&started));
            day_hour[d][h] += 1;
            hour_counts[h] += 1;
        }
    }

    let mut hours: Vec<(usize, i64)> = hour_counts.iter().copied().enumerate().collect();
    hours.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    let peak_hours: Vec<Value> = hours
        .iter()
        .take(5)
        .map(|(h, c)| json!({ "hour": h, "count": c }))
        .collect();

    let mut days: Vec<(usize, i64)> = day_hour
        .iter()
        .map(|row| row.iter().sum())
        .enumerate()
        .collect();
    days.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    let peak_days: Vec<Value> = days
        .iter()
        .take(3)
        .map(|(d, c)| json!({ "day": d, "count": c }))
        .collect();

    let mut matrix = Map::new();
    for (d, row) in day_hour.iter().enumerate() {
        let mut hours_map = Map::new();
        for (h, c) in row.iter().enumerate() {
            hours_map.insert(h.to_string(), json!(c));
        }
        matrix.insert(d.to_string(), Value::Object(hours_map));
    }

    Json(json!({
        "peak_hours": peak_hours,
        "peak_days": peak_days,
        "day_hour_matrix": matrix,
        "generated_at": isoformat(now_local()),
    }))
    .into_response()
}

/// `GET /api/timeline`.
pub async fn handle_timeline(State(app): State<SharedApp>, Query(params): Params) -> Response {
    let metric = params
        .get("metric")
        .cloned()
        .unwrap_or_else(|| "players".to_string());
    let days = match parse_positive_int(params.get("days"), 7, 1, 365, "days") {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let now = now_local();
    let cutoff = Utc::now() - Duration::days(days);
    let hours_metric = metric == "hours";
    let rows = match sqlx::query!(
        r#"
        SELECT started_at, user_id, duration_seconds
        FROM activity.voice_session_log
        WHERE started_at >= $1
        ORDER BY started_at
        "#,
        cutoff,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let mut resolver = RankResolver::new(&app.pool);
    let mut counts: HashMap<&str, [i64; 24]> = RANK_ORDER.iter().map(|r| (*r, [0; 24])).collect();
    let mut hours_sum: HashMap<&str, [f64; 24]> =
        RANK_ORDER.iter().map(|r| (*r, [0.0; 24])).collect();
    for row in rows {
        let started = utc_naive(&row.started_at);
        let h = hour(&started);
        let rank = match resolver.rank_of(row.user_id).await {
            Ok(rank) => rank,
            Err(err) => return db_error(err),
        };
        if let Some(rank) = rank {
            counts.get_mut(rank).expect("init")[h] += 1;
            hours_sum.get_mut(rank).expect("init")[h] += row.duration_seconds as f64 / 3600.0;
        }
    }

    let timeline: Vec<Value> = (0..24)
        .map(|h| {
            let ranks: Map<String, Value> = RANK_ORDER
                .iter()
                .map(|r| {
                    let value = if hours_metric {
                        json!(py_round(hours_sum[*r][h], 2))
                    } else {
                        json!(counts[*r][h])
                    };
                    (r.to_string(), value)
                })
                .collect();
            json!({ "hour": h, "ranks": ranks })
        })
        .collect();

    let mut peak_times = Map::new();
    for rank in RANK_ORDER {
        let entries: Vec<Value> = if hours_metric {
            let mut peaks: Vec<(usize, f64)> =
                hours_sum[rank].iter().copied().enumerate().collect();
            peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            peaks
                .iter()
                .take(3)
                .filter(|(_, c)| *c > 0.0)
                .map(|(h, c)| json!({ "hour": h, "count": py_round(*c, 2) }))
                .collect()
        } else {
            let mut peaks: Vec<(usize, i64)> = counts[rank].iter().copied().enumerate().collect();
            peaks.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
            peaks
                .iter()
                .take(3)
                .filter(|(_, c)| *c > 0)
                .map(|(h, c)| json!({ "hour": h, "count": c }))
                .collect()
        };
        peak_times.insert(rank.to_string(), Value::Array(entries));
    }

    Json(json!({
        "timeline": timeline,
        "peak_times": peak_times,
        "rank_order": RANK_ORDER,
        "metric": metric,
        "generated_at": isoformat(now),
    }))
    .into_response()
}

/// `GET /api/best-times?rank=<rank>`.
pub async fn handle_best_times(State(app): State<SharedApp>, Query(params): Params) -> Response {
    let rank = params.get("rank").cloned().unwrap_or_default();
    if !RANK_ORDER.contains(&rank.as_str()) {
        return dl_webcore::envelope::error_code(StatusCode::BAD_REQUEST, "Invalid rank");
    }
    let cutoff = Utc::now() - Duration::days(7);
    let wanted = rank.clone();
    let rows = match sqlx::query!(
        r#"
        SELECT started_at, user_id
        FROM activity.voice_session_log
        WHERE started_at >= $1
        ORDER BY started_at
        "#,
        cutoff,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };

    let mut resolver = RankResolver::new(&app.pool);
    let mut hourly = [0i64; 24];
    let mut day_hour = [[0i64; 24]; 7];
    for row in rows {
        let started = utc_naive(&row.started_at);
        let resolved = match resolver.rank_of(row.user_id).await {
            Ok(rank) => rank,
            Err(err) => return db_error(err),
        };
        if resolved == Some(wanted.as_str()) {
            hourly[hour(&started)] += 1;
            day_hour[weekday_mon0(&started)][hour(&started)] += 1;
        }
    }

    let mut peaks: Vec<(usize, i64)> = hourly.iter().copied().enumerate().collect();
    peaks.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    let peak_hours: Vec<Value> = peaks
        .iter()
        .take(3)
        .filter(|(_, c)| *c > 0)
        .map(|(h, c)| json!({ "hour": h, "count": c }))
        .collect();

    let day_distribution: Vec<Value> = day_hour
        .iter()
        .enumerate()
        .map(|(d, row)| json!({ "day": d, "count": row.iter().sum::<i64>() }))
        .collect();

    Json(json!({
        "rank": rank,
        "peak_hours": peak_hours,
        "day_distribution": day_distribution,
        "generated_at": isoformat(now_local()),
    }))
    .into_response()
}

/// `GET /api/voice-history` — SQL-Aggregation, clampt statt 400.
pub async fn handle_voice_history(State(app): State<SharedApp>, Query(params): Params) -> Response {
    let days = params
        .get("range")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map(|d| d.clamp(1, 90))
        .unwrap_or(14);
    let mode_raw = params
        .get("mode")
        .cloned()
        .unwrap_or_else(|| "hour".to_string());
    let mode = {
        let m = mode_raw.trim().to_lowercase();
        if ["hour", "day", "week", "month"].contains(&m.as_str()) {
            m
        } else {
            "hour".to_string()
        }
    };
    let cutoff = Utc::now() - Duration::days(days);
    let now = now_local();

    let payload = match voice_history_payload(&app.pool, cutoff, &mode).await {
        Ok(payload) => payload,
        Err(err) => {
            tracing::error!(%err, "voice-history fehlgeschlagen");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Voice history unavailable" })),
            )
                .into_response();
        }
    };

    let avg = if payload.total_sessions > 0 {
        json!(py_round(
            payload.total_seconds as f64 / payload.total_sessions as f64 / 60.0,
            1
        ))
    } else {
        json!(0)
    };
    Json(json!({
        "summary": {
            "total_sessions": payload.total_sessions,
            "total_seconds": payload.total_seconds,
            "total_users": payload.total_users,
            "avg_session_minutes": avg,
            "days": days,
        },
        "daily": payload.daily,
        "hourly": payload.hourly,
        "mode": mode,
        "generated_at": isoformat(now),
    }))
    .into_response()
}

struct VoiceHistoryPayload {
    daily: Vec<Value>,
    total_sessions: i64,
    total_seconds: i64,
    total_users: i64,
    hourly: Vec<Value>,
}

async fn voice_history_payload(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    mode: &str,
) -> Result<VoiceHistoryPayload, sqlx::Error> {
    let daily_rows = sqlx::query!(
        r#"
        SELECT (started_at AT TIME ZONE 'UTC')::date::text AS "day!",
               COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
               COUNT(*) AS "sessions!",
               COUNT(DISTINCT user_id) AS "unique_users!"
        FROM activity.voice_session_log
        WHERE started_at >= $1
        GROUP BY (started_at AT TIME ZONE 'UTC')::date
        ORDER BY (started_at AT TIME ZONE 'UTC')::date DESC
        "#,
        cutoff,
    )
    .fetch_all(pool)
    .await?;
    let daily = daily_rows
        .into_iter()
        .map(|row| {
            json!({
                "day": row.day,
                "total_seconds": row.total_seconds,
                "sessions": row.sessions,
                "unique_users": row.unique_users,
            })
        })
        .collect();

    let summary = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "total_sessions!",
               COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
               COUNT(DISTINCT user_id) AS "total_users!"
        FROM activity.voice_session_log
        WHERE started_at >= $1
        "#,
        cutoff,
    )
    .fetch_one(pool)
    .await?;

    let bucket_rows = sqlx::query!(
        r#"
        WITH source AS (
            SELECT started_at AT TIME ZONE 'UTC' AS utc_started,
                   duration_seconds
            FROM activity.voice_session_log
            WHERE started_at >= $2
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
                   duration_seconds
            FROM source
        )
        SELECT bucket AS "bucket!",
               COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
               COUNT(*) AS "sessions!"
        FROM grouped
        GROUP BY bucket
        ORDER BY bucket
        "#,
        mode,
        cutoff,
    )
    .fetch_all(pool)
    .await?;
    let hourly = bucket_rows
        .into_iter()
        .map(|row| {
            let avg = if row.sessions > 0 {
                json!(py_round(
                    row.total_seconds as f64 / row.sessions as f64 / 60.0,
                    1
                ))
            } else {
                json!(0)
            };
            json!({
                "bucket": row.bucket,
                "total_seconds": row.total_seconds,
                "sessions": row.sessions,
                "avg_session_minutes": avg,
            })
        })
        .collect();

    Ok(VoiceHistoryPayload {
        daily,
        total_sessions: summary.total_sessions,
        total_seconds: summary.total_seconds,
        total_users: summary.total_users,
        hourly,
    })
}

/// Display-Namen aus member_events.
pub(crate) async fn resolve_display_names(
    pool: &PgPool,
    user_ids: &[i64],
) -> Result<HashMap<i64, String>, sqlx::Error> {
    let mut names = HashMap::new();
    let mut unique: Vec<i64> = user_ids
        .iter()
        .copied()
        .filter(|id| *id != 0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unique.sort_unstable();

    if unique.is_empty() {
        return Ok(names);
    }

    let rows = sqlx::query!(
        r#"
        SELECT DISTINCT ON (user_id)
               user_id,
               display_name AS "display_name!"
        FROM activity.member_events
        WHERE user_id = ANY($1)
          AND display_name IS NOT NULL
          AND BTRIM(display_name) <> ''
        ORDER BY user_id, occurred_at DESC NULLS LAST, id DESC
        "#,
        &unique[..],
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        names.insert(row.user_id, row.display_name);
    }
    for uid in unique {
        names.entry(uid).or_insert_with(|| format!("User {uid}"));
    }
    Ok(names)
}

async fn voice_updated_at(pool: &PgPool) -> Result<String, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT MAX(last_update) AS "updated_at?"
        FROM voice.voice_stats
        "#
    )
    .fetch_one(pool)
    .await?;
    Ok(to_iso(row.updated_at).unwrap_or_else(|| isoformat(now_local())))
}

async fn text_updated_at(pool: &PgPool) -> Result<String, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT MAX(last_update) AS "updated_at?"
        FROM activity.text_stats
        "#
    )
    .fetch_one(pool)
    .await?;
    Ok(to_iso(row.updated_at).unwrap_or_else(|| isoformat(now_local())))
}

/// `GET /api/public/leaderboard/voice`.
pub async fn handle_voice_leaderboard(
    State(app): State<SharedApp>,
    Query(params): Params,
) -> Response {
    let limit = match parse_positive_int(params.get("limit"), 50, 1, 100, "limit") {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let rows = match sqlx::query!(
        r#"
        SELECT user_id, total_seconds, total_points
        FROM voice.voice_stats
        ORDER BY total_points DESC, total_seconds DESC
        LIMIT $1
        "#,
        limit,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };
    let ids: Vec<i64> = rows.iter().map(|row| row.user_id).collect();
    let names = match resolve_display_names(&app.pool, &ids).await {
        Ok(names) => names,
        Err(err) => return db_error(err),
    };
    let updated_at = match voice_updated_at(&app.pool).await {
        Ok(updated_at) => updated_at,
        Err(err) => return db_error(err),
    };
    let entries: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            json!({
                "rank": index + 1,
                "user_id": row.user_id.to_string(),
                "name": names.get(&row.user_id).cloned().unwrap_or_else(|| format!("User {}", row.user_id)),
                "avatar_url": null,
                "total_seconds": row.total_seconds,
                "total_points": row.total_points,
                "hours": py_round(row.total_seconds as f64 / 3600.0, 1),
            })
        })
        .collect();
    Json(json!({ "entries": entries, "updated_at": updated_at })).into_response()
}

/// `GET /api/public/leaderboard/text`.
pub async fn handle_text_leaderboard(
    State(app): State<SharedApp>,
    Query(params): Params,
) -> Response {
    let limit = match parse_positive_int(params.get("limit"), 50, 1, 100, "limit") {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let rows = match sqlx::query!(
        r#"
        SELECT user_id, total_messages, total_points
        FROM activity.text_stats
        ORDER BY total_points DESC, total_messages DESC
        LIMIT $1
        "#,
        limit,
    )
    .fetch_all(&app.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => return db_error(err),
    };
    let ids: Vec<i64> = rows.iter().map(|row| row.user_id).collect();
    let names = match resolve_display_names(&app.pool, &ids).await {
        Ok(names) => names,
        Err(err) => return db_error(err),
    };
    let updated_at = match text_updated_at(&app.pool).await {
        Ok(updated_at) => updated_at,
        Err(err) => return db_error(err),
    };
    let entries: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            json!({
                "rank": index + 1,
                "user_id": row.user_id.to_string(),
                "name": names.get(&row.user_id).cloned().unwrap_or_else(|| format!("User {}", row.user_id)),
                "avatar_url": null,
                "total_messages": row.total_messages,
                "total_points": row.total_points,
            })
        })
        .collect();
    Json(json!({ "entries": entries, "updated_at": updated_at })).into_response()
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

    fn field_for<'a>(rows: &'a [Value], key: &str, expected: &str, field: &str) -> &'a Value {
        rows.iter()
            .find(|row| row[key] == json!(expected))
            .unwrap_or_else(|| panic!("Zeile mit {key}={expected} fehlt"))
            .get(field)
            .unwrap_or_else(|| panic!("Feld {field} fehlt"))
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn voice_history_empty_aggregation_is_zero() -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let cutoff = Utc::now() - Duration::days(7);

        let payload = voice_history_payload(db.pool(), cutoff, "hour").await?;

        assert_eq!(payload.total_sessions, 0);
        assert_eq!(payload.total_seconds, 0);
        assert_eq!(payload.total_users, 0);
        assert!(payload.daily.is_empty());
        assert!(payload.hourly.is_empty());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn public_voice_history_bucket_utc_wall_time_edges(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let cases = [
            (ts("2026-06-30T23:30:00Z"), ts("2026-06-30T23:40:00Z")),
            (ts("2026-07-01T00:30:00Z"), ts("2026-07-01T00:40:00Z")),
            (ts("2026-01-04T23:30:00Z"), ts("2026-01-04T23:40:00Z")),
            (ts("2026-01-05T00:30:00Z"), ts("2026-01-05T00:40:00Z")),
            (ts("2026-03-29T01:30:00Z"), ts("2026-03-29T01:45:00Z")),
        ];

        for (idx, (started_at, ended_at)) in cases.iter().copied().enumerate() {
            insert_voice_session(db.pool(), 30_000 + idx as i64, 5151, started_at, ended_at)
                .await?;
        }

        let cutoff = ts("2025-01-01T00:00:00Z");
        let week_payload = voice_history_payload(db.pool(), cutoff, "week").await?;
        assert_eq!(week_payload.total_sessions, 5);
        assert_eq!(week_payload.total_seconds, 3_300);
        assert_eq!(week_payload.total_users, 1);
        assert_eq!(
            field_for(&week_payload.daily, "day", "2026-06-30", "sessions"),
            &json!(1)
        );
        assert_eq!(
            field_for(&week_payload.daily, "day", "2026-07-01", "sessions"),
            &json!(1)
        );
        assert_eq!(
            field_for(&week_payload.hourly, "bucket", "2026-00", "sessions"),
            &json!(1)
        );
        assert_eq!(
            field_for(&week_payload.hourly, "bucket", "2026-01", "sessions"),
            &json!(1)
        );

        let hour_payload = voice_history_payload(db.pool(), cutoff, "hour").await?;
        assert_eq!(
            field_for(&hour_payload.hourly, "bucket", "01", "sessions"),
            &json!(1)
        );
        assert!(!hour_payload
            .hourly
            .iter()
            .any(|row| row["bucket"] == json!("03")));
        Ok(())
    }
}
