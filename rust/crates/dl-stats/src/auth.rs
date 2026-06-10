//! Discord-OAuth-Flow — delegiert ans (noch Python-)Dashboard, exakt wie
//! `_handle_discord_login/_complete/_logout` im Original.

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::{
    cookie_value, SharedApp, DEFAULT_REDIRECT, PRE_AUTH_COOKIE, PRE_AUTH_TTL, SESSION_COOKIE,
    SESSION_TTL,
};

type Params = Query<HashMap<String, String>>;

/// `_sanitize_redirect_path`: nur absolute Pfade ohne Schema/Host/CR/LF/NUL.
fn sanitize_redirect(value: Option<&str>) -> String {
    let raw = value.unwrap_or("").trim();
    if raw.is_empty()
        || !raw.starts_with('/')
        || raw.starts_with("//")
        || raw.contains(['\r', '\n', '\0'])
    {
        return DEFAULT_REDIRECT.to_string();
    }
    raw.to_string()
}

fn set_cookie(value: String) -> HeaderValue {
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn cookie_header(app: &SharedApp, name: &str, value: &str, max_age: i64) -> HeaderValue {
    let secure = if app.cookie_secure { "; Secure" } else { "" };
    set_cookie(format!(
        "{name}={value}; Max-Age={max_age}; HttpOnly; Path=/; SameSite=Lax{secure}"
    ))
}

fn delete_cookie_header(name: &str) -> HeaderValue {
    set_cookie(format!("{name}=; Max-Age=0; Path=/"))
}

fn redirect_to(location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, location.to_string())],
    )
        .into_response()
}

/// `GET /auth/discord/login`.
pub async fn handle_login(State(app): State<SharedApp>, Query(params): Params) -> Response {
    let redirect_path = sanitize_redirect(params.get("redirect").map(String::as_str));

    let Some(initiate) = app
        .dashboard
        .discord_initiate(&app.callback_url, "activity")
        .await
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Auth-Service nicht erreichbar.",
        )
            .into_response();
    };
    if initiate.authorize_url.is_empty() || initiate.state_id.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Auth-Service Antwort ungültig.",
        )
            .into_response();
    }

    let exp = chrono::Utc::now().timestamp() + PRE_AUTH_TTL;
    let payload: BTreeMap<String, Value> = BTreeMap::from([
        ("state_id".to_string(), json!(initiate.state_id)),
        ("redirect".to_string(), json!(redirect_path)),
        ("exp".to_string(), json!(exp)),
    ]);
    let Some(pre_auth) = app.codec.sign(&payload) else {
        tracing::warn!("Kein Session-Secret konfiguriert — Login nicht möglich");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Auth-Service nicht erreichbar.",
        )
            .into_response();
    };

    let mut response = redirect_to(&initiate.authorize_url);
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie_header(&app, PRE_AUTH_COOKIE, &pre_auth, PRE_AUTH_TTL),
    );
    response
}

/// `GET /auth/discord/complete`.
pub async fn handle_complete(
    State(app): State<SharedApp>,
    headers: HeaderMap,
    Query(params): Params,
) -> Response {
    let state_id = params
        .get("state_id")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let mut redirect_path = DEFAULT_REDIRECT.to_string();
    if let Some(pre_auth_cookie) = cookie_value(&headers, PRE_AUTH_COOKIE) {
        let now = chrono::Utc::now().timestamp();
        if let Some(pre_auth) = app.codec.verify(&pre_auth_cookie, now) {
            redirect_path = sanitize_redirect(pre_auth.get("redirect").and_then(Value::as_str));
        }
    }

    let mut response = redirect_to(&redirect_path);
    response
        .headers_mut()
        .append(header::SET_COOKIE, delete_cookie_header(PRE_AUTH_COOKIE));

    if state_id.is_empty() {
        tracing::warn!("discord_complete: kein state_id → kein Login");
        return response;
    }
    let Some(consume) = app.dashboard.discord_consume(&state_id).await else {
        tracing::warn!("discord_complete: kein discord_id in consume-result → kein Login");
        return response;
    };

    let now = chrono::Utc::now().timestamp();
    let payload: BTreeMap<String, Value> = BTreeMap::from([
        ("user_id".to_string(), json!(consume.discord_id)),
        ("name".to_string(), json!(consume.discord_name)),
        (
            "avatar".to_string(),
            consume
                .discord_avatar
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        ("iat".to_string(), json!(now)),
        ("exp".to_string(), json!(now + SESSION_TTL)),
    ]);
    if let Some(session) = app.codec.sign(&payload) {
        response.headers_mut().append(
            header::SET_COOKIE,
            cookie_header(&app, SESSION_COOKIE, &session, SESSION_TTL),
        );
    }
    response
}

/// `POST /auth/discord/logout` → 204 + Cookie löschen.
pub async fn handle_logout() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .append(header::SET_COOKIE, delete_cookie_header(SESSION_COOKIE));
    response
}

#[cfg(test)]
mod tests {
    use super::sanitize_redirect;

    #[test]
    fn redirect_sanitisierung() {
        assert_eq!(sanitize_redirect(None), "/aktivitaet/");
        assert_eq!(sanitize_redirect(Some("")), "/aktivitaet/");
        assert_eq!(sanitize_redirect(Some("/stats")), "/stats");
        assert_eq!(sanitize_redirect(Some("//evil.example")), "/aktivitaet/");
        assert_eq!(
            sanitize_redirect(Some("https://evil.example")),
            "/aktivitaet/"
        );
        assert_eq!(
            sanitize_redirect(Some("/x\r\nSet-Cookie: a=b")),
            "/aktivitaet/"
        );
        assert_eq!(sanitize_redirect(Some("relativ")), "/aktivitaet/");
    }
}
