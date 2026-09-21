//! Die Broker-Routen — Reihenfolge der Checks und alle Fehlertexte
//! exakt wie master_broker.py.

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{json, Map, Value};

use crate::payload::{self};
use crate::port::{MemberPresence, PortError, RichMessage};
use crate::{
    authorize, error_body, payload_hash, request_id, require_loopback, respond, run_idempotent,
    success_body, SharedBroker, IDEMPOTENCY_HEADER,
};

type Peer = ConnectInfo<SocketAddr>;

fn bad_request(rid: &str, message: &str) -> Response {
    respond(400, error_body(rid, None, "bad_request", message))
}

/// Body als JSON-Objekt; alles andere → "invalid JSON payload".
fn json_object(body: &[u8]) -> Result<Map<String, Value>, ()> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .ok_or(())
}

fn allowlist_check(
    rid: &str,
    idem: Option<&str>,
    scope: &str,
    value: u64,
    allowlist: &crate::Allowlist,
) -> Result<(), Response> {
    if allowlist.permits(value) {
        Ok(())
    } else {
        Err(respond(
            403,
            error_body(
                rid,
                idem,
                "forbidden",
                &format!("{scope}_id {value} is not permitted"),
            ),
        ))
    }
}

// ── GET /health ────────────────────────────────────────────────────────────

pub async fn health(State(state): State<SharedBroker>, peer: Peer, headers: HeaderMap) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = authorize(&state, &peer, &headers, &rid) {
        return resp;
    }
    let result = json!({
        "status": "ok",
        "bot_ready": state.port.is_ready().await,
        "runtime_role": "master",
    });
    respond(200, success_body(&rid, None, result))
}

// ── Diagnose (loopback-only, ohne Token) ───────────────────────────────────

pub async fn list_roles(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    let guild_id = params
        .get("guild_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0);
    match state.port.list_roles(guild_id).await {
        Ok(info) => respond(
            200,
            json!({
                "ok": true,
                "guild_id": info.guild_id.to_string(),
                "chunked": info.chunked,
                "roles": info.roles.iter().map(|r| json!({
                    "id": r.id.to_string(),
                    "name": r.name,
                    "position": r.position,
                    "member_count": r.member_count,
                })).collect::<Vec<_>>(),
            }),
        ),
        Err(_) => respond(404, error_body(&rid, None, "not_found", "guild not found")),
    }
}

pub async fn role_members(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    let guild_id = params
        .get("guild_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0);
    let role_id = params
        .get("role_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    match state.port.role_members(guild_id, role_id).await {
        Ok(info) => respond(
            200,
            json!({
                "ok": true,
                "role_id": info.role_id.to_string(),
                "name": info.name,
                "members": info.members.iter().map(|m| json!({
                    "id": m.user_id.to_string(),
                    "display_name": m.display_name,
                })).collect::<Vec<_>>(),
            }),
        ),
        Err(PortError::GuildNotFound) => {
            respond(404, error_body(&rid, None, "not_found", "guild not found"))
        }
        Err(_) => respond(
            404,
            error_body(
                &rid,
                None,
                "not_found",
                &format!("role {role_id} not found"),
            ),
        ),
    }
}

pub async fn channel_info(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    let guild_id = params
        .get("guild_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0);
    let channel_id = params
        .get("channel_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    match state.channel_info.channel_info(guild_id, channel_id).await {
        Ok(info) => respond(
            200,
            json!({
                "ok": true,
                "channel_id": info.channel_id.to_string(),
                "name": info.name,
                "parent_id": info.parent_id.map(|value| value.to_string()),
                "last_message_id": info.last_message_id.map(|value| value.to_string()),
            }),
        ),
        Err(PortError::GuildNotFound) => {
            respond(404, error_body(&rid, None, "not_found", "guild not found"))
        }
        Err(PortError::ChannelNotFound) => respond(
            404,
            error_body(
                &rid,
                None,
                "not_found",
                &format!("channel {channel_id} not found"),
            ),
        ),
        Err(err) => {
            tracing::error!(%err, channel_id, "channel_info fehlgeschlagen");
            respond(
                502,
                error_body(&rid, None, "discord_error", "failed to read channel info"),
            )
        }
    }
}

pub async fn message_reactions(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = authorize(&state, &peer, &headers, &rid) {
        return resp;
    }
    let params: Map<String, Value> = params
        .into_iter()
        .map(|(key, value)| (key, Value::String(value)))
        .collect();
    let channel_id = match payload::positive_int(&params, "channel_id") {
        Ok(value) => value,
        Err(message) => return bad_request(&rid, &message),
    };
    let message_id = match payload::positive_int(&params, "message_id") {
        Ok(value) => value,
        Err(message) => return bad_request(&rid, &message),
    };
    match state
        .port
        .fetch_message_reactions(channel_id, message_id)
        .await
    {
        Ok(reactions) => respond(200, json!({ "found": true, "reactions": reactions })),
        Err(PortError::MessageNotFound) => respond(200, json!({ "found": false, "reactions": [] })),
        Err(err) => {
            tracing::error!(%err, channel_id, message_id, "message_reactions fehlgeschlagen");
            respond(502, json!({ "error": "Discord request failed" }))
        }
    }
}

/// Zugriffsstatus eines Mitglieds (Admin + Rollen) für den Dashboard-Login.
/// Loopback-only, ohne Token — wie die übrigen Diagnose-Routen.
pub async fn member_access(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    let guild_id = params
        .get("guild_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0);
    let user_id = params
        .get("user_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if user_id == 0 {
        return bad_request(&rid, "user_id must be a positive integer");
    }
    match state.port.member_access(guild_id, user_id).await {
        Ok(access) => respond(
            200,
            json!({
                "ok": true,
                "found": access.found,
                "user_id": access.user_id.to_string(),
                "display_name": access.display_name,
                "is_administrator": access.is_administrator,
                "role_ids": access.role_ids.iter().map(u64::to_string).collect::<Vec<_>>(),
            }),
        ),
        Err(_) => respond(404, error_body(&rid, None, "not_found", "guild not found")),
    }
}

/// Live-Mitgliedschaftsprüfung (`?guild_id=&user_id=`) für den Steam-Bot
/// Leave-Reconcile. Loopback-only, ohne Token — wie die übrigen Diagnose-Routen.
/// Antwortet tri-state: `present` | `absent` | `unknown`.
pub async fn member_present(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    let guild_id = params
        .get("guild_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let user_id = params
        .get("user_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if guild_id == 0 {
        return bad_request(&rid, "guild_id must be a positive integer");
    }
    if user_id == 0 {
        return bad_request(&rid, "user_id must be a positive integer");
    }
    // Tri-state nie als Fehler: selbst ein interner Port-Fehler wird zu
    // "unknown", damit der Reconcile niemals fälschlich Cleanup auslöst.
    let status = match state.port.member_present(guild_id, user_id).await {
        Ok(MemberPresence::Present) => "present",
        Ok(MemberPresence::Absent) => "absent",
        Ok(MemberPresence::Unknown) | Err(_) => "unknown",
    };
    respond(
        200,
        json!({
            "ok": true,
            "status": status,
            "guild_id": guild_id.to_string(),
            "user_id": user_id.to_string(),
        }),
    )
}

/// Anzeigenamen zu mehreren User-IDs (`?user_ids=1,2,3`). Loopback-only,
/// ohne Token — für die Dashboard-Analytics-Namensauflösung.
pub async fn resolve_names(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    let user_ids: Vec<u64> = params
        .get("user_ids")
        .map(|raw| {
            raw.split(',')
                .filter_map(|s| s.trim().parse::<u64>().ok())
                .filter(|v| *v > 0)
                .collect()
        })
        .unwrap_or_default();
    match state.port.resolve_names(&user_ids).await {
        Ok(members) => {
            let names: serde_json::Map<String, serde_json::Value> = members
                .into_iter()
                .map(|m| (m.user_id.to_string(), json!(m.display_name)))
                .collect();
            respond(200, json!({ "ok": true, "names": names }))
        }
        Err(_) => respond(200, json!({ "ok": true, "names": {} })),
    }
}

/// Alle nicht-Bot-Mitglieder der Default-Gilde (`GET .../discord/members`).
/// Loopback-only, ohne Token (wie `_handle_list_members`). Antwort
/// `{ok, members:[{id,name,global_name,nick}]}`; wird vom Twitch-Bot-Relay
/// (`list_members`) konsumiert.
pub async fn members(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    match state.port.list_members().await {
        Ok(list) => {
            let members: Vec<serde_json::Value> = list
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.user_id.to_string(),
                        "name": m.name,
                        "global_name": m.global_name,
                        "nick": m.nick,
                    })
                })
                .collect();
            respond(200, json!({ "ok": true, "members": members }))
        }
        Err(_) => respond(404, error_body(&rid, None, "not_found", "guild not found")),
    }
}

/// Einen einzelnen Discord-User auflösen (`POST .../discord/resolve-user`).
/// Token-authentifiziert (Body `{user_id}`); antwortet immer 200 mit
/// `{ok, result:{found,...}}` (nicht gefunden → `found:false`), wie
/// `_handle_resolve_user`. Wird vom Twitch-Bot-Relay konsumiert.
pub async fn resolve_user(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = authorize(&state, &peer, &headers, &rid) {
        return resp;
    }
    let Ok(payload) = json_object(&body) else {
        return bad_request(&rid, "invalid JSON payload");
    };
    let user_id = match payload::positive_int(&payload, "user_id") {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };
    match state.port.resolve_user(user_id).await {
        Ok(Some(u)) => respond(
            200,
            json!({
                "ok": true,
                "result": {
                    "found": true,
                    "user_id": u.user_id.to_string(),
                    "name": u.name,
                    "global_name": u.global_name,
                    "display_name": u.display_name,
                }
            }),
        ),
        Ok(None) | Err(_) => respond(200, json!({ "ok": true, "result": { "found": false } })),
    }
}

/// Live-Kennzahlen einer Gilde (`?guild_id=` optional). Loopback-only, ohne
/// Token — für die öffentliche Server-Statistik des Dashboards.
pub async fn guild_stats(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = require_loopback(&peer, &rid) {
        return resp;
    }
    let guild_id = params
        .get("guild_id")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0);
    match state.port.guild_stats(guild_id).await {
        Ok(s) => respond(
            200,
            json!({
                "ok": true,
                "found": s.found,
                "guild_id": s.guild_id.to_string(),
                "name": s.name,
                "member_count": s.member_count,
                "online_count": s.online_count,
                "voice_count": s.voice_count,
                "vanity_url_code": s.vanity_url_code,
            }),
        ),
        Err(_) => respond(404, error_body(&rid, None, "not_found", "guild not found")),
    }
}

