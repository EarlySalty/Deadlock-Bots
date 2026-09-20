//! Admin-Editor mit dem vorhandenen Session-/CSRF-Gate.
use crate::web::{err_text, DashboardApp};
use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use dl_core::{
    bot_config::BotConfigStore,
    operating_config::{EditError, OperatingOptions, SavedConfig},
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRequest {
    pub revision: String,
    pub options: OperatingOptions,
}

pub fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

pub async fn ui(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return no_store(response);
    }
    no_store(
        (
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            include_str!("../../../../service/static/operating-config.js"),
        )
            .into_response(),
    )
}

async fn active_bot_fingerprint() -> Option<(String, u64)> {
    let url = format!(
        "{}/internal/master/v1/health",
        dl_core::runtime_config::lookup("MASTER_BROKER_BASE_URL")?.trim_end_matches('/')
    );
    let token = [
        "MASTER_BROKER_TOKEN",
        "MAIN_BOT_INTERNAL_TOKEN",
        "TWITCH_INTERNAL_API_TOKEN",
    ]
    .into_iter()
    .find_map(dl_core::runtime_config::secret_value)?;
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;
    let mut response = http
        .get(url)
        .header("X-Internal-Token", token)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > 4096 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if body.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let process_id = body
        .get("result")?
        .get("process_id")?
        .as_u64()
        .filter(|pid| *pid > 0)?;
    body.get("result")?
        .get("config_fingerprint")?
        .as_str()
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|fingerprint| (fingerprint.to_owned(), process_id))
}

async fn output(saved: SavedConfig, active: &str) -> Response {
    let bot = active_bot_fingerprint().await;
    no_store(
        Json(json!({
            "revision": saved.revision,
            "saved_fingerprint": saved.fingerprint,
            "options": OperatingOptions::from(&saved.config),
            "services": [
                {"name": "Discord-Web", "restart_required": saved.fingerprint != active, "process_id": std::process::id(), "observed_at": crate::now_unix()},
                {"name": "Discord-Bot", "restart_required": bot.as_ref().map(|(fingerprint, _)| fingerprint != &saved.fingerprint), "process_id": bot.as_ref().map(|(_, pid)| pid), "observed_at": bot.as_ref().map(|_| crate::now_unix())}
            ]
        }))
        .into_response(),
    )
}

fn store() -> Result<(BotConfigStore, &'static str), Response> {
    let active = dl_core::config::process_bot_config()
        .map_err(|_| err_text(503, "Betriebseinstellungen sind nicht verfügbar."))?;
    let store = BotConfigStore::open(active.source()).map_err(|_| {
        err_text(
            503,
            "Die gespeicherten Einstellungen sind ungültig oder nicht lesbar.",
        )
    })?;
    Ok((store, active.fingerprint()))
}

pub async fn get(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return no_store(response);
    }
    let (store, active) = match store() {
        Ok(value) => value,
        Err(response) => return no_store(response),
    };
    match store.read_versioned() {
        Ok(saved) => output(saved, active).await,
        Err(_) => no_store(err_text(
            503,
            "Die gespeicherten Einstellungen sind nicht lesbar.",
        )),
    }
}

pub async fn save(State(app): State<DashboardApp>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(response) = app.guard_mutate(&headers, true).await {
        return no_store(response);
    }
    if body.len() > 8192 {
        return no_store(StatusCode::PAYLOAD_TOO_LARGE.into_response());
    }
    let request: SaveRequest = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            return no_store(err_text(
                400,
                "Die Eingaben sind unvollständig oder ungültig.",
            ))
        }
    };
    if request.revision.len() != 64
        || !request
            .revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return no_store(err_text(
            400,
            "Der gespeicherte Stand fehlt. Bitte neu laden.",
        ));
    }
    let (store, active) = match store() {
        Ok(value) => value,
        Err(response) => return no_store(response),
    };
    let result = tokio::task::spawn_blocking(move || {
        store.save_if_revision(&request.revision, &request.options)
    })
    .await;
    match result {
        Ok(Ok(saved)) => output(saved, active).await,
        Ok(Err(error)) => {
            let status = match error {
                EditError::Conflict => 409,
                EditError::Invalid(_) => 422,
                EditError::Busy | EditError::UnsafeLocation => 503,
                EditError::Io | EditError::Durability => 500,
            };
            no_store(err_text(status, &error.to_string()))
        }
        Err(_) => no_store(err_text(
            500,
            "Speichern konnte nicht abgeschlossen werden. Bitte den Stand neu laden.",
        )),
    }
}

async fn steam_request(save: Option<dl_bridges::steam_operating::SaveRequest>) -> Response {
    let config = match dl_core::config::process_bot_config() {
        Ok(value) => value.snapshot(),
        Err(_) => {
            return no_store(err_text(
                503,
                "Die Steam-Verbindung ist nicht eingerichtet.",
            ))
        }
    };
    // Bestehende Infisical-Secrets, kein zweiter Token oder Browserzugriff.
    let token = [
        "TWITCH_INTERNAL_API_TOKEN",
        "STEAM_INTERNAL_API_TOKEN",
        "INTERNAL_API_TOKEN",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok())
    .map(|value| value.trim().to_owned())
    .find(|value| !value.is_empty());
    let client = dl_bridges::steam::SteamBotClient::new(&config.services.steam_api_url, token);
    match client.operating_config(save.as_ref()).await {
        Ok(value) => no_store(Json(value).into_response()),
        Err(error) => {
            use dl_bridges::steam_operating::Error;
            let (status, message) = match error {
                Error::Unavailable => (503, "Steam ist momentan nicht erreichbar oder noch nicht eingerichtet."),
                Error::Conflict => (409, "Der gespeicherte Stand wurde inzwischen geändert. Dein Entwurf bleibt erhalten."),
                Error::Invalid => (422, "Die Steam-Einstellungen sind ungültig. Bitte die Grenzen und Konten prüfen."),
                Error::Upstream => (502, "Steam konnte die Anfrage nicht bestätigen. Bitte den Stand neu laden."),
            };
            no_store(err_text(status, message))
        }
    }
}

pub async fn steam_get(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return no_store(response);
    }
    steam_request(None).await
}

pub async fn steam_save(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(response) = app.guard_mutate(&headers, true).await {
        return no_store(response);
    }
    if body.len() > 8192 {
        return no_store(StatusCode::PAYLOAD_TOO_LARGE.into_response());
    }
    let request: dl_bridges::steam_operating::SaveRequest = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            return no_store(err_text(
                400,
                "Die Steam-Eingaben sind unvollständig oder ungültig.",
            ))
        }
    };
    if request.revision.len() != 64
        || !request
            .revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return no_store(err_text(
            400,
            "Der gespeicherte Stand fehlt. Bitte neu laden.",
        ));
    }
    steam_request(Some(request)).await
}
