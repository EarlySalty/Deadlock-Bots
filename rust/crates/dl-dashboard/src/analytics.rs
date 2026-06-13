//! Lesende Analytics-Routen (`/api/...`) — Port der Dashboard-Reads.
//!
//! Reine DB-Reads auf bekannte Tabellen-Verträge, session-gegatet über
//! [`DashboardApp::guard_read`]. SQL und JSON-Form sind 1:1 zum Original
//! (`service/dashboard.py`), damit die bestehende Admin-SPA unverändert
//! weiterläuft. Namen zu User-IDs kommen über den Broker-Resolver.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
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
