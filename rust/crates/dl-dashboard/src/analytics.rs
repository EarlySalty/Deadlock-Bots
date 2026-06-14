//! Lesende Analytics-Routen (`/api/...`) — Port der Dashboard-Reads.
//!
//! Reine DB-Reads auf bekannte Tabellen-Verträge, session-gegatet über
//! [`DashboardApp::guard_read`]. SQL und JSON-Form sind 1:1 zum Original
//! (`service/dashboard.py`), damit die bestehende Admin-SPA unverändert
//! weiterläuft. Namen zu User-IDs kommen über den Broker-Resolver.

use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{NaiveDateTime, Utc};
use dl_core::pyfloat::py_round;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};

use crate::names::display_name_or_default;
use crate::web::{err_text, ok_json, DashboardApp};

// ── Parameter-Helfer (Fehlertexte wortgleich) ───────────────────────────────

fn parse_limit(params: &HashMap<String, String>, default: i64, max: i64) -> Result<i64, Response> {
    match params.get("limit").map(|s| s.trim()) {
        None | Some("") => Ok(default),
        Some(raw) => match raw.parse::<i64>() {
            Ok(n) if n > 0 => Ok(n.min(max)),
            _ => Err(err_text(
                400,
                &format!("limit must be a positive integer (max {max})"),
            )),
        },
    }
}

/// `guild_id`-Parameter → Filter. Fehlt/leer/0 ⇒ kein Filter (wie Pythons
/// `guild_id if guild_id else None`).
fn parse_guild_filter(params: &HashMap<String, String>) -> Result<Option<i64>, Response> {
    match params.get("guild_id").map(|s| s.trim()) {
        None | Some("") => Ok(None),
        Some(raw) => raw
            .parse::<i64>()
            .map(|v| (v != 0).then_some(v))
            .map_err(|_| err_text(400, "guild_id must be an integer")),
    }
}

// ── /api/member-events ──────────────────────────────────────────────────────

pub async fn member_events(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    let limit = match parse_limit(&params, 50, 200) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match parse_guild_filter(&params) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let event_filter = params
        .get("type")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let result = app
        .db()
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, guild_id, event_type, timestamp,
                        display_name, account_created_at, join_position, metadata
                 FROM member_events
                 WHERE (? IS NULL OR guild_id = ?)
                   AND (? IS NULL OR event_type = ?)
                 ORDER BY timestamp DESC
                 LIMIT ?",
            )?;
            let events: Vec<Value> = stmt
                .query_map(
                    params![guild, guild, event_filter, event_filter, limit],
                    |row| {
                        Ok(json!({
                            "id": row.get::<_, Option<i64>>(0)?,
                            "user_id": row.get::<_, Option<i64>>(1)?,
                            "guild_id": row.get::<_, Option<i64>>(2)?,
                            "event_type": row.get::<_, Option<String>>(3)?,
                            "timestamp": row.get::<_, Option<String>>(4)?,
                            "display_name": row.get::<_, Option<String>>(5)?,
                            "account_created_at": row.get::<_, Option<String>>(6)?,
                            "join_position": row.get::<_, Option<i64>>(7)?,
                            "metadata": row.get::<_, Option<String>>(8)?,
                        }))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let mut counts_stmt = conn.prepare(
                "SELECT event_type, COUNT(*) as count
                 FROM member_events
                 WHERE (? IS NULL OR guild_id = ?)
                   AND (? IS NULL OR event_type = ?)
                 GROUP BY event_type
                 ORDER BY count DESC",
            )?;
            let mut counts = Map::new();
            let mut rows = counts_stmt.query(params![guild, guild, event_filter, event_filter])?;
            while let Some(row) = rows.next()? {
                let key: Option<String> = row.get(0)?;
                let value: i64 = row.get(1)?;
                counts.insert(key.unwrap_or_default(), json!(value));
            }

            let recent_joins: i64 = conn.query_row(
                "SELECT COUNT(*) FROM member_events
                 WHERE event_type = 'join'
                   AND timestamp >= datetime('now', '-7 days')
                   AND (? IS NULL OR guild_id = ?)",
                params![guild, guild],
                |row| row.get(0),
            )?;
            let recent_leaves: i64 = conn.query_row(
                "SELECT COUNT(*) FROM member_events
                 WHERE event_type = 'leave'
                   AND timestamp >= datetime('now', '-7 days')
                   AND (? IS NULL OR guild_id = ?)",
                params![guild, guild],
                |row| row.get(0),
            )?;

            Ok(json!({
                "events": events,
                "summary": {
                    "total_events": events.len(),
                    "event_counts": counts,
                    "recent_joins_7d": recent_joins,
                    "recent_leaves_7d": recent_leaves,
                },
            }))
        })
        .await;

    match result {
        Ok(payload) => ok_json(payload),
        Err(err) => {
            tracing::error!(%err, "member_events fehlgeschlagen");
            err_text(500, "Member events unavailable")
        }
    }
}

// ── /api/message-activity ───────────────────────────────────────────────────

struct MaRow {
    user_id: i64,
    guild_id: Option<i64>,
    channel_id: Option<i64>,
    message_count: i64,
    last_message_at: Option<String>,
    first_message_at: Option<String>,
}

struct MaSummary {
    total_users: i64,
    total_messages: Option<i64>,
    avg: Option<f64>,
}

