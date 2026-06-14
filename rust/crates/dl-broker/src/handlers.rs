//! Die Broker-Routen — Reihenfolge der Checks und alle Fehlertexte
//! exakt wie master_broker.py.

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{json, Map, Value};

use crate::payload::{self};
use crate::port::{PortError, RichMessage};
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
pub async fn members(State(state): State<SharedBroker>, peer: Peer, headers: HeaderMap) -> Response {
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
    let (rid, payload, _idem) = match begin_action(&state, &peer, &headers, &body) {
        Ok(v) => v,
        Err(resp) => return resp,
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
                let user_id = user_id.expect("user_id gesetzt wenn channel_id fehlt");
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
        let allowed_user_ids = payload::id_list(payload, "allowed_user_ids")?;
        let allowed_role_ids = payload::id_list(payload, "allowed_role_ids")?;
        let view_spec = payload::view_spec(payload)?;
        if content.is_none() && embed.is_empty() {
            return Err("content or embed is required".to_string());
        }
        Ok(RichMessage {
            channel_id,
            content,
            embed: Value::Object(embed),
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

    run_idempotent(&state, &rid, "discord.create_role", &idem, &hash, || async {
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
    })
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