// ── Aktionen (Token + Idempotenz) ──────────────────────────────────────────

/// Gemeinsamer Einstieg: Auth + JSON-Objekt + Idempotency-Key.
fn begin_action(
    state: &SharedBroker,
    peer: &Peer,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(String, Map<String, Value>, String), Response> {
    let rid = request_id(headers);
    authorize(state, peer, headers, &rid)?;
    let Ok(payload) = json_object(body) else {
        return Err(bad_request(&rid, "invalid JSON payload"));
    };
    let header_key = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok());
    let idem =
        payload::idempotency_key(header_key, &payload).map_err(|msg| bad_request(&rid, &msg))?;
    Ok((rid, payload, idem))
}

pub async fn send_message(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let parsed = (|| -> Result<(String, Option<u64>, Option<u64>), String> {
        let content = payload::required_content(&payload)?;
        let has_channel = payload
            .get("channel_id")
            .map(|v| !v.is_null())
            .unwrap_or(false);
        let has_user = payload
            .get("user_id")
            .map(|v| !v.is_null())
            .unwrap_or(false);
        if !has_channel && !has_user {
            return Err("channel_id or user_id is required".to_string());
        }
        if has_channel && has_user {
            return Err("channel_id and user_id are mutually exclusive".to_string());
        }
        let channel_id = if has_channel {
            Some(payload::positive_int(&payload, "channel_id")?)
        } else {
            None
        };
        let user_id = if has_user {
            Some(payload::positive_int(&payload, "user_id")?)
        } else {
            None
        };
        Ok((content, channel_id, user_id))
    })();
    let (content, channel_id, user_id) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    if let Some(channel_id) = channel_id {
        if let Err(resp) = allowlist_check(
            &rid,
            Some(&idem),
            "channel",
            channel_id,
            &state.channel_allowlist,
        ) {
            return resp;
        }
    }

    let mut op = Map::new();
    op.insert("content".into(), json!(content));
    if let Some(c) = channel_id {
        op.insert("channel_id".into(), json!(c));
    }
    if let Some(u) = user_id {
        op.insert("user_id".into(), json!(u));
    }
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.send_message",
        &idem,
        &hash,
        || async {
            if let Some(channel_id) = channel_id {
                match state.port.send_channel_message(channel_id, &content).await {
                    Ok(message_id) => (
                        200,
                        success_body(
                            &rid,
                            Some(&idem),
                            json!({
                                "channel_id": channel_id,
                                "user_id": null,
                                "message_id": message_id,
                            }),
                        ),
                    ),
                    Err(PortError::ChannelNotFound) => (
                        404,
                        error_body(
                            &rid,
                            Some(&idem),
                            "not_found",
                            &format!("channel {channel_id} not found"),
                        ),
                    ),
                    Err(err) => {
                        tracing::error!(%err, channel_id, "send_message fehlgeschlagen");
                        (
                            502,
                            error_body(
                                &rid,
                                Some(&idem),
                                "discord_error",
                                "failed to send message",
                            ),
                        )
                    }
                }
            } else {
                let Some(user_id) = user_id else {
                    return (
                        400,
                        error_body(
                            &rid,
                            Some(&idem),
                            "bad_request",
                            "channel_id or user_id is required",
                        ),
                    );
                };
                match state.port.send_dm(user_id, &content).await {
                    Ok((dm_channel_id, message_id)) => (
                        200,
                        success_body(
                            &rid,
                            Some(&idem),
                            json!({
                                "channel_id": dm_channel_id,
                                "user_id": user_id,
                                "message_id": message_id,
                            }),
                        ),
                    ),
                    Err(PortError::UserNotFound) => (
                        404,
                        error_body(
                            &rid,
                            Some(&idem),
                            "not_found",
                            &format!("user {user_id} not found"),
                        ),
                    ),
                    Err(PortError::DmOpenFailed) => (
                        502,
                        error_body(
                            &rid,
                            Some(&idem),
                            "discord_error",
                            "failed to open DM channel",
                        ),
                    ),
                    Err(err) => {
                        tracing::error!(%err, user_id, "send_message(DM) fehlgeschlagen");
                        (
                            502,
                            error_body(
                                &rid,
                                Some(&idem),
                                "discord_error",
                                "failed to send message",
                            ),
                        )
                    }
                }
            }
        },
    )
    .await
}

