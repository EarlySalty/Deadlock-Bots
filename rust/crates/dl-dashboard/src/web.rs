//! HTTP-Schicht des Dashboards (:8766) — der Auth-Provider.
//!
//! Hier liegt der axum-Router mit den Login- und Delegations-Endpunkten.
//! Die eigentliche Sicherheits- und OAuth-Logik steckt in den getesteten
//! Bausteinen ([`crate::auth`], [`crate::authority`], [`crate::oauth`],
//! [`crate::oauth_state`], [`crate::session`]); die Handler bleiben dünn:
//! Request parsen → Baustein aufrufen → Antwort formen.
//!
//! WICHTIG: Der Server muss mit
//! `into_make_service_with_connect_info::<SocketAddr>()` gebunden werden —
//! die internen Routen sind loopback-only und brauchen die Peer-Adresse.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use sqlx::PgPool;

use crate::auth::{self, InternalReject};
use crate::authority::{decide_access, BrokerMemberLookup, MemberLookup};
use crate::config::{AccessLevel, DashboardConfig};
use crate::db::DashboardDbError;
use crate::names::{BrokerNameResolver, NameResolver};
use crate::oauth::{extract_steam_connection_ids, DiscordUser, OAuthClient};
use crate::oauth_state::{NewOAuthState, OAuthStateStore};
use crate::session::{NewSession, SessionStore};
use crate::{now_unix, now_unix_f64};

/// Name des Dashboard-Session-Cookies (opakes Token, serverseitig geprüft).
pub const SESSION_COOKIE: &str = "master_dash_session";
const DEFAULT_SCOPE: &str = "identify guilds.members.read";
const OWN_LOGIN_STATE_TTL: f64 = 21_600.0; // 6 h, wie der OAuth-State
const ADMIN_LOGIN_URL: &str = "/auth/discord/login";
const AUTH_MISCONFIGURED_MESSAGE: &str = "Dashboard Auth ist nicht korrekt konfiguriert. \
Discord OAuth Client-ID/Secret fehlen im Windows-Tresor (DeadlockBot).";

// ── App-Zustand ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DashboardApp {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: DashboardConfig,
    pool: PgPool,
    data_dir: PathBuf,
    oauth: OAuthClient,
    states: OAuthStateStore,
    sessions: SessionStore,
    lookup: Arc<dyn MemberLookup>,
    names: Arc<dyn NameResolver>,
    /// In-Memory-States des Admin-Logins (≠ DB-gestützte delegierte States).
    login_states: Mutex<HashMap<String, LoginState>>,
    /// Per-IP-Zeitstempel für die Callback-Ratenbegrenzung.
    rate: Mutex<HashMap<String, Vec<f64>>>,
    /// 30-Sekunden-Cache der öffentlichen Guild-Stats (Wert, Zeitpunkt).
    guild_stats_cache: Mutex<Option<(Value, f64)>>,
}

struct LoginState {
    next_path: String,
    redirect_uri: String,
    created_at: f64,
}

impl DashboardApp {
    /// Baut die App mit explizitem Mitglieds-Lookup und Namens-Resolver
    /// (für Tests).
    pub async fn new(
        cfg: DashboardConfig,
        pool: PgPool,
        lookup: Arc<dyn MemberLookup>,
        names: Arc<dyn NameResolver>,
    ) -> Result<Self, DashboardDbError> {
        let oauth = OAuthClient::new(
            cfg.discord_client_id.clone(),
            cfg.discord_client_secret.clone(),
            cfg.discord_api_base.clone(),
        );
        let states = OAuthStateStore::new(pool.clone(), cfg.oauth_state_ttl_secs);
        let sessions =
            SessionStore::persistent(pool.clone(), cfg.session_ttl_secs, now_unix_f64()).await?;
        let data_dir = cfg.data_dir.clone();
        Ok(Self {
            inner: Arc::new(Inner {
                cfg,
                pool,
                data_dir,
                oauth,
                states,
                sessions,
                lookup,
                names,
                login_states: Mutex::new(HashMap::new()),
                rate: Mutex::new(HashMap::new()),
                guild_stats_cache: Mutex::new(None),
            }),
        })
    }

    /// Produktions-Konstruktor: Lookup und Namensauflösung gehen über den
    /// Master-Broker.
    pub async fn from_config(cfg: DashboardConfig, pool: PgPool) -> Result<Self, DashboardDbError> {
        let lookup = Arc::new(BrokerMemberLookup::new(cfg.broker_base.clone()));
        let names = Arc::new(BrokerNameResolver::new(cfg.broker_base.clone()));
        Self::new(cfg, pool, lookup, names).await
    }

    fn cfg(&self) -> &DashboardConfig {
        &self.inner.cfg
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub(crate) fn repo_root(&self) -> Option<&Path> {
        self.data_dir().parent()
    }

    pub(crate) fn names(&self) -> &Arc<dyn NameResolver> {
        &self.inner.names
    }

    pub(crate) fn audit_bot_user_id(&self) -> u64 {
        self.cfg().audit_bot_user_id
    }

    pub(crate) fn broker_base(&self) -> &str {
        &self.cfg().broker_base
    }

    /// Frischer (≤30 s) Guild-Stats-Cache-Wert, sonst `None`.
    pub(crate) fn guild_stats_cached(&self, now: f64) -> Option<Value> {
        let guard = self
            .inner
            .guild_stats_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some((value, ts)) if now - *ts < 30.0 => Some(value.clone()),
            _ => None,
        }
    }

    pub(crate) fn set_guild_stats_cache(&self, value: Value, now: f64) {
        let mut guard = self
            .inner
            .guild_stats_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some((value, now));
    }

    /// Auth-Gate für lesende `/api`-Routen: ohne erzwungene Auth offen, sonst
    /// gültige Session nötig (wie `_check_auth` ohne CSRF/Full-Access).
    pub(crate) async fn guard_read(&self, headers: &HeaderMap) -> Result<(), Response> {
        if self.cfg().auth_misconfigured() {
            return Err(auth_misconfigured_response());
        }
        if !self.cfg().auth_enforced() {
            return Ok(());
        }
        if self.session_from_headers(headers).await?.is_some() {
            Ok(())
        } else {
            Err(err_text(401, "Authentication required"))
        }
    }

    /// Gate für Routen mit `required=True, require_full_access=True`: immer
    /// erzwungen, gültige Session mit Voll-Zugriff nötig (401/403 wie Original).
    pub(crate) async fn guard_full(&self, headers: &HeaderMap) -> Result<(), Response> {
        if self.cfg().auth_misconfigured() {
            return Err(auth_misconfigured_response());
        }
        match self.session_from_headers(headers).await? {
            Some(session) if session.has_full_access() => Ok(()),
            Some(_) => Err(err_text(403, "Full dashboard access required")),
            None => Err(err_text(401, "Authentication required")),
        }
    }

    /// Gate für mutierende Routen (POST/PUT/PATCH/DELETE): Session + Origin-
    /// Prüfung + CSRF-Token, dann ggf. Voll-Zugriff. Reihenfolge/Status wie
    /// `_check_auth` (401 Session, 403 Origin/CSRF/Full). Gibt die Session
    /// für das Audit-Log zurück.
    pub(crate) async fn guard_mutate(
        &self,
        headers: &HeaderMap,
        require_full: bool,
    ) -> Result<crate::session::Session, Response> {
        if self.cfg().auth_misconfigured() {
            return Err(auth_misconfigured_response());
        }
        let Some(session) = self.session_from_headers(headers).await? else {
            return Err(err_text(401, "Authentication required"));
        };
        if !self.allowed_origin(headers) {
            return Err(err_text(403, "Origin validation failed"));
        }
        if !check_csrf(headers, &session) {
            return Err(err_text(403, "CSRF validation failed"));
        }
        if require_full && !session.has_full_access() {
            return Err(err_text(403, "Full dashboard access required"));
        }
        Ok(session)
    }

