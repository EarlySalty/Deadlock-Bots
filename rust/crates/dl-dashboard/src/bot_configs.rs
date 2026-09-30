//! Gemeinsamer TOML-Editor: öffentliche Admin-Session bzw. bestehende
//! Twitch-Dienstauth, bevor Registry, Dateien oder Prozesse berührt werden.
use crate::{operating_config::no_store, web::DashboardApp};
use axum::{
    body::Bytes,
    extract::{ConnectInfo, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use dl_core::admin_config::{Dashboard, Error, Registry, Target};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

fn registry_path() -> Result<PathBuf, Error> {
    let active = dl_core::config::process_bot_config().map_err(|_| Error::Unavailable)?;
    Ok(active
        .source()
        .parent()
        .ok_or(Error::Unavailable)?
        .join("admin-bots.toml"))
}

fn error_response(error: Error) -> Response {
    let code = match error {
        Error::NotFound => 404,
        Error::Conflict => 409,
        Error::Busy => 423,
        Error::TooLarge => 413,
        Error::Invalid(_) => 422,
        Error::Unavailable => 503,
        Error::Io | Error::Durability => 500,
    };
    let message = error.to_string();
    no_store(
        (
            StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({"error": message, "message": message})),
        )
            .into_response(),
    )
}

/// Begrenzt gleichzeitige Prüfprozesse auch über mehrere Browser hinweg.
async fn work<F>(operation: F) -> Response
where
    F: FnOnce() -> Result<Value, Error> + Send + 'static,
{
    static LIMIT: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    let limit = LIMIT
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
        .clone();
    let Ok(permit) = limit.try_acquire_owned() else {
        return error_response(Error::Busy);
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation()
    })
    .await
    {
        Ok(Ok(value)) => no_store(Json(value).into_response()),
        Ok(Err(error)) => error_response(error),
        Err(_) => error_response(Error::Io),
    }
}

fn target(id: &str, dashboard: Dashboard) -> Result<(PathBuf, Target), Error> {
    let path = registry_path()?;
    let registry = Registry::load(&path)?;
    Ok((path, registry.target(id, dashboard)?))
}

fn snapshot(id: &str, dashboard: Dashboard) -> Result<Value, Error> {
    let (path, target) = target(id, dashboard)?;
    let snapshot = target.snapshot()?;
    let status = dl_core::admin_config_activation::status(&path, &target, &snapshot.revision).ok();
    Ok(json!({"snapshot": snapshot, "runtime": status}))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Validate { revision: String, toml: String },
    Save { revision: String, toml: String },
    Activate { revision: String },
    History { revision: String },
    Status { revision: String },
}

fn decode(body: &[u8]) -> Result<Request, Error> {
    // JSON escapiert im ungünstigsten Fall jedes Byte als sechs Zeichen.
    if body.len() > dl_core::admin_config::MAX_BYTES * 6 + 1024 {
        return Err(Error::TooLarge);
    }
    serde_json::from_slice(body)
        .map_err(|_| Error::Invalid("Ungültige Anfrage. Bitte den gespeicherten Stand neu laden."))
}

fn execute(id: &str, dashboard: Dashboard, request: Request, actor: &str) -> Result<Value, Error> {
    let (path, target) = target(id, dashboard)?;
    match request {
        Request::Validate { revision, toml } => Ok(json!(target.preview(&revision, &toml)?)),
        Request::Save { revision, toml } => {
            let saved = target.save(&revision, &toml)?;
            tracing::info!(actor, bot_id = id, previous_revision = %revision, revision = %saved.revision, "Bot-TOML im Admin-Dashboard gespeichert");
            let runtime =
                dl_core::admin_config_activation::status(&path, &target, &saved.revision).ok();
            Ok(json!({"snapshot": saved, "runtime": runtime}))
        }
        Request::Activate { revision } => {
            let operation = dl_core::admin_config_activation::queue(&path, &target, &revision)?;
            tracing::info!(
                actor,
                bot_id = id,
                revision,
                "Bot-TOML-Aktivierung im Admin-Dashboard angefordert"
            );
            Ok(json!({"operation": operation}))
        }
        Request::History { revision } => Ok(json!({"toml": target.history_snapshot(&revision)?})),
        Request::Status { revision } => {
            if !dl_core::admin_config::valid_revision(&revision) {
                return Err(Error::Invalid("Ungültige Revision."));
            }
            // Nur die tatsächlich gespeicherte Revision darf als aktiv erscheinen.
            let saved = target.snapshot()?;
            let runtime =
                dl_core::admin_config_activation::status(&path, &target, &saved.revision).ok();
            Ok(json!({"revision": saved.revision, "runtime": runtime}))
        }
    }
}

pub async fn list(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return no_store(response);
    }
    work(|| {
        let registry = Registry::load(&registry_path()?)?;
        Ok(json!({"bots": registry.entries(Dashboard::Discord)}))
    })
    .await
}

pub async fn get(
    State(app): State<DashboardApp>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return no_store(response);
    }
    work(move || snapshot(&id, Dashboard::Discord)).await
}

pub async fn mutate(
    State(app): State<DashboardApp>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match app.guard_mutate(&headers, true).await {
        Ok(session) => session,
        Err(response) => return no_store(response),
    };
    let request = match decode(&body) {
        Ok(value) => value,
        Err(error) => return error_response(error),
    };
    work(move || {
        execute(
            &id,
            Dashboard::Discord,
            request,
            &session.user_id.to_string(),
        )
    })
    .await
}

pub async fn twitch_get(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = crate::web::guard_twitch(&app, &peer, &headers) {
        return no_store(response);
    }
    work(|| snapshot("twitch", Dashboard::Twitch)).await
}

pub async fn twitch_mutate(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(response) = crate::web::guard_twitch(&app, &peer, &headers) {
        return no_store(response);
    }
    let request = match decode(&body) {
        Ok(value) => value,
        Err(error) => return error_response(error),
    };
    // Identität kommt ausschließlich aus dem authentifizierten Admin-Proxy.
    let actor = headers
        .get("X-Admin-Actor")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 24
                && value.bytes().all(|byte| byte.is_ascii_digit())
        })
        .unwrap_or("twitch-admin")
        .to_owned();
    work(move || execute("twitch", Dashboard::Twitch, request, &actor)).await
}

fn asset(name: &str) -> Response {
    match name {
        "js" => no_store(
            (
                [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                include_str!("../../../../service/static/bot-config-editor.js"),
            )
                .into_response(),
        ),
        "css" => no_store(
            (
                [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                include_str!("../../../../service/static/bot-config-editor.css"),
            )
                .into_response(),
        ),
        _ => no_store(StatusCode::NOT_FOUND.into_response()),
    }
}
pub async fn ui(
    State(app): State<DashboardApp>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return no_store(response);
    }
    asset(&name)
}
pub async fn twitch_ui(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = crate::web::guard_twitch(&app, &peer, &headers) {
        return no_store(response);
    }
    asset(&name)
}

#[cfg(test)]
mod tests {
    use super::decode;
    #[test]
    fn requests_do_not_accept_paths_units_or_programs() {
        for field in ["path", "program", "units", "id"] {
            let body = serde_json::json!({"action":"activate","revision":"a".repeat(64),field:"untrusted"});
            assert!(decode(body.to_string().as_bytes()).is_err());
        }
        assert!(decode(br#"{"action":"unknown"}"#).is_err());
        assert!(decode(br#"{"action":"save","revision":"x"}"#).is_err());
    }
}
