//! Turnier-Admin-Mutationen (`/api/turnier/...`) — Port von
//! `service/dashboard.py` (6410–6580).
//!
//! Schreibende Admin-Aktionen über `dl-tournament::store`, gegatet via
//! [`DashboardApp::guard_mutate`] (Turnier-Mod ODER Voll-Zugriff, wie
//! `_check_turnier_auth`). IDs werden in der Antwort wie im Original zu
//! Strings gewandelt (`_stringify_ids`). Die lesenden overview/bracket-Routen
//! brauchen Bot-Daten (Gilden-Namen/Member) und folgen separat.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use dl_tournament::store::TournamentStore;
use serde_json::{json, Map, Value};

use crate::web::{err_json, err_text, ok_json, ok_json_status, DashboardApp};

fn store(app: &DashboardApp) -> TournamentStore {
    TournamentStore::new(app.db().clone())
}

/// JSON-Objekt aus dem Body (Fehlertexte wie im Original).
fn parse_obj(body: &Bytes) -> Result<Value, Response> {
    match serde_json::from_slice::<Value>(body) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(err_text(400, "Payload must be JSON object")),
        Err(_) => Err(err_text(400, "Invalid JSON payload")),
    }
}

/// Int-Coercion wie `_coerce_int`: Zahl ODER numerischer String.
fn coerce_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn coerce_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn string_field(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

/// Gilde aus dem Payload, sonst die Standard-Gilde (wie
/// `_resolve_tournament_guild_id`, nur ohne Bot — Default aus der Config).
fn resolve_guild(app: &DashboardApp, payload: &Value) -> Result<u64, Response> {
    if let Some(g) = coerce_u64(payload.get("guild_id")) {
        return Ok(g);
    }
    let default = app.tournament_default_guild();
    if default != 0 {
        return Ok(default);
    }
    Err(err_text(400, "guild_id is required"))
}

/// Wandelt große Int-IDs in Strings (rekursiv) — wie `_stringify_ids`.
pub(crate) fn stringify_ids(value: Value) -> Value {
    const ID_KEYS: [&str; 8] = [
        "id",
        "guild_id",
        "user_id",
        "created_by",
        "team_id",
        "period_id",
        "message_id",
        "channel_id",
    ];
    match value {
        Value::Array(arr) => Value::Array(arr.into_iter().map(stringify_ids).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                let big_id = ID_KEYS.contains(&key.as_str())
                    && val
                        .as_i64()
                        .or_else(|| val.as_u64().map(|u| u as i64))
                        .map(|n| n > 1_000_000)
                        .unwrap_or(false);
                if big_id {
                    let n = val.as_u64().unwrap_or_default();
                    out.insert(key, json!(n.to_string()));
                } else {
                    out.insert(key, stringify_ids(val));
                }
            }
            Value::Object(out)
        }
        other => other,
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

// ── Handler ─────────────────────────────────────────────────────────────────

pub async fn period_create(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, false) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match resolve_guild(&app, &payload) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    let name = string_field(&payload, "name");
    if name.is_empty() {
        return err_text(400, "'name' is required");
    }
    let reg_start = string_field(&payload, "registration_start");
    let reg_end = string_field(&payload, "registration_end");
    if reg_start.is_empty() || reg_end.is_empty() {
        return err_text(400, "'registration_start' and 'registration_end' required");
    }
    let team_size = coerce_i64(payload.get("team_size"))
        .unwrap_or(6)
        .clamp(2, 20);
    let created_by = coerce_u64(payload.get("created_by"));

    let s = store(&app);
    if let Err(err) = s
        .create_period(guild, &name, &reg_start, &reg_end, team_size, created_by)
        .await
    {
        tracing::error!(%err, "create_period fehlgeschlagen");
        return err_text(500, "Saving period failed");
    }
    let period = s.active_period_json(guild).await.unwrap_or(Value::Null);
    ok_json_status(201, stringify_ids(json!({ "ok": true, "period": period })))
}

pub async fn period_close(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, false) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match resolve_guild(&app, &payload) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    let s = store(&app);
    let period_id = match coerce_i64(payload.get("period_id")) {
        Some(id) => id,
        None => match s.active_period(guild).await {
            Some((id, _, _)) => id,
            None => return err_text(404, "No active period found"),
        },
    };
    let closed = match s.close_period(guild, period_id).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "close_period fehlgeschlagen");
            return err_text(500, "Closing period failed");
        }
    };
    ok_json(stringify_ids(json!({ "ok": closed, "closed": closed })))
}