pub async fn create_channel(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let parsed = (|| -> Result<(String, u64, Option<String>), String> {
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            return Err("name is required".to_string());
        }
        if name.chars().count() > 100 {
            return Err("name exceeds Discord limit (100)".to_string());
        }
        let category_id = payload::positive_int(&payload, "category_id")?;
        let topic = payload
            .get("topic")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        Ok((name, category_id, topic))
    })();
    let (name, category_id, topic) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    let mut op = Map::new();
    op.insert("name".into(), json!(name));
    op.insert("category_id".into(), json!(category_id));
    op.insert("topic".into(), json!(topic));
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.create_channel",
        &idem,
        &hash,
        || async {
            match state
                .port
                .create_text_channel(category_id, &name, topic.as_deref())
                .await
            {
                Ok(channel_id) => (
                    200,
                    success_body(
                        &rid,
                        Some(&idem),
                        json!({
                            "channel_id": channel_id,
                            "category_id": category_id,
                            "name": name,
                        }),
                    ),
                ),
                Err(PortError::CategoryNotFound) => (
                    404,
                    error_body(
                        &rid,
                        Some(&idem),
                        "not_found",
                        &format!("category {category_id} not found"),
                    ),
                ),
                Err(PortError::GuildUnavailable) => (
                    502,
                    error_body(
                        &rid,
                        Some(&idem),
                        "discord_error",
                        "guild unavailable for channel creation",
                    ),
                ),
                Err(err) => {
                    tracing::error!(%err, category_id, "create_channel fehlgeschlagen");
                    (
                        502,
                        error_body(
                            &rid,
                            Some(&idem),
                            "discord_error",
                            "failed to create channel",
                        ),
                    )
                }
            }
        },
    )
    .await
}

pub async fn delete_channel(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let channel_id = match payload::positive_int(&payload, "channel_id") {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };
    let mut op = Map::new();
    op.insert("channel_id".into(), json!(channel_id));
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.delete_channel",
        &idem,
        &hash,
        || async {
            match state.port.delete_channel(channel_id).await {
                Ok(()) => (
                    200,
                    success_body(&rid, Some(&idem), json!({ "channel_id": channel_id })),
                ),
                Err(PortError::ChannelNotFound) => (
                    404,
                    error_body(
                        &rid,
                        Some(&idem),
                        "not_found",
                        &format!("channel {channel_id} not found"),
                    ),
                ),
                Err(err) => {
                    tracing::error!(%err, channel_id, "delete_channel fehlgeschlagen");
                    (
                        502,
                        error_body(
                            &rid,
                            Some(&idem),
                            "discord_error",
                            "failed to delete channel",
                        ),
                    )
                }
            }
        },
    )
    .await
}

pub async fn delete_message(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let parsed = (|| -> Result<(u64, u64, String), String> {
        let channel_id = payload::positive_int(&payload, "channel_id")?;
        let message_id = payload::positive_int(&payload, "message_id")?;
        let reason = payload
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .ok_or_else(|| "reason is required".to_string())?
            .to_string();
        Ok((channel_id, message_id, reason))
    })();
    let (channel_id, message_id, reason) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    if let Err(resp) = allowlist_check(
        &rid,
        Some(&idem),
        "channel",
        channel_id,
        &state.channel_allowlist,
    ) {
        return resp;
    }

    let mut op = Map::new();
    op.insert("channel_id".into(), json!(channel_id));
    op.insert("message_id".into(), json!(message_id));
    op.insert("reason".into(), json!(reason));
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.delete_message",
        &idem,
        &hash,
        || async {
            match state
                .port
                .delete_message(channel_id, message_id, &reason)
                .await
            {
                Ok(()) => (
                    200,
                    success_body(
                        &rid,
                        Some(&idem),
                        json!({
                            "channel_id": channel_id,
                            "message_id": message_id,
                            "already_absent": false,
                        }),
                    ),
                ),
                Err(PortError::MessageNotFound | PortError::ChannelNotFound) => (
                    200,
                    success_body(
                        &rid,
                        Some(&idem),
                        json!({
                            "channel_id": channel_id,
                            "message_id": message_id,
                            "already_absent": true,
                        }),
                    ),
                ),
                Err(err) => {
                    tracing::error!(%err, channel_id, message_id, "delete_message fehlgeschlagen");
                    (
                        502,
                        error_body(
                            &rid,
                            Some(&idem),
                            "discord_error",
                            "failed to delete message",
                        ),
                    )
                }
            }
        },
    )
    .await
}

/// Rich-Payload parsen + Allowlists (Channel + Rollen) prüfen.
fn parse_rich(
    state: &SharedBroker,
    rid: &str,
    idem: &str,
    payload: &Map<String, Value>,
) -> Result<(RichMessage, Map<String, Value>), Response> {
    let parsed = (|| -> Result<RichMessage, String> {
        let channel_id = payload::positive_int(payload, "channel_id")?;
        let content = payload::optional_content(payload, "content")?;
        let embed = payload::embed_dict(payload)?;
        let components = payload::components(payload)?;
        let allowed_user_ids = payload::id_list(payload, "allowed_user_ids")?;
        let allowed_role_ids = payload::id_list(payload, "allowed_role_ids")?;
        let view_spec = payload::view_spec(payload)?;
        if content.is_none() && embed.is_empty() && components.is_none() {
            return Err("content or embed is required".to_string());
        }
        Ok(RichMessage {
            channel_id,
            content,
            embed: Value::Object(embed),
            components,
            allowed_user_ids,
            allowed_role_ids,
            view_spec,
        })
    })();
    let rich = parsed.map_err(|msg| bad_request(rid, &msg))?;

    allowlist_check(
        rid,
        Some(idem),
        "channel",
        rich.channel_id,
        &state.channel_allowlist,
    )?;
    for role_id in &rich.allowed_role_ids {
        allowlist_check(rid, Some(idem), "role", *role_id, &state.role_allowlist)?;
    }

    let mut op = Map::new();
    op.insert("channel_id".into(), json!(rich.channel_id));
    op.insert("content".into(), json!(rich.content));
    op.insert("embed".into(), rich.embed.clone());
    if let Some(components) = &rich.components {
        op.insert("components".into(), components.clone());
    }
    op.insert("allowed_user_ids".into(), json!(rich.allowed_user_ids));
    op.insert("allowed_role_ids".into(), json!(rich.allowed_role_ids));
    op.insert(
        "view_spec".into(),
        match &rich.view_spec {
            None => Value::Null,
            Some(crate::port::ViewSpec::LinkButton { label, url }) => {
                json!({"type": "link_button", "label": label, "url": url})
            }
            Some(crate::port::ViewSpec::TwitchLiveTracking {
                streamer_login,
                referral_url,
                tracking_token,
                button_label,
            }) => json!({
                "type": "twitch_live_tracking",
                "streamer_login": streamer_login,
                "referral_url": referral_url,
                "tracking_token": tracking_token,
                "button_label": button_label,
            }),
            Some(crate::port::ViewSpec::ScamRevoke {
                verdict_id,
                channel_login,
                chatter_login,
                action_taken,
            }) => json!({
                "type": "scam_revoke",
                "verdict_id": verdict_id,
                "channel_login": channel_login,
                "chatter_login": chatter_login,
                "action_taken": action_taken,
            }),
        },
    );
    Ok((rich, op))
}

pub async fn send_rich_message(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let (rich, op) = match parse_rich(&state, &rid, &idem, &payload) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let hash = payload_hash(&op);

    run_idempotent(&state, &rid, "discord.send_rich_message", &idem, &hash, || async {
        match state.port.send_rich_message(&rich).await {
            Ok(message_id) => (
                200,
                success_body(&rid, Some(&idem), json!({
                    "channel_id": rich.channel_id,
                    // Discord-Snowflake als String (wie das alte Python-master_broker
                    // via str(message_id) und wie die Discord-API selbst) — eine rohe
                    // u64-Zahl sprengt JS-Zahlen und brach das String-Decoding der
                    // Consumer (Twitch-Bot Live-Pings → Doppel-Postings).
                    "message_id": message_id.to_string(),
                })),
            ),
            Err(PortError::ChannelNotFound) => (
                404,
                error_body(&rid, Some(&idem), "not_found",
                    &format!("channel {} not found", rich.channel_id)),
            ),
            Err(err) => {
                tracing::error!(%err, channel_id = rich.channel_id, "send_rich_message fehlgeschlagen");
                (502, error_body(&rid, Some(&idem), "discord_error", "failed to send message"))
            }
        }
    })
    .await
}