    /// `_is_allowed_request_origin`: Origins werden aus Basis-URLs + Env
    /// geseedet; eine leere Liste ist Misconfig und schliesst ab.
    fn allowed_origin(&self, headers: &HeaderMap) -> bool {
        let allowed = &self.cfg().allowed_origins;
        if allowed.is_empty() {
            return false;
        }
        match request_origin(headers) {
            Some(origin) => allowed.iter().any(|a| a == &origin),
            None => false,
        }
    }

    fn login_states(&self) -> MutexGuard<'_, HashMap<String, LoginState>> {
        self.inner
            .login_states
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Ratenbegrenzung pro IP (gleitendes Fenster). `true` = erlaubt.
    fn rate_ok(&self, ip: &str, max: usize, window: f64, now: f64) -> bool {
        let mut map = self.inner.rate.lock().unwrap_or_else(|p| p.into_inner());
        let hits = map.entry(ip.to_string()).or_default();
        hits.retain(|t| now - *t < window);
        if hits.len() >= max {
            return false;
        }
        hits.push(now);
        true
    }

    /// Liest die aktuelle Session aus dem Cookie und verlängert sie gleitend.
    async fn session_from_headers(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<crate::session::Session>, Response> {
        if self.cfg().auth_misconfigured() || !self.cfg().auth_enforced() {
            return Ok(None);
        }
        for session_id in read_cookies(headers, SESSION_COOKIE) {
            match self.inner.sessions.touch(&session_id, now_unix_f64()).await {
                Ok(Some(session)) => return Ok(Some(session)),
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(%error, "Admin-Session konnte nicht persistiert werden");
                    return Err(err_text(500, "Session persistence failed"));
                }
            }
        }
        Ok(None)
    }
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn router(app: DashboardApp) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/admin", get(index))
        .route("/insights", get(insights_page))
        .route("/api/auth/me", get(auth_me))
        .route("/auth/discord/login", get(login))
        .route("/auth/discord/callback", get(own_callback))
        .route("/callback/discord", get(callback))
        .route("/auth/logout", get(logout).post(logout))
        .route("/internal/v1/discord/initiate", post(initiate))
        .route("/internal/v1/discord/consume-result", post(consume_result))
        .route(
            "/internal/turnier/v1/discord/authorize-url",
            post(turnier_authorize_url),
        )
        .route(
            "/internal/turnier/v1/discord/session",
            post(turnier_session),
        )
        .route(
            "/internal/discord/v1/steam-link-session",
            post(steam_link_session),
        )
        .route(
            "/internal/twitch/v1/discord/authorize-url",
            post(twitch_authorize_url),
        )
        .route("/internal/twitch/v1/discord/session", post(twitch_session))
        .route(
            "/internal/twitch/v1/discord/validate-session",
            post(validate_session),
        )
        .route(
            "/internal/twitch/v1/discord/import-session",
            post(import_session),
        )
        .route(
            "/internal/twitch/v1/discord/revoke-session",
            post(revoke_session),
        )
        .route(
            "/internal/coaching/v1/no-show-ban",
            post(coaching_no_show_ban),
        )
        // Analytics-Reads (Phase 9b) — Session-gegatet, reine DB-Reads.
        .route("/api/server-stats", get(crate::server_stats::server_stats))
        .route("/api/member-events", get(crate::analytics::member_events))
        .route(
            "/api/message-activity",
            get(crate::analytics::message_activity),
        )
        .route("/api/voice-history", get(crate::analytics::voice_history))
        .route(
            "/api/repo-activity",
            get(crate::repo_activity::repo_activity),
        )
        .route(
            "/api/repo-activity/refresh",
            post(crate::repo_activity::repo_activity_refresh),
        )
        .route("/api/leave-surveys", get(crate::analytics::leave_surveys))
        .route("/api/user-retention", get(crate::analytics::user_retention))
        .route("/api/voice-stats", get(crate::analytics::voice_stats))
        .route("/api/audit-log", get(crate::audit::audit_log))
        .route("/api/brain/overview", get(crate::brain::overview))
        .route("/api/brain/plan", get(crate::brain::plan))
        .route("/api/brain/plan/runs", get(crate::brain::plan_runs))
        .route(
            "/api/brain/plan/item/{id}",
            post(crate::brain::update_plan_item),
        )
        .route("/api/brain/wiki", get(crate::brain::wiki))
        .route("/api/brain/wiki/page", get(crate::brain::wiki_page))
        .route(
            "/api/co-player-network",
            get(crate::analytics::co_player_network),
        )
        .route(
            "/api/co-player-network/",
            get(crate::analytics::co_player_network),
        )
        .route("/api/insights/overview", get(crate::insights::overview))
        .route("/api/insights/growth", get(crate::insights::growth))
        .route("/api/insights/activation", get(crate::insights::activation))
        .route("/api/insights/retention", get(crate::insights::retention))
        .route("/api/insights/engagement", get(crate::insights::engagement))
        .route("/api/insights/audience", get(crate::insights::audience))
        .route(
            "/api/insights/top-invites",
            get(crate::insights::top_invites),
        )
        .route(
            "/api/insights/import",
            post(crate::insights::import)
                .layer(axum::extract::DefaultBodyLimit::max(30 * 1024 * 1024)),
        )
        // Deadlock-Konfiguration (Phase 9c) — Read-Seite, Full-Access.
        .route(
            "/api/deadlock/config",
            get(crate::deadlock::deadlock_config).post(crate::deadlock::deadlock_config_update),
        )
        .route(
            "/api/deadlock/heroes",
            get(crate::deadlock::deadlock_heroes).post(crate::deadlock::deadlock_upsert_hero),
        )
        .route(
            "/api/deadlock/heroes/{hero_id}",
            axum::routing::delete(crate::deadlock::deadlock_delete_hero),
        )
        .route(
            "/api/reaction-roles",
            get(crate::reaction_roles::reaction_roles)
                .post(crate::reaction_roles::reaction_role_upsert),
        )
        .route(
            "/api/reaction-roles/{id}",
            axum::routing::delete(crate::reaction_roles::reaction_role_delete),
        )
        .route("/api/scrims", get(crate::scrims::scrims_overview))
        .route(
            "/api/scrims/matches",
            post(crate::scrims::scrims_create_match),
        )
        .route(
            "/api/scrims/match-requests/defaults",
            get(crate::scrims::scrims_match_request_defaults),
        )
        .route(
            "/api/scrims/match-requests",
            post(crate::scrims::scrims_create_match_request_batch),
        )
        .route(
            "/api/scrims/matches/{match_id}/start",
            post(crate::scrims::scrims_start_match),
        )
        .route(
            "/api/scrims/matches/{match_id}/lobby-code",
            post(crate::scrims::scrims_set_lobby_code),
        )
        .route(
            "/api/scrims/matches/{match_id}/result",
            post(crate::scrims::scrims_request_result),
        )
        .route(
            "/api/scrims/participants/{participant_id}/notes",
            post(crate::scrims::scrims_update_participant_notes),
        )
        // Öffentlicher Austritts-Umfrage-Flow (Phase 9e) — token-basiert.
        // POST nimmt bis zu 5 Bilder (5 MiB) → Body-Limit hochsetzen.
        .route(
            "/api/leave-survey/{token}",
            get(crate::survey::leave_survey_get)
                .post(crate::survey::leave_survey_post)
                .layer(axum::extract::DefaultBodyLimit::max(30 * 1024 * 1024)),
        )
        // Admin-geschützter Abruf hochgeladener Umfrage-Bilder.
        .route(
            "/api/leave-surveys/image/{token}/{filename}",
            get(crate::survey::leave_survey_image),
        )
        // Öffentliche Endpunkte (Phase 9e) — kein Auth, CORS für die Website.
        .route(
            "/api/public/patch-notes",
            get(crate::public::patch_notes).options(crate::public::public_cors),
        )
        .route(
            "/api/public/guild-stats",
            get(crate::public::guild_stats).options(crate::public::public_cors),
        )
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(app)
}