pub async fn team_create(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, false) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match resolve_guild(&app, &payload) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    let name = string_field(&payload, "name");
    if name.is_empty() {
        return err_text(400, "'name' is required");
    }
    let created_by = coerce_u64(payload.get("created_by"));

    match store(&app)
        .get_or_create_team(guild, &name, created_by)
        .await
    {
        Ok(team) => {
            let payload = json!({
                "ok": true,
                "team": { "id": team.id, "name": team.name, "created": team.created },
                "created": team.created,
            });
            let status = if team.created { 201 } else { 200 };
            ok_json_status(status, stringify_ids(payload))
        }
        // CodeQL: generische Meldung (keine Detail-Exposition), wie im Original.
        Err(_) => err_text(400, "Ungültiger Team-Name oder Daten."),
    }
}

pub async fn team_delete(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, false) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match resolve_guild(&app, &payload) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    let Some(team_id) = coerce_i64(payload.get("team_id")) else {
        return err_text(400, "'team_id' is required");
    };
    match store(&app).delete_team(guild, team_id).await {
        Ok(true) => ok_json(json!({ "ok": true, "deleted": true })),
        Ok(false) => err_text(404, "Team not found"),
        Err(err) => {
            tracing::error!(%err, "delete_team fehlgeschlagen");
            err_text(500, "Deleting team failed")
        }
    }
}

pub async fn assign(State(app): State<DashboardApp>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, false) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match resolve_guild(&app, &payload) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    let Some(user_id) = coerce_u64(payload.get("user_id")) else {
        return err_text(400, "'user_id' is required");
    };
    // team_id: None/"" → entfernen (null), sonst coerce.
    let team_id = match payload.get("team_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        other => coerce_i64(other),
    };

    let s = store(&app);
    let updated = match s.assign_signup_team(guild, user_id, team_id).await {
        Ok(v) => v,
        // generische Meldung wie im Original.
        Err(_) => return err_text(400, "Ungültige Zuweisung oder Team existiert nicht."),
    };
    if !updated {
        return err_text(404, "Signup not found");
    }
    let signup = match s.get_signup(guild, user_id).await {
        Some(sg) => decorate_signup(&app, guild, sg).await,
        None => Value::Null,
    };
    ok_json(stringify_ids(json!({ "ok": true, "signup": signup })))
}

pub async fn remove(State(app): State<DashboardApp>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, false) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match resolve_guild(&app, &payload) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    let Some(user_id) = coerce_u64(payload.get("user_id")) else {
        return err_text(400, "'user_id' is required");
    };
    let removed = store(&app).remove_signup(guild, user_id).await;
    ok_json(json!({ "ok": removed, "removed": removed }))
}

pub async fn clear(State(app): State<DashboardApp>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(resp) = app.guard_mutate(&headers, false) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let guild = match resolve_guild(&app, &payload) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    match store(&app).clear_all_signups(guild).await {
        Ok(count) => ok_json(json!({ "ok": true, "cleared": count })),
        Err(err) => {
            tracing::error!(%err, "clear_all_signups fehlgeschlagen");
            err_json(500, "clear_failed")
        }
    }
}

/// Reichert einen Signup an wie `_decorate_tournament_signups`: IDs als
/// Strings, Anzeigename (Broker), Mention, Rang-Label.
async fn decorate_signup(
    app: &DashboardApp,
    guild_id: u64,
    signup: dl_tournament::store::Signup,
) -> Value {
    let names = app.names().resolve(&[signup.user_id]).await;
    let (display_name, mention) = match names.get(&signup.user_id) {
        Some(name) => (name.clone(), Some(format!("<@{}>", signup.user_id))),
        None => (format!("User {}", signup.user_id), None),
    };
    json!({
        "user_id": signup.user_id.to_string(),
        "guild_id": guild_id.to_string(),
        "registration_mode": signup.registration_mode,
        "rank": signup.rank,
        "rank_value": signup.rank_value,
        "rank_subvalue": signup.rank_subvalue,
        "team_id": signup.team_id,
        "team_name": signup.team_name,
        "assigned_by_admin": signup.assigned_by_admin,
        "display_name": display_name,
        "mention": mention,
        "rank_label": capitalize(&signup.rank),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stringify_nur_grosse_id_keys() {
        let v = json!({
            "id": 5,                    // klein → bleibt int
            "guild_id": 1289721245281292288_i64, // gross → string
            "team_id": 42,              // klein → int
            "name": "Cup",
            "nested": [{ "user_id": 685573558281175043_i64 }],
        });
        let out = stringify_ids(v);
        assert_eq!(out["id"], 5);
        assert_eq!(out["guild_id"], "1289721245281292288");
        assert_eq!(out["team_id"], 42);
        assert_eq!(out["nested"][0]["user_id"], "685573558281175043");
    }

    #[test]
    fn capitalize_wie_python() {
        assert_eq!(capitalize("phantom"), "Phantom");
        assert_eq!(capitalize("ETERNUS"), "Eternus");
        assert_eq!(capitalize(""), "");
    }
}