pub async fn edit_rich_message(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let message_id = match payload::positive_int(&payload, "message_id") {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };
    let (rich, mut op) = match parse_rich(&state, &rid, &idem, &payload) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    op.insert("message_id".into(), json!(message_id));
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.edit_rich_message",
        &idem,
        &hash,
        || async {
            match state.port.edit_rich_message(message_id, &rich).await {
                Ok(()) => (
                    200,
                    success_body(
                        &rid,
                        Some(&idem),
                        json!({
                            "channel_id": rich.channel_id,
                            "message_id": message_id,
                        }),
                    ),
                ),
                Err(PortError::ChannelNotFound) => (
                    404,
                    error_body(
                        &rid,
                        Some(&idem),
                        "not_found",
                        &format!("channel {} not found", rich.channel_id),
                    ),
                ),
                Err(PortError::MessageNotFound) => (
                    404,
                    error_body(
                        &rid,
                        Some(&idem),
                        "not_found",
                        &format!("message {message_id} not found"),
                    ),
                ),
                Err(err) => {
                    tracing::error!(%err, message_id, "edit_rich_message fehlgeschlagen");
                    (
                        502,
                        error_body(&rid, Some(&idem), "discord_error", "failed to edit message"),
                    )
                }
            }
        },
    )
    .await
}

/// add-role / remove-role teilen sich Parsing + Ablauf.
async fn role_action(
    state: SharedBroker,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
    add: bool,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let default_reason = if add {
        "master-broker:add-role"
    } else {
        "master-broker:remove-role"
    };
    let parsed = (|| -> Result<(u64, u64, u64, String), String> {
        let guild_id = payload::positive_int(&payload, "guild_id")?;
        let user_id = payload::positive_int(&payload, "user_id")?;
        let role_id = payload::positive_int(&payload, "role_id")?;
        let reason = payload
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(default_reason)
            .to_string();
        Ok((guild_id, user_id, role_id, reason))
    })();
    let (guild_id, user_id, role_id, reason) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    if let Err(resp) = allowlist_check(&rid, Some(&idem), "guild", guild_id, &state.guild_allowlist)
    {
        return resp;
    }
    if let Err(resp) = allowlist_check(&rid, Some(&idem), "role", role_id, &state.role_allowlist) {
        return resp;
    }

    let mut op = Map::new();
    op.insert("guild_id".into(), json!(guild_id));
    op.insert("user_id".into(), json!(user_id));
    op.insert("role_id".into(), json!(role_id));
    op.insert("reason".into(), json!(reason));
    let hash = payload_hash(&op);
    let action = if add {
        "discord.add_role"
    } else {
        "discord.remove_role"
    };
    let fail_message = if add {
        "failed to add role"
    } else {
        "failed to remove role"
    };

    run_idempotent(&state, &rid, action, &idem, &hash, || async {
        let result = if add {
            state
                .port
                .add_role(guild_id, user_id, role_id, &reason)
                .await
        } else {
            state
                .port
                .remove_role(guild_id, user_id, role_id, &reason)
                .await
        };
        match result {
            Ok(()) => (
                200,
                success_body(
                    &rid,
                    Some(&idem),
                    json!({
                        "guild_id": guild_id,
                        "user_id": user_id,
                        "role_id": role_id,
                    }),
                ),
            ),
            Err(PortError::GuildNotFound) => (
                404,
                error_body(
                    &rid,
                    Some(&idem),
                    "not_found",
                    &format!("guild {guild_id} not found"),
                ),
            ),
            Err(PortError::RoleNotFound) => (
                404,
                error_body(
                    &rid,
                    Some(&idem),
                    "not_found",
                    &format!("role {role_id} not found"),
                ),
            ),
            Err(PortError::MemberNotFound) => (
                404,
                error_body(
                    &rid,
                    Some(&idem),
                    "not_found",
                    &format!("member {user_id} not found"),
                ),
            ),
            Err(err) => {
                tracing::error!(%err, guild_id, user_id, role_id, "role_action fehlgeschlagen");
                (
                    502,
                    error_body(&rid, Some(&idem), "discord_error", fail_message),
                )
            }
        }
    })
    .await
}

pub async fn add_role(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    role_action(state, peer, headers, body, true).await
}

pub async fn remove_role(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    role_action(state, peer, headers, body, false).await
}

pub async fn create_role(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let parsed = (|| -> Result<(u64, String, bool, String), String> {
        let guild_id = payload::positive_int(&payload, "guild_id")?;
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            return Err("name is required".to_string());
        }
        if name.chars().count() > 100 {
            return Err("name exceeds Discord limit (100)".to_string());
        }
        let mentionable = payload
            .get("mentionable")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let reason = payload
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("master-broker:create-role")
            .to_string();
        Ok((guild_id, name, mentionable, reason))
    })();
    let (guild_id, name, mentionable, reason) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    if let Err(resp) = allowlist_check(&rid, Some(&idem), "guild", guild_id, &state.guild_allowlist)
    {
        return resp;
    }

    let mut op = Map::new();
    op.insert("guild_id".into(), json!(guild_id));
    op.insert("name".into(), json!(name));
    op.insert("mentionable".into(), json!(mentionable));
    op.insert("reason".into(), json!(reason));
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.create_role",
        &idem,
        &hash,
        || async {
            match state
                .port
                .create_role(guild_id, &name, mentionable, &reason)
                .await
            {
                Ok(role_id) => (
                    200,
                    success_body(
                        &rid,
                        Some(&idem),
                        json!({
                            "guild_id": guild_id.to_string(),
                            "role_id": role_id,
                            "name": name,
                        }),
                    ),
                ),
                Err(PortError::GuildNotFound) => (
                    404,
                    error_body(
                        &rid,
                        Some(&idem),
                        "not_found",
                        &format!("guild {guild_id} not found"),
                    ),
                ),
                Err(err) => {
                    tracing::error!(%err, guild_id, "create_role fehlgeschlagen");
                    (
                        502,
                        error_body(&rid, Some(&idem), "discord_error", &err.to_string()),
                    )
                }
            }
        },
    )
    .await
}

pub async fn move_voice(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let parsed = (|| -> Result<(u64, u64, Option<u64>), String> {
        let guild_id = payload::positive_int(&payload, "guild_id")?;
        let user_id = payload::positive_int(&payload, "user_id")?;
        let channel_id = if payload
            .get("channel_id")
            .map(|v| !v.is_null())
            .unwrap_or(false)
        {
            Some(payload::positive_int(&payload, "channel_id")?)
        } else {
            None
        };
        Ok((guild_id, user_id, channel_id))
    })();
    let (guild_id, user_id, channel_id) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    if let Err(resp) = allowlist_check(&rid, Some(&idem), "guild", guild_id, &state.guild_allowlist)
    {
        return resp;
    }
    if let Some(channel_id) = channel_id {
        if let Err(resp) = allowlist_check(
            &rid,
            Some(&idem),
            "channel",
            channel_id,
            &state.channel_allowlist,
        ) {
            return resp;
        }
    }

    let mut op = Map::new();
    op.insert("guild_id".into(), json!(guild_id));
    op.insert("user_id".into(), json!(user_id));
    op.insert("channel_id".into(), json!(channel_id));
    let hash = payload_hash(&op);

    run_idempotent(&state, &rid, "discord.move_voice", &idem, &hash, || async {
        match state.port.move_voice(guild_id, user_id, channel_id).await {
            Ok(()) => (
                200,
                success_body(
                    &rid,
                    Some(&idem),
                    json!({
                        "guild_id": guild_id,
                        "user_id": user_id,
                        "channel_id": channel_id,
                    }),
                ),
            ),
            Err(PortError::GuildNotFound) => (
                404,
                error_body(
                    &rid,
                    Some(&idem),
                    "not_found",
                    &format!("guild {guild_id} not found"),
                ),
            ),
            Err(PortError::MemberNotFound) => (
                404,
                error_body(
                    &rid,
                    Some(&idem),
                    "not_found",
                    &format!("member {user_id} not found"),
                ),
            ),
            Err(PortError::ChannelNotFound) => (
                404,
                error_body(
                    &rid,
                    Some(&idem),
                    "not_found",
                    &format!("channel {} not found", channel_id.unwrap_or(0)),
                ),
            ),
            Err(err) => {
                tracing::error!(%err, guild_id, user_id, "move_voice fehlgeschlagen");
                (
                    502,
                    error_body(&rid, Some(&idem), "discord_error", "failed to move member"),
                )
            }
        }
    })
    .await
}