async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if !headers.contains_key(header::X_FRAME_OPTIONS) {
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    if !headers.contains_key(header::X_CONTENT_TYPE_OPTIONS) {
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
    }
    if !headers.contains_key("X-XSS-Protection") {
        headers.insert(
            "X-XSS-Protection",
            HeaderValue::from_static("1; mode=block"),
        );
    }
    if !headers.contains_key(header::REFERRER_POLICY) {
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        );
    }
    if !headers.contains_key("Permissions-Policy") {
        headers.insert(
            "Permissions-Policy",
            HeaderValue::from_static("geolocation=(), microphone=(), camera=(), payment=()"),
        );
    }
    response
}

// ── Browser-Routen (Login/Callback/Logout) ──────────────────────────────────

/// Escaping wie Pythons `html.escape(quote=True)` — für in HTML eingesetzte
/// Werte (hier das Nutzer-Label, das aus der Session stammt).
fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Lädt eine SPA-Datei aus `service/static/` (neben der DB: `repo/data/` →
/// `repo/`). Kein Caching — wie das Original; fehlt die Datei, gibt der
/// aufrufende Handler 500.
async fn load_static_html(app: &DashboardApp, name: &str) -> Option<String> {
    let repo_root = app.repo_root()?;
    tokio::fs::read_to_string(repo_root.join("service/static").join(name))
        .await
        .ok()
}

/// Setzt die drei Auth-Platzhalter in eine SPA-Datei ein (Nutzer-Label
/// HTML-escaped; `next` steuert das Login-Ziel) und liefert die HTML-Antwort.
fn render_spa(html: String, display_name: &str, login_next: &str) -> Response {
    let login_url = format!("/auth/discord/login?next={login_next}");
    let rendered = html
        .replace("{{AUTH_USER_LABEL}}", &html_escape_attr(display_name))
        .replace("{{DISCORD_LOGIN_URL}}", &login_url)
        .replace("{{AUTH_LOGOUT_URL}}", "/auth/logout");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        rendered,
    )
        .into_response()
}

