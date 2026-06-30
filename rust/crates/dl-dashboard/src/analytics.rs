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
use chrono::{Duration, NaiveDateTime, Utc};
use dl_core::pyfloat::py_round;
use serde_json::{json, Map, Value};

use crate::db::{i64_to_i32, DashboardDbResult};
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
    if let Err(resp) = app.guard_read(&headers).await {
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

    let result: DashboardDbResult<Value> = async {
        let rows = sqlx::query!(
            r#"
            SELECT id, user_id, guild_id, event_type,
                   to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "timestamp?",
                   display_name,
                   to_char(account_created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "account_created_at?",
                   join_position,
                   metadata::text AS "metadata?"
              FROM activity.member_events
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
               AND ($2::TEXT IS NULL OR event_type = $2)
             ORDER BY occurred_at DESC NULLS LAST
             LIMIT $3
            "#,
            guild,
            event_filter.as_deref(),
            limit,
        )
        .fetch_all(app.pool())
        .await?;
        let events: Vec<Value> = rows
            .into_iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "user_id": row.user_id,
                    "guild_id": row.guild_id,
                    "event_type": row.event_type,
                    "timestamp": row.timestamp,
                    "display_name": row.display_name,
                    "account_created_at": row.account_created_at,
                    "join_position": row.join_position,
                    "metadata": row.metadata,
                })
            })
            .collect();

        let count_rows = sqlx::query!(
            r#"
            SELECT event_type, COUNT(*) AS "count!"
              FROM activity.member_events
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
               AND ($2::TEXT IS NULL OR event_type = $2)
             GROUP BY event_type
             ORDER BY COUNT(*) DESC
            "#,
            guild,
            event_filter.as_deref(),
        )
        .fetch_all(app.pool())
        .await?;
        let mut counts = Map::new();
        for row in count_rows {
            counts.insert(row.event_type, json!(row.count));
        }

        let cutoff = Utc::now() - Duration::days(7);
        let recent_joins = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.member_events
             WHERE event_type = 'join'
               AND occurred_at >= $2
               AND ($1::BIGINT IS NULL OR guild_id = $1)
            "#,
            guild,
            cutoff,
        )
        .fetch_one(app.pool())
        .await?;
        let recent_leaves = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.member_events
             WHERE event_type = 'leave'
               AND occurred_at >= $2
               AND ($1::BIGINT IS NULL OR guild_id = $1)
            "#,
            guild,
            cutoff,
        )
        .fetch_one(app.pool())
        .await?;

        Ok(json!({
            "events": events,
            "summary": {
                "total_events": events.len(),
                "event_counts": counts,
                "recent_joins_7d": recent_joins,
                "recent_leaves_7d": recent_leaves,
            },
        }))
    }
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
    if let Err(resp) = app.guard_read(&headers).await {
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

    let read: DashboardDbResult<(Vec<MaRow>, MaSummary)> = async {
        let rows = sqlx::query!(
            r#"
            SELECT user_id, guild_id, channel_id, COALESCE(message_count, 0)::BIGINT AS "message_count!",
                   to_char(last_message_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_message_at?",
                   to_char(first_message_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "first_message_at?"
              FROM activity.message_activity
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
             ORDER BY message_count DESC
             LIMIT $2
            "#,
            guild,
            limit,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|row| MaRow {
            user_id: row.user_id,
            guild_id: Some(row.guild_id),
            channel_id: row.channel_id,
            message_count: row.message_count,
            last_message_at: row.last_message_at,
            first_message_at: row.first_message_at,
        })
        .collect::<Vec<_>>();

        let summary = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "total_users!",
                   SUM(message_count)::BIGINT AS "total_messages?",
                   AVG(message_count)::DOUBLE PRECISION AS "avg?"
              FROM activity.message_activity
             WHERE ($1::BIGINT IS NULL OR guild_id = $1)
            "#,
            guild,
        )
        .fetch_one(app.pool())
        .await?;
        Ok((
            rows,
            MaSummary {
                total_users: summary.total_users,
                total_messages: summary.total_messages,
                avg: summary.avg,
            },
        ))
    }
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
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let result: DashboardDbResult<Value> = async {
        let totals = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "total!",
                   COALESCE(SUM(CASE WHEN responded_at IS NOT NULL THEN 1 ELSE 0 END), 0)::BIGINT AS "responded!",
                   COALESCE(SUM(CASE WHEN web_submitted_at IS NOT NULL THEN 1 ELSE 0 END), 0)::BIGINT AS "web!"
              FROM activity.member_leave_surveys
            "#
        )
        .fetch_one(app.pool())
        .await?;

        let user_bucket_rows = sqlx::query!(
            r#"
            SELECT COALESCE(NULLIF(user_bucket, ''), 'unknown') AS "bucket!",
                   COUNT(*) AS "count!"
              FROM activity.member_leave_surveys
             GROUP BY COALESCE(NULLIF(user_bucket, ''), 'unknown')
             ORDER BY 1 ASC
            "#
        )
        .fetch_all(app.pool())
        .await?;
        let mut by_user_bucket = Map::new();
        for row in user_bucket_rows {
            by_user_bucket.insert(row.bucket, json!(row.count));
        }

        let dm_rows = sqlx::query!(
            r#"
            SELECT COALESCE(NULLIF(dm_status, ''), 'unknown') AS "status!",
                   COUNT(*) AS "count!"
              FROM activity.member_leave_surveys
             GROUP BY COALESCE(NULLIF(dm_status, ''), 'unknown')
             ORDER BY 1 ASC
            "#
        )
        .fetch_all(app.pool())
        .await?;
        let mut by_dm_status = Map::new();
        for row in dm_rows {
            by_dm_status.insert(row.status, json!(row.count));
        }

        let by_reason = sqlx::query!(
            r#"
            SELECT COALESCE(NULLIF(reason_code, ''), 'unknown') AS "reason_code!",
                   COUNT(*) AS "count!"
              FROM activity.member_leave_surveys
             GROUP BY COALESCE(NULLIF(reason_code, ''), 'unknown')
             ORDER BY COUNT(*) DESC, 1 ASC
            "#
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|row| json!({ "reason_code": row.reason_code, "count": row.count }))
        .collect::<Vec<_>>();

        let recent = sqlx::query!(
            r#"
            SELECT display_name, survey_token, user_bucket, reason_code,
                   follow_up_question, follow_up_text, extra_text,
                   EXTRACT(EPOCH FROM web_submitted_at)::BIGINT AS "web_submitted_at?",
                   web_payload::text AS "web_payload?",
                   EXTRACT(EPOCH FROM left_at)::BIGINT AS "left_at!",
                   EXTRACT(EPOCH FROM responded_at)::BIGINT AS "responded_at?"
              FROM activity.member_leave_surveys
             WHERE responded_at IS NOT NULL OR web_submitted_at IS NOT NULL
             ORDER BY COALESCE(web_submitted_at, responded_at) DESC, id DESC
             LIMIT 30
            "#
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|row| {
            json!({
                "display_name": row.display_name,
                "survey_token": row.survey_token,
                "user_bucket": row.user_bucket,
                "reason_code": row.reason_code,
                "follow_up_question": row.follow_up_question,
                "follow_up_text": row.follow_up_text,
                "extra_text": row.extra_text,
                "web_submitted_at": row.web_submitted_at,
                "web_payload": parse_survey_json(row.web_payload),
                "left_at": row.left_at,
                "responded_at": row.responded_at,
            })
        })
        .collect::<Vec<_>>();

            let total = totals.total;
            let responded = totals.responded;
            let web = totals.web;
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
    }
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
    if let Err(resp) = app.guard_read(&headers).await {
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

    let read: DashboardDbResult<Vec<CpRow>> = async {
        let min_sessions = i64_to_i32(min_sessions, "min_sessions")?;
        let rows = sqlx::query!(
            r#"
            SELECT user_id, co_player_id,
                   COALESCE(sessions_together, 0)::BIGINT AS "sessions!",
                   COALESCE(total_minutes_together, 0)::BIGINT AS "minutes!",
                   to_char(last_played_together AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_played?",
                   user_display_name, co_player_display_name
              FROM activity.user_co_players
             WHERE COALESCE(sessions_together, 0) >= $1
             ORDER BY sessions_together DESC NULLS LAST,
                      total_minutes_together DESC NULLS LAST,
                      last_played_together DESC NULLS LAST
             LIMIT $2
            "#,
            min_sessions,
            limit * 2,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|row| CpRow {
            user_id: row.user_id,
            co_player_id: row.co_player_id,
            sessions: row.sessions,
            minutes: row.minutes,
            last_played: row.last_played,
            user_name: row.user_display_name,
            co_name: row.co_player_display_name,
        })
        .collect();
        Ok(rows)
    }
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

struct RetentionData {
    total_tracked: i64,
    opted_out: i64,
    regular_active: i64,
    inactive_candidates: i64,
    miss_you_sent: i64,
    feedback_received: i64,
    candidates: Vec<Value>,
}

struct VoiceStatsData {
    tracked_users: i64,
    total_seconds: i64,
    total_points: i64,
    last_update: Option<String>,
    top_time: Vec<Value>,
    top_points: Vec<Value>,
}

pub async fn voice_history(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers).await {
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
    let cutoff = Utc::now() - Duration::days(days);
    let mode_sql = mode.clone();

    let read: DashboardDbResult<VhData> = async {
        let daily = sqlx::query!(
            r#"
            SELECT (started_at AT TIME ZONE 'UTC')::date::text AS "day!",
                   COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
                   COUNT(*) AS "sessions!",
                   COUNT(DISTINCT user_id) AS "users!"
              FROM activity.voice_session_log
             WHERE started_at >= $1
             GROUP BY (started_at AT TIME ZONE 'UTC')::date
             ORDER BY (started_at AT TIME ZONE 'UTC')::date DESC
            "#,
            cutoff,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|row| VhDaily {
            day: Some(row.day),
            total_seconds: row.total_seconds,
            sessions: row.sessions,
            users: row.users,
        })
        .collect();

        let top = sqlx::query!(
            r#"
            SELECT user_id, MAX(display_name) AS "display_name?",
                   COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
                   COALESCE(SUM(points), 0)::BIGINT AS "total_points!",
                   COUNT(*) AS "sessions!"
              FROM activity.voice_session_log
             WHERE started_at >= $1
               AND ($2::BIGINT IS NULL OR user_id = $2)
             GROUP BY user_id
             ORDER BY SUM(duration_seconds) DESC, SUM(points) DESC
             LIMIT $3
            "#,
            cutoff,
            user_id,
            top_limit,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|row| VhTop {
            user_id: row.user_id,
            display_name: row.display_name,
            total_seconds: row.total_seconds,
            total_points: row.total_points,
            sessions: row.sessions,
        })
        .collect();

        let buckets = sqlx::query!(
            r#"
            WITH source AS (
                SELECT started_at AT TIME ZONE 'UTC' AS utc_started,
                       duration_seconds,
                       COALESCE(peak_users, 0) AS peak_users
                  FROM activity.voice_session_log
                 WHERE started_at >= $2
                   AND ($3::BIGINT IS NULL OR user_id = $3)
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
            mode_sql,
            cutoff,
            user_id,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|row| VhBucket {
            bucket: Some(row.bucket),
            total_seconds: row.total_seconds,
            sessions: row.sessions,
            sum_peak: row.sum_peak,
        })
        .collect();

        let user = if let Some(uid) = user_id {
            let range_row = sqlx::query!(
                r#"
                SELECT COALESCE(SUM(duration_seconds), 0)::BIGINT AS "total_seconds!",
                       COALESCE(SUM(points), 0)::BIGINT AS "total_points!",
                       COUNT(*) AS "sessions!",
                       COALESCE(SUM(COALESCE(peak_users, 0)), 0)::BIGINT AS "sum_peak!",
                       COUNT(DISTINCT (started_at AT TIME ZONE 'UTC')::date) AS "active_days!",
                       to_char(MAX(ended_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_session?"
                  FROM activity.voice_session_log
                 WHERE started_at >= $1
                   AND user_id = $2
                "#,
                cutoff,
                uid,
            )
            .fetch_one(app.pool())
            .await?;
            let range = VhRange {
                total_seconds: range_row.total_seconds,
                total_points: range_row.total_points,
                sessions: range_row.sessions,
                sum_peak: range_row.sum_peak,
                active_days: range_row.active_days,
                last_session: range_row.last_session,
            };
            let lifetime = sqlx::query!(
                r#"
                SELECT total_seconds, total_points,
                       to_char(last_update AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_update?"
                  FROM voice.voice_stats
                 WHERE user_id = $1
                "#,
                uid,
            )
            .fetch_optional(app.pool())
            .await?
            .map(|row| VhLifetime {
                total_seconds: row.total_seconds,
                total_points: row.total_points,
                last_update: row.last_update,
            });
            let lifetime_row = sqlx::query!(
                r#"
                SELECT COUNT(*) AS "sessions!",
                       to_char(MAX(ended_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_session?"
                  FROM activity.voice_session_log
                 WHERE user_id = $1
                "#,
                uid,
            )
            .fetch_one(app.pool())
            .await?;
            let recent = sqlx::query!(
                r#"
                SELECT id, guild_id, channel_id, channel_name,
                       to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "started_at?",
                       to_char(ended_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "ended_at?",
                       duration_seconds,
                       points::BIGINT AS "points!",
                       COALESCE(peak_users, 0)::BIGINT AS "peak_users!",
                       co_player_ids::text AS "co_player_ids?"
                  FROM activity.voice_session_log
                 WHERE user_id = $1
                 ORDER BY ended_at DESC, id DESC
                 LIMIT $2
                "#,
                uid,
                recent_limit,
            )
            .fetch_all(app.pool())
            .await?
            .into_iter()
            .map(|row| VhRecent {
                id: row.id,
                guild_id: row.guild_id,
                channel_id: row.channel_id,
                channel_name: row.channel_name,
                started_at: row.started_at,
                ended_at: row.ended_at,
                duration_seconds: row.duration_seconds,
                points: row.points,
                peak_users: row.peak_users,
                co_player_ids: row.co_player_ids,
            })
            .collect();
            Some(VhUserData {
                range,
                lifetime,
                lifetime_sessions: lifetime_row.sessions,
                lifetime_last_session: lifetime_row.last_session,
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
    }
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
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    const MIN_WEEKLY: f64 = 0.5;
    const MIN_DAYS: i64 = 3;
    const INACTIVITY: i64 = 14;
    const MIN_BETWEEN: i64 = 30;
    const MAX_MISS: i64 = 1;

    let data: DashboardDbResult<RetentionData> = async {
        let min_days = i64_to_i32(MIN_DAYS, "MIN_DAYS")?;
        let max_miss = i64_to_i32(MAX_MISS, "MAX_MISS")?;
        let now = Utc::now();
        let inactive_cutoff = now - Duration::days(INACTIVITY);
        let miss_cutoff = now - Duration::days(MIN_BETWEEN);

        let total_tracked = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.user_retention_tracking
            "#
        )
        .fetch_one(app.pool())
        .await?;
        let opted_out = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.user_retention_tracking
             WHERE opted_out = TRUE
            "#
        )
        .fetch_one(app.pool())
        .await?;
        let regular_active = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.user_retention_tracking
             WHERE avg_weekly_sessions >= $1
               AND total_active_days >= $2
            "#,
            MIN_WEEKLY,
            min_days,
        )
        .fetch_one(app.pool())
        .await?;
        let miss_you_sent = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.user_retention_messages
             WHERE message_type = 'miss_you'
            "#
        )
        .fetch_one(app.pool())
        .await?;
        let feedback_received = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.user_retention_messages
             WHERE message_type = 'feedback'
            "#
        )
        .fetch_one(app.pool())
        .await?;
        let inactive_candidates = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM activity.user_retention_tracking
             WHERE avg_weekly_sessions >= $1
               AND total_active_days >= $2
               AND last_active_at <= $3
               AND opted_out = FALSE
               AND (last_miss_you_sent_at IS NULL OR last_miss_you_sent_at <= $4)
               AND miss_you_count < $5
            "#,
            MIN_WEEKLY,
            min_days,
            inactive_cutoff,
            miss_cutoff,
            max_miss,
        )
        .fetch_one(app.pool())
        .await?;

        let cands = sqlx::query!(
            r#"
            SELECT urt.user_id, urt.guild_id,
                   EXTRACT(EPOCH FROM urt.last_active_at)::BIGINT AS "last_active_at!",
                   urt.total_active_days,
                   urt.avg_weekly_sessions,
                   GREATEST(FLOOR(EXTRACT(EPOCH FROM ($6 - urt.last_active_at)) / 86400)::BIGINT, 0) AS "days_inactive!",
                   msg.delivery_status AS "last_message_status?",
                   EXTRACT(EPOCH FROM msg.sent_at)::BIGINT AS "last_message_at?",
                   EXTRACT(EPOCH FROM urt.last_miss_you_sent_at)::BIGINT AS "last_miss_you_sent_at?",
                   urt.miss_you_count
              FROM activity.user_retention_tracking urt
              LEFT JOIN LATERAL (
                    SELECT delivery_status, sent_at
                      FROM activity.user_retention_messages m
                     WHERE m.user_id = urt.user_id
                       AND m.message_type = 'miss_you'
                     ORDER BY m.sent_at DESC
                     LIMIT 1
              ) msg ON TRUE
             WHERE urt.avg_weekly_sessions >= $1
               AND urt.total_active_days >= $2
               AND urt.last_active_at <= $3
               AND urt.opted_out = FALSE
               AND (urt.last_miss_you_sent_at IS NULL OR urt.last_miss_you_sent_at <= $4)
               AND urt.miss_you_count < $5
             ORDER BY 6 DESC
             LIMIT 50
            "#,
            MIN_WEEKLY,
            min_days,
            inactive_cutoff,
            miss_cutoff,
            max_miss,
            now,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|r| {
            json!({
                "user_id": r.user_id,
                "guild_id": r.guild_id,
                "last_active_at": r.last_active_at,
                "total_active_days": r.total_active_days,
                "avg_weekly_sessions": r.avg_weekly_sessions,
                "days_inactive": r.days_inactive,
                "last_message_status": r.last_message_status,
                "last_message_at": r.last_message_at,
                "last_miss_you_sent_at": r.last_miss_you_sent_at,
                "miss_you_count": r.miss_you_count,
            })
        })
        .collect::<Vec<_>>();

        Ok(RetentionData {
            total_tracked,
            opted_out,
            regular_active,
            inactive_candidates,
            miss_you_sent,
            feedback_received,
            candidates: cands,
        })
    }
    .await;

    let mut data = match data {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "user_retention fehlgeschlagen");
            return err_text(500, "Failed to load user retention data");
        }
    };

    let ids: Vec<u64> = data
        .candidates
        .iter()
        .filter_map(|c| c["user_id"].as_i64().and_then(|i| u64::try_from(i).ok()))
        .collect();
    let names = app.names().resolve(&ids).await;
    for c in data.candidates.iter_mut() {
        let uid = c["user_id"]
            .as_i64()
            .and_then(|i| u64::try_from(i).ok())
            .unwrap_or(0);
        c["display_name"] = json!(display_name_or_default(&names, uid));
    }
    let recent = data.candidates.clone();

    ok_json(json!({
        "summary": {
            "total_tracked": data.total_tracked,
            "opted_out": data.opted_out,
            "regular_active": data.regular_active,
            "inactive_candidates": data.inactive_candidates,
            "miss_you_sent": data.miss_you_sent,
            "feedback_received": data.feedback_received,
        },
        "candidates": data.candidates,
        "recent": recent,
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
    if let Err(resp) = app.guard_read(&headers).await {
        return resp;
    }
    let limit = match parse_limit(&params, 10, 50) {
        Ok(n) => n,
        Err(resp) => return resp,
    };

    let data: DashboardDbResult<VoiceStatsData> = async {
        let summary = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "tracked_users!",
                   COALESCE(SUM(total_seconds), 0)::BIGINT AS "total_seconds!",
                   COALESCE(SUM(total_points), 0)::BIGINT AS "total_points!",
                   to_char(MAX(last_update) AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_update?"
              FROM voice.voice_stats
            "#
        )
        .fetch_one(app.pool())
        .await?;
        let top_time = sqlx::query!(
            r#"
            SELECT user_id, total_seconds, total_points,
                   to_char(last_update AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_update?"
              FROM voice.voice_stats
             ORDER BY total_seconds DESC, total_points DESC
             LIMIT $1
            "#,
            limit,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|r| {
            json!({
                "user_id": r.user_id,
                "total_seconds": r.total_seconds,
                "total_points": r.total_points,
                "last_update": r.last_update,
            })
        })
        .collect::<Vec<_>>();
        let top_points = sqlx::query!(
            r#"
            SELECT user_id, total_seconds, total_points,
                   to_char(last_update AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS "last_update?"
              FROM voice.voice_stats
             ORDER BY total_points DESC, total_seconds DESC
             LIMIT $1
            "#,
            limit,
        )
        .fetch_all(app.pool())
        .await?
        .into_iter()
        .map(|r| {
            json!({
                "user_id": r.user_id,
                "total_seconds": r.total_seconds,
                "total_points": r.total_points,
                "last_update": r.last_update,
            })
        })
        .collect::<Vec<_>>();
        Ok(VoiceStatsData {
            tracked_users: summary.tracked_users,
            total_seconds: summary.total_seconds,
            total_points: summary.total_points,
            last_update: summary.last_update,
            top_time,
            top_points,
        })
    }
    .await;

    let mut data = match data {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "voice_stats fehlgeschlagen");
            return err_text(500, "Voice stats unavailable");
        }
    };

    let ids: Vec<u64> = data
        .top_time
        .iter()
        .chain(data.top_points.iter())
        .filter_map(|c| c["user_id"].as_i64().and_then(|i| u64::try_from(i).ok()))
        .collect();
    let names = app.names().resolve(&ids).await;
    for c in data.top_time.iter_mut().chain(data.top_points.iter_mut()) {
        let uid = c["user_id"]
            .as_i64()
            .and_then(|i| u64::try_from(i).ok())
            .unwrap_or(0);
        c["display_name"] = json!(display_name_or_default(&names, uid));
    }

    let avg = if data.tracked_users > 0 {
        json!(data.total_seconds as f64 / data.tracked_users as f64)
    } else {
        json!(0)
    };

    ok_json(json!({
        "summary": {
            "tracked_users": data.tracked_users,
            "total_seconds": data.total_seconds,
            "total_points": data.total_points,
            "last_update": data.last_update,
            "avg_seconds_per_user": avg,
        },
        "top_by_time": data.top_time,
        "top_by_points": data.top_points,
        "live": { "summary": { "active_sessions": 0, "total_seconds": 0 }, "sessions": [] },
    }))
}