/// voice-channel/members: Token-Auth, KEINE Idempotenz (reine Abfrage).
pub async fn voice_members(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = authorize(&state, &peer, &headers, &rid) {
        return resp;
    }
    let Ok(payload) = json_object(&body) else {
        return bad_request(&rid, "invalid JSON payload");
    };
    let channel_id = match payload::positive_int(&payload, "channel_id") {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };
    if let Err(resp) = allowlist_check(&rid, None, "channel", channel_id, &state.channel_allowlist)
    {
        return resp;
    }
    match state.port.voice_members(channel_id).await {
        Ok(members) => respond(
            200,
            success_body(
                &rid,
                None,
                json!({
                    "channel_id": channel_id,
                    "members": members.iter().map(|m| json!({
                        "user_id": m.user_id,
                        "display_name": m.display_name,
                    })).collect::<Vec<_>>(),
                }),
            ),
        ),
        Err(PortError::ChannelNotFound) => respond(
            404,
            error_body(
                &rid,
                None,
                "not_found",
                &format!("channel {channel_id} not found"),
            ),
        ),
        Err(PortError::NoVoiceMembers) => respond(
            400,
            error_body(
                &rid,
                None,
                "bad_request",
                &format!("channel {channel_id} does not expose voice members"),
            ),
        ),
        Err(err) => {
            tracing::error!(%err, channel_id, "voice_members fehlgeschlagen");
            respond(
                502,
                error_body(&rid, None, "discord_error", "failed to list voice members"),
            )
        }
    }
}

pub async fn create_invite(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let channel_id = match payload::positive_int(&payload, "channel_id") {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };
    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(512).collect::<String>())
        .unwrap_or_else(|| "master-broker:create-invite".to_string());

    if let Err(resp) = allowlist_check(
        &rid,
        Some(&idem),
        "channel",
        channel_id,
        &state.channel_allowlist,
    ) {
        return resp;
    }

    let mut op = Map::new();
    op.insert("channel_id".into(), json!(channel_id));
    op.insert("reason".into(), json!(reason));
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.create_invite",
        &idem,
        &hash,
        || async {
            match state.port.create_invite(channel_id, &reason).await {
                Ok(invite) => (
                    200,
                    success_body(
                        &rid,
                        Some(&idem),
                        json!({
                            "invite_url": invite.invite_url,
                            "code": invite.code,
                            "channel_id": channel_id,
                            "guild_id": invite.guild_id,
                        }),
                    ),
                ),
                Err(PortError::ChannelNotFound) => (
                    404,
                    error_body(
                        &rid,
                        Some(&idem),
                        "not_found",
                        &format!("channel {channel_id} not found or does not support invites"),
                    ),
                ),
                Err(err) => {
                    tracing::error!(%err, channel_id, "create_invite fehlgeschlagen");
                    (
                        502,
                        error_body(
                            &rid,
                            Some(&idem),
                            "discord_error",
                            "failed to create invite",
                        ),
                    )
                }
            }
        },
    )
    .await
}

pub async fn send_dm(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let parsed = (|| -> Result<(u64, String), String> {
        let user_id = payload::positive_int(&payload, "user_id")?;
        let content = payload::required_content(&payload)?;
        Ok((user_id, content))
    })();
    let (user_id, content) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    let mut op = Map::new();
    op.insert("user_id".into(), json!(user_id));
    op.insert("content".into(), json!(content));
    let hash = payload_hash(&op);

    run_idempotent(&state, &rid, "discord.send_dm", &idem, &hash, || async {
        match state.port.send_dm(user_id, &content).await {
            Ok((dm_channel_id, message_id)) => (
                200,
                success_body(
                    &rid,
                    Some(&idem),
                    json!({
                        "user_id": user_id,
                        "channel_id": dm_channel_id,
                        "message_id": message_id,
                    }),
                ),
            ),
            Err(PortError::UserNotFound) => (
                404,
                error_body(
                    &rid,
                    Some(&idem),
                    "not_found",
                    &format!("user {user_id} not found"),
                ),
            ),
            Err(PortError::DmOpenFailed) => (
                502,
                error_body(
                    &rid,
                    Some(&idem),
                    "discord_error",
                    "failed to open DM channel",
                ),
            ),
            Err(err) => {
                tracing::error!(%err, user_id, "send_dm fehlgeschlagen");
                (
                    502,
                    error_body(&rid, Some(&idem), "discord_error", "failed to send DM"),
                )
            }
        }
    })
    .await
}

