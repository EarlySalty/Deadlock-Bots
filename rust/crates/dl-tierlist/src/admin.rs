//! Admin-Endpunkte — Auth über den Dashboard-Session-Cookie
//! `master_dash_session`, validiert per interner Dashboard-API (statt des
//! Python-In-Process-Griffs in fremde Session-Dicts).

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use dl_webcore::dashboard::ValidatedSession;
use dl_webcore::envelope::{error_message, unauthorized_tierlist};
use rusqlite::OptionalExtension;
use serde_json::{json, Map, Value};

use crate::data::admin_hero_payload;
use crate::refresh::{refresh_once, RefreshError};
use crate::settings::{normalize_thresholds, read_settings, set_setting, SESSION_COOKIE};
use crate::util::{coerce_bool, coerce_int, now_ts, parse_unix_or_iso};
use crate::SharedApp;

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let (k, v) = pair.trim().split_once('=')?;
        if k == name {
            return Some(v.trim().to_string());
        }
    }
    None
}

async fn require_admin(app: &SharedApp, headers: &HeaderMap) -> Result<ValidatedSession, Response> {
    let Some(session_id) = cookie_value(headers, SESSION_COOKIE) else {
        return Err(unauthorized_tierlist());
    };
    match app.dashboard.validate_session(&session_id).await {
        Some(session) => Ok(session),
        None => Err(unauthorized_tierlist()),
    }
}

fn bad_request(code: &str, message: &str) -> Response {
    error_message(StatusCode::BAD_REQUEST, code, message)
}

fn internal(err: impl std::fmt::Display) -> Response {
    tracing::error!(%err, "Tierlist-Admin: interner Fehler");
    error_message(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Interner Serverfehler.",
    )
}

pub async fn handle_admin_me(State(app): State<SharedApp>, headers: HeaderMap) -> Response {
    let session = match require_admin(&app, &headers).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let id = session.user_id.parse::<i64>().unwrap_or(0);
    let username = if session.display_name.is_empty() {
        session.username
    } else {
        session.display_name
    };
    Json(json!({ "id": id, "username": username })).into_response()
}

