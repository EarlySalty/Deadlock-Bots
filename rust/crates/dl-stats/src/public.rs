//! Öffentliche Analytics-Endpunkte — Logik 1:1 aus public_stats.py.

use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Duration;
use dl_core::pyfloat::py_round;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::ranks::{detect_lane, RankResolver, LANES, RANK_COLORS, RANK_ORDER};
use crate::timeutil::{hour, isoformat, now_local, parse_iso, to_iso, weekday_mon0};
use crate::{internal_error, is_missing_table, parse_positive_int, safe_int, SharedApp};

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

/// `GET /api/activity-heatmap` — 7×24-Buckets pro Rang, letzte 14 Tage.
pub async fn handle_activity_heatmap(State(app): State<SharedApp>) -> Response {
    let now = now_local();
    let cutoff = isoformat(now - Duration::days(14));
    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT started_at, user_id FROM voice_session_log
                  WHERE started_at >= ?1 ORDER BY started_at",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([&cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;

            let mut resolver = RankResolver::new(conn);
            // counts[rank][day][hour] — zählt Zeilen (wie Python len(list))
            let mut counts: HashMap<&str, [[i64; 24]; 7]> =
                RANK_ORDER.iter().map(|r| (*r, [[0; 24]; 7])).collect();
            for (started_at, user_id) in &rows {
                let Some(started) = parse_iso(started_at) else {
                    continue;
                };
                let Some(rank) = resolver.rank_of(*user_id)? else {
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
            Ok(heatmap)
        })
        .await;

    match result {
        Ok(heatmap) => Json(json!({
            "heatmap": heatmap,
            "rank_order": RANK_ORDER,
            "generated_at": isoformat(now),
        }))
        .into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/rank-distribution`.
pub async fn handle_rank_distribution(
    State(app): State<SharedApp>,
    Query(params): Params,
) -> Response {
    let now = now_local();
    let weeks = match parse_positive_int(params.get("weeks"), 4, 1, 52, "weeks") {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let result = app
        .db
        .read(move |conn| {
            let mut distribution: Map<String, Value> = RANK_ORDER
                .iter()
                .map(|r| (r.to_string(), json!(0)))
                .collect();
            let mut stmt = conn.prepare(
                "SELECT deadlock_rank_name, COUNT(*) as cnt FROM steam_links
                  WHERE verified = 1 AND deadlock_rank_name IS NOT NULL
                  GROUP BY deadlock_rank_name",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            for (name, cnt) in rows {
                let name = name.to_lowercase();
                if distribution.contains_key(&name) {
                    distribution.insert(name, json!(cnt));
                }
            }

            let mut resolver = RankResolver::new(conn);
            let mut weekly = Vec::new();
            for week in 0..weeks {
                let week_start = isoformat(now - Duration::weeks(week + 1));
                let week_end = isoformat(now - Duration::weeks(week));
                let mut stmt = conn.prepare(
                    "SELECT user_id FROM voice_session_log
                      WHERE started_at >= ?1 AND started_at < ?2",
                )?;
                let user_ids: Vec<i64> = stmt
                    .query_map([&week_start, &week_end], |row| row.get(0))?
                    .collect::<Result<_, _>>()?;
                let mut week_data: HashMap<&str, i64> =
                    RANK_ORDER.iter().map(|r| (*r, 0)).collect();
                let mut seen: HashSet<i64> = HashSet::new();
                for uid in user_ids {
                    if seen.contains(&uid) {
                        continue;
                    }
                    if let Some(rank) = resolver.rank_of(uid)? {
                        *week_data.get_mut(rank).expect("init") += 1;
                        seen.insert(uid);
                    }
                }
                let data: Map<String, Value> = RANK_ORDER
                    .iter()
                    .map(|r| (r.to_string(), json!(week_data[*r])))
                    .collect();
                weekly.push(json!({ "week": week + 1, "data": data }));
            }
            weekly.reverse();
            Ok((distribution, weekly))
        })
        .await;

    match result {
        Ok((distribution, weekly)) => Json(json!({
            "distribution": distribution,
            "rank_order": RANK_ORDER,
            "weekly_trend": weekly,
            "generated_at": isoformat(now),
        }))
        .into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/lane-preferences`.
pub async fn handle_lane_preferences(State(app): State<SharedApp>) -> Response {
    let now = now_local();
    let cutoff = isoformat(now - Duration::days(30));
    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT user_id, channel_name FROM voice_session_log WHERE started_at >= ?1",
            )?;
            let rows: Vec<(i64, Option<String>)> = stmt
                .query_map([&cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;

            let mut resolver = RankResolver::new(conn);
            let mut data: HashMap<&str, HashMap<&str, i64>> = RANK_ORDER
                .iter()
                .map(|r| (*r, LANES.iter().map(|l| (*l, 0)).collect()))
                .collect();
            let mut seen_per_lane: HashMap<&str, HashSet<i64>> =
                LANES.iter().map(|l| (*l, HashSet::new())).collect();

            for (user_id, channel_name) in &rows {
                let lane = detect_lane(channel_name.as_deref()).unwrap_or("unknown");
                let seen = seen_per_lane.get_mut(lane).expect("init");
                if seen.contains(user_id) {
                    continue;
                }
                if let Some(rank) = resolver.rank_of(*user_id)? {
                    *data
                        .get_mut(rank)
                        .expect("init")
                        .get_mut(lane)
                        .expect("init") += 1;
                    seen.insert(*user_id);
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
            Ok(preferences)
        })
        .await;

    match result {
        Ok(preferences) => Json(json!({
            "preferences": preferences,
            "rank_order": RANK_ORDER,
            "lanes": LANES,
            "generated_at": isoformat(now),
        }))
        .into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/new-player-windows`.
pub async fn handle_new_player_windows(State(app): State<SharedApp>) -> Response {
    let now = now_local();
    let cutoff = isoformat(now - Duration::days(30));
    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT started_at, channel_name FROM voice_session_log WHERE started_at >= ?1",
            )?;
            let rows: Vec<(String, Option<String>)> = stmt
                .query_map([&cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;

            let mut hour_counts = [0i64; 24];
            let mut day_hour = [[0i64; 24]; 7];
            for (started_at, channel_name) in &rows {
                let Some(started) = parse_iso(started_at) else {
                    continue;
                };
                if detect_lane(channel_name.as_deref()) == Some("new_player") {
                    let (d, h) = (weekday_mon0(&started), hour(&started));
                    day_hour[d][h] += 1;
                    hour_counts[h] += 1;
                }
            }
            Ok((hour_counts, day_hour))
        })
        .await;

    let (hour_counts, day_hour) = match result {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

    // Top 5 Stunden (stabil: bei Gleichstand frühere Stunde zuerst, wie Python)
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
    let cutoff = isoformat(now - Duration::days(days));
    let hours_metric = metric == "hours";

    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT started_at, user_id, duration_seconds FROM voice_session_log
                  WHERE started_at >= ?1 ORDER BY started_at",
            )?;
            let rows: Vec<(String, i64, Option<f64>)> = stmt
                .query_map([&cutoff], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?;

            let mut resolver = RankResolver::new(conn);
            let mut counts: HashMap<&str, [i64; 24]> =
                RANK_ORDER.iter().map(|r| (*r, [0; 24])).collect();
            let mut hours_sum: HashMap<&str, [f64; 24]> =
                RANK_ORDER.iter().map(|r| (*r, [0.0; 24])).collect();
            for (started_at, user_id, duration) in &rows {
                let Some(started) = parse_iso(started_at) else {
                    continue;
                };
                let h = hour(&started);
                if let Some(rank) = resolver.rank_of(*user_id)? {
                    counts.get_mut(rank).expect("init")[h] += 1;
                    hours_sum.get_mut(rank).expect("init")[h] += duration.unwrap_or(0.0) / 3600.0;
                }
            }
            Ok((counts, hours_sum))
        })
        .await;

    let (counts, hours_sum) = match result {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

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
        "generated_at": isoformat(now_local()),
    }))
    .into_response()
}

/// `GET /api/best-times?rank=<rank>`.
pub async fn handle_best_times(State(app): State<SharedApp>, Query(params): Params) -> Response {
    let rank = params.get("rank").cloned().unwrap_or_default();
    if !RANK_ORDER.contains(&rank.as_str()) {
        return dl_webcore::envelope::error_code(StatusCode::BAD_REQUEST, "Invalid rank");
    }
    let now = now_local();
    let cutoff = isoformat(now - Duration::days(7));
    let wanted = rank.clone();

    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT started_at, user_id FROM voice_session_log
                  WHERE started_at >= ?1 ORDER BY started_at",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([&cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;

            let mut resolver = RankResolver::new(conn);
            let mut hourly = [0i64; 24];
            let mut day_hour = [[0i64; 24]; 7];
            for (started_at, user_id) in &rows {
                let Some(started) = parse_iso(started_at) else {
                    continue;
                };
                if resolver.rank_of(*user_id)? == Some(wanted.as_str()) {
                    hourly[hour(&started)] += 1;
                    day_hour[weekday_mon0(&started)][hour(&started)] += 1;
                }
            }
            Ok((hourly, day_hour))
        })
        .await;

    let (hourly, day_hour) = match result {
        Ok(v) => v,
        Err(err) => return db_error(err),
    };

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
    let cutoff = format!("-{days} day");
    let now = now_local();
    let mode_for_db = mode.clone();

    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT date(started_at) AS day,
                        SUM(duration_seconds) AS total_seconds,
                        COUNT(*) AS sessions,
                        COUNT(DISTINCT user_id) AS unique_users
                   FROM voice_session_log
                  WHERE started_at >= datetime('now', ?1)
                  GROUP BY date(started_at) ORDER BY day DESC",
            )?;
            let daily: Vec<Value> = stmt
                .query_map([&cutoff], |row| {
                    Ok(json!({
                        "day": row.get::<_, Option<String>>(0)?,
                        "total_seconds": row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        "sessions": row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        "unique_users": row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    }))
                })?
                .collect::<Result<_, _>>()?;

            let (total_sessions, total_seconds, total_users): (i64, i64, i64) = conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(duration_seconds), 0), COUNT(DISTINCT user_id)
                   FROM voice_session_log WHERE started_at >= datetime('now', ?1)",
                [&cutoff],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;

            let mut stmt = conn.prepare(
                "WITH grouped AS (
                    SELECT CASE
                        WHEN ?1 = 'hour' THEN strftime('%H', started_at)
                        WHEN ?1 = 'day' THEN strftime('%w', started_at)
                        WHEN ?1 = 'week' THEN strftime('%Y-%W', started_at)
                        ELSE strftime('%Y-%m', started_at)
                    END AS bucket, duration_seconds
                    FROM voice_session_log WHERE started_at >= datetime('now', ?2)
                 )
                 SELECT bucket, SUM(duration_seconds) AS total_seconds, COUNT(*) AS sessions
                   FROM grouped GROUP BY bucket ORDER BY bucket",
            )?;
            let hourly: Vec<Value> = stmt
                .query_map(rusqlite::params![&mode_for_db, &cutoff], |row| {
                    let total_s: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
                    let sessions: i64 = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
                    let avg = if sessions > 0 {
                        json!(py_round(total_s as f64 / sessions as f64 / 60.0, 1))
                    } else {
                        json!(0)
                    };
                    Ok(json!({
                        "bucket": row.get::<_, Option<String>>(0)?,
                        "total_seconds": total_s,
                        "sessions": sessions,
                        "avg_session_minutes": avg,
                    }))
                })?
                .collect::<Result<_, _>>()?;

            Ok((daily, total_sessions, total_seconds, total_users, hourly))
        })
        .await;

    match result {
        Ok((daily, total_sessions, total_seconds, total_users, hourly)) => {
            let avg = if total_sessions > 0 {
                json!(py_round(
                    total_seconds as f64 / total_sessions as f64 / 60.0,
                    1
                ))
            } else {
                json!(0)
            };
            Json(json!({
                "summary": {
                    "total_sessions": total_sessions,
                    "total_seconds": total_seconds,
                    "total_users": total_users,
                    "avg_session_minutes": avg,
                    "days": days,
                },
                "daily": daily,
                "hourly": hourly,
                "mode": mode,
                "generated_at": isoformat(now),
            }))
            .into_response()
        }
        Err(err) => {
            tracing::error!(%err, "voice-history fehlgeschlagen");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Voice history unavailable" })),
            )
                .into_response()
        }
    }
}

/// Display-Namen aus member_events (optionale Tabelle).
pub(crate) fn resolve_display_names(
    conn: &Connection,
    user_ids: &[i64],
) -> rusqlite::Result<HashMap<i64, String>> {
    let mut names = HashMap::new();
    let mut unique: Vec<i64> = user_ids
        .iter()
        .copied()
        .filter(|id| *id != 0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unique.sort_unstable();

    let mut stmt = match conn.prepare(
        "SELECT display_name FROM member_events
          WHERE user_id = ?1 AND display_name IS NOT NULL AND TRIM(display_name) != ''
          ORDER BY datetime(timestamp) DESC, id DESC LIMIT 1",
    ) {
        Ok(stmt) => stmt,
        Err(err) if err.to_string().to_lowercase().contains("no such table") => {
            for uid in unique {
                names.insert(uid, format!("User {uid}"));
            }
            return Ok(names);
        }
        Err(err) => return Err(err),
    };
    for uid in unique {
        let name: Option<String> =
            stmt.query_row([uid], |row| row.get(0))
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
        names.insert(uid, name.unwrap_or_else(|| format!("User {uid}")));
    }
    Ok(names)
}

fn updated_at(conn: &Connection, table: &str) -> String {
    let value: Option<rusqlite::types::Value> = conn
        .query_row(
            &format!("SELECT MAX(last_update) AS updated_at FROM {table}"),
            [],
            |row| row.get(0),
        )
        .ok();
    value
        .and_then(to_iso)
        .unwrap_or_else(|| isoformat(now_local()))
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
    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT user_id, total_seconds, total_points FROM voice_stats
                  ORDER BY total_points DESC, total_seconds DESC LIMIT ?1",
            )?;
            let rows: Vec<(
                rusqlite::types::Value,
                rusqlite::types::Value,
                rusqlite::types::Value,
            )> = stmt
                .query_map([limit], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?;
            let ids: Vec<i64> = rows.iter().map(|(id, _, _)| safe_int(id)).collect();
            let names = resolve_display_names(conn, &ids)?;
            let entries: Vec<Value> = rows
                .iter()
                .enumerate()
                .map(|(index, (id, seconds, points))| {
                    let uid = safe_int(id);
                    let total_seconds = safe_int(seconds);
                    json!({
                        "rank": index + 1,
                        "user_id": uid.to_string(),
                        "name": names.get(&uid).cloned().unwrap_or_else(|| format!("User {uid}")),
                        "avatar_url": null,
                        "total_seconds": total_seconds,
                        "total_points": safe_int(points),
                        "hours": py_round(total_seconds as f64 / 3600.0, 1),
                    })
                })
                .collect();
            Ok(json!({ "entries": entries, "updated_at": updated_at(conn, "voice_stats") }))
        })
        .await;
    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => db_error(err),
    }
}

/// `GET /api/public/leaderboard/text` — text_stats ist optional.
pub async fn handle_text_leaderboard(
    State(app): State<SharedApp>,
    Query(params): Params,
) -> Response {
    let limit = match parse_positive_int(params.get("limit"), 50, 1, 100, "limit") {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let result = app
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT user_id, total_messages, total_points FROM text_stats
                  ORDER BY total_points DESC, total_messages DESC LIMIT ?1",
            )?;
            let rows: Vec<(
                rusqlite::types::Value,
                rusqlite::types::Value,
                rusqlite::types::Value,
            )> = stmt
                .query_map([limit], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?;
            let ids: Vec<i64> = rows.iter().map(|(id, _, _)| safe_int(id)).collect();
            let names = resolve_display_names(conn, &ids)?;
            let entries: Vec<Value> = rows
                .iter()
                .enumerate()
                .map(|(index, (id, messages, points))| {
                    let uid = safe_int(id);
                    json!({
                        "rank": index + 1,
                        "user_id": uid.to_string(),
                        "name": names.get(&uid).cloned().unwrap_or_else(|| format!("User {uid}")),
                        "avatar_url": null,
                        "total_messages": safe_int(messages),
                        "total_points": safe_int(points),
                    })
                })
                .collect();
            Ok(json!({ "entries": entries, "updated_at": updated_at(conn, "text_stats") }))
        })
        .await;
    match result {
        Ok(payload) => Json(payload).into_response(),
        Err(ref err) if is_missing_table(err, &["text_stats"]) => {
            Json(json!({ "entries": [], "updated_at": isoformat(now_local()) })).into_response()
        }
        Err(err) => db_error(err),
    }
}
