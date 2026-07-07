//! Öffentliche Aktivitäts-Statistiken — Rust-Port von `service/public_stats.py`.
//!
//! Vertrag: feldgenau identische JSON-Antworten; das bestehende
//! `activity_stats.html`-Frontend und alle Cookies bleiben kompatibel.

// Handler-Helfer geben `Result<T, Response>` zurück (frühe HTTP-Fehlerantwort
// als Err) — das ist in axum idiomatisch; die Größe des Err-Typs ist hier egal,
// weil die Helfer genau einmal pro Request laufen.
#![allow(clippy::result_large_err)]

mod auth;
mod me;
mod public;
mod rank_history;
mod ranks;
mod timeutil;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use dl_webcore::{DashboardClient, SessionCodec, WebConfig};
use sqlx::PgPool;

pub use ranks::RANK_ORDER;

pub const SESSION_COOKIE: &str = "dl_session";
pub const PRE_AUTH_COOKIE: &str = "dl_pre_auth";
pub const SESSION_TTL: i64 = 14 * 24 * 60 * 60;
pub const PRE_AUTH_TTL: i64 = 10 * 60;
pub const DEFAULT_REDIRECT: &str = "/aktivitaet/";

pub struct StatsApp {
    pub pool: PgPool,
    pub codec: SessionCodec,
    pub dashboard: DashboardClient,
    pub cookie_secure: bool,
    pub cors_origins: HashSet<String>,
    pub callback_url: String,
    pub static_dir: PathBuf,
}

pub type SharedApp = Arc<StatsApp>;

impl StatsApp {
    pub fn new(pool: PgPool, dashboard: DashboardClient, web_cfg: &WebConfig) -> SharedApp {
        Arc::new(Self {
            pool,
            codec: SessionCodec::new(web_cfg.session_secret.clone()),
            dashboard,
            cookie_secure: web_cfg.cookie_secure,
            cors_origins: web_cfg.cors_origins.iter().cloned().collect(),
            callback_url: web_cfg.stats_callback_url.clone(),
            static_dir: PathBuf::from(&web_cfg.static_dir),
        })
    }
}

pub fn router(app: SharedApp) -> Router {
    let mut router = Router::new()
        .route("/", get(handle_index))
        .route("/health", get(public::handle_health))
        .route(
            "/api/activity-heatmap",
            get(public::handle_activity_heatmap),
        )
        .route(
            "/api/rank-distribution",
            get(public::handle_rank_distribution),
        )
        .route(
            "/api/lane-preferences",
            get(public::handle_lane_preferences),
        )
        .route(
            "/api/new-player-windows",
            get(public::handle_new_player_windows),
        )
        .route("/api/timeline", get(public::handle_timeline))
        .route("/api/rank-colors", get(public::handle_rank_colors))
        .route("/api/best-times", get(public::handle_best_times))
        .route("/api/voice-history", get(public::handle_voice_history))
        .route(
            "/api/public/leaderboard/voice",
            get(public::handle_voice_leaderboard),
        )
        .route(
            "/api/public/leaderboard/text",
            get(public::handle_text_leaderboard),
        )
        .route(
            "/api/public/leaderboard/rank",
            get(rank_history::handle_rank_leaderboard),
        )
        .route(
            "/api/public/rank-history/{user_id}",
            get(rank_history::handle_public_rank_history),
        )
        .route("/api/public/me", get(me::handle_me))
        .route("/api/public/me/stats", get(me::handle_me_stats))
        .route(
            "/api/public/me/rank-history",
            get(rank_history::handle_me_rank_history),
        )
        .route(
            "/api/public/me/rank-visibility",
            put(rank_history::handle_me_rank_visibility),
        )
        .route(
            "/api/public/me/voice-history",
            get(me::handle_me_voice_history),
        )
        .route(
            "/api/public/me/text-history",
            get(me::handle_me_text_history),
        )
        .route("/api/public/me/heatmap", get(me::handle_me_heatmap))
        .route("/api/public/me/co-players", get(me::handle_me_co_players))
        .route("/auth/discord/login", get(auth::handle_login))
        .route("/auth/discord/complete", get(auth::handle_complete))
        .route("/auth/discord/logout", post(auth::handle_logout))
        .route(
            "/api/public/{*tail}",
            axum::routing::options(handle_preflight),
        )
        .route("/auth/{*tail}", axum::routing::options(handle_preflight));

    let icons = app.static_dir.join("rank_icons");
    if icons.is_dir() {
        router = router.nest_service("/rank_icons", tower_http::services::ServeDir::new(icons));
    }

    router
        .layer(middleware::from_fn_with_state(app.clone(), security_mw))
        .with_state(app)
}

