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
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, options, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};

use crate::auth::{self, InternalReject};
use crate::authority::{decide_access, BrokerMemberLookup, MemberLookup};
use crate::config::{AccessLevel, DashboardConfig};
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

// ── App-Zustand ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DashboardApp {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: DashboardConfig,
    db: dl_db::Db,
    oauth: OAuthClient,
    states: OAuthStateStore,
    sessions: SessionStore,
    lookup: Arc<dyn MemberLookup>,
    names: Arc<dyn NameResolver>,
    /// In-Memory-States des Admin-Logins (≠ DB-gestützte delegierte States).
    login_states: Mutex<HashMap<String, LoginState>>,
    /// Per-IP-Zeitstempel für die Callback-Ratenbegrenzung.
    rate: Mutex<HashMap<String, Vec<f64>>>,
}

struct LoginState {
    next_path: String,
    redirect_uri: String,
    created_at: f64,
}

impl DashboardApp {
    /// Baut die App mit explizitem Mitglieds-Lookup und Namens-Resolver
    /// (für Tests).
    pub fn new(
        cfg: DashboardConfig,
        db: dl_db::Db,
        lookup: Arc<dyn MemberLookup>,
        names: Arc<dyn NameResolver>,
    ) -> Self {
        let oauth = OAuthClient::new(
            cfg.discord_client_id.clone(),
            cfg.discord_client_secret.clone(),
            cfg.discord_api_base.clone(),
        );
        let states = OAuthStateStore::new(db.clone(), cfg.oauth_state_ttl_secs);
        let sessions = SessionStore::new(cfg.session_ttl_secs);
        Self {
            inner: Arc::new(Inner {
                cfg,
                db,
                oauth,
                states,
                sessions,
                lookup,
                names,
                login_states: Mutex::new(HashMap::new()),
                rate: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Produktions-Konstruktor: Lookup und Namensauflösung gehen über den
    /// Master-Broker.
    pub fn from_config(cfg: DashboardConfig, db: dl_db::Db) -> Self {
        let lookup = Arc::new(BrokerMemberLookup::new(cfg.broker_base.clone()));
        let names = Arc::new(BrokerNameResolver::new(cfg.broker_base.clone()));
        Self::new(cfg, db, lookup, names)
    }

    fn cfg(&self) -> &DashboardConfig {
        &self.inner.cfg
    }

    pub(crate) fn db(&self) -> &dl_db::Db {
        &self.inner.db
    }

    pub(crate) fn names(&self) -> &Arc<dyn NameResolver> {
        &self.inner.names
    }

    /// Auth-Gate für lesende `/api`-Routen: ohne erzwungene Auth offen, sonst
    /// gültige Session nötig (wie `_check_auth` ohne CSRF/Full-Access).
    pub(crate) fn guard_read(&self, headers: &HeaderMap) -> Result<(), Response> {
        if !self.cfg().auth_enforced() {
            return Ok(());
        }
        if self.session_from_headers(headers).is_some() {
            Ok(())
        } else {
            Err(err_text(401, "Authentication required"))
        }
    }

    /// Gate für Routen mit `required=True, require_full_access=True`: immer
    /// erzwungen, gültige Session mit Voll-Zugriff nötig (401/403 wie Original).
    pub(crate) fn guard_full(&self, headers: &HeaderMap) -> Result<(), Response> {
        match self.session_from_headers(headers) {
            Some(session) if session.has_full_access() => Ok(()),
            Some(_) => Err(err_text(403, "Full dashboard access required")),
            None => Err(err_text(401, "Authentication required")),
        }
    }

    /// Gate für mutierende Routen (POST/PUT/PATCH/DELETE): Session + Origin-
    /// Prüfung + CSRF-Token, dann ggf. Voll-Zugriff. Reihenfolge/Status wie
    /// `_check_auth` (401 Session, 403 Origin/CSRF/Full). Gibt die Session
    /// für das Audit-Log zurück.
    pub(crate) fn guard_mutate(
        &self,
        headers: &HeaderMap,
        require_full: bool,
    ) -> Result<crate::session::Session, Response> {
        let Some(session) = self.session_from_headers(headers) else {
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

    /// `_is_allowed_request_origin`: ohne konfigurierte Origins offen (CSRF-
    /// Token bleibt die eigentliche Absicherung), sonst muss der Origin-Header
    /// in der Liste stehen.
    fn allowed_origin(&self, headers: &HeaderMap) -> bool {
        let allowed = &self.cfg().allowed_origins;
        if allowed.is_empty() {
            return true;
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
    fn session_from_headers(&self, headers: &HeaderMap) -> Option<crate::session::Session> {
        if !self.cfg().auth_enforced() {
            return None;
        }
        let session_id = read_cookie(headers, SESSION_COOKIE)?;
        self.inner.sessions.touch(&session_id, now_unix_f64())
    }
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn router(app: DashboardApp) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/admin", get(index))
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
        // Analytics-Reads (Phase 9b) — Session-gegatet, reine DB-Reads.
        .route("/api/member-events", get(crate::analytics::member_events))
        .route(
            "/api/message-activity",
            get(crate::analytics::message_activity),
        )
        .route("/api/voice-history", get(crate::analytics::voice_history))
        .route("/api/leave-surveys", get(crate::analytics::leave_surveys))
        .route(
            "/api/co-player-network",
            get(crate::analytics::co_player_network),
        )
        .route(
            "/api/co-player-network/",
            get(crate::analytics::co_player_network),
        )
        // Deadlock-Konfiguration (Phase 9c) — Read-Seite, Full-Access.
        .route(
            "/api/deadlock/config",
            get(crate::deadlock::deadlock_config).post(crate::deadlock::deadlock_config_update),
        )
        .route(
            "/api/deadlock/heroes",
            get(crate::deadlock::deadlock_heroes),
        )
        // Öffentlicher Austritts-Umfrage-Flow (Phase 9e) — token-basiert.
        .route(
            "/api/leave-survey/{token}",
            get(crate::survey::leave_survey_get),
        )
        // Öffentliche Endpunkte (Phase 9e) — kein Auth, CORS für die Website.
        .route(
            "/api/public/patch-notes",
            get(crate::public::patch_notes).options(crate::public::public_cors),
        )
        .route(
            "/api/public/guild-stats",
            options(crate::public::public_cors),
        )
        .with_state(app)
}

// ── Browser-Routen (Login/Callback/Logout) ──────────────────────────────────

async fn index() -> Response {
    // Die SPA (admin_dashboard/dist) wird beim Cutover über den Binär-Prozess
    // statisch ausgeliefert; bis dahin ist dies ein Platzhalter. Der
    // Auth-Provider funktioniert unabhängig davon (interne Endpunkte).
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><title>Deadlock Dashboard</title><p>Dashboard (Rust). Auth-Provider aktiv.</p>",
    )
        .into_response()
}

async fn auth_me(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if !app.cfg().auth_enforced() {
        return ok_json(json!({
            "enabled": false,
            "authenticated": false,
            "mode": "none",
        }));
    }
    match app.session_from_headers(&headers) {
        Some(session) => ok_json(json!({
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
        None => err_text(401, "Authentication required"),
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
    if !app.cfg().auth_enforced() {
        return redirect("/admin", None);
    }
    let next_path = auth::safe_href(
        params.get("next").map(String::as_str).unwrap_or(""),
        "/admin",
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
    if !app.cfg().auth_enforced() {
        return err_text(503, "Discord OAuth ist nicht konfiguriert.");
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

    let session_id = app.inner.sessions.create(
        NewSession {
            user_id,
            username: user.username.clone(),
            display_name: user.display_name(),
            reason: outcome.reason.to_string(),
            access_level: level,
        },
        now,
    );
    let cookie = build_session_cookie(
        &session_id,
        app.cfg().session_ttl_secs,
        request_is_secure(headers, app.cfg()),
    );
    redirect(
        &auth::safe_href(&login_state.next_path, "/admin"),
        Some(cookie),
    )
}

async fn logout(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Some(session_id) = read_cookie(&headers, SESSION_COOKIE) {
        app.inner.sessions.remove(&session_id);
    }
    let target = if app.cfg().auth_enforced() {
        ADMIN_LOGIN_URL
    } else {
        "/admin"
    };
    redirect(
        &auth::safe_href(target, "/admin"),
        Some(clear_session_cookie()),
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
    if !app.cfg().auth_enforced() {
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
    if !app.cfg().auth_enforced() {
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
    if !app.cfg().auth_enforced() {
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
    if !app.cfg().auth_enforced() {
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
    if !app.cfg().auth_enforced() {
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
    if !app.cfg().auth_enforced() {
        return ok_json(json!({ "valid": false }));
    }
    match app.inner.sessions.touch(&session_id, now_unix_f64()) {
        Some(session) => ok_json(json!({
            "valid": true,
            "user_id": session.user_id.to_string(),
            "username": session.username,
            "display_name": session.display_name,
            "expires_at": session.expires_at,
        })),
        None => ok_json(json!({ "valid": false })),
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
    app.inner.sessions.import(
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
    );
    ok_json(json!({ "ok": true }))
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

fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{name}=");
    raw.split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn build_session_cookie(value: &str, max_age: i64, secure: bool) -> String {
    let mut cookie =
        format!("{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).expect("name"),
            value.parse().expect("value"),
        );
        h
    }

    #[test]
    fn cookie_lesen_findet_richtigen_namen() {
        let h = headers_with("cookie", "a=1; master_dash_session=tok123 ; b=2");
        assert_eq!(read_cookie(&h, SESSION_COOKIE).as_deref(), Some("tok123"));
        assert!(read_cookie(&h, "fehlt").is_none());
        // Leerer Wert zählt als nicht vorhanden.
        let empty = headers_with("cookie", "master_dash_session=");
        assert!(read_cookie(&empty, SESSION_COOKIE).is_none());
    }

    #[test]
    fn cookie_bauen_und_loeschen() {
        let secure = build_session_cookie("v", 100, true);
        assert!(secure
            .starts_with("master_dash_session=v; Path=/; HttpOnly; SameSite=Lax; Max-Age=100"));
        assert!(secure.ends_with("; Secure"));
        let insecure = build_session_cookie("v", 100, false);
        assert!(!insecure.contains("Secure"));
        assert!(clear_session_cookie().contains("Max-Age=0"));
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
        assert!(!request_is_secure(&HeaderMap::new(), &cfg));
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