pub async fn message_activity(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    let limit = match parse_limit(&params, 20, 100) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match parse_guild_filter(&params) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let read = app
        .db()
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT user_id, guild_id, channel_id, message_count,
                        last_message_at, first_message_at
                 FROM message_activity
                 WHERE (? IS NULL OR guild_id = ?)
                 ORDER BY message_count DESC
                 LIMIT ?",
            )?;
            let rows: Vec<MaRow> = stmt
                .query_map(params![guild, guild, limit], |row| {
                    Ok(MaRow {
                        user_id: row.get(0)?,
                        guild_id: row.get(1)?,
                        channel_id: row.get(2)?,
                        message_count: row.get(3)?,
                        last_message_at: row.get(4)?,
                        first_message_at: row.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let summary = conn.query_row(
                "SELECT COUNT(*), SUM(message_count), AVG(message_count)
                 FROM message_activity
                 WHERE (? IS NULL OR guild_id = ?)",
                params![guild, guild],
                |row| {
                    Ok(MaSummary {
                        total_users: row.get(0)?,
                        total_messages: row.get(1)?,
                        avg: row.get(2)?,
                    })
                },
            )?;
            Ok((rows, summary))
        })
        .await;

    let (rows, summary) = match read {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "message_activity fehlgeschlagen");
            return err_text(500, "Message activity unavailable");
        }
    };

    let ids: Vec<u64> = rows
        .iter()
        .filter_map(|r| u64::try_from(r.user_id).ok())
        .collect();
    let names = app.names().resolve(&ids).await;

    let users: Vec<Value> = rows
        .iter()
        .map(|r| {
            let uid = u64::try_from(r.user_id).unwrap_or(0);
            json!({
                "user_id": r.user_id,
                "display_name": display_name_or_default(&names, uid),
                "guild_id": r.guild_id,
                "channel_id": r.channel_id,
                "message_count": r.message_count,
                "last_message_at": r.last_message_at,
                "first_message_at": r.first_message_at,
            })
        })
        .collect();

    // avg_per_user: round(avg, 1), aber 0 (Integer) wenn leer/0 — wie Python.
    let avg_value = match summary.avg {
        Some(a) if a != 0.0 => json!(py_round(a, 1)),
        _ => json!(0),
    };

    ok_json(json!({
        "top_users": users,
        "summary": {
            "total_users": summary.total_users,
            "total_messages": summary.total_messages,
            "avg_per_user": avg_value,
        },
    }))
}

// ── /api/leave-surveys (Admin-Übersicht) ────────────────────────────────────

/// Parst `web_payload` wie `_parse_leave_survey_json`: JSON → Wert, sonst Null.
fn parse_survey_json(raw: Option<String>) -> Value {
    match raw {
        None => Value::Null,
        Some(text) if text.trim().is_empty() => Value::Null,
        Some(text) => serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
    }
}

pub async fn leave_surveys(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    let result = app
        .db()
        .read(move |conn| {
            let (total, responded_count, web_count): (i64, Option<i64>, Option<i64>) = conn
                .query_row(
                    "SELECT COUNT(*),
                            SUM(CASE WHEN responded_at IS NOT NULL THEN 1 ELSE 0 END),
                            SUM(CASE WHEN web_submitted_at IS NOT NULL THEN 1 ELSE 0 END)
                     FROM member_leave_surveys",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;

            let count_map = |sql: &str| -> rusqlite::Result<Map<String, Value>> {
                let mut stmt = conn.prepare(sql)?;
                let mut out = Map::new();
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    let count: i64 = row.get(1)?;
                    out.insert(key, json!(count));
                }
                Ok(out)
            };
            let by_user_bucket = count_map(
                "SELECT COALESCE(NULLIF(user_bucket, ''), 'unknown') AS user_bucket, COUNT(*)
                 FROM member_leave_surveys
                 GROUP BY COALESCE(NULLIF(user_bucket, ''), 'unknown')
                 ORDER BY user_bucket ASC",
            )?;
            let by_dm_status = count_map(
                "SELECT COALESCE(NULLIF(dm_status, ''), 'unknown') AS dm_status, COUNT(*)
                 FROM member_leave_surveys
                 GROUP BY COALESCE(NULLIF(dm_status, ''), 'unknown')
                 ORDER BY dm_status ASC",
            )?;

            let mut by_reason = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT COALESCE(NULLIF(reason_code, ''), 'unknown') AS reason_code, COUNT(*)
                     FROM member_leave_surveys
                     GROUP BY COALESCE(NULLIF(reason_code, ''), 'unknown')
                     ORDER BY COUNT(*) DESC, reason_code ASC",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let reason: String = row.get(0)?;
                    let count: i64 = row.get(1)?;
                    by_reason.push(json!({ "reason_code": reason, "count": count }));
                }
            }

            let mut recent = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT display_name, survey_token, user_bucket, reason_code,
                            follow_up_question, follow_up_text, extra_text,
                            web_submitted_at, web_payload, left_at, responded_at
                     FROM member_leave_surveys
                     WHERE responded_at IS NOT NULL OR web_submitted_at IS NOT NULL
                     ORDER BY COALESCE(web_submitted_at, responded_at) DESC, id DESC
                     LIMIT 30",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let web_payload: Option<String> = row.get(8)?;
                    recent.push(json!({
                        "display_name": row.get::<_, Option<String>>(0)?,
                        "survey_token": row.get::<_, Option<String>>(1)?,
                        "user_bucket": row.get::<_, Option<String>>(2)?,
                        "reason_code": row.get::<_, Option<String>>(3)?,
                        "follow_up_question": row.get::<_, Option<String>>(4)?,
                        "follow_up_text": row.get::<_, Option<String>>(5)?,
                        "extra_text": row.get::<_, Option<String>>(6)?,
                        "web_submitted_at": row.get::<_, Option<i64>>(7)?,
                        "web_payload": parse_survey_json(web_payload),
                        "left_at": row.get::<_, Option<i64>>(9)?,
                        "responded_at": row.get::<_, Option<i64>>(10)?,
                    }));
                }
            }

            let responded = responded_count.unwrap_or(0);
            let web = web_count.unwrap_or(0);
            let rate = |count: i64| -> Value {
                if total > 0 {
                    json!(count as f64 / total as f64)
                } else {
                    json!(0)
                }
            };

            Ok(json!({
                "totals": {
                    "total": total,
                    "by_user_bucket": by_user_bucket,
                    "by_dm_status": by_dm_status,
                },
                "response_rate": {
                    "dm": { "count": responded, "rate": rate(responded) },
                    "web": { "count": web, "rate": rate(web) },
                },
                "by_reason": by_reason,
                "recent": recent,
            }))
        })
        .await;

    match result {
        Ok(payload) => ok_json(payload),
        Err(err) => {
            tracing::error!(%err, "leave_surveys fehlgeschlagen");
            err_text(500, "Failed to load leave surveys data")
        }
    }
}