pub async fn add_reaction(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (rid, payload, idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let parsed = (|| -> Result<(u64, u64, String), String> {
        let channel_id = payload::positive_int(&payload, "channel_id")?;
        let message_id = payload::positive_int(&payload, "message_id")?;
        let emoji = payload::optional_content(&payload, "emoji")?
            .ok_or_else(|| "emoji is required".to_string())?;
        Ok((channel_id, message_id, emoji))
    })();
    let (channel_id, message_id, emoji) = match parsed {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };

    if let Err(resp) = allowlist_check(
        &rid,
        Some(&idem),
        "channel",
        channel_id,
        &state.channel_allowlist,
    ) {
        return resp;
    }

    let mut op = Map::new();
    op.insert("channel_id".into(), json!(channel_id));
    op.insert("message_id".into(), json!(message_id));
    op.insert("emoji".into(), json!(emoji));
    let hash = payload_hash(&op);

    run_idempotent(
        &state,
        &rid,
        "discord.add_reaction",
        &idem,
        &hash,
        || async {
            match state
                .port
                .add_reaction(channel_id, message_id, &emoji)
                .await
            {
                Ok(()) => (
                    200,
                    success_body(
                        &rid,
                        Some(&idem),
                        json!({
                            "channel_id": channel_id,
                            "message_id": message_id,
                            "emoji": emoji,
                        }),
                    ),
                ),
                Err(PortError::MessageNotFound) => (
                    404,
                    error_body(
                        &rid,
                        Some(&idem),
                        "not_found",
                        &format!("message {message_id} not found"),
                    ),
                ),
                Err(err) => {
                    tracing::error!(%err, channel_id, message_id, "add_reaction fehlgeschlagen");
                    (
                        502,
                        error_body(&rid, Some(&idem), "discord_error", "failed to add reaction"),
                    )
                }
            }
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::port::{
        ChannelInfo, ChannelInfoPort, DiscordPort, GuildMemberInfo, GuildRoles, GuildStats,
        InviteInfo, MemberAccess, MemberInfo, MemberPresence, MessageReaction, ResolvedUser,
        RichMessage, RoleMembers,
    };

    struct UnusedDiscordPort {
        message_reactions: Result<Vec<MessageReaction>, PortError>,
        add_reaction_calls: Mutex<Vec<(u64, u64, String)>>,
        delete_message_calls: Mutex<Vec<(u64, u64, String)>>,
        delete_message_result: Result<(), PortError>,
    }

    #[async_trait::async_trait]
    impl DiscordPort for UnusedDiscordPort {
        async fn community_lobbies(
            &self,
            _guild_id: u64,
            user_id: u64,
        ) -> Result<Vec<crate::port::CommunityLobby>, PortError> {
            if user_id == 99 {
                return Err(PortError::MemberNotFound);
            }
            Ok([42_u64, 43]
                .into_iter()
                .map(|id| crate::port::CommunityLobby {
                    channel_id: id.to_string(),
                    name: "Visible voice".into(),
                    member_count: 2,
                    user_limit: Some(6),
                    mode: Some("normal".into()),
                    intent: None,
                    rank_average: None,
                    rank_samples: 0,
                    requester_present: false,
                    is_streamer_vc: false,
                })
                .collect())
        }

        async fn is_ready(&self) -> bool {
            true
        }

        async fn send_channel_message(
            &self,
            _channel_id: u64,
            _content: &str,
        ) -> Result<u64, PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn send_dm(
            &self,
            _user_id: u64,
            _content: &str,
        ) -> Result<(Option<u64>, u64), PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn create_text_channel(
            &self,
            _category_id: u64,
            _name: &str,
            _topic: Option<&str>,
        ) -> Result<u64, PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn delete_channel(&self, _channel_id: u64) -> Result<(), PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn delete_message(
            &self,
            channel_id: u64,
            message_id: u64,
            reason: &str,
        ) -> Result<(), PortError> {
            self.delete_message_calls
                .lock()
                .expect("delete message calls")
                .push((channel_id, message_id, reason.to_string()));
            self.delete_message_result.clone()
        }

        async fn fetch_message_reactions(
            &self,
            _channel_id: u64,
            _message_id: u64,
        ) -> Result<Vec<MessageReaction>, PortError> {
            self.message_reactions.clone()
        }

        async fn add_reaction(
            &self,
            channel_id: u64,
            message_id: u64,
            emoji: &str,
        ) -> Result<(), PortError> {
            self.add_reaction_calls
                .lock()
                .expect("reaction calls")
                .push((channel_id, message_id, emoji.to_string()));
            Ok(())
        }

        async fn send_rich_message(&self, _message: &RichMessage) -> Result<u64, PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn edit_rich_message(
            &self,
            _message_id: u64,
            _message: &RichMessage,
        ) -> Result<(), PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn add_role(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _role_id: u64,
            _reason: &str,
        ) -> Result<(), PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn create_role(
            &self,
            _guild_id: u64,
            _name: &str,
            _mentionable: bool,
            _reason: &str,
        ) -> Result<u64, PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn remove_role(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _role_id: u64,
            _reason: &str,
        ) -> Result<(), PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn move_voice(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _channel_id: Option<u64>,
        ) -> Result<(), PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn voice_members(&self, _channel_id: u64) -> Result<Vec<MemberInfo>, PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn create_invite(
            &self,
            _channel_id: u64,
            _reason: &str,
        ) -> Result<InviteInfo, PortError> {
            Err(PortError::Discord("unused".to_string()))
        }

        async fn list_roles(&self, _guild_id: Option<u64>) -> Result<GuildRoles, PortError> {
            Err(PortError::GuildNotFound)
        }

        async fn role_members(
            &self,
            _guild_id: Option<u64>,
            _role_id: u64,
        ) -> Result<RoleMembers, PortError> {
            Err(PortError::GuildNotFound)
        }

        async fn member_access(
            &self,
            _guild_id: Option<u64>,
            _user_id: u64,
        ) -> Result<MemberAccess, PortError> {
            Err(PortError::GuildNotFound)
        }

        async fn member_present(
            &self,
            _guild_id: u64,
            _user_id: u64,
        ) -> Result<MemberPresence, PortError> {
            Err(PortError::GuildNotFound)
        }

        async fn resolve_names(&self, _user_ids: &[u64]) -> Result<Vec<MemberInfo>, PortError> {
            Ok(Vec::new())
        }

        async fn resolve_user(&self, _user_id: u64) -> Result<Option<ResolvedUser>, PortError> {
            Ok(None)
        }

        async fn list_members(&self) -> Result<Vec<GuildMemberInfo>, PortError> {
            Ok(Vec::new())
        }

        async fn guild_stats(&self, _guild_id: Option<u64>) -> Result<GuildStats, PortError> {
            Err(PortError::GuildNotFound)
        }
    }

    struct MockChannelInfoPort;

    #[async_trait::async_trait]
    impl ChannelInfoPort for MockChannelInfoPort {
        async fn channel_info(
            &self,
            guild_id: Option<u64>,
            channel_id: u64,
        ) -> Result<ChannelInfo, PortError> {
            if guild_id == Some(99) {
                return Err(PortError::GuildNotFound);
            }
            if channel_id != 42 {
                return Err(PortError::ChannelNotFound);
            }
            Ok(ChannelInfo {
                channel_id,
                name: "builds".to_string(),
                parent_id: Some(7),
                last_message_id: Some(700),
            })
        }
    }

    #[tokio::test]
    async fn community_directory_requires_token_and_confirmed_membership() {
        let state = test_state().unwrap();
        let peer = ConnectInfo("127.0.0.1:12345".parse::<SocketAddr>().unwrap());
        let body = axum::body::Bytes::from(r#"{"guild_id":"1289721245281292288","user_id":"99"}"#);
        let missing =
            community_lobbies(State(state.clone()), peer, HeaderMap::new(), body.clone()).await;
        assert_eq!(missing.status(), 401);
        let mut headers = HeaderMap::new();
        headers.insert("X-Internal-Token", "secret".parse().unwrap());
        let response = community_lobbies(State(state), peer, headers, body).await;
        assert_eq!(response.status(), 403);
    }

    #[tokio::test]
    async fn community_directory_respects_channel_allowlist_and_omits_member_identities() {
        let (state, _) = reaction_test_state("42").unwrap();
        let peer = ConnectInfo("127.0.0.1:12345".parse::<SocketAddr>().unwrap());
        let mut headers = HeaderMap::new();
        headers.insert("X-Internal-Token", "secret".parse().unwrap());
        let body = axum::body::Bytes::from(r#"{"guild_id":"1289721245281292288","user_id":"123"}"#);
        let response = community_lobbies(State(state), peer, headers, body).await;
        assert_eq!(response.status(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 10000)
            .await
            .unwrap();
        let data: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(data["result"]["lobbies"].as_array().unwrap().len(), 1);
        assert_eq!(data["result"]["lobbies"][0]["channel_id"], "42");
        assert!(data["result"]["captured_at"].as_u64().unwrap() > 0);
        assert!(!String::from_utf8(bytes.to_vec())
            .unwrap()
            .contains("user_id"));
    }

    fn test_state() -> Result<SharedBroker, String> {
        test_state_with_reactions(Err(PortError::Discord("unused".to_string())))
    }

    fn test_state_with_reactions(
        message_reactions: Result<Vec<MessageReaction>, PortError>,
    ) -> Result<SharedBroker, String> {
        crate::BrokerState::new_with_channel_info(
            Arc::new(UnusedDiscordPort {
                message_reactions,
                add_reaction_calls: Mutex::new(Vec::new()),
                delete_message_calls: Mutex::new(Vec::new()),
                delete_message_result: Err(PortError::Discord("unused".to_string())),
            }),
            Arc::new(MockChannelInfoPort),
            "secret".to_string(),
            |_| None,
        )
    }

    fn reaction_test_state(
        allowed_channel_ids: &str,
    ) -> Result<(SharedBroker, Arc<UnusedDiscordPort>), String> {
        let port = Arc::new(UnusedDiscordPort {
            message_reactions: Err(PortError::Discord("unused".to_string())),
            add_reaction_calls: Mutex::new(Vec::new()),
            delete_message_calls: Mutex::new(Vec::new()),
            delete_message_result: Err(PortError::Discord("unused".to_string())),
        });
        let allowed_channel_ids = allowed_channel_ids.to_string();
        let state = crate::BrokerState::new_with_channel_info(
            port.clone(),
            Arc::new(MockChannelInfoPort),
            "secret".to_string(),
            move |key| {
                (key == "MASTER_BROKER_ALLOWED_CHANNEL_IDS").then(|| allowed_channel_ids.clone())
            },
        )?;
        Ok((state, port))
    }

    fn delete_message_test_state(
        result: Result<(), PortError>,
    ) -> Result<(SharedBroker, Arc<UnusedDiscordPort>), String> {
        let port = Arc::new(UnusedDiscordPort {
            message_reactions: Err(PortError::Discord("unused".to_string())),
            add_reaction_calls: Mutex::new(Vec::new()),
            delete_message_calls: Mutex::new(Vec::new()),
            delete_message_result: result,
        });
        let state = crate::BrokerState::new_with_channel_info(
            port.clone(),
            Arc::new(MockChannelInfoPort),
            "secret".to_string(),
            |key| (key == "MASTER_BROKER_ALLOWED_CHANNEL_IDS").then(|| "42".to_string()),
        )?;
        Ok((state, port))
    }

    fn action_headers(idempotency_key: &str) -> Result<HeaderMap, axum::http::Error> {
        let mut headers = HeaderMap::new();
        headers.insert(crate::TOKEN_HEADER, "secret".parse()?);
        headers.insert(IDEMPOTENCY_HEADER, idempotency_key.parse()?);
        Ok(headers)
    }

    fn peer(addr: &str) -> Result<Peer, std::net::AddrParseError> {
        Ok(ConnectInfo(addr.parse()?))
    }

    #[test]
    fn parse_rich_akzeptiert_components_ohne_content_oder_embed() -> Result<(), String> {
        let state = test_state()?;
        let components = json!([{
            "type": 17,
            "accent_color": 0xC8A86B,
            "components": [
                {"type": 10, "content": "LIVE"},
                {"type": 12, "items": [{
                    "media": {"url": "https://example.test/preview.jpg"},
                }]},
            ],
        }]);
        let payload = json!({
            "channel_id": 123,
            "embed": {},
            "components": components.clone(),
        })
        .as_object()
        .expect("payload object")
        .clone();

        let parsed = parse_rich(&state, "rid", "idem", &payload);
        assert!(parsed.is_ok());
        let (rich, op) = parsed.expect("parsed rich payload");

        assert_eq!(rich.content, None);
        assert_eq!(rich.embed, json!({}));
        assert_eq!(rich.components.as_ref(), Some(&components));
        assert_eq!(op.get("components"), Some(&components));
        Ok(())
    }

    async fn response_json(response: Response) -> Result<(u16, Value), Box<dyn std::error::Error>> {
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await?;
        let value = serde_json::from_slice(&bytes)?;
        Ok((status, value))
    }

    #[tokio::test]
    async fn channel_info_returns_loopback_metadata_without_token(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut params = HashMap::new();
        params.insert("channel_id".to_string(), "42".to_string());
        let response = channel_info(
            State(test_state()?),
            peer("127.0.0.1:3456")?,
            HeaderMap::new(),
            Query(params),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(
            body,
            json!({
                "ok": true,
                "channel_id": "42",
                "name": "builds",
                "parent_id": "7",
                "last_message_id": "700",
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn channel_info_keeps_python_not_found_errors() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut channel_params = HashMap::new();
        channel_params.insert("channel_id".to_string(), "404".to_string());
        let channel_response = channel_info(
            State(test_state()?),
            peer("127.0.0.1:3456")?,
            HeaderMap::new(),
            Query(channel_params),
        )
        .await;
        let (channel_status, channel_body) = response_json(channel_response).await?;
        assert_eq!(channel_status, 404);
        assert_eq!(channel_body["error"]["message"], "channel 404 not found");

        let mut guild_params = HashMap::new();
        guild_params.insert("guild_id".to_string(), "99".to_string());
        guild_params.insert("channel_id".to_string(), "42".to_string());
        let guild_response = channel_info(
            State(test_state()?),
            peer("127.0.0.1:3456")?,
            HeaderMap::new(),
            Query(guild_params),
        )
        .await;
        let (guild_status, guild_body) = response_json(guild_response).await?;
        assert_eq!(guild_status, 404);
        assert_eq!(guild_body["error"]["message"], "guild not found");

        Ok(())
    }

    #[tokio::test]
    async fn channel_info_rejects_non_loopback_without_token_check(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let response = channel_info(
            State(test_state()?),
            peer("10.0.0.5:3456")?,
            HeaderMap::new(),
            Query(HashMap::new()),
        )
        .await;
        let (status, body) = response_json(response).await?;
        assert_eq!(status, 403);
        assert_eq!(body["error"]["code"], "forbidden");
        Ok(())
    }

    #[tokio::test]
    async fn resolve_user_is_authorized_read_without_idempotency(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(crate::TOKEN_HEADER, "secret".parse()?);
        let response = resolve_user(
            State(test_state()?),
            peer("127.0.0.1:3456")?,
            headers,
            axum::body::Bytes::from_static(br#"{"user_id":42}"#),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["result"]["found"], false);
        Ok(())
    }

    fn reaction_params(channel_id: &str, message_id: &str) -> HashMap<String, String> {
        HashMap::from([
            ("channel_id".to_string(), channel_id.to_string()),
            ("message_id".to_string(), message_id.to_string()),
        ])
    }

    #[tokio::test]
    async fn message_reactions_requires_auth() -> Result<(), Box<dyn std::error::Error>> {
        let response = message_reactions(
            State(test_state()?),
            peer("127.0.0.1:3456")?,
            HeaderMap::new(),
            Query(reaction_params("42", "700")),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 401);
        assert_eq!(body["error"]["code"], "unauthorized");
        Ok(())
    }

    #[tokio::test]
    async fn message_reactions_rejects_invalid_query_params(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(crate::TOKEN_HEADER, "secret".parse()?);
        let missing_response = message_reactions(
            State(test_state()?),
            peer("127.0.0.1:3456")?,
            headers.clone(),
            Query(HashMap::new()),
        )
        .await;
        let (missing_status, missing_body) = response_json(missing_response).await?;
        assert_eq!(missing_status, 400);
        assert_eq!(missing_body["error"]["code"], "bad_request");
        assert_eq!(
            missing_body["error"]["message"],
            "channel_id must be a positive integer"
        );

        let response = message_reactions(
            State(test_state()?),
            peer("127.0.0.1:3456")?,
            headers,
            Query(reaction_params("broken", "700")),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 400);
        assert_eq!(body["error"]["code"], "bad_request");
        assert_eq!(
            body["error"]["message"],
            "channel_id must be a positive integer"
        );
        Ok(())
    }

    #[tokio::test]
    async fn message_reactions_maps_not_found_to_empty_result(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(crate::TOKEN_HEADER, "secret".parse()?);
        let response = message_reactions(
            State(test_state_with_reactions(Err(PortError::MessageNotFound))?),
            peer("127.0.0.1:3456")?,
            headers,
            Query(reaction_params("42", "700")),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(body, json!({"found": false, "reactions": []}));
        Ok(())
    }

    #[tokio::test]
    async fn message_reactions_returns_unicode_and_custom_emoji(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(crate::TOKEN_HEADER, "secret".parse()?);
        let response = message_reactions(
            State(test_state_with_reactions(Ok(vec![
                MessageReaction {
                    emoji: "👍".to_string(),
                    count: 2,
                },
                MessageReaction {
                    emoji: "party:123".to_string(),
                    count: 5,
                },
            ]))?),
            peer("127.0.0.1:3456")?,
            headers,
            Query(reaction_params("42", "700")),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(
            body,
            json!({
                "found": true,
                "reactions": [
                    {"emoji": "👍", "count": 2},
                    {"emoji": "party:123", "count": 5},
                ],
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn message_reactions_maps_discord_errors_to_bad_gateway(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(crate::TOKEN_HEADER, "secret".parse()?);
        let response = message_reactions(
            State(test_state_with_reactions(Err(PortError::Discord(
                "timeout".to_string(),
            )))?),
            peer("127.0.0.1:3456")?,
            headers,
            Query(reaction_params("42", "700")),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 502);
        assert_eq!(body, json!({"error": "Discord request failed"}));
        Ok(())
    }

    #[tokio::test]
    async fn add_reaction_calls_port_with_valid_payload() -> Result<(), Box<dyn std::error::Error>>
    {
        let (state, port) = reaction_test_state("42")?;
        let response = add_reaction(
            State(state),
            peer("127.0.0.1:3456")?,
            action_headers("reaction-happy")?,
            axum::body::Bytes::from_static(
                br#"{"channel_id":42,"message_id":700,"emoji":"\u2705"}"#,
            ),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(
            *port.add_reaction_calls.lock().expect("reaction calls"),
            vec![(42, 700, "✅".to_string())]
        );
        Ok(())
    }

    #[tokio::test]
    async fn add_reaction_rejects_channel_outside_allowlist_without_calling_port(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (state, port) = reaction_test_state("41")?;
        let response = add_reaction(
            State(state),
            peer("127.0.0.1:3456")?,
            action_headers("reaction-forbidden")?,
            axum::body::Bytes::from_static(
                br#"{"channel_id":42,"message_id":700,"emoji":"\u2705"}"#,
            ),
        )
        .await;

        let (status, _) = response_json(response).await?;
        assert_eq!(status, 403);
        assert!(port
            .add_reaction_calls
            .lock()
            .expect("reaction calls")
            .is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn add_reaction_rejects_missing_or_invalid_fields(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (state, port) = reaction_test_state("42")?;
        for (key, body) in [
            (
                "reaction-missing",
                br#"{"channel_id":42,"message_id":700}"#.as_slice(),
            ),
            (
                "reaction-invalid",
                br#"{"channel_id":42,"message_id":0,"emoji":"\u2705"}"#.as_slice(),
            ),
        ] {
            let response = add_reaction(
                State(state.clone()),
                peer("127.0.0.1:3456")?,
                action_headers(key)?,
                axum::body::Bytes::copy_from_slice(body),
            )
            .await;
            let (status, _) = response_json(response).await?;
            assert_eq!(status, 400);
        }
        assert!(port
            .add_reaction_calls
            .lock()
            .expect("reaction calls")
            .is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn add_reaction_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let (state, port) = reaction_test_state("42")?;
        let body = axum::body::Bytes::from_static(
            br#"{"channel_id":42,"message_id":700,"emoji":"\u2705"}"#,
        );

        for _ in 0..2 {
            let response = add_reaction(
                State(state.clone()),
                peer("127.0.0.1:3456")?,
                action_headers("reaction-idempotent")?,
                body.clone(),
            )
            .await;
            let (status, _) = response_json(response).await?;
            assert_eq!(status, 200);
        }
        assert_eq!(
            port.add_reaction_calls
                .lock()
                .expect("reaction calls")
                .len(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn delete_message_ruft_port_mit_channel_message_und_reason(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (state, port) = delete_message_test_state(Ok(()))?;
        let response = delete_message(
            State(state),
            peer("127.0.0.1:3456")?,
            action_headers("delete-message-happy")?,
            axum::body::Bytes::from_static(
                br#"{"channel_id":"42","message_id":"700","reason":"review retention"}"#,
            ),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(body["result"]["already_absent"], false);
        assert_eq!(
            *port
                .delete_message_calls
                .lock()
                .expect("delete message calls"),
            vec![(42, 700, "review retention".to_string())]
        );
        Ok(())
    }

    #[tokio::test]
    async fn delete_message_ist_bei_fehlender_nachricht_idempotent(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (state, port) = delete_message_test_state(Err(PortError::MessageNotFound))?;
        let response = delete_message(
            State(state),
            peer("127.0.0.1:3456")?,
            action_headers("delete-message-absent")?,
            axum::body::Bytes::from_static(
                br#"{"channel_id":"42","message_id":"700","reason":"review retention"}"#,
            ),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(body["result"]["already_absent"], true);
        assert_eq!(
            port.delete_message_calls
                .lock()
                .expect("delete message calls")
                .len(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn delete_message_ist_bei_fehlendem_kanal_idempotent(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (state, _) = delete_message_test_state(Err(PortError::ChannelNotFound))?;
        let response = delete_message(
            State(state),
            peer("127.0.0.1:3456")?,
            action_headers("delete-message-channel-absent")?,
            axum::body::Bytes::from_static(
                br#"{"channel_id":"42","message_id":"700","reason":"review retention"}"#,
            ),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 200);
        assert_eq!(body["result"]["already_absent"], true);
        Ok(())
    }

    #[tokio::test]
    async fn delete_message_verdeckt_interne_discord_fehler(
    ) -> Result<(), Box<dyn std::error::Error>> {
        const SENTINEL: &str = "INTERNAL_DELETE_SENTINEL";
        let (state, _) = delete_message_test_state(Err(PortError::Discord(SENTINEL.to_string())))?;
        let response = delete_message(
            State(state),
            peer("127.0.0.1:3456")?,
            action_headers("delete-message-discord-error")?,
            axum::body::Bytes::from_static(
                br#"{"channel_id":"42","message_id":"700","reason":"review retention"}"#,
            ),
        )
        .await;

        let (status, body) = response_json(response).await?;
        assert_eq!(status, 502);
        assert!(!body.to_string().contains(SENTINEL));
        Ok(())
    }
}

/// Authenticated, aggregate-only, member-scoped community directory.
pub async fn community_lobbies(
    State(state): State<SharedBroker>,
    peer: Peer,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let rid = request_id(&headers);
    if let Err(resp) = authorize(&state, &peer, &headers, &rid) {
        return resp;
    }
    let Ok(payload) = json_object(&body) else {
        return bad_request(&rid, "invalid JSON payload");
    };
    let guild_id = match payload::positive_int(&payload, "guild_id") {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };
    let user_id = match payload::positive_int(&payload, "user_id") {
        Ok(v) => v,
        Err(msg) => return bad_request(&rid, &msg),
    };
    if guild_id != 1289721245281292288 {
        return bad_request(&rid, "unsupported community");
    }
    if let Err(resp) = allowlist_check(&rid, None, "guild", guild_id, &state.guild_allowlist) {
        return resp;
    }
    if !state.port.is_ready().await {
        return respond(
            503,
            error_body(&rid, None, "unavailable", "Discord gateway unavailable"),
        );
    }
    match state.port.community_lobbies(guild_id, user_id).await {
        Ok(mut lobbies) => {
            lobbies.retain(|lobby| {
                lobby.channel_id.parse::<u64>().is_ok_and(|id| {
                    allowlist_check(&rid, None, "channel", id, &state.channel_allowlist).is_ok()
                })
            });
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            respond(
                200,
                success_body(&rid, None, json!({"captured_at": now, "lobbies": lobbies})),
            )
        }
        Err(PortError::MemberNotFound) => respond(
            403,
            error_body(
                &rid,
                None,
                "member_required",
                "Discord membership could not be confirmed",
            ),
        ),
        Err(err) => {
            tracing::warn!(%err, "Community lobby directory unavailable");
            respond(
                503,
                error_body(
                    &rid,
                    None,
                    "unavailable",
                    "Discord lobby directory unavailable",
                ),
            )
        }
    }
}
