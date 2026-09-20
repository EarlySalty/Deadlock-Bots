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
    operating_config::{digest, EditError, OperatingOptions, SavedConfig},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};
use tokio::process::Command;

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
    fetch_bot_fingerprint(&url, &token).await
}

async fn fetch_bot_fingerprint(url: &str, token: &str) -> Option<(String, u64)> {
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

#[cfg(test)]
mod active_tests {
    use super::fetch_bot_fingerprint;
    use axum::{http::HeaderMap, routing::get, Json, Router};
    use serde_json::json;

    #[tokio::test]
    async fn broker_evidence_requires_authenticated_fresh_envelope_and_pid() {
        for (body, expected) in [
            (
                json!({"ok":true,"result":{"config_fingerprint":"a".repeat(64),"process_id":1234}}),
                Some(("a".repeat(64), 1234)),
            ),
            (
                json!({"ok":true,"result":{"config_fingerprint":"a".repeat(64)}}),
                None,
            ),
            (
                json!({"ok":false,"result":{"config_fingerprint":"a".repeat(64),"process_id":1234}}),
                None,
            ),
            (
                json!({"ok":true,"result":{"config_fingerprint":"broken","process_id":1234}}),
                None,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("Testport");
            let address = listener.local_addr().expect("Testadresse");
            let app = Router::new().route(
                "/internal/master/v1/health",
                get(move |headers: HeaderMap| {
                    let body = body.clone();
                    async move {
                        assert_eq!(
                            headers.get("X-Internal-Token").expect("interner Token"),
                            "synthetic-token"
                        );
                        Json(body)
                    }
                }),
            );
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("Testserver");
            });
            let url = format!("http://{address}/internal/master/v1/health");
            assert_eq!(
                fetch_bot_fingerprint(&url, "synthetic-token").await,
                expected
            );
            server.abort();
            let _ = server.await;
            assert!(fetch_bot_fingerprint(&url, "synthetic-token")
                .await
                .is_none());
        }
    }
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

const PATCHNOTES_UNIT: &str = "deadlock-patchnotes.service";
const PATCHNOTES_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PatchnotesDiscordOptions {
    pub retranslate_cooldown_seconds: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PatchnotesPollingOptions {
    pub interval_seconds: f64,
    pub steam_news_enabled: bool,
    pub steam_news_interval_seconds: f64,
    pub steam_version_enabled: bool,
    pub steam_version_check_seconds: f64,
    pub burst_duration_seconds: i64,
    pub burst_interval_seconds: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PatchnotesPublishingOptions {
    pub max_auto_post_age_days: i64,
    pub max_catchup_posts: i64,
    pub include_ping: bool,
    pub force_latest_on_start: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PatchnotesPreparedOptions {
    pub post_on_start: bool,
    pub include_ping: bool,
    pub translate: bool,
    pub use_logs_channel: bool,
    pub only_mode: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PatchnotesFormattingOptions {
    pub embed_v2: bool,
    pub chunk_limit: i64,
    pub char_budget: i64,
    pub component_budget: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PatchnotesOptions {
    pub discord: PatchnotesDiscordOptions,
    pub polling: PatchnotesPollingOptions,
    pub publishing: PatchnotesPublishingOptions,
    pub prepared: PatchnotesPreparedOptions,
    pub formatting: PatchnotesFormattingOptions,
}

impl PatchnotesOptions {
    fn validate(&self) -> bool {
        let seconds = [
            self.discord.retranslate_cooldown_seconds,
            self.polling.interval_seconds,
            self.polling.steam_news_interval_seconds,
            self.polling.steam_version_check_seconds,
            self.polling.burst_interval_seconds,
        ];
        seconds.iter().all(|value| (1.0..=86400.0).contains(value))
            && (1..=86400).contains(&self.polling.burst_duration_seconds)
            && (1..=365).contains(&self.publishing.max_auto_post_age_days)
            && (1..=100).contains(&self.publishing.max_catchup_posts)
            && (100..=2000).contains(&self.formatting.chunk_limit)
            && (500..=4000).contains(&self.formatting.char_budget)
            && (10..=40).contains(&self.formatting.component_budget)
    }
}

fn read_bool(section: Option<&toml::Value>, key: &str, default: bool) -> Result<bool, ()> {
    match section.and_then(|table| table.get(key)) {
        None => Ok(default),
        Some(toml::Value::Boolean(value)) => Ok(*value),
        Some(_) => Err(()),
    }
}

fn read_f64(section: Option<&toml::Value>, key: &str, default: f64) -> Result<f64, ()> {
    match section.and_then(|table| table.get(key)) {
        None => Ok(default),
        Some(toml::Value::Float(value)) => Ok(*value),
        Some(toml::Value::Integer(value)) => Ok(*value as f64),
        Some(_) => Err(()),
    }
}

fn read_i64(section: Option<&toml::Value>, key: &str, default: i64) -> Result<i64, ()> {
    match section.and_then(|table| table.get(key)) {
        None => Ok(default),
        Some(toml::Value::Integer(value)) => Ok(*value),
        Some(_) => Err(()),
    }
}

fn patchnotes_options_from_document(
    document: &toml::Value,
) -> Result<PatchnotesOptions, ()> {
    let discord = document.get("discord");
    let polling = document.get("polling");
    let publishing = document.get("publishing");
    let prepared = document.get("prepared");
    let formatting = document.get("formatting");
    Ok(PatchnotesOptions {
        discord: PatchnotesDiscordOptions {
            retranslate_cooldown_seconds: read_f64(
                discord,
                "retranslate_cooldown_seconds",
                60.0,
            )?,
        },
        polling: PatchnotesPollingOptions {
            interval_seconds: read_f64(polling, "interval_seconds", 35.0)?,
            steam_news_enabled: read_bool(polling, "steam_news_enabled", true)?,
            steam_news_interval_seconds: read_f64(
                polling,
                "steam_news_interval_seconds",
                2.0,
            )?,
            steam_version_enabled: read_bool(polling, "steam_version_enabled", true)?,
            steam_version_check_seconds: read_f64(
                polling,
                "steam_version_check_seconds",
                2.0,
            )?,
            burst_duration_seconds: read_i64(polling, "burst_duration_seconds", 1200)?,
            burst_interval_seconds: read_f64(polling, "burst_interval_seconds", 20.0)?,
        },
        publishing: PatchnotesPublishingOptions {
            max_auto_post_age_days: read_i64(publishing, "max_auto_post_age_days", 2)?,
            max_catchup_posts: read_i64(publishing, "max_catchup_posts", 1)?,
            include_ping: read_bool(publishing, "include_ping", true)?,
            force_latest_on_start: read_bool(publishing, "force_latest_on_start", false)?,
            dry_run: read_bool(publishing, "dry_run", false)?,
        },
        prepared: PatchnotesPreparedOptions {
            post_on_start: read_bool(prepared, "post_on_start", false)?,
            include_ping: read_bool(prepared, "include_ping", true)?,
            translate: read_bool(prepared, "translate", false)?,
            use_logs_channel: read_bool(prepared, "use_logs_channel", false)?,
            only_mode: read_bool(prepared, "only_mode", false)?,
        },
        formatting: PatchnotesFormattingOptions {
            embed_v2: read_bool(formatting, "embed_v2", false)?,
            chunk_limit: read_i64(formatting, "chunk_limit", 1950)?,
            char_budget: read_i64(formatting, "char_budget", 3500)?,
            component_budget: read_i64(formatting, "component_budget", 35)?,
        },
    })
}

fn apply_patchnotes_options(
    document: &mut toml_edit::DocumentMut,
    options: &PatchnotesOptions,
) {
    let updates = [
        (
            "discord",
            "retranslate_cooldown_seconds",
            toml_edit::value(options.discord.retranslate_cooldown_seconds),
        ),
        (
            "polling",
            "interval_seconds",
            toml_edit::value(options.polling.interval_seconds),
        ),
        (
            "polling",
            "steam_news_enabled",
            toml_edit::value(options.polling.steam_news_enabled),
        ),
        (
            "polling",
            "steam_news_interval_seconds",
            toml_edit::value(options.polling.steam_news_interval_seconds),
        ),
        (
            "polling",
            "steam_version_enabled",
            toml_edit::value(options.polling.steam_version_enabled),
        ),
        (
            "polling",
            "steam_version_check_seconds",
            toml_edit::value(options.polling.steam_version_check_seconds),
        ),
        (
            "polling",
            "burst_duration_seconds",
            toml_edit::value(options.polling.burst_duration_seconds),
        ),
        (
            "polling",
            "burst_interval_seconds",
            toml_edit::value(options.polling.burst_interval_seconds),
        ),
        (
            "publishing",
            "max_auto_post_age_days",
            toml_edit::value(options.publishing.max_auto_post_age_days),
        ),
        (
            "publishing",
            "max_catchup_posts",
            toml_edit::value(options.publishing.max_catchup_posts),
        ),
        (
            "publishing",
            "include_ping",
            toml_edit::value(options.publishing.include_ping),
        ),
        (
            "publishing",
            "force_latest_on_start",
            toml_edit::value(options.publishing.force_latest_on_start),
        ),
        (
            "publishing",
            "dry_run",
            toml_edit::value(options.publishing.dry_run),
        ),
        (
            "prepared",
            "post_on_start",
            toml_edit::value(options.prepared.post_on_start),
        ),
        (
            "prepared",
            "include_ping",
            toml_edit::value(options.prepared.include_ping),
        ),
        (
            "prepared",
            "translate",
            toml_edit::value(options.prepared.translate),
        ),
        (
            "prepared",
            "use_logs_channel",
            toml_edit::value(options.prepared.use_logs_channel),
        ),
        (
            "prepared",
            "only_mode",
            toml_edit::value(options.prepared.only_mode),
        ),
        (
            "formatting",
            "embed_v2",
            toml_edit::value(options.formatting.embed_v2),
        ),
        (
            "formatting",
            "chunk_limit",
            toml_edit::value(options.formatting.chunk_limit),
        ),
        (
            "formatting",
            "char_budget",
            toml_edit::value(options.formatting.char_budget),
        ),
        (
            "formatting",
            "component_budget",
            toml_edit::value(options.formatting.component_budget),
        ),
    ];
    for (section, key, mut replacement) in updates {
        if !document.contains_key(section) {
            document[section] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        if let (Some(old), Some(new)) = (
            document
                .get(section)
                .and_then(toml_edit::Item::as_table_like)
                .and_then(|table| table.get(key))
                .and_then(toml_edit::Item::as_value),
            replacement.as_value_mut(),
        ) {
            *new.decor_mut() = old.decor().clone();
        }
        document[section][key] = replacement;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PatchnotesSaveError {
    #[error("Die Datei wurde inzwischen geändert. Der Entwurf bleibt erhalten.")]
    Conflict,
    #[error("Die Patchnotes-Einstellungen werden gerade gespeichert. Bitte erneut versuchen.")]
    Busy,
    #[error("Der Ablageort der Patchnotes-Einstellungen ist nicht schreibbar.")]
    UnsafeLocation,
    #[error("Die Patchnotes-Einstellungen konnten nicht gespeichert werden.")]
    Io,
    #[error("Die Datei wurde ersetzt, die dauerhafte Speicherung ist aber nicht bestätigt. Bitte den Stand neu laden.")]
    Durability,
}

fn read_patchnotes_text(path: &Path) -> Result<String, ()> {
    let metadata = fs::metadata(path).map_err(|_| ())?;
    if !metadata.is_file() || metadata.len() > PATCHNOTES_MAX_BYTES {
        return Err(());
    }
    let mut open = OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = open.open(path).map_err(|_| ())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| ())?;
    if bytes.len() as u64 > PATCHNOTES_MAX_BYTES {
        return Err(());
    }
    String::from_utf8(bytes).map_err(|_| ())
}

fn save_patchnotes_options(
    path: &Path,
    expected: &str,
    options: &PatchnotesOptions,
) -> Result<String, PatchnotesSaveError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| PatchnotesSaveError::UnsafeLocation)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(PatchnotesSaveError::UnsafeLocation);
    }
    let source = path.canonicalize().map_err(|_| PatchnotesSaveError::Io)?;
    if source
        .ancestors()
        .any(|parent| parent.join(".git").exists())
    {
        return Err(PatchnotesSaveError::UnsafeLocation);
    }
    let directory = source
        .parent()
        .ok_or(PatchnotesSaveError::UnsafeLocation)?;
    let mut open = OpenOptions::new();
    open.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let lock = open
        .open(directory.join(".bot.toml.lock"))
        .map_err(|_| PatchnotesSaveError::Io)?;
    if !lock
        .metadata()
        .map_err(|_| PatchnotesSaveError::Io)?
        .is_file()
    {
        return Err(PatchnotesSaveError::UnsafeLocation);
    }
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| match error.kind() {
        std::io::ErrorKind::WouldBlock => PatchnotesSaveError::Busy,
        _ => PatchnotesSaveError::Io,
    })?;
    let original = read_patchnotes_text(&source).map_err(|_| PatchnotesSaveError::Io)?;
    if digest(original.as_bytes()) != expected {
        return Err(PatchnotesSaveError::Conflict);
    }
    let mut document = original
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| PatchnotesSaveError::Io)?;
    apply_patchnotes_options(&mut document, options);
    let text = document.to_string();
    let parsed: toml::Value = text.parse().map_err(|_| PatchnotesSaveError::Io)?;
    if patchnotes_options_from_document(&parsed) != Ok(options.clone()) {
        return Err(PatchnotesSaveError::Io);
    }
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".bot.toml.{}.{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut create = OpenOptions::new();
    create.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        create.mode(0o600);
    }
    let mut file = create.open(&temporary).map_err(|_| PatchnotesSaveError::Io)?;
    let result = (|| {
        let metadata = fs::metadata(&source).map_err(|_| PatchnotesSaveError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{fchown, MetadataExt};
            fchown(&file, Some(metadata.uid()), Some(metadata.gid()))
                .map_err(|_| PatchnotesSaveError::Io)?;
        }
        file.set_permissions(metadata.permissions())
            .map_err(|_| PatchnotesSaveError::Io)?;
        file.write_all(text.as_bytes())
            .map_err(|_| PatchnotesSaveError::Io)?;
        file.sync_all().map_err(|_| PatchnotesSaveError::Io)?;
        if path.canonicalize().map_err(|_| PatchnotesSaveError::Io)? != source
            || read_patchnotes_text(&source).map_err(|_| PatchnotesSaveError::Io)? != original
        {
            return Err(PatchnotesSaveError::Conflict);
        }
        fs::rename(&temporary, &source).map_err(|_| PatchnotesSaveError::Io)?;
        File::open(directory)
            .and_then(|parent| parent.sync_all())
            .map_err(|_| PatchnotesSaveError::Durability)?;
        Ok(digest(text.as_bytes()))
    })();
    let _ = fs::remove_file(&temporary);
    result
}

struct PatchnotesObservation {
    active: bool,
    process_id: Option<u64>,
    started_at: Option<f64>,
}

fn parse_service_show(output: &str) -> Option<(bool, Option<u64>, Option<u64>)> {
    let mut active_state = None;
    let mut process_id = None;
    let mut started = None;
    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            match key {
                "ActiveState" => active_state = Some(value.to_owned()),
                "MainPID" => process_id = value.parse::<u64>().ok(),
                "ExecMainStartTimestampMonotonic" => started = value.parse::<u64>().ok(),
                _ => {}
            }
        }
    }
    let active_state = active_state?;
    Some((
        active_state == "active",
        process_id.filter(|pid| *pid > 0),
        started,
    ))
}

