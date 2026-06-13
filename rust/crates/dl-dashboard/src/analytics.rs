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
use rusqlite::params;
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