pub async fn handle_admin_hero_get(
    State(app): State<SharedApp>,
    Path(hero_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = require_admin(&app, &headers).await {
        return resp;
    }
    let Ok(hero_id) = hero_id.trim().parse::<i64>() else {
        return bad_request("invalid_hero_id", "Ungültige Hero-ID.");
    };
    match app.db.read(move |c| admin_hero_payload(c, hero_id)).await {
        Ok(Some(payload)) => Json(payload).into_response(),
        Ok(None) => error_message(
            StatusCode::NOT_FOUND,
            "hero_not_found",
            "Hero nicht gefunden.",
        ),
        Err(err) => internal(err),
    }
}

pub async fn handle_admin_hero_put(
    State(app): State<SharedApp>,
    Path(hero_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    if let Err(resp) = require_admin(&app, &headers).await {
        return resp;
    }
    let Ok(hero_id) = hero_id.trim().parse::<i64>() else {
        return bad_request("invalid_hero_id", "Ungültige Hero-ID.");
    };
    let Some(payload) = read_json_object(body) else {
        return bad_request("invalid_json", "Ungültiger JSON-Body.");
    };

    let hero_exists = app
        .db
        .read(move |conn| {
            conn.query_row(
                "SELECT 1 FROM deadlock_heroes WHERE hero_id = ?1 LIMIT 1",
                [hero_id],
                |_| Ok(()),
            )
            .optional()
        })
        .await;
    match hero_exists {
        Ok(Some(())) => {}
        Ok(None) => {
            return error_message(
                StatusCode::NOT_FOUND,
                "hero_not_found",
                "Hero nicht gefunden.",
            )
        }
        Err(err) => return internal(err),
    }

    let has_description = payload.contains_key("description");
    let has_streamers = payload.contains_key("streamers");
    let has_builds_meta = payload.contains_key("builds_meta") || payload.contains_key("buildsMeta");

    let description = payload
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // Streamer parsen + validieren (Fehlertexte wie Python, 1-basierter Index)
    let mut parsed_streamers: Vec<(String, String, i64, bool)> = Vec::new();
    if has_streamers {
        let Some(raw) = payload.get("streamers").and_then(Value::as_array) else {
            return bad_request("invalid_streamers", "streamers muss ein Array sein.");
        };
        for (index, item) in raw.iter().enumerate() {
            let index = index + 1;
            let Some(obj) = item.as_object() else {
                return bad_request(
                    "invalid_streamers",
                    &format!("streamers[{index}] muss ein Objekt sein."),
                );
            };
            let login = obj
                .get("twitch_login")
                .or_else(|| obj.get("twitchLogin"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            if login.is_empty() {
                return bad_request(
                    "invalid_streamers",
                    &format!("streamers[{index}].twitch_login fehlt."),
                );
            }
            let display_name = obj
                .get("display_name")
                .or_else(|| obj.get("displayName"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&login)
                .to_string();
            let sort_order = coerce_int(
                obj.get("sort_order").or_else(|| obj.get("sortOrder")),
                Some(100),
            )
            .filter(|v| *v != 0)
            .unwrap_or(100);
            let is_active = coerce_bool(obj.get("is_active").or_else(|| obj.get("isActive")), true);
            parsed_streamers.push((login, display_name, sort_order, is_active));
        }
    }

    // builds_meta parsen + gegen vorhandene Builds validieren
    let mut parsed_builds: Vec<(i64, i64, bool)> = Vec::new();
    if has_builds_meta {
        let raw = payload
            .get("builds_meta")
            .or_else(|| payload.get("buildsMeta"));
        let Some(raw) = raw.and_then(Value::as_array) else {
            return bad_request("invalid_builds_meta", "builds_meta muss ein Array sein.");
        };
        let existing: Result<std::collections::HashSet<i64>, _> = app
            .db
            .read(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT build_id FROM deadlock_hero_builds WHERE hero_id = ?1")?;
                let rows = stmt.query_map([hero_id], |row| row.get::<_, i64>(0))?;
                rows.collect()
            })
            .await;
        let existing = match existing {
            Ok(set) => set,
            Err(err) => return internal(err),
        };
        for (index, item) in raw.iter().enumerate() {
            let index = index + 1;
            let Some(obj) = item.as_object() else {
                return bad_request(
                    "invalid_builds_meta",
                    &format!("builds_meta[{index}] muss ein Objekt sein."),
                );
            };
            let build_id = coerce_int(obj.get("build_id").or_else(|| obj.get("buildId")), None);
            let Some(build_id) = build_id.filter(|id| existing.contains(id)) else {
                return bad_request(
                    "invalid_builds_meta",
                    &format!("builds_meta[{index}].build_id ist unbekannt."),
                );
            };
            let sort_order = coerce_int(
                obj.get("sort_order").or_else(|| obj.get("sortOrder")),
                Some(100),
            )
            .filter(|v| *v != 0)
            .unwrap_or(100);
            let is_active = coerce_bool(obj.get("is_active").or_else(|| obj.get("isActive")), true);
            parsed_builds.push((build_id, sort_order, is_active));
        }
    }

    let write_result = app
        .db
        .write(move |conn| {
            let ts = now_ts();
            let tx = conn.transaction()?;
            if has_description {
                tx.execute(
                    "INSERT INTO tierlist_hero_meta(hero_id, description, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(hero_id) DO UPDATE SET
                       description = excluded.description,
                       updated_at = excluded.updated_at",
                    (hero_id, &description, ts),
                )?;
            }
            if has_streamers {
                tx.execute(
                    "DELETE FROM tierlist_streamers WHERE hero_id = ?1",
                    [hero_id],
                )?;
                for (login, display_name, sort_order, is_active) in &parsed_streamers {
                    tx.execute(
                        "INSERT INTO tierlist_streamers(
                             hero_id, twitch_login, display_name, sort_order, is_active, created_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        (
                            hero_id,
                            login,
                            display_name,
                            sort_order,
                            i64::from(*is_active),
                            ts,
                        ),
                    )?;
                }
            }
            if has_builds_meta {
                for (build_id, sort_order, is_active) in &parsed_builds {
                    tx.execute(
                        "UPDATE deadlock_hero_builds
                            SET sort_order = ?1, is_active = ?2, updated_at = ?3
                          WHERE hero_id = ?4 AND build_id = ?5",
                        (sort_order, i64::from(*is_active), ts, hero_id, build_id),
                    )?;
                }
            }
            tx.commit()?;
            admin_hero_payload(conn, hero_id)
        })
        .await;

    match write_result {
        Ok(Some(hero)) => Json(json!({ "ok": true, "hero": hero })).into_response(),
        Ok(None) => error_message(
            StatusCode::NOT_FOUND,
            "hero_not_found",
            "Hero nicht gefunden.",
        ),
        Err(err) => internal(err),
    }
}

pub async fn handle_admin_settings_get(
    State(app): State<SharedApp>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = require_admin(&app, &headers).await {
        return resp;
    }
    match app.db.write(|c| read_settings(c)).await {
        Ok(settings) => Json(settings).into_response(),
        Err(err) => internal(err),
    }
}

pub async fn handle_admin_settings_put(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    if let Err(resp) = require_admin(&app, &headers).await {
        return resp;
    }
    let Some(payload) = read_json_object(body) else {
        return bad_request("invalid_json", "Ungültiger JSON-Body.");
    };

    let current = match app.db.write(|c| read_settings(c)).await {
        Ok(s) => s,
        Err(err) => return internal(err),
    };
    let mut current = current;

    // Pro Feld: validieren → persistieren → in-memory mergen (wie Python).
    let mut writes: Vec<(&'static str, String)> = Vec::new();

    if let Some(raw) = payload.get("thresholds") {
        match normalize_thresholds(raw) {
            Ok(thresholds) => {
                let json_value = serde_json::to_value(thresholds).unwrap_or_default();
                let sorted: std::collections::BTreeMap<String, Value> = json_value
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                writes.push((
                    "thresholds_json",
                    serde_json::to_string(&sorted).unwrap_or_default(),
                ));
                current.thresholds = thresholds;
            }
            Err(msg) => return bad_request("invalid_thresholds", &msg),
        }
    }

    if payload.contains_key("refresh_interval_seconds")
        || payload.contains_key("refreshIntervalSeconds")
    {
        let raw = payload
            .get("refresh_interval_seconds")
            .or_else(|| payload.get("refreshIntervalSeconds"));
        match coerce_int(raw, None).filter(|v| *v > 0) {
            Some(value) => {
                writes.push(("refresh_interval_seconds", value.to_string()));
                current.refresh_interval_seconds = value;
            }
            None => {
                return bad_request(
                    "invalid_refresh_interval",
                    "refresh_interval_seconds muss > 0 sein.",
                )
            }
        }
    }

    if payload.contains_key("min_matches") || payload.contains_key("minMatches") {
        let raw = payload
            .get("min_matches")
            .or_else(|| payload.get("minMatches"));
        match coerce_int(raw, None).filter(|v| *v >= 0) {
            Some(value) => {
                writes.push(("min_matches", value.to_string()));
                current.min_matches = value;
            }
            None => return bad_request("invalid_min_matches", "min_matches muss >= 0 sein."),
        }
    }

    if payload.contains_key("patch_override_unix") || payload.contains_key("patchOverrideUnix") {
        let raw = payload
            .get("patch_override_unix")
            .or_else(|| payload.get("patchOverrideUnix"));
        let override_value = parse_unix_or_iso(raw);
        let raw_is_clearing = matches!(raw, None | Some(Value::Null) | Some(Value::Bool(false)))
            || matches!(raw, Some(Value::String(s)) if s.is_empty());
        if !raw_is_clearing && override_value.is_none() {
            return bad_request(
                "invalid_patch_override",
                "patch_override_unix muss ein Unix-Timestamp, ISO-Datum oder null sein.",
            );
        }
        writes.push((
            "patch_override_unix",
            override_value.map(|v| v.to_string()).unwrap_or_default(),
        ));
        current.patch_override_unix = override_value;
    }

    if payload.contains_key("description_text") || payload.contains_key("descriptionText") {
        let text = payload
            .get("description_text")
            .or_else(|| payload.get("descriptionText"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        writes.push(("description_text", text.clone()));
        current.description_text = text;
    }

    if !writes.is_empty() {
        let result = app
            .db
            .write(move |conn| {
                for (key, value) in &writes {
                    set_setting(conn, key, value)?;
                }
                Ok(())
            })
            .await;
        if let Err(err) = result {
            return internal(err);
        }
    }

    Json(current).into_response()
}

pub async fn handle_admin_refresh(State(app): State<SharedApp>, headers: HeaderMap) -> Response {
    if let Err(resp) = require_admin(&app, &headers).await {
        return resp;
    }
    match refresh_once(&app).await {
        Ok(summary) => Json(summary).into_response(),
        Err(RefreshError::Retryable(msg)) => {
            error_message(StatusCode::BAD_GATEWAY, "upstream_unavailable", &msg)
        }
        Err(RefreshError::Fatal(msg)) => internal(msg),
    }
}

fn read_json_object(body: Option<Json<Value>>) -> Option<Map<String, Value>> {
    body.and_then(|Json(v)| v.as_object().cloned())
}