async fn handle_preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// `GET /` — liefert activity_stats.html.
async fn handle_index(State(app): State<SharedApp>) -> Response {
    match tokio::fs::read_to_string(app.static_dir.join("activity_stats.html")).await {
        Ok(html) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Seite nicht verfügbar.").into_response(),
    }
}

fn is_cors_path(path: &str) -> bool {
    path.starts_with("/api/public/") || path.starts_with("/auth/")
}

fn is_private_path(path: &str) -> bool {
    path.starts_with("/api/public/me") || path.starts_with("/auth/")
}

/// Cache-Control + nosniff + CORS auf jeder Antwort — wie `_security_mw`.
async fn security_mw(State(app): State<SharedApp>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let acr_headers = request
        .headers()
        .get("access-control-request-headers")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    let cache = if is_private_path(&path) {
        "no-store"
    } else {
        "public, max-age=60"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    if is_cors_path(&path) {
        if let Some(origin) = origin.filter(|o| app.cors_origins.contains(o)) {
            if let Ok(value) = HeaderValue::from_str(&origin) {
                headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
                headers.insert(
                    header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                    HeaderValue::from_static("true"),
                );
                headers.insert(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    HeaderValue::from_static("GET,POST,PUT,OPTIONS"),
                );
                let allow = acr_headers.unwrap_or_else(|| "Content-Type".to_string());
                if let Ok(allow) = HeaderValue::from_str(&allow) {
                    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, allow);
                }
                headers.insert(
                    header::ACCESS_CONTROL_MAX_AGE,
                    HeaderValue::from_static("600"),
                );
                headers.append(header::VARY, HeaderValue::from_static("Origin"));
            }
        }
    }
    response
}

// ── geteilte Handler-Helfer ────────────────────────────────────────────────

/// 401 exakt wie Python: `{"error":"unauthenticated"}`.
pub(crate) fn unauthenticated() -> Response {
    dl_webcore::envelope::unauthenticated()
}

pub(crate) fn internal_error() -> Response {
    dl_webcore::envelope::error_code(StatusCode::INTERNAL_SERVER_ERROR, "internal")
}

/// Session aus dem dl_session-Cookie verifizieren.
pub(crate) fn require_session(
    app: &StatsApp,
    headers: &HeaderMap,
) -> Result<serde_json::Map<String, serde_json::Value>, Response> {
    let cookie = cookie_value(headers, SESSION_COOKIE).unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    app.codec.verify(&cookie, now).ok_or_else(unauthenticated)
}

/// `_parse_user_id_from_session`: int > 0, sonst 401.
pub(crate) fn session_user_id(
    session: &serde_json::Map<String, serde_json::Value>,
) -> Result<i64, Response> {
    let raw = session.get("user_id");
    let parsed = match raw {
        Some(serde_json::Value::String(s)) => s.trim().parse::<i64>().ok(),
        Some(serde_json::Value::Number(n)) => n.as_i64(),
        _ => None,
    };
    parsed.filter(|v| *v > 0).ok_or_else(unauthenticated)
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            if k == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// `_parse_positive_int`: 400 `{"error":"invalid_<field>"}` bei Parse-/Bereichsfehler.
pub(crate) fn parse_positive_int(
    raw: Option<&String>,
    default: i64,
    minimum: i64,
    maximum: i64,
    field: &str,
) -> Result<i64, Response> {
    let invalid =
        || dl_webcore::envelope::error_code(StatusCode::BAD_REQUEST, &format!("invalid_{field}"));
    let value = match raw {
        None => default,
        Some(s) => s.trim().parse::<i64>().map_err(|_| invalid())?,
    };
    if value < minimum || value > maximum {
        return Err(invalid());
    }
    Ok(value)
}