/// `/` und `/admin` — liefert die Dashboard-SPA mit eingesetzten Auth-Platzhaltern
/// (Port von `_handle_index`). Bei erzwungener Auth ohne Session → Discord-Login.
async fn index(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if app.cfg().auth_misconfigured() {
        return auth_misconfigured_response();
    }
    let session = match app.session_from_headers(&headers).await {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    if app.cfg().auth_enforced() && session.is_none() {
        return redirect("/auth/discord/login?next=%2Fadmin", None);
    }
    let display_name = session
        .as_ref()
        .map(|s| s.display_name.clone())
        .unwrap_or_else(|| "Nicht angemeldet".to_string());
    let Some(html) = load_static_html(&app, "dashboard.html").await else {
        return err_text(500, "dashboard.html nicht ladbar");
    };
    render_spa(html, &display_name, "%2Fadmin")
}

/// `/insights` — Server-Einblicke-Seite (statisch, Daten via `/api/insights/*`).
/// Gleiches Auth-Verhalten wie `index`.
async fn insights_page(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if app.cfg().auth_misconfigured() {
        return auth_misconfigured_response();
    }
    let session = match app.session_from_headers(&headers).await {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    if app.cfg().auth_enforced() && session.is_none() {
        return redirect("/auth/discord/login?next=%2Finsights", None);
    }
    let display_name = session
        .as_ref()
        .map(|s| s.display_name.clone())
        .unwrap_or_else(|| "Nicht angemeldet".to_string());
    let Some(html) = load_static_html(&app, "insights.html").await else {
        return err_text(500, "insights.html nicht ladbar");
    };
    render_spa(html, &display_name, "%2Finsights")
}

async fn auth_me(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if app.cfg().auth_misconfigured() {
        return auth_misconfigured_response();
    }
    if !app.cfg().auth_enforced() {
        return ok_json(json!({
            "enabled": false,
            "authenticated": false,
            "mode": "none",
        }));
    }
    match app.session_from_headers(&headers).await {
        Ok(Some(session)) => ok_json(json!({
            "enabled": true,
            "authenticated": true,
            "mode": "discord",
            "user": {
                "id": session.user_id,
                "display_name": session.display_name,
                "username": session.username,
            },
            "csrf_token": session.csrf_token,
        })),
        Ok(None) => err_text(401, "Authentication required"),
        Err(resp) => resp,
    }
}

async fn login(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let now = now_unix_f64();
    if !app.rate_ok(&peer.ip().to_string(), 10, 60.0, now) {
        return err_text(
            429,
            "Zu viele Login-Anfragen. Bitte in einer Minute erneut.",
        );
    }
    if app.cfg().auth_misconfigured() {
        return auth_misconfigured_response();
    }
    if !app.cfg().auth_enforced() {
        return redirect("/admin", None);
    }
    let next_path = public_dashboard_redirect_url(
        app.cfg(),
        params.get("next").map(String::as_str).unwrap_or(""),
    );
    let redirect_uri = app.cfg().discord_redirect_uri.clone();
    let state = crate::token::session_token();
    {
        let mut states = app.login_states();
        prune_login_states(&mut states, now);
        states.insert(
            state.clone(),
            LoginState {
                next_path,
                redirect_uri: redirect_uri.clone(),
                created_at: now,
            },
        );
    }
    let authorize_url = app
        .inner
        .oauth
        .authorize_url("identify", &redirect_uri, &state);
    redirect(&authorize_url, None)
}

/// `/callback/discord` — delegierter Flow ODER (Fallback) Admin-Login.
async fn callback(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let now_f = now_unix_f64();
    if !app.rate_ok(&peer.ip().to_string(), 20, 60.0, now_f) {
        return err_text(
            429,
            "Zu viele OAuth-Callback-Anfragen. Bitte in einer Minute erneut.",
        );
    }
    if !app.cfg().discord_oauth_configured() {
        return auth_misconfigured_response();
    }
    let state = params.get("state").map(|s| s.trim()).unwrap_or("");
    if state.is_empty() {
        return err_text(400, "Fehlender OAuth state.");
    }
    let now = now_unix();
    let state_data = match app.inner.states.validate(state, now).await {
        Ok(Some(data)) => data,
        Ok(None) => {
            // Kein DB-State → evtl. ein In-Memory-Admin-Login-State.
            if app.login_states().contains_key(state) {
                return own_login_complete(&app, state, &params, &headers).await;
            }
            return err_text(400, "OAuth state ungültig oder abgelaufen.");
        }
        Err(_) => return err_text(500, "OAuth state lookup failed."),
    };
    if state_data.provider.trim().to_lowercase() != "discord" {
        return err_text(400, "OAuth state provider mismatch.");
    }

    let is_delegated = state_data.flow_type.starts_with("delegated:");
    if !is_delegated {
        match app.inner.states.consume(state, now).await {
            Ok(true) => {}
            _ => return err_text(400, "OAuth state ungültig oder bereits verwendet."),
        }
    }

    let redirect_after = auth::safe_href(&state_data.redirect_after, "/admin");
    let metadata = state_data.metadata.clone().unwrap_or(Value::Null);
    let completed_at = now;

    // Hilfs-Closure-Ersatz: Ergebnis speichern und passend weiterleiten.
    let error = params.get("error").map(|s| s.trim()).unwrap_or("");
    if !error.is_empty() {
        store_oauth_error(&app, state, &metadata, completed_at, error).await;
        return delegated_redirect(&redirect_after, is_delegated, state);
    }
    let code = params.get("code").map(|s| s.trim()).unwrap_or("");
    if code.is_empty() {
        store_oauth_error(&app, state, &metadata, completed_at, "missing_code").await;
        return delegated_redirect(&redirect_after, is_delegated, state);
    }
    let redirect_uri = metadata
        .get("redirect_uri")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| app.cfg().discord_redirect_uri.clone());

    let token = app.inner.oauth.exchange_code(code, &redirect_uri).await;
    let Some(token) = token.filter(|t| !t.access_token.trim().is_empty()) else {
        store_oauth_error(
            &app,
            state,
            &metadata,
            completed_at,
            "token_exchange_failed",
        )
        .await;
        return delegated_redirect(&redirect_after, is_delegated, state);
    };

    let user = app.inner.oauth.fetch_user(&token.access_token).await;
    let mut result = Map::new();
    result.insert("provider".into(), json!("discord"));
    result.insert("status".into(), json!("success"));
    result.insert("completed_at".into(), json!(completed_at));
    result.insert(
        "token".into(),
        serde_json::to_value(&token).unwrap_or(Value::Null),
    );
    if let Some(user) = user {
        result.insert(
            "user".into(),
            serde_json::to_value(&user).unwrap_or(Value::Null),
        );
    }
    store_oauth_result(&app, state, &metadata, Value::Object(result)).await;
    delegated_redirect(&redirect_after, is_delegated, state)
}

/// `/auth/discord/callback` — reiner Admin-Login-Callback.
async fn own_callback(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let now = now_unix_f64();
    if !app.rate_ok(&peer.ip().to_string(), 20, 60.0, now) {
        return err_text(
            429,
            "Zu viele OAuth-Callback-Anfragen. Bitte in einer Minute erneut.",
        );
    }
    let state = params.get("state").map(|s| s.trim()).unwrap_or("");
    own_login_complete(&app, state, &params, &headers).await
}

/// Gemeinsamer Admin-Login-Abschluss (Code → Profil → Zugriffsentscheidung →
/// Session). Wird von beiden Callback-Routen genutzt.
async fn own_login_complete(
    app: &DashboardApp,
    state: &str,
    params: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Response {
    if app.cfg().auth_misconfigured() {
        return auth_misconfigured_response();
    }
    if !app.cfg().auth_enforced() {
        return redirect("/admin", None);
    }
    let error = params.get("error").map(|s| s.trim()).unwrap_or("");
    if !error.is_empty() {
        return err_text(401, &format!("Discord OAuth Fehler: {error}"));
    }
    let code = params.get("code").map(|s| s.trim()).unwrap_or("");
    if state.is_empty() || code.is_empty() {
        return err_text(400, "Fehlender OAuth state/code.");
    }
    let now = now_unix_f64();
    let login_state = {
        let mut states = app.login_states();
        prune_login_states(&mut states, now);
        states.remove(state)
    };
    let Some(login_state) = login_state else {
        return err_text(400, "OAuth state ungültig oder abgelaufen.");
    };

    let token = app
        .inner
        .oauth
        .exchange_code(code, &login_state.redirect_uri)
        .await;
    let Some(token) = token.filter(|t| !t.access_token.trim().is_empty()) else {
        return err_text(401, "OAuth Austausch fehlgeschlagen.");
    };
    let Some(user) = app.inner.oauth.fetch_user(&token.access_token).await else {
        return err_text(401, "Discord-User konnte nicht geladen werden.");
    };
    let Some(user_id) = user.id_u64() else {
        return err_text(401, "Ungültige Discord-User-ID.");
    };

    let info = app
        .inner
        .lookup
        .member_access(None, user_id)
        .await
        .unwrap_or_default();
    let outcome = decide_access(app.cfg(), user_id, &info);
    let Some(level) = outcome.level else {
        tracing::warn!(
            user_id,
            reason = outcome.reason,
            "AUDIT dashboard login denied"
        );
        return err_text(
            403,
            "Kein Zugriff auf das Admin-Dashboard. Benötigt: Administrator-Recht, \
             Moderator-Rolle oder Community-Moderator-Rolle.",
        );
    };

    let session_id = match app
        .inner
        .sessions
        .create(
            NewSession {
                user_id,
                username: user.username.clone(),
                display_name: user.display_name(),
                reason: outcome.reason.to_string(),
                access_level: level,
            },
            now,
        )
        .await
    {
        Ok(session_id) => session_id,
        Err(error) => {
            tracing::error!(%error, "Admin-Session konnte nicht persistiert werden");
            return err_text(500, "Session persistence failed");
        }
    };
    let cookie = build_session_cookie(
        &session_id,
        app.cfg().session_ttl_secs,
        request_is_secure(headers, app.cfg()),
        session_cookie_domain(&app.cfg().discord_redirect_uri).as_deref(),
    );
    redirect(
        &auth::safe_href(&login_state.next_path, "/admin"),
        Some(cookie),
    )
}

async fn logout(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    for session_id in read_cookies(&headers, SESSION_COOKIE) {
        if let Err(error) = app.inner.sessions.remove(&session_id).await {
            tracing::warn!(%error, "Persistierte Admin-Session konnte nicht gelöscht werden");
            return err_text(500, "Session persistence failed");
        }
    }
    let target = if app.cfg().auth_enforced() {
        ADMIN_LOGIN_URL
    } else {
        "/admin"
    };
    redirect(
        &auth::safe_href(target, "/admin"),
        Some(clear_session_cookie(
            session_cookie_domain(&app.cfg().discord_redirect_uri).as_deref(),
        )),
    )
}

// ── Interne Routen: delegierter Flow ────────────────────────────────────────

async fn initiate(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_any(&app, &peer, &headers) {
        return resp;
    }
    if !app.cfg().discord_oauth_configured() {
        return err_json(503, "discord_oauth_not_configured");
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let scope = string_field(&payload, "scope").unwrap_or_else(|| DEFAULT_SCOPE.to_string());
    let redirect_after = string_field(&payload, "redirect_after").unwrap_or_default();
    let requesting_service = string_field(&payload, "requesting_service").unwrap_or_default();
    let service_metadata = payload
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if redirect_after.is_empty() || !auth::is_allowed_redirect_after(&redirect_after) {
        return err_json(400, "invalid_redirect_after");
    }
    if requesting_service.is_empty() {
        return err_json(400, "missing_requesting_service");
    }

    let state = crate::token::session_token();
    let redirect_uri = app.cfg().discord_redirect_uri.clone();
    let mut metadata = Map::new();
    metadata.insert("redirect_uri".into(), json!(redirect_uri));
    metadata.insert("scope".into(), json!(scope));
    for (key, value) in service_metadata {
        metadata.insert(key, value);
    }
    let metadata = Value::Object(metadata);

    let created = app
        .inner
        .states
        .create(
            NewOAuthState {
                state: &state,
                provider: "discord",
                flow_type: &format!("delegated:{requesting_service}"),
                redirect_after: &redirect_after,
                metadata: Some(&metadata),
                requesting_service: Some(&requesting_service),
            },
            now_unix(),
        )
        .await;
    if created.is_err() {
        return err_json(500, "state_create_failed");
    }

    let authorize_url = app.inner.oauth.authorize_url(&scope, &redirect_uri, &state);
    ok_json(json!({ "authorize_url": authorize_url, "state_id": state }))
}

async fn consume_result(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_any(&app, &peer, &headers) {
        return resp;
    }
    if !app.cfg().discord_oauth_configured() {
        return err_json(503, "discord_oauth_not_configured");
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let state_id = string_field(&payload, "state_id").unwrap_or_default();
    if state_id.is_empty() {
        return err_json(400, "missing_state_id");
    }
    let now = now_unix();
    let state_data = match app.inner.states.validate(&state_id, now).await {
        Ok(Some(data)) => data,
        Ok(None) => return err_json(404, "state_not_found_or_expired"),
        Err(_) => return err_json(500, "state_lookup_failed"),
    };

    let metadata = state_data
        .metadata
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let Some(oauth_result) = metadata.get("oauth_result").and_then(Value::as_object) else {
        return err_json(202, "oauth_not_completed");
    };
    if oauth_result.get("status").and_then(Value::as_str) == Some("error") {
        let _ = app.inner.states.consume(&state_id, now).await;
        let code = oauth_result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("oauth_failed");
        return err_json(400, code);
    }
    match app.inner.states.consume(&state_id, now).await {
        Ok(true) => {}
        Ok(false) => return err_json(409, "state_already_consumed"),
        Err(_) => return err_json(500, "state_consume_failed"),
    }

    let token_obj = oauth_result.get("token").and_then(Value::as_object);
    let access_token = token_obj
        .and_then(|t| t.get("access_token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let user: Option<DiscordUser> = oauth_result
        .get("user")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());
    let Some(user) = user else {
        return err_json(502, "invalid_discord_user");
    };
    let Some(user_id) = user.id_u64() else {
        return err_json(502, "invalid_discord_user");
    };

    let guild_id = metadata.get("guild_id").and_then(coerce_u64);
    let discord_roles = lookup_role_strings(&app, guild_id, user_id).await;

    let service_metadata: Map<String, Value> = metadata
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "redirect_uri" | "scope" | "oauth_result"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let mut result = json!({
        "discord_id": user_id.to_string(),
        "discord_name": user.delegated_name(),
        "discord_avatar": user.avatar_url().unwrap_or_default(),
        "discord_roles": discord_roles,
        "service_metadata": service_metadata,
    });

    let scope = metadata.get("scope").and_then(Value::as_str).unwrap_or("");
    if scope.contains("connections") && !access_token.is_empty() {
        if let Some(connections) = app.inner.oauth.fetch_connections(&access_token).await {
            result["steam_connection_ids"] = json!(extract_steam_connection_ids(&connections));
        }
    }
    ok_json(result)
}

// ── Interne Routen: authorize-url + session (turnier/twitch) ─────────────────

async fn turnier_authorize_url(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_turnier(&app, &peer, &headers) {
        return resp;
    }
    authorize_url_impl(&app, &body)
}

async fn twitch_authorize_url(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_twitch(&app, &peer, &headers) {
        return resp;
    }
    authorize_url_impl(&app, &body)
}

fn authorize_url_impl(app: &DashboardApp, body: &Bytes) -> Response {
    if !app.cfg().discord_oauth_configured() {
        return err_json(503, "discord_oauth_not_configured");
    }
    let payload = match parse_obj(body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let redirect_uri = string_field(&payload, "redirect_uri").unwrap_or_default();
    if redirect_uri.is_empty() {
        return err_json(400, "missing_redirect_uri");
    }
    let scope = string_field(&payload, "scope").unwrap_or_else(|| DEFAULT_SCOPE.to_string());
    let authorize_url = app
        .inner
        .oauth
        .authorize_url_no_state(&scope, &redirect_uri);
    ok_json(json!({ "authorize_url": authorize_url }))
}

async fn turnier_session(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_turnier(&app, &peer, &headers) {
        return resp;
    }
    discord_session_impl(&app, &body).await
}

async fn twitch_session(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_twitch(&app, &peer, &headers) {
        return resp;
    }
    discord_session_impl(&app, &body).await
}

async fn discord_session_impl(app: &DashboardApp, body: &Bytes) -> Response {
    if !app.cfg().discord_oauth_configured() {
        return err_json(503, "discord_oauth_not_configured");
    }
    let payload = match parse_obj(body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let code = string_field(&payload, "code").unwrap_or_default();
    let redirect_uri = string_field(&payload, "redirect_uri").unwrap_or_default();
    let guild_id = payload.get("guild_id").and_then(coerce_u64);
    if code.is_empty() || redirect_uri.is_empty() {
        return err_json(400, "missing_code_or_redirect_uri");
    }
    let token = app.inner.oauth.exchange_code(&code, &redirect_uri).await;
    let Some(token) = token.filter(|t| !t.access_token.trim().is_empty()) else {
        return err_json(502, "discord_code_exchange_failed");
    };
    let Some(user) = app.inner.oauth.fetch_user(&token.access_token).await else {
        return err_json(502, "discord_user_lookup_failed");
    };
    let Some(user_id) = user.id_u64() else {
        return err_json(502, "invalid_discord_user_id");
    };
    let discord_roles = lookup_role_strings(app, guild_id, user_id).await;
    ok_json(json!({
        "discord_id": user_id.to_string(),
        "discord_name": user.delegated_name(),
        "discord_avatar": user.avatar_url().unwrap_or_default(),
        "discord_roles": discord_roles,
    }))
}

async fn steam_link_session(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_turnier(&app, &peer, &headers) {
        return resp;
    }
    if !app.cfg().discord_oauth_configured() {
        return err_json(503, "discord_oauth_not_configured");
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let code = string_field(&payload, "code").unwrap_or_default();
    let redirect_uri = string_field(&payload, "redirect_uri").unwrap_or_default();
    if code.is_empty() || redirect_uri.is_empty() {
        return err_json(400, "missing_code_or_redirect_uri");
    }
    let token = app.inner.oauth.exchange_code(&code, &redirect_uri).await;
    let Some(token) = token.filter(|t| !t.access_token.trim().is_empty()) else {
        return err_json(502, "discord_code_exchange_failed");
    };
    let Some(user) = app.inner.oauth.fetch_user(&token.access_token).await else {
        return err_json(502, "discord_user_lookup_failed");
    };
    let Some(connections) = app.inner.oauth.fetch_connections(&token.access_token).await else {
        return err_json(502, "discord_connections_lookup_failed");
    };
    let Some(user_id) = user.id_u64() else {
        return err_json(502, "invalid_discord_user_id");
    };
    ok_json(json!({
        "discord_id": user_id.to_string(),
        "discord_name": user.delegated_name(),
        "steam_connection_ids": extract_steam_connection_ids(&connections),
    }))
}

// ── Interne Routen: Session-Validierung/-Import (Twitch-SSO) ─────────────────

async fn validate_session(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_twitch(&app, &peer, &headers) {
        return resp;
    }
    let payload = match parse_value(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = string_field(&payload, "session_id").unwrap_or_default();
    if session_id.is_empty() {
        return err_json(400, "missing_session_id");
    }
    if !app.cfg().discord_oauth_configured() {
        return ok_json(json!({ "valid": false }));
    }
    match app.inner.sessions.touch(&session_id, now_unix_f64()).await {
        Ok(Some(session)) => ok_json(json!({
            "valid": true,
            "user_id": session.user_id.to_string(),
            "username": session.username,
            "display_name": session.display_name,
            "expires_at": session.expires_at,
        })),
        Ok(None) => ok_json(json!({ "valid": false })),
        Err(error) => {
            tracing::error!(%error, "Admin-Session konnte nicht persistiert werden");
            err_json(500, "session_persistence_failed")
        }
    }
}

async fn import_session(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_twitch(&app, &peer, &headers) {
        return resp;
    }
    let payload = match parse_value(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = string_field(&payload, "session_id").unwrap_or_default();
    let user_id = payload.get("user_id").and_then(coerce_u64);
    let username = string_field(&payload, "username").unwrap_or_default();
    let display_name = string_field(&payload, "display_name").unwrap_or_else(|| username.clone());
    let (Some(user_id), false) = (user_id, session_id.is_empty()) else {
        return err_json(400, "missing_session_id_or_user_id");
    };
    let now = now_unix_f64();
    let expires_at = payload
        .get("expires_at")
        .and_then(Value::as_f64)
        .filter(|v| *v > 0.0)
        .unwrap_or(now + app.cfg().session_ttl_secs as f64);
    match app
        .inner
        .sessions
        .import(
            &session_id,
            NewSession {
                user_id,
                username,
                display_name,
                reason: "twitch_dashboard_import".to_string(),
                access_level: AccessLevel::Full,
            },
            Some(expires_at),
            now,
        )
        .await
    {
        Ok(true) => ok_json(json!({ "ok": true })),
        Ok(false) => err_json(400, "missing_session_id_or_user_id"),
        Err(error) => {
            tracing::error!(%error, "Admin-Session konnte nicht persistiert werden");
            err_json(500, "session_persistence_failed")
        }
    }
}

async fn revoke_session(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_twitch(&app, &peer, &headers) {
        return resp;
    }
    let payload = match parse_value(&body) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let session_id = string_field(&payload, "session_id").unwrap_or_default();
    if session_id.is_empty() {
        return err_json(400, "missing_session_id");
    }
    match app.inner.sessions.remove(&session_id).await {
        Ok(()) => ok_json(json!({ "ok": true })),
        Err(error) => {
            tracing::error!(%error, "Admin-Session konnte nicht widerrufen werden");
            err_json(500, "session_persistence_failed")
        }
    }
}

// ── Interne Routen: Coaching ────────────────────────────────────────────────

async fn coaching_no_show_ban(
    State(app): State<DashboardApp>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = guard_any(&app, &peer, &headers) {
        return resp;
    }
    let payload = match parse_obj(&body) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(user_id) = payload.get("discord_user_id").and_then(coerce_u64) else {
        return err_json(400, "missing_discord_user_id");
    };
    let Ok(discord_user_id) = i64::try_from(user_id) else {
        return err_json(400, "invalid_discord_user_id");
    };
    let lookup = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, Option<String>)>(
        r#"
        SELECT expires_at, reason
          FROM coaching.bans
         WHERE discord_user_id = $1 AND expires_at > now()
         ORDER BY expires_at DESC
         LIMIT 1
        "#,
    )
    .bind(discord_user_id)
    .fetch_optional(app.pool())
    .await;

    match lookup {
        Ok(Some((expires_at, reason))) => ok_json(json!({
            "banned": true,
            "expires_at": expires_at.timestamp(),
            "reason": reason,
        })),
        Ok(None) => ok_json(json!({ "banned": false })),
        Err(err) => {
            tracing::warn!(%err, "Coaching No-Show-Ban-Lookup fehlgeschlagen");
            err_json(500, "ban_lookup_failed")
        }
    }
}

// ── Guards ──────────────────────────────────────────────────────────────────

fn guard_any(app: &DashboardApp, peer: &SocketAddr, headers: &HeaderMap) -> Result<(), Response> {
    auth::check_internal_any(
        peer.ip().is_loopback(),
        header_token(headers),
        &app.cfg().turnier_tokens,
        &app.cfg().twitch_tokens,
    )
    .map_err(reject)
}

fn guard_turnier(
    app: &DashboardApp,
    peer: &SocketAddr,
    headers: &HeaderMap,
) -> Result<(), Response> {
    auth::check_internal_specific(
        peer.ip().is_loopback(),
        header_token(headers),
        &app.cfg().turnier_tokens,
    )
    .map_err(reject)
}

fn guard_twitch(
    app: &DashboardApp,
    peer: &SocketAddr,
    headers: &HeaderMap,
) -> Result<(), Response> {
    auth::check_internal_specific(
        peer.ip().is_loopback(),
        header_token(headers),
        &app.cfg().twitch_tokens,
    )
    .map_err(reject)
}

fn reject(rejection: InternalReject) -> Response {
    err_text(rejection.status(), rejection.message())
}

fn auth_misconfigured_response() -> Response {
    err_text(503, AUTH_MISCONFIGURED_MESSAGE)
}

// ── OAuth-Ergebnis persistieren ─────────────────────────────────────────────

async fn store_oauth_result(app: &DashboardApp, state: &str, metadata: &Value, result: Value) {
    let mut payload = match metadata {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        other => {
            let mut m = Map::new();
            m.insert("initial_metadata".into(), other.clone());
            m
        }
    };
    payload.insert("oauth_result".into(), result);
    if let Err(err) = app
        .inner
        .states
        .update_metadata(state, &Value::Object(payload))
        .await
    {
        tracing::warn!(%err, "OAuth-Ergebnis konnte nicht gespeichert werden");
    }
}

async fn store_oauth_error(
    app: &DashboardApp,
    state: &str,
    metadata: &Value,
    completed_at: i64,
    error: &str,
) {
    store_oauth_result(
        app,
        state,
        metadata,
        json!({
            "provider": "discord",
            "status": "error",
            "completed_at": completed_at,
            "error": error,
        }),
    )
    .await;
}

async fn lookup_role_strings(
    app: &DashboardApp,
    guild_id: Option<u64>,
    user_id: u64,
) -> Vec<String> {
    app.inner
        .lookup
        .member_access(guild_id, user_id)
        .await
        .map(|info| info.role_ids.iter().map(u64::to_string).collect())
        .unwrap_or_default()
}

// ── kleine Helfer (Antworten, Parsing, Cookies) ─────────────────────────────

pub(crate) fn ok_json(value: Value) -> Response {
    (StatusCode::OK, Json(value)).into_response()
}

pub(crate) fn err_json(status: u16, code: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(json!({ "error": code })),
    )
        .into_response()
}

pub(crate) fn err_text(status: u16, msg: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        msg.to_string(),
    )
        .into_response()
}

/// JSON-Body, der ein Objekt sein MUSS (invalid_json / invalid_payload).
fn parse_obj(body: &Bytes) -> Result<Value, Response> {
    match serde_json::from_slice::<Value>(body) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(err_json(400, "invalid_payload")),
        Err(_) => Err(err_json(400, "invalid_json")),
    }
}

/// JSON-Body beliebigen Typs (nur invalid_json) — für validate/import, die
/// im Original keine Objekt-Prüfung machen.
fn parse_value(body: &Bytes) -> Result<Value, Response> {
    serde_json::from_slice::<Value>(body).map_err(|_| err_json(400, "invalid_json"))
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Int-Coercion wie `_coerce_int`: akzeptiert Zahl ODER numerischen String.
fn coerce_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn header_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("X-Internal-Token")
        .and_then(|v| v.to_str().ok())
}

fn request_origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `X-CSRF-Token` == Session-CSRF-Token (konstante Zeit), wie `_check_csrf`.
fn check_csrf(headers: &HeaderMap, session: &crate::session::Session) -> bool {
    let provided = headers
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match provided {
        Some(token) => constant_time_eq(token.as_bytes(), session.csrf_token.as_bytes()),
        None => false,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn read_cookies(headers: &HeaderMap, name: &str) -> Vec<String> {
    let Some(raw) = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
    else {
        return Vec::new();
    };
    let prefix = format!("{name}=");
    raw.split(';')
        .map(str::trim)
        .filter_map(|part| part.strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn session_cookie_domain(redirect_uri: &str) -> Option<String> {
    let url = url::Url::parse(redirect_uri.trim()).ok()?;
    let host = url.host_str()?.trim().to_ascii_lowercase();
    if host.is_empty() || matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return None;
    }
    Some(host)
}

fn public_dashboard_redirect_url(cfg: &DashboardConfig, candidate: &str) -> String {
    let path = auth::safe_href(candidate, "/admin");
    let path = if path.starts_with('/') && !path.starts_with("//") {
        path
    } else {
        "/admin".to_string()
    };
    let base = cfg
        .public_base_url
        .as_deref()
        .and_then(|value| url::Url::parse(value.trim()).ok())
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|url| url.as_str().trim_end_matches('/').to_string());
    match base {
        Some(base) => format!("{base}{path}"),
        None => path,
    }
}

fn build_session_cookie(value: &str, max_age: i64, secure: bool, domain: Option<&str>) -> String {
    let mut cookie =
        format!("{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if let Some(domain) = domain.filter(|value| !value.trim().is_empty()) {
        cookie.push_str("; Domain=");
        cookie.push_str(domain);
    }
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

fn clear_session_cookie(domain: Option<&str>) -> String {
    let mut cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if let Some(domain) = domain.filter(|value| !value.trim().is_empty()) {
        cookie.push_str("; Domain=");
        cookie.push_str(domain);
    }
    cookie
}

fn request_is_secure(headers: &HeaderMap, cfg: &DashboardConfig) -> bool {
    if let Some(proto) = headers
        .get("X-Forwarded-Proto")
        .and_then(|v| v.to_str().ok())
    {
        if proto.split(',').next().map(str::trim) == Some("https") {
            return true;
        }
    }
    cfg.public_base_url
        .as_deref()
        .map(|base| base.starts_with("https://"))
        .unwrap_or(false)
}

fn redirect(location: &str, set_cookie: Option<String>) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location);
    if let Some(cookie) = set_cookie {
        builder = builder.header(header::SET_COOKIE, cookie);
    }
    builder
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Delegierte Flows hängen `?state_id=<state>` an, damit der aufrufende Dienst
/// das Ergebnis abholen kann; der Admin-Login leitet schlicht weiter.
fn delegated_redirect(redirect_after: &str, is_delegated: bool, state: &str) -> Response {
    if is_delegated {
        let sep = if redirect_after.contains('?') {
            '&'
        } else {
            '?'
        };
        redirect(&format!("{redirect_after}{sep}state_id={state}"), None)
    } else {
        redirect(redirect_after, None)
    }
}

fn prune_login_states(states: &mut HashMap<String, LoginState>, now: f64) {
    states.retain(|_, s| now - s.created_at < OWN_LOGIN_STATE_TTL);
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use tower::ServiceExt;

    struct NoMemberLookup;

    #[async_trait::async_trait]
    impl MemberLookup for NoMemberLookup {
        async fn member_access(
            &self,
            _guild_id: Option<u64>,
            _user_id: u64,
        ) -> Option<crate::authority::MemberAccessInfo> {
            None
        }
    }

    struct NoNameResolver;

    #[async_trait::async_trait]
    impl NameResolver for NoNameResolver {
        async fn resolve(&self, _user_ids: &[u64]) -> HashMap<u64, String> {
            HashMap::new()
        }
    }

    async fn test_app(
        cfg: DashboardConfig,
    ) -> (tempfile::TempDir, dl_central_db::TestDb, DashboardApp) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let mut cfg = cfg;
        cfg.data_dir = dir.path().to_path_buf();
        let app = DashboardApp::new(
            cfg,
            db.pool().clone(),
            Arc::new(NoMemberLookup),
            Arc::new(NoNameResolver),
        )
        .await
        .expect("app");
        (dir, db, app)
    }

    async fn test_router(
        cfg: DashboardConfig,
    ) -> (tempfile::TempDir, dl_central_db::TestDb, Router) {
        let (dir, db, app) = test_app(cfg).await;
        (dir, db, router(app))
    }

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).expect("name"),
            value.parse().expect("value"),
        );
        h
    }

    fn internal_post(uri: &str, token: &str, body: Value) -> axum::http::Request<Body> {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("X-Internal-Token", token)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 45678))));
        request
    }

    #[test]
    fn html_escape_attr_neutralisiert_xss() {
        assert_eq!(
            html_escape_attr(r#"<script>"a"&'b'"#),
            "&lt;script&gt;&quot;a&quot;&amp;&#x27;b&#x27;"
        );
        // Ampersand zuerst, damit Entities nicht doppelt escaped werden.
        assert_eq!(html_escape_attr("a&b"), "a&amp;b");
        assert_eq!(html_escape_attr("harmlos"), "harmlos");
    }

    #[tokio::test]
    async fn misconfigured_auth_liefert_503_statt_dashboard() {
        let cfg = DashboardConfig::from_lookup(|_| None);
        let (_dir, _db, app) = test_router(cfg).await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .expect("body");
        assert_eq!(&body[..], AUTH_MISCONFIGURED_MESSAGE.as_bytes());
    }

    #[tokio::test]
    async fn security_headers_stehen_auf_jeder_antwort() {
        let cfg = DashboardConfig::from_lookup(|_| None);
        let (_dir, _db, app) = test_router(cfg).await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/auth/me")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        let headers = response.headers();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            headers
                .get(header::X_FRAME_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("DENY")
        );
        assert_eq!(
            headers
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            headers
                .get("X-XSS-Protection")
                .and_then(|v| v.to_str().ok()),
            Some("1; mode=block")
        );
        assert_eq!(
            headers
                .get(header::REFERRER_POLICY)
                .and_then(|v| v.to_str().ok()),
            Some("strict-origin-when-cross-origin")
        );
        assert_eq!(
            headers
                .get("Permissions-Policy")
                .and_then(|v| v.to_str().ok()),
            Some("geolocation=(), microphone=(), camera=(), payment=()")
        );
    }

    #[tokio::test]
    async fn coaching_no_show_ban_internal_check_liefert_aktive_sperre() {
        let cfg = DashboardConfig::from_lookup(|key| match key {
            "DISCORD_OAUTH_CLIENT_ID" => Some("id".to_string()),
            "DISCORD_OAUTH_CLIENT_SECRET" => Some("secret".to_string()),
            "TWITCH_INTERNAL_API_TOKEN" => Some("test-token".to_string()),
            _ => None,
        });
        let (_dir, db, app_state) = test_app(cfg).await;
        let user_id = 424242_i64;
        let expires_at = now_unix() + 3600;
        sqlx::query(
            r#"
            INSERT INTO coaching.bans(discord_user_id, banned_at, expires_at, reason)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(user_id)
        .bind(chrono::Utc::now())
        .bind(chrono::DateTime::from_timestamp(expires_at, 0).expect("valid expires_at"))
        .bind("no_show")
        .execute(db.pool())
        .await
        .expect("ban fixture");

        let app = router(app_state);
        let response = app
            .oneshot(internal_post(
                "/internal/coaching/v1/no-show-ban",
                "test-token",
                json!({ "discord_user_id": user_id }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 2048)
            .await
            .expect("body");
        let data: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(data["banned"], json!(true));
        assert_eq!(data["expires_at"], json!(expires_at));
        assert_eq!(data["reason"], json!("no_show"));
    }

    #[tokio::test]
    async fn coaching_no_show_ban_internal_check_ignoriert_abgelaufene_sperre() {
        let cfg = DashboardConfig::from_lookup(|key| match key {
            "DISCORD_OAUTH_CLIENT_ID" => Some("id".to_string()),
            "DISCORD_OAUTH_CLIENT_SECRET" => Some("secret".to_string()),
            "TWITCH_INTERNAL_API_TOKEN" => Some("test-token".to_string()),
            _ => None,
        });
        let (_dir, db, app_state) = test_app(cfg).await;
        let user_id = 515151_i64;
        sqlx::query(
            r#"
            INSERT INTO coaching.bans(discord_user_id, banned_at, expires_at, reason)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(user_id)
        .bind(chrono::DateTime::from_timestamp(now_unix() - 7200, 0).expect("valid banned_at"))
        .bind(chrono::DateTime::from_timestamp(now_unix() - 3600, 0).expect("valid expires_at"))
        .bind("old")
        .execute(db.pool())
        .await
        .expect("ban fixture");

        let response = router(app_state)
            .oneshot(internal_post(
                "/internal/coaching/v1/no-show-ban",
                "test-token",
                json!({ "discord_user_id": user_id }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 2048)
            .await
            .expect("body");
        let data: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(data, json!({ "banned": false }));
    }

    #[test]
    fn cookie_lesen_findet_richtigen_namen() {
        let h = headers_with("cookie", "a=1; master_dash_session=tok123 ; b=2");
        assert_eq!(read_cookies(&h, SESSION_COOKIE), vec!["tok123"]);
        assert!(read_cookies(&h, "fehlt").is_empty());
        // Leerer Wert zählt als nicht vorhanden.
        let empty = headers_with("cookie", "master_dash_session=");
        assert!(read_cookies(&empty, SESSION_COOKIE).is_empty());
        let duplicate = headers_with(
            "cookie",
            "master_dash_session=alt; master_dash_session=gueltig",
        );
        assert_eq!(
            read_cookies(&duplicate, SESSION_COOKIE),
            vec!["alt".to_string(), "gueltig".to_string()]
        );
    }

    #[tokio::test]
    async fn revoke_session_entfernt_gemeinsame_session() {
        let cfg = DashboardConfig::from_lookup(|key| match key {
            "DISCORD_OAUTH_CLIENT_ID" => Some("id".to_string()),
            "DISCORD_OAUTH_CLIENT_SECRET" => Some("secret".to_string()),
            "TWITCH_INTERNAL_API_TOKEN" => Some("twitch-test-token".to_string()),
            _ => None,
        });
        let (_dir, _db, app) = test_app(cfg).await;
        let session_id = app
            .inner
            .sessions
            .create(
                NewSession {
                    user_id: 42,
                    username: "admin".to_string(),
                    display_name: "Admin".to_string(),
                    reason: "test".to_string(),
                    access_level: AccessLevel::Full,
                },
                now_unix_f64(),
            )
            .await
            .expect("session");

        let response = router(app.clone())
            .oneshot(internal_post(
                "/internal/twitch/v1/discord/revoke-session",
                "twitch-test-token",
                json!({ "session_id": session_id }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(app
            .inner
            .sessions
            .touch(&session_id, now_unix_f64())
            .await
            .expect("lookup")
            .is_none());
    }

    #[tokio::test]
    async fn read_guard_akzeptiert_gueltige_non_full_session() {
        let cfg = DashboardConfig::from_lookup(|key| match key {
            "DISCORD_OAUTH_CLIENT_ID" => Some("id".to_string()),
            "DISCORD_OAUTH_CLIENT_SECRET" => Some("secret".to_string()),
            _ => None,
        });
        let (_dir, _db, app) = test_app(cfg).await;
        let session_id = app
            .inner
            .sessions
            .create(
                NewSession {
                    user_id: 42,
                    username: "nani".to_string(),
                    display_name: "Nani".to_string(),
                    reason: "turnier_only".to_string(),
                    access_level: AccessLevel::TurnierOnly,
                },
                now_unix_f64(),
            )
            .await
            .expect("create");
        let headers = headers_with("cookie", &format!("{SESSION_COOKIE}={session_id}"));

        assert!(app.guard_read(&headers).await.is_ok());
        assert!(app.guard_full(&headers).await.is_err());
    }

    #[test]
    fn cookie_bauen_und_loeschen() {
        let secure = build_session_cookie("v", 100, true, Some("deutsche-deadlock-community.de"));
        assert!(secure
            .starts_with("master_dash_session=v; Path=/; HttpOnly; SameSite=Lax; Max-Age=100"));
        assert!(secure.ends_with("; Secure"));
        assert!(secure.contains("; Domain=deutsche-deadlock-community.de"));
        let insecure = build_session_cookie("v", 100, false, None);
        assert!(!insecure.contains("Secure"));
        assert!(!insecure.contains("Domain="));
        let cleared = clear_session_cookie(Some("deutsche-deadlock-community.de"));
        assert!(cleared.contains("Max-Age=0"));
        assert!(cleared.contains("; Domain=deutsche-deadlock-community.de"));
    }

    #[test]
    fn session_cookie_domain_folgt_python_redirect_domain() {
        assert_eq!(
            session_cookie_domain("https://deutsche-deadlock-community.de/callback/discord"),
            Some("deutsche-deadlock-community.de".to_string())
        );
        assert_eq!(
            session_cookie_domain("http://localhost:8766/callback/discord"),
            None
        );
        assert_eq!(session_cookie_domain("not-a-url"), None);
    }

    #[test]
    fn public_dashboard_redirect_nutzt_konfigurierte_admin_domain() {
        let cfg = DashboardConfig::from_lookup(|key| match key {
            "MASTER_DASHBOARD_PUBLIC_URL" => {
                Some("https://admin.deutsche-deadlock-community.de".to_string())
            }
            _ => None,
        });
        assert_eq!(
            public_dashboard_redirect_url(&cfg, "/admin"),
            "https://admin.deutsche-deadlock-community.de/admin"
        );
        assert_eq!(
            public_dashboard_redirect_url(&cfg, "https://evil.example/path"),
            "https://admin.deutsche-deadlock-community.de/admin"
        );
    }

    #[test]
    fn coerce_u64_zahl_oder_string() {
        assert_eq!(coerce_u64(&json!(42)), Some(42));
        assert_eq!(coerce_u64(&json!("123")), Some(123));
        assert_eq!(coerce_u64(&json!(" 7 ")), Some(7));
        assert_eq!(coerce_u64(&json!(-1)), None);
        assert_eq!(coerce_u64(&json!("abc")), None);
        assert_eq!(coerce_u64(&Value::Null), None);
    }

    #[test]
    fn secure_aus_forwarded_proto_oder_public_base() {
        let cfg = DashboardConfig::from_lookup(|_| None);
        assert!(request_is_secure(
            &headers_with("x-forwarded-proto", "https"),
            &cfg
        ));
        assert!(request_is_secure(
            &headers_with("x-forwarded-proto", "https, http"),
            &cfg
        ));
        assert!(request_is_secure(&HeaderMap::new(), &cfg));
    }

    #[test]
    fn delegierter_redirect_haengt_state_an() {
        // Ohne Query-Teil → ?state_id=…
        let r = delegated_redirect("https://x/cb", true, "S1");
        let loc = r
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(loc, "https://x/cb?state_id=S1");
        // Mit Query-Teil → &state_id=…
        let r = delegated_redirect("https://x/cb?a=1", true, "S2");
        let loc = r
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(loc, "https://x/cb?a=1&state_id=S2");
        // Nicht-delegiert → schlichte Weiterleitung.
        let r = delegated_redirect("/admin", false, "S3");
        let loc = r
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(loc, "/admin");
    }
}