// ── /api/co-player-network ──────────────────────────────────────────────────

struct CpRow {
    user_id: i64,
    co_player_id: i64,
    sessions: i64,
    minutes: i64,
    last_played: Option<String>,
    user_name: Option<String>,
    co_name: Option<String>,
}

struct Edge {
    sessions: i64,
    minutes: i64,
    last_played: Option<String>,
    last_played_ts: f64,
}

/// ISO/`YYYY-MM-DD HH:MM:SS`-Zeitstempel → Unix-Sekunden (sonst 0.0), wie das
/// inline-`_ts` im Original (nur für „neuestes last_played"-Vergleich).
fn parse_ts(value: &Option<String>) -> f64 {
    let Some(text) = value.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return 0.0;
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(text) {
        return dt.timestamp() as f64;
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(text, fmt) {
            return dt.and_utc().timestamp() as f64;
        }
    }
    0.0
}

fn parse_min_sessions(params: &HashMap<String, String>) -> Result<i64, Response> {
    match params.get("min_sessions").map(|s| s.trim()) {
        None | Some("") => Ok(1),
        Some(raw) => match raw.parse::<i64>() {
            Ok(n) if n > 0 => Ok(n.min(100_000)),
            _ => Err(err_text(400, "min_sessions must be a positive integer")),
        },
    }
}

pub async fn co_player_network(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    let limit = match parse_limit(&params, 120, 400) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let min_sessions = match parse_min_sessions(&params) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let read = app
        .db()
        .read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT user_id, co_player_id, sessions_together, total_minutes_together,
                        last_played_together, user_display_name, co_player_display_name
                 FROM user_co_players
                 WHERE sessions_together >= ?
                 ORDER BY sessions_together DESC, total_minutes_together DESC,
                          last_played_together DESC
                 LIMIT ?",
            )?;
            let rows: Vec<CpRow> = stmt
                .query_map(params![min_sessions, limit * 2], |row| {
                    Ok(CpRow {
                        user_id: row.get(0)?,
                        co_player_id: row.get(1)?,
                        sessions: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        minutes: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        last_played: row.get(4)?,
                        user_name: row.get(5)?,
                        co_name: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await;

    let rows = match read {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "co_player_network fehlgeschlagen");
            return err_text(500, "Co-player network unavailable");
        }
    };

    // Bidirektionale Kanten auf kanonische (min,max)-Schlüssel mergen.
    let mut edges: HashMap<(u64, u64), Edge> = HashMap::new();
    let mut db_names: HashMap<u64, String> = HashMap::new();
    for row in &rows {
        let (Ok(uid), Ok(coid)) = (u64::try_from(row.user_id), u64::try_from(row.co_player_id))
        else {
            continue;
        };
        if let Some(name) = row.user_name.as_deref().filter(|s| !s.is_empty()) {
            db_names.entry(uid).or_insert_with(|| name.to_string());
        }
        if let Some(name) = row.co_name.as_deref().filter(|s| !s.is_empty()) {
            db_names.entry(coid).or_insert_with(|| name.to_string());
        }
        let key = if uid < coid { (uid, coid) } else { (coid, uid) };
        let ts = parse_ts(&row.last_played);
        let edge = edges.entry(key).or_insert(Edge {
            sessions: 0,
            minutes: 0,
            last_played: None,
            last_played_ts: f64::NEG_INFINITY,
        });
        edge.sessions = edge.sessions.max(row.sessions);
        edge.minutes = edge.minutes.max(row.minutes);
        if ts > edge.last_played_ts {
            edge.last_played_ts = ts;
            edge.last_played = row.last_played.clone();
        }
    }

    let total_edges = edges.len();
    // Sortieren wie das Original (sessions, minutes) absteigend; stabile
    // Tiebreaker über die kanonischen IDs für Determinismus.
    let mut sorted: Vec<((u64, u64), Edge)> = edges.into_iter().collect();
    sorted.sort_by(|a, b| {
        b.1.sessions
            .cmp(&a.1.sessions)
            .then(b.1.minutes.cmp(&a.1.minutes))
            .then(b.1.last_played_ts.total_cmp(&a.1.last_played_ts))
            .then(a.0 .0.cmp(&b.0 .0))
            .then(a.0 .1.cmp(&b.0 .1))
    });
    sorted.truncate(limit as usize);

    // Namen für die Knoten der getrimmten Kanten: DB-Namen, fehlende per Broker.
    let node_ids: HashSet<u64> = sorted.iter().flat_map(|((s, t), _)| [*s, *t]).collect();
    let missing: Vec<u64> = node_ids
        .iter()
        .filter(|id| !db_names.contains_key(id))
        .copied()
        .collect();
    let mut name_map = db_names;
    if !missing.is_empty() {
        for (id, name) in app.names().resolve(&missing).await {
            name_map.entry(id).or_insert(name);
        }
    }

    // Knoten aus den getrimmten Kanten akkumulieren.
    struct Node {
        sessions: i64,
        minutes: i64,
        degree: i64,
    }
    let mut nodes: HashMap<u64, Node> = HashMap::new();
    let mut links = Vec::with_capacity(sorted.len());
    for ((source, target), edge) in &sorted {
        for id in [*source, *target] {
            let node = nodes.entry(id).or_insert(Node {
                sessions: 0,
                minutes: 0,
                degree: 0,
            });
            node.degree += 1;
            node.sessions += edge.sessions;
            node.minutes += edge.minutes;
        }
        links.push(json!({
            "source": source,
            "target": target,
            "sessions": edge.sessions,
            "minutes": edge.minutes,
            "last_played": edge.last_played,
        }));
    }

    let mut node_list: Vec<(u64, Node)> = nodes.into_iter().collect();
    node_list.sort_by(|a, b| b.1.sessions.cmp(&a.1.sessions).then(a.0.cmp(&b.0)));
    let nodes_json: Vec<Value> = node_list
        .iter()
        .map(|(id, node)| {
            json!({
                "id": id,
                "name": display_name_or_default(&name_map, *id),
                "sessions": node.sessions,
                "minutes": node.minutes,
                "degree": node.degree,
                "weight": node.sessions.max(node.minutes / 10),
            })
        })
        .collect();

    let generated_at = format!(
        "{}Z",
        Utc::now().naive_utc().format("%Y-%m-%dT%H:%M:%S%.6f")
    );
    ok_json(json!({
        "nodes": nodes_json,
        "links": links,
        "meta": {
            "total_edges": total_edges,
            "returned_edges": sorted.len(),
            "total_nodes": node_list.len(),
            "generated_at": generated_at,
            "min_sessions": min_sessions,
        },
    }))
}

// ── /api/voice-history ──────────────────────────────────────────────────────

const WEEKDAYS_DE: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];

