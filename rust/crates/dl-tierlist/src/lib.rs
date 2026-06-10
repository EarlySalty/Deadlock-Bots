//! Öffentliche Tierlist — Rust-Port von `service/tierlist_public.py`.
//!
//! Vertrag: feldgenau identische JSON-Antworten; das externe SPA-Frontend
//! (Website/dl-tierlist, via Caddy unter /builds/ → strip_prefix → :8771)
//! bleibt unverändert. Admin-Auth über den Dashboard-Session-Cookie,
//! validiert per interner Dashboard-API.

mod admin;
mod data;
mod refresh;
mod settings;
mod util;
mod votes;

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use dl_db::Db;
use dl_webcore::envelope::error_message;
use dl_webcore::DashboardClient;

pub use refresh::refresh_loop;
pub use settings::SESSION_COOKIE;

pub const DEADLOCK_API_BASE: &str = "https://api.deadlock-api.com/v1";

pub struct TierlistApp {
    pub db: Db,
    pub dashboard: DashboardClient,
    /// HTTP-Client für api.deadlock-api.com (30-s-Timeout wie das Original).
    pub api: reqwest::Client,
    pub api_base: String,
    pub votes: std::sync::Mutex<votes::VoteRateLimiter>,
    pub refresh_lock: tokio::sync::Mutex<()>,
}

pub type SharedApp = Arc<TierlistApp>;

impl TierlistApp {
    pub fn new(db: Db, dashboard: DashboardClient, api_base: impl Into<String>) -> SharedApp {
        let api = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest-Client bauen");
        Arc::new(Self {
            db,
            dashboard,
            api,
            api_base: api_base.into().trim_end_matches('/').to_string(),
            votes: std::sync::Mutex::new(votes::VoteRateLimiter::default()),
            refresh_lock: tokio::sync::Mutex::new(()),
        })
    }
}

pub fn router(app: SharedApp) -> Router {
    Router::new()
        .route("/api/heroes", get(handle_heroes))
        .route("/api/tierlist", get(handle_tierlist))
        .route("/api/tierlist/history", get(handle_tierlist_history))
        .route(
            "/api/builds/{build_id}/vote",
            post(votes::handle_build_vote),
        )
        .route("/api/admin/me", get(admin::handle_admin_me))
        .route(
            "/api/admin/hero/{hero_id}",
            get(admin::handle_admin_hero_get).put(admin::handle_admin_hero_put),
        )
        .route(
            "/api/admin/settings",
            get(admin::handle_admin_settings_get).put(admin::handle_admin_settings_put),
        )
        .route("/api/admin/refresh", post(admin::handle_admin_refresh))
        .layer(middleware::from_fn(security_headers))
        .with_state(app)
}

/// Security-Header auf jeder Antwort — exakt wie _security_headers_mw.
async fn security_headers(request: axum::extract::Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );
    response
}

/// `None` = unbekannter Bucket (Antwort: 400 invalid_bucket).
fn parse_bucket(params: &std::collections::HashMap<String, String>) -> Option<String> {
    let bucket = params.get("bucket").map(String::as_str).unwrap_or("all");
    settings::BUCKETS
        .iter()
        .any(|(name, _, _)| *name == bucket)
        .then(|| bucket.to_string())
}

fn invalid_bucket() -> Response {
    error_message(
        StatusCode::BAD_REQUEST,
        "invalid_bucket",
        "Ungültiger Bucket.",
    )
}

fn internal(err: impl std::fmt::Display) -> Response {
    tracing::error!(%err, "Tierlist: interner Fehler");
    error_message(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Interner Serverfehler.",
    )
}

async fn handle_heroes(State(app): State<SharedApp>) -> Response {
    match app.db.read(data::heroes_payload).await {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => internal(err),
    }
}

async fn handle_tierlist(
    State(app): State<SharedApp>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(bucket) = parse_bucket(&params) else {
        return invalid_bucket();
    };
    let settings = match app.db.write(|c| settings::read_settings(c)).await {
        Ok(s) => s,
        Err(err) => return internal(err),
    };
    match app
        .db
        .read(move |conn| data::tierlist_payload(conn, &bucket, &settings))
        .await
    {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => internal(err),
    }
}

async fn handle_tierlist_history(
    State(app): State<SharedApp>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(bucket) = parse_bucket(&params) else {
        return invalid_bucket();
    };
    let settings = match app.db.write(|c| settings::read_settings(c)).await {
        Ok(s) => s,
        Err(err) => return internal(err),
    };
    match app
        .db
        .read(move |conn| data::history_payload(conn, &bucket, &settings))
        .await
    {
        Ok(payload) => Json(payload).into_response(),
        Err(err) => internal(err),
    }
}