fn boot_time_seconds() -> Option<f64> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let value = stat.lines().find_map(|line| line.strip_prefix("btime "))?;
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds as f64)
}

async fn observe_patchnotes_service() -> Option<PatchnotesObservation> {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new("systemctl")
            .args([
                "--user",
                "show",
                PATCHNOTES_UNIT,
                "--property=ActiveState",
                "--property=MainPID",
                "--property=ExecMainStartTimestampMonotonic",
                "--no-pager",
            ])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = std::str::from_utf8(&output.stdout).ok()?;
    let (active, process_id, started_us) = parse_service_show(text)?;
    let started_at = started_us
        .and_then(|value| boot_time_seconds().map(|boot| boot + value as f64 / 1_000_000.0));
    Some(PatchnotesObservation {
        active,
        process_id,
        started_at,
    })
}

fn modified_seconds(path: &Path) -> Option<f64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|value| value.as_secs_f64())
}

fn patchnotes_path() -> Result<PathBuf, Response> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            no_store(err_text(
                503,
                "Die Patchnotes-Einstellungen sind nicht verfügbar.",
            ))
        })?;
    Ok(PathBuf::from(home).join(".config/deadlock-bots/patchnotes/bot.toml"))
}

async fn patchnotes_output(path: &Path, revision: String, options: PatchnotesOptions) -> Response {
    let observation = observe_patchnotes_service().await;
    let restart_required = match (&observation, modified_seconds(path)) {
        (Some(value), Some(modified))
            if value.active && value.started_at.is_some() =>
        {
            Some(modified > value.started_at.unwrap())
        }
        _ => None,
    };
    no_store(
        Json(json!({
            "revision": revision,
            "options": options,
            "services": [
                {
                    "name": "Patchnotes-Bot",
                    "restart_required": restart_required,
                    "process_id": observation.as_ref().and_then(|value| value.process_id),
                    "observed_at": observation.as_ref().map(|_| crate::now_unix())
                }
            ]
        }))
        .into_response(),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchnotesSaveRequest {
    pub revision: String,
    pub options: PatchnotesOptions,
}

pub async fn patchnotes_get(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return no_store(response);
    }
    let path = match patchnotes_path() {
        Ok(value) => value,
        Err(response) => return response,
    };
    let text = match read_patchnotes_text(&path) {
        Ok(value) => value,
        Err(_) => {
            return no_store(err_text(
                503,
                "Die Patchnotes-Einstellungen sind nicht lesbar.",
            ))
        }
    };
    let document: toml::Value = match text.parse() {
        Ok(value) => value,
        Err(_) => {
            return no_store(err_text(
                503,
                "Die gespeicherten Patchnotes-Einstellungen sind ungültig.",
            ))
        }
    };
    let options = match patchnotes_options_from_document(&document) {
        Ok(value) => value,
        Err(_) => {
            return no_store(err_text(
                503,
                "Die gespeicherten Patchnotes-Einstellungen sind ungültig.",
            ))
        }
    };
    patchnotes_output(&path, digest(text.as_bytes()), options).await
}

pub async fn patchnotes_save(
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
    let request: PatchnotesSaveRequest = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            return no_store(err_text(
                400,
                "Die Patchnotes-Eingaben sind unvollständig oder ungültig.",
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
    if !request.options.validate() {
        return no_store(err_text(
            422,
            "Die Patchnotes-Eingaben liegen außerhalb der erlaubten Grenzen.",
        ));
    }
    let path = match patchnotes_path() {
        Ok(value) => value,
        Err(response) => return response,
    };
    let save_path = path.clone();
    let revision = request.revision.clone();
    let options = request.options.clone();
    let result =
        tokio::task::spawn_blocking(move || save_patchnotes_options(&save_path, &revision, &options))
            .await;
    match result {
        Ok(Ok(saved_revision)) => patchnotes_output(&path, saved_revision, request.options).await,
        Ok(Err(error)) => {
            let status = match error {
                PatchnotesSaveError::Conflict => 409,
                PatchnotesSaveError::Busy | PatchnotesSaveError::UnsafeLocation => 503,
                PatchnotesSaveError::Io | PatchnotesSaveError::Durability => 500,
            };
            no_store(err_text(status, &error.to_string()))
        }
        Err(_) => no_store(err_text(
            500,
            "Speichern konnte nicht abgeschlossen werden. Bitte den Stand neu laden.",
        )),
    }
}

#[cfg(test)]
mod patchnotes_tests {
    use super::*;

    const FIXTURE: &str = "\
# Betriebstagebuch der Patchnotes
schema_version = 1

[discord]
channel_id = 1326973956825284628
logs_channel_id = 1374364800817303632
role_id = 1330994309524357140
retranslate_cooldown_seconds = 60.0

[polling]
interval_seconds = 35.0
steam_news_enabled = true
steam_news_interval_seconds = 2.0
steam_version_enabled = true
steam_version_file = \"/tmp/version_trigger.json\"
steam_version_check_seconds = 2.0
burst_duration_seconds = 1200
burst_interval_seconds = 20.0

[publishing]
max_auto_post_age_days = 2
max_catchup_posts = 1
include_ping = true
force_latest_on_start = false
dry_run = false

[prepared]
post_on_start = false
include_ping = true
translate = false
use_logs_channel = false
only_mode = false

[formatting]
embed_v2 = true
chunk_limit = 1950
char_budget = 3500
component_budget = 35
accent_colour = 13150315

[sources]
forum_url = \"https://forums.playdeadlock.com/forums/changelog.10/\"
steam_app_id = 1422450
";

    fn write_fixture(directory: &Path) -> PathBuf {
        let path = directory.join("bot.toml");
        std::fs::write(&path, FIXTURE).expect("Fixture schreiben");
        path
    }

    fn changed_options() -> PatchnotesOptions {
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.polling.interval_seconds = 90.0;
        options.publishing.dry_run = true;
        options.publishing.max_catchup_posts = 3;
        options.formatting.chunk_limit = 1800;
        options
    }

    #[test]
    fn options_stammen_aus_dem_dokument() {
        let document: toml::Value = FIXTURE.parse().unwrap();
        let options = patchnotes_options_from_document(&document).unwrap();
        assert_eq!(options.discord.retranslate_cooldown_seconds, 60.0);
        assert!(options.polling.steam_news_enabled);
        assert_eq!(options.polling.burst_duration_seconds, 1200);
        assert!(options.formatting.embed_v2);
        assert!(!options.publishing.dry_run);
    }

    #[test]
    fn fehlende_felder_erhalten_die_festen_defaults() {
        let document: toml::Value = "schema_version = 1".parse().unwrap();
        let options = patchnotes_options_from_document(&document).unwrap();
        assert_eq!(options.discord.retranslate_cooldown_seconds, 60.0);
        assert_eq!(options.polling.interval_seconds, 35.0);
        assert!(options.polling.steam_version_enabled);
        assert_eq!(options.publishing.max_auto_post_age_days, 2);
        assert!(options.prepared.include_ping);
        assert!(!options.prepared.only_mode);
        assert_eq!(options.formatting.char_budget, 3500);
    }

    #[test]
    fn fremde_typen_werden_abgelehnt() {
        let document: toml::Value = "[polling]\ninterval_seconds = \"oft\"\n".parse().unwrap();
        assert!(patchnotes_options_from_document(&document).is_err());
        let document: toml::Value = "[formatting]\nchunk_limit = 19.5\n".parse().unwrap();
        assert!(patchnotes_options_from_document(&document).is_err());
        let document: toml::Value = "[publishing]\ndry_run = 1\n".parse().unwrap();
        assert!(patchnotes_options_from_document(&document).is_err());
    }

    #[test]
    fn grenzen_entsprechen_dem_bot() {
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        assert!(options.validate());
        options.discord.retranslate_cooldown_seconds = 0.5;
        assert!(!options.validate());
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.polling.burst_interval_seconds = 90000.0;
        assert!(!options.validate());
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.publishing.max_auto_post_age_days = 366;
        assert!(!options.validate());
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.publishing.max_catchup_posts = 0;
        assert!(!options.validate());
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.formatting.chunk_limit = 2001;
        assert!(!options.validate());
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.formatting.char_budget = 499;
        assert!(!options.validate());
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.formatting.component_budget = 41;
        assert!(!options.validate());
        let mut options = patchnotes_options_from_document(&FIXTURE.parse().unwrap()).unwrap();
        options.discord.retranslate_cooldown_seconds = f64::NAN;
        assert!(!options.validate());
    }

    #[test]
    fn speichern_aendert_nur_freigegebene_felder() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_fixture(directory.path());
        let original = read_patchnotes_text(&path).unwrap();
        let saved = save_patchnotes_options(
            &path,
            &digest(original.as_bytes()),
            &changed_options(),
        )
        .unwrap();
        assert_ne!(saved, digest(original.as_bytes()));
        let text = read_patchnotes_text(&path).unwrap();
        assert!(text.starts_with("# Betriebstagebuch der Patchnotes\n"));
        assert!(text.contains("steam_version_file = \"/tmp/version_trigger.json\""));
        assert!(text.contains("accent_colour = 13150315"));
        assert!(text.contains("channel_id = 1326973956825284628"));
        assert!(text.contains(
            "forum_url = \"https://forums.playdeadlock.com/forums/changelog.10/\""
        ));
        assert!(text.contains("interval_seconds = 90.0"));
        assert!(text.contains("dry_run = true"));
        assert!(text.contains("max_catchup_posts = 3"));
        assert!(text.contains("chunk_limit = 1800"));
        let document: toml::Value = text.parse().unwrap();
        assert_eq!(
            patchnotes_options_from_document(&document).unwrap(),
            changed_options()
        );
    }

    #[test]
    fn speichern_mit_fremder_revision_gibt_konflikt() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_fixture(directory.path());
        assert!(matches!(
            save_patchnotes_options(&path, &"a".repeat(64), &changed_options()),
            Err(PatchnotesSaveError::Conflict)
        ));
        let text = read_patchnotes_text(&path).unwrap();
        assert_eq!(text, FIXTURE);
    }

    #[test]
    fn speichern_legt_fehlende_abschnitte_an() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bot.toml");
        std::fs::write(&path, "schema_version = 1\n").unwrap();
        let original = read_patchnotes_text(&path).unwrap();
        save_patchnotes_options(&path, &digest(original.as_bytes()), &changed_options())
            .unwrap();
        let text = read_patchnotes_text(&path).unwrap();
        let document: toml::Value = text.parse().unwrap();
        let options = patchnotes_options_from_document(&document).unwrap();
        assert_eq!(options, changed_options());
        assert!(text.contains("[publishing]"));
    }

    #[test]
    fn show_ausgabe_wird_gelesen() {
        assert_eq!(
            parse_service_show(
                "ActiveState=active\nMainPID=3301961\nExecMainStartTimestampMonotonic=52994094608\n"
            ),
            Some((true, Some(3301961), Some(52994094608)))
        );
        assert_eq!(
            parse_service_show(
                "ActiveState=failed\nMainPID=0\nExecMainStartTimestampMonotonic=1\n"
            ),
            Some((false, None, Some(1)))
        );
        assert_eq!(parse_service_show("MainPID=7\n"), None);
        assert_eq!(
            parse_service_show("ActiveState=activating\n"),
            Some((false, None, None))
        );
    }
}