fn parse_capped(
    params: &HashMap<String, String>,
    key: &str,
    default: i64,
    max: i64,
    err: &str,
) -> Result<i64, Response> {
    match params.get(key).map(|s| s.trim()) {
        None | Some("") => Ok(default),
        Some(raw) => match raw.parse::<i64>() {
            Ok(n) if n > 0 => Ok(n.min(max)),
            _ => Err(err_text(400, err)),
        },
    }
}

/// `num/den` als Float, sonst Integer-`0` — wie die avg-Felder im Original
/// ( KEIN Runden).
fn div_or_zero(num: i64, den: i64) -> Value {
    if den > 0 {
        json!(num as f64 / den as f64)
    } else {
        json!(0)
    }
}

struct VhDaily {
    day: Option<String>,
    total_seconds: i64,
    sessions: i64,
    users: i64,
}
struct VhTop {
    user_id: i64,
    display_name: Option<String>,
    total_seconds: i64,
    total_points: i64,
    sessions: i64,
}
struct VhBucket {
    bucket: Option<String>,
    total_seconds: i64,
    sessions: i64,
    sum_peak: i64,
}
struct VhRange {
    total_seconds: i64,
    total_points: i64,
    sessions: i64,
    sum_peak: i64,
    active_days: i64,
    last_session: Option<String>,
}
struct VhLifetime {
    total_seconds: i64,
    total_points: i64,
    last_update: Option<String>,
}
struct VhRecent {
    id: i64,
    guild_id: Option<i64>,
    channel_id: Option<i64>,
    channel_name: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
    duration_seconds: i64,
    points: i64,
    peak_users: i64,
    co_player_ids: Option<String>,
}
struct VhUserData {
    range: VhRange,
    lifetime: Option<VhLifetime>,
    lifetime_sessions: i64,
    lifetime_last_session: Option<String>,
    recent: Vec<VhRecent>,
}
struct VhData {
    daily: Vec<VhDaily>,
    top: Vec<VhTop>,
    buckets: Vec<VhBucket>,
    user: Option<VhUserData>,
}

pub async fn voice_history(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    let days = match parse_capped(
        &params,
        "range",
        14,
        90,
        "range must be a positive integer (days, max 90)",
    ) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let top_limit = match parse_capped(
        &params,
        "top",
        10,
        50,
        "top must be a positive integer (max 50)",
    ) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let recent_limit = match parse_capped(
        &params,
        "sessions",
        12,
        50,
        "sessions must be a positive integer (max 50)",
    ) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let mode = params
        .get("mode")
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "hour".to_string());
    if !matches!(mode.as_str(), "hour" | "day" | "week" | "month") {
        return err_text(400, "mode must be one of hour, day, week, month");
    }
    let user_id: Option<i64> = match params
        .get("user_id")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => match raw.parse::<i64>() {
            Ok(v) => Some(v),
            Err(_) => return err_text(400, "user_id must be an integer"),
        },
    };
    let cutoff = format!("-{days} day");
    let mode_sql = mode.clone();

    let read = app
        .db()
        .read(move |conn| {
            let daily = {
                let mut stmt = conn.prepare(
                    "SELECT date(started_at) AS day, SUM(duration_seconds), COUNT(*),
                            COUNT(DISTINCT user_id)
                     FROM voice_session_log
                     WHERE started_at >= datetime('now', ?)
                     GROUP BY date(started_at)
                     ORDER BY day DESC",
                )?;
                let rows = stmt
                    .query_map(params![cutoff], |r| {
                        Ok(VhDaily {
                            day: r.get(0)?,
                            total_seconds: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            sessions: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            users: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            let top = {
                let mut stmt = conn.prepare(
                    "SELECT user_id, MAX(display_name), SUM(duration_seconds),
                            SUM(points), COUNT(*)
                     FROM voice_session_log
                     WHERE started_at >= datetime('now', ?)
                       AND (? IS NULL OR user_id = ?)
                     GROUP BY user_id
                     ORDER BY SUM(duration_seconds) DESC, SUM(points) DESC
                     LIMIT ?",
                )?;
                let rows = stmt
                    .query_map(params![cutoff, user_id, user_id, top_limit], |r| {
                        Ok(VhTop {
                            user_id: r.get(0)?,
                            display_name: r.get(1)?,
                            total_seconds: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            total_points: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                            sessions: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            let buckets = {
                let mut stmt = conn.prepare(
                    "WITH grouped AS (
                        SELECT CASE
                                 WHEN ? = 'hour' THEN strftime('%H', started_at)
                                 WHEN ? = 'day' THEN strftime('%w', started_at)
                                 WHEN ? = 'week' THEN strftime('%Y-%W', started_at)
                                 ELSE strftime('%Y-%m', started_at)
                               END AS bucket,
                               duration_seconds,
                               COALESCE(peak_users, 0) AS peak_users
                        FROM voice_session_log
                        WHERE started_at >= datetime('now', ?)
                          AND (? IS NULL OR user_id = ?)
                     )
                     SELECT bucket, SUM(duration_seconds), COUNT(*), SUM(peak_users)
                     FROM grouped GROUP BY bucket ORDER BY bucket",
                )?;
                let rows = stmt
                    .query_map(
                        params![mode_sql, mode_sql, mode_sql, cutoff, user_id, user_id],
                        |r| {
                            Ok(VhBucket {
                                bucket: r.get(0)?,
                                total_seconds: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                                sessions: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                                sum_peak: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                            })
                        },
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            let user = if let Some(uid) = user_id {
                let range = conn.query_row(
                    "SELECT SUM(duration_seconds), SUM(points), COUNT(*),
                            SUM(COALESCE(peak_users, 0)),
                            COUNT(DISTINCT date(started_at)), MAX(ended_at)
                     FROM voice_session_log
                     WHERE started_at >= datetime('now', ?) AND (? IS NULL OR user_id = ?)",
                    params![cutoff, user_id, user_id],
                    |r| {
                        Ok(VhRange {
                            total_seconds: r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                            total_points: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            sessions: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            sum_peak: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                            active_days: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                            last_session: r.get(5)?,
                        })
                    },
                )?;
                let lifetime = conn
                    .query_row(
                        "SELECT total_seconds, total_points, last_update
                         FROM voice_stats WHERE user_id = ?",
                        params![uid],
                        |r| {
                            Ok(VhLifetime {
                                total_seconds: r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                                total_points: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                                last_update: r.get(2)?,
                            })
                        },
                    )
                    .optional()?;
                let (lifetime_sessions, lifetime_last_session): (i64, Option<String>) = conn
                    .query_row(
                        "SELECT COUNT(*), MAX(ended_at) FROM voice_session_log WHERE user_id = ?",
                        params![uid],
                        |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get(1)?)),
                    )?;
                let recent = {
                    let mut stmt = conn.prepare(
                        "SELECT id, guild_id, channel_id, channel_name, started_at, ended_at,
                                duration_seconds, points, peak_users, co_player_ids
                         FROM voice_session_log
                         WHERE user_id = ?
                         ORDER BY datetime(ended_at) DESC, id DESC
                         LIMIT ?",
                    )?;
                    let rows = stmt
                        .query_map(params![uid, recent_limit], |r| {
                            Ok(VhRecent {
                                id: r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                                guild_id: r.get(1)?,
                                channel_id: r.get(2)?,
                                channel_name: r.get(3)?,
                                started_at: r.get(4)?,
                                ended_at: r.get(5)?,
                                duration_seconds: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                                points: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                                peak_users: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                                co_player_ids: r.get(9)?,
                            })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    rows
                };
                Some(VhUserData {
                    range,
                    lifetime,
                    lifetime_sessions,
                    lifetime_last_session,
                    recent,
                })
            } else {
                None
            };

            Ok(VhData {
                daily,
                top,
                buckets,
                user,
            })
        })
        .await;

    let data = match read {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "voice_history fehlgeschlagen");
            return err_text(500, "Voice history unavailable");
        }
    };

    // Namen: Top-User-IDs (+ ggf. der gefragte User) über den Broker.
    let mut name_ids: Vec<u64> = data
        .top
        .iter()
        .filter_map(|t| u64::try_from(t.user_id).ok())
        .collect();
    if let Some(uid) = user_id.filter(|v| *v != 0) {
        if let Ok(u) = u64::try_from(uid) {
            name_ids.push(u);
        }
    }
    let name_map = app.names().resolve(&name_ids).await;

    // buckets bauen + nach mode auffüllen.
    let mut raw_buckets: Vec<(String, Value, i64, i64)> = data
        .buckets
        .iter()
        .map(|b| {
            let label = b.bucket.clone().unwrap_or_default();
            (
                label,
                div_or_zero(b.sum_peak, b.sessions),
                b.total_seconds,
                b.sessions,
            )
        })
        .collect();
    let buckets_json: Vec<Value> = match mode.as_str() {
        "hour" => {
            let existing: HashMap<String, (Value, i64, i64)> = raw_buckets
                .drain(..)
                .map(|(label, avg, secs, sess)| (label, (avg, secs, sess)))
                .collect();
            (0..24)
                .map(|h| {
                    let key = format!("{h:02}");
                    match existing.get(&key) {
                        Some((avg, secs, sess)) => json!({
                            "label": key, "total_seconds": secs, "sessions": sess, "avg_peak": avg,
                        }),
                        None => json!({
                            "label": key, "total_seconds": 0, "sessions": 0, "avg_peak": 0,
                        }),
                    }
                })
                .collect()
        }
        "day" => {
            let existing: HashMap<String, (Value, i64, i64)> = raw_buckets
                .drain(..)
                .map(|(label, avg, secs, sess)| (label, (avg, secs, sess)))
                .collect();
            (0..7)
                .map(|d| {
                    let key = d.to_string();
                    let (avg, secs, sess) = existing.get(&key).cloned().unwrap_or((json!(0), 0, 0));
                    json!({
                        "label": WEEKDAYS_DE[d as usize],
                        "total_seconds": secs,
                        "sessions": sess,
                        "avg_peak": avg,
                    })
                })
                .collect()
        }
        _ => raw_buckets
            .iter()
            .map(|(label, avg, secs, sess)| {
                json!({
                    "label": label, "total_seconds": secs, "sessions": sess, "avg_peak": avg,
                })
            })
            .collect(),
    };

    let daily: Vec<Value> = data
        .daily
        .iter()
        .map(|d| {
            json!({
                "day": d.day,
                "total_seconds": d.total_seconds,
                "sessions": d.sessions,
                "users": d.users,
            })
        })
        .collect();

    let top_users: Vec<Value> = data
        .top
        .iter()
        .map(|t| {
            let name = t
                .display_name
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    u64::try_from(t.user_id)
                        .map(|u| display_name_or_default(&name_map, u))
                        .unwrap_or_else(|_| format!("User {}", t.user_id))
                });
            json!({
                "user_id": t.user_id,
                "display_name": name,
                "total_seconds": t.total_seconds,
                "total_points": t.total_points,
                "sessions": t.sessions,
            })
        })
        .collect();

    // user-Feld + user_summary + recent_sessions (nur bei truthy user_id).
    let user_field = user_id.filter(|v| *v != 0).map(|uid| {
        let name = u64::try_from(uid)
            .map(|u| display_name_or_default(&name_map, u))
            .unwrap_or_else(|_| format!("User {uid}"));
        json!({ "user_id": uid, "display_name": name })
    });

    let (user_summary, recent_sessions) = match (user_id, data.user) {
        (Some(uid), Some(u)) => {
            // Co-Player-IDs je Session parsen + Namen auflösen.
            let mut per_session: Vec<Vec<i64>> = Vec::with_capacity(u.recent.len());
            let mut all_co: Vec<u64> = Vec::new();
            for row in &u.recent {
                let mut ids = Vec::new();
                let mut seen = HashSet::new();
                if let Some(raw) = &row.co_player_ids {
                    if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(raw) {
                        for v in arr {
                            let co = v
                                .as_i64()
                                .or_else(|| v.as_str().and_then(|s| s.parse().ok()));
                            if let Some(co) = co {
                                if co > 0 && co != uid && seen.insert(co) {
                                    ids.push(co);
                                    if let Ok(c) = u64::try_from(co) {
                                        all_co.push(c);
                                    }
                                }
                            }
                        }
                    }
                }
                per_session.push(ids);
            }
            let co_names = if all_co.is_empty() {
                HashMap::new()
            } else {
                app.names().resolve(&all_co).await
            };

            let recent: Vec<Value> = u
                .recent
                .iter()
                .enumerate()
                .map(|(idx, row)| {
                    let co_ids = per_session.get(idx).cloned().unwrap_or_default();
                    let co_players: Vec<Value> = co_ids
                        .iter()
                        .map(|co| {
                            let name = u64::try_from(*co)
                                .map(|c| display_name_or_default(&co_names, c))
                                .unwrap_or_else(|_| format!("User {co}"));
                            json!({ "user_id": co, "display_name": name })
                        })
                        .collect();
                    json!({
                        "id": row.id,
                        "guild_id": row.guild_id.filter(|v| *v != 0),
                        "channel_id": row.channel_id.filter(|v| *v != 0),
                        "channel_name": row.channel_name.clone().filter(|s| !s.is_empty()),
                        "started_at": row.started_at,
                        "ended_at": row.ended_at,
                        "duration_seconds": row.duration_seconds,
                        "points": row.points,
                        "peak_users": row.peak_users,
                        "co_player_count": co_ids.len(),
                        "co_players": co_players,
                    })
                })
                .collect();

            let last_session = u
                .range
                .last_session
                .clone()
                .filter(|s| !s.is_empty())
                .or(u.lifetime_last_session.clone());
            let display = u64::try_from(uid)
                .map(|u2| display_name_or_default(&name_map, u2))
                .unwrap_or_else(|_| format!("User {uid}"));
            let summary = json!({
                "user_id": uid,
                "display_name": display,
                "range_seconds": u.range.total_seconds,
                "range_points": u.range.total_points,
                "range_sessions": u.range.sessions,
                "range_days": u.range.active_days,
                "range_avg_session_seconds": div_or_zero(u.range.total_seconds, u.range.sessions),
                "range_avg_peak": div_or_zero(u.range.sum_peak, u.range.sessions),
                "lifetime_seconds": u.lifetime.as_ref().map(|l| l.total_seconds).unwrap_or(0),
                "lifetime_points": u.lifetime.as_ref().map(|l| l.total_points).unwrap_or(0),
                "lifetime_sessions": u.lifetime_sessions,
                "lifetime_last_update": u.lifetime.as_ref().and_then(|l| l.last_update.clone()),
                "last_session": last_session,
            });
            (summary, recent)
        }
        _ => (Value::Null, Vec::new()),
    };

    ok_json(json!({
        "range_days": days,
        "mode": mode,
        "user": user_field,
        "daily": daily,
        "top_users": top_users,
        "buckets": buckets_json,
        "user_summary": user_summary,
        "recent_sessions_limit": recent_limit,
        "recent_sessions": recent_sessions,
    }))
}

/// `GET /api/user-retention` — Retention-Kennzahlen + Inaktiv-Kandidaten
/// (Port von `_handle_user_retention`). Schwellen wie `RetentionConfig`.
/// v1-Vereinfachung ggü. Python: der Ausschluss-Rollen-Filter (Bot-Cache)
/// entfällt — Kandidaten werden rein über die DB-Schwellen bestimmt. Beide
/// Retention-Tabellen werden existenz-geschützt gelesen.
pub async fn user_retention(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    const MIN_WEEKLY: f64 = 0.5;
    const MIN_DAYS: i64 = 3;
    const INACTIVITY: i64 = 14;
    const MIN_BETWEEN: i64 = 30;
    const MAX_MISS: i64 = 1;

    let data = app
        .db()
        .read(move |conn| {
            let exists = |name: &str| -> rusqlite::Result<bool> {
                Ok(conn
                    .query_row(
                        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                        [name],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            };
            if !exists("user_retention_tracking")? {
                return Ok((0i64, 0i64, 0i64, 0i64, 0i64, 0i64, Vec::<Value>::new()));
            }
            let has_msgs = exists("user_retention_messages")?;

            let total_tracked: i64 =
                conn.query_row("SELECT COUNT(*) FROM user_retention_tracking", [], |r| r.get(0))?;
            let opted_out: i64 = conn.query_row(
                "SELECT COUNT(*) FROM user_retention_tracking WHERE opted_out=1",
                [],
                |r| r.get(0),
            )?;
            let regular_active: i64 = conn.query_row(
                "SELECT COUNT(*) FROM user_retention_tracking WHERE avg_weekly_sessions>=?1 AND total_active_days>=?2",
                params![MIN_WEEKLY, MIN_DAYS],
                |r| r.get(0),
            )?;
            let (miss_you_sent, feedback_received): (i64, i64) = if has_msgs {
                (
                    conn.query_row("SELECT COUNT(*) FROM user_retention_messages WHERE message_type='miss_you'", [], |r| r.get(0))?,
                    conn.query_row("SELECT COUNT(*) FROM user_retention_messages WHERE message_type='feedback'", [], |r| r.get(0))?,
                )
            } else {
                (0, 0)
            };

            let where_sql = "avg_weekly_sessions>=?1 AND total_active_days>=?2
                AND (strftime('%s','now')-last_active_at)/86400 >= ?3 AND opted_out=0
                AND (last_miss_you_sent_at IS NULL OR (strftime('%s','now')-last_miss_you_sent_at)/86400 >= ?4)
                AND (miss_you_count IS NULL OR miss_you_count < ?5)";
            let inactive_candidates: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM user_retention_tracking WHERE {where_sql}"),
                params![MIN_WEEKLY, MIN_DAYS, INACTIVITY, MIN_BETWEEN, MAX_MISS],
                |r| r.get(0),
            )?;

            let (status_sql, at_sql) = if has_msgs {
                (
                    "(SELECT m.delivery_status FROM user_retention_messages m WHERE m.user_id=urt.user_id AND m.message_type='miss_you' ORDER BY m.sent_at DESC LIMIT 1)",
                    "(SELECT m.sent_at FROM user_retention_messages m WHERE m.user_id=urt.user_id AND m.message_type='miss_you' ORDER BY m.sent_at DESC LIMIT 1)",
                )
            } else {
                ("NULL", "NULL")
            };
            let sql = format!(
                "SELECT urt.user_id, urt.guild_id, urt.last_active_at, urt.total_active_days,
                        urt.avg_weekly_sessions,
                        (strftime('%s','now')-urt.last_active_at)/86400 AS days_inactive,
                        {status_sql} AS last_message_status, {at_sql} AS last_message_at,
                        urt.last_miss_you_sent_at, urt.miss_you_count
                   FROM user_retention_tracking urt
                  WHERE {where_sql}
                  ORDER BY days_inactive DESC LIMIT 50"
            );
            let mut stmt = conn.prepare(&sql)?;
            let cands: Vec<Value> = stmt
                .query_map(
                    params![MIN_WEEKLY, MIN_DAYS, INACTIVITY, MIN_BETWEEN, MAX_MISS],
                    |r| {
                        Ok(json!({
                            "user_id": r.get::<_, i64>(0)?,
                            "guild_id": r.get::<_, i64>(1)?,
                            "last_active_at": r.get::<_, Option<i64>>(2)?,
                            "total_active_days": r.get::<_, Option<i64>>(3)?,
                            "avg_weekly_sessions": r.get::<_, Option<f64>>(4)?,
                            "days_inactive": r.get::<_, Option<i64>>(5)?.unwrap_or(0).max(0),
                            "last_message_status": r.get::<_, Option<String>>(6)?,
                            "last_message_at": r.get::<_, Option<i64>>(7)?,
                            "last_miss_you_sent_at": r.get::<_, Option<i64>>(8)?,
                            "miss_you_count": r.get::<_, Option<i64>>(9)?,
                        }))
                    },
                )?
                .collect::<rusqlite::Result<_>>()?;
            Ok((
                total_tracked,
                opted_out,
                regular_active,
                inactive_candidates,
                miss_you_sent,
                feedback_received,
                cands,
            ))
        })
        .await;

    let (
        total_tracked,
        opted_out,
        regular_active,
        inactive_candidates,
        miss_you_sent,
        feedback_received,
        mut candidates,
    ) = match data {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "user_retention fehlgeschlagen");
            return err_text(500, "Failed to load user retention data");
        }
    };

    let ids: Vec<u64> = candidates
        .iter()
        .filter_map(|c| c["user_id"].as_i64().and_then(|i| u64::try_from(i).ok()))
        .collect();
    let names = app.names().resolve(&ids).await;
    for c in candidates.iter_mut() {
        let uid = c["user_id"]
            .as_i64()
            .and_then(|i| u64::try_from(i).ok())
            .unwrap_or(0);
        c["display_name"] = json!(display_name_or_default(&names, uid));
    }

    ok_json(json!({
        "summary": {
            "total_tracked": total_tracked,
            "opted_out": opted_out,
            "regular_active": regular_active,
            "inactive_candidates": inactive_candidates,
            "miss_you_sent": miss_you_sent,
            "feedback_received": feedback_received,
        },
        "candidates": candidates,
        "recent": candidates,
    }))
}

/// `GET /api/voice-stats?limit=N` — Voice-Bestenlisten + Summary (Port von
/// `_handle_voice_stats`). Der Live-Sessions-Teil liest im Original den
/// In-Memory-Zustand des Voice-Trackers (dl-bot-Prozess) — ohne Broker-Endpunkt
/// hier eine bewusste v1-Lücke (leer).
pub async fn voice_stats(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    let limit = match parse_limit(&params, 10, 50) {
        Ok(n) => n,
        Err(resp) => return resp,
    };

    let data = app
        .db()
        .read(move |conn| {
            let (tracked_users, total_seconds, total_points, last_update): (i64, i64, i64, Option<String>) =
                conn.query_row(
                    "SELECT COUNT(*), COALESCE(SUM(total_seconds),0), COALESCE(SUM(total_points),0), MAX(last_update)
                       FROM voice_stats",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
            let rows = |sql: &str| -> rusqlite::Result<Vec<Value>> {
                let mut stmt = conn.prepare(sql)?;
                stmt.query_map(params![limit], |r| {
                    Ok(json!({
                        "user_id": r.get::<_, i64>(0)?,
                        "total_seconds": r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        "total_points": r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        "last_update": r.get::<_, Option<String>>(3)?,
                    }))
                })?
                .collect()
            };
            let top_time = rows(
                "SELECT user_id, total_seconds, total_points, last_update FROM voice_stats
                  ORDER BY total_seconds DESC, total_points DESC LIMIT ?1",
            )?;
            let top_points = rows(
                "SELECT user_id, total_seconds, total_points, last_update FROM voice_stats
                  ORDER BY total_points DESC, total_seconds DESC LIMIT ?1",
            )?;
            Ok((tracked_users, total_seconds, total_points, last_update, top_time, top_points))
        })
        .await;

    let (tracked_users, total_seconds, total_points, last_update, mut top_time, mut top_points) =
        match data {
            Ok(v) => v,
            Err(err) => {
                tracing::error!(%err, "voice_stats fehlgeschlagen");
                return err_text(500, "Voice stats unavailable");
            }
        };

    let ids: Vec<u64> = top_time
        .iter()
        .chain(top_points.iter())
        .filter_map(|c| c["user_id"].as_i64().and_then(|i| u64::try_from(i).ok()))
        .collect();
    let names = app.names().resolve(&ids).await;
    for c in top_time.iter_mut().chain(top_points.iter_mut()) {
        let uid = c["user_id"]
            .as_i64()
            .and_then(|i| u64::try_from(i).ok())
            .unwrap_or(0);
        c["display_name"] = json!(display_name_or_default(&names, uid));
    }

    let avg = if tracked_users > 0 {
        json!(total_seconds as f64 / tracked_users as f64)
    } else {
        json!(0)
    };

    ok_json(json!({
        "summary": {
            "tracked_users": tracked_users,
            "total_seconds": total_seconds,
            "total_points": total_points,
            "last_update": last_update,
            "avg_seconds_per_user": avg,
        },
        "top_by_time": top_time,
        "top_by_points": top_points,
        "live": { "summary": { "active_sessions": 0, "total_seconds": 0 }, "sessions": [] },
    }))
}
