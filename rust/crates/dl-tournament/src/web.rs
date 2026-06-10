//! Turnier-Website (:8767) — Port von `service/turnier_public.py`.
//!
//! 12 Routen wie das Original: Index-HTML, OAuth-Relay
//! (`/auth/login` → Link-Dienst, `/auth/complete` löst den Einmal-Token aus
//! `turnier_auth_tokens` ein → Session-Cookie), Overview mit Bracket,
//! Me/Signup/Withdraw/Team-Verwaltung mit CSRF-Header-Schutz.
//!
//! Architektur-Hinweis: Das Original lief IM Bot-Prozess und prüfte die
//! Turnier-Rolle über den Gateway-Cache — wenn Guild/Member dort fehlten,
//! wurde NICHT geprüft. dl-web ist ein eigener Prozess ohne Gateway; der
//! Rollen-Check läuft über [`RoleCheck`] (Default: nicht prüfbar →
//! durchlassen, exakt der Original-Fallback). Eine strikte Prüfung kann
//! später über die interne Bot-API nachgerüstet werden.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::store::{Signup, TeamRow, TournamentStore};

pub const SESSION_COOKIE: &str = "turnier_pub_session";
pub const SESSION_TTL_SECONDS: f64 = 6.0 * 3600.0;
pub const SESSION_TOKEN_HEX_LEN: usize = 64;
pub const TURNIER_ROLE_ID: u64 = 1474210107255554331;
pub const DEFAULT_HTML_PATH: &str = "service/static/turnier_public.html";

// ── Bracket-Generator (Referenz-JSON aus CPython im Test) ──────────────────

fn rank_display(rank: &str, subrank: i64) -> String {
    let label = if rank.is_empty() {
        "Unbekannt".to_string()
    } else {
        let key = crate::store::normalize_rank(rank);
        let mut chars = key.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => "Unbekannt".to_string(),
        }
    };
    if subrank > 0 {
        format!("{label} {subrank}")
    } else {
        label
    }
}

/// Wie `_generate_bracket`: Scores (Team = Ø, gerundet auf 2), Seeding
/// 1-gegen-letzter, Byes mit Auto-Winner, Runden-Labels.
pub fn generate_bracket(signups: &[Signup], teams: &[TeamRow]) -> Value {
    let team_map: HashMap<i64, &TeamRow> = teams.iter().map(|t| (t.id, t)).collect();
    let mut team_scores: Vec<(i64, Vec<i64>)> = Vec::new();
    let mut solo_entries: Vec<Value> = Vec::new();

    for signup in signups {
        let rank_value = signup.rank_value.max(1);
        // Original: `or 3` — Subrank 0/None fällt auf die Mitte zurück
        let subrank = if signup.rank_subvalue > 0 {
            signup.rank_subvalue
        } else {
            3
        };
        let score = rank_value * 6 + subrank.clamp(1, 6);
        match signup.team_id {
            Some(team_id) => match team_scores.iter_mut().find(|(id, _)| *id == team_id) {
                Some((_, scores)) => scores.push(score),
                None => team_scores.push((team_id, vec![score])),
            },
            None => {
                let name = signup
                    .display_name
                    .clone()
                    .unwrap_or_else(|| format!("User {}", signup.user_id));
                solo_entries.push(json!({
                    "type": "solo",
                    "id": format!("solo_{}", signup.user_id),
                    "name": name,
                    "score": score,
                }));
            }
        }
    }

    let mut entries: Vec<Value> = Vec::new();
    for (team_id, scores) in &team_scores {
        let team_name = team_map
            .get(team_id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| format!("Team {team_id}"));
        let avg = scores.iter().sum::<i64>() as f64 / scores.len() as f64;
        entries.push(json!({
            "type": "team",
            "id": format!("team_{team_id}"),
            "name": team_name,
            "score": (avg * 100.0).round() / 100.0,
            "member_count": scores.len(),
        }));
    }
    entries.extend(solo_entries);

    if entries.len() < 2 {
        return json!({
            "error": "Mindestens 2 Einträge für einen Bracket benötigt.",
            "entries": entries,
            "rounds": [],
        });
    }

    entries.sort_by(|a, b| {
        let score_a = a["score"].as_f64().unwrap_or(0.0);
        let score_b = b["score"].as_f64().unwrap_or(0.0);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let n = entries.len();
    let slots: usize = 1 << (n as f64).log2().ceil() as u32;
    let num_rounds = (slots as f64).log2() as usize;

    let round_label = |round: usize| -> String {
        match num_rounds - round {
            0 => "🏆 Finale".to_string(),
            1 => "🥊 Halbfinale".to_string(),
            2 => "⚔️ Viertelfinale".to_string(),
            _ => format!("Runde {round}"),
        }
    };

    let mut rounds: Vec<Value> = Vec::new();
    let mut current: Vec<Value> = Vec::new();
    for i in 0..slots / 2 {
        let entry_a = entries.get(i).cloned();
        let entry_b = entries.get(slots - 1 - i).cloned();
        let auto_winner = match (&entry_a, &entry_b) {
            (None, Some(b)) => Some(b.clone()),
            (Some(a), None) => Some(a.clone()),
            _ => None,
        };
        let is_bye = entry_a.is_none() || entry_b.is_none();
        current.push(json!({
            "match_id": format!("R1M{}", i + 1),
            "entry_a": entry_a,
            "entry_b": entry_b,
            "winner": auto_winner,
            "is_bye": is_bye,
        }));
    }
    rounds.push(json!({ "round": 1, "label": round_label(1), "matches": current.clone() }));

    for round in 2..=num_rounds {
        let next: Vec<Value> = (0..current.len() / 2)
            .map(|i| {
                json!({
                    "match_id": format!("R{round}M{}", i + 1),
                    "entry_a": null,
                    "entry_b": null,
                    "winner": null,
                    "is_bye": false,
                })
            })
            .collect();
        rounds.push(json!({ "round": round, "label": round_label(round), "matches": next }));
        current = rounds
            .last()
            .and_then(|r| r["matches"].as_array().cloned())
            .unwrap_or_default();
    }

    json!({
        "entries": entries,
        "rounds": rounds,
        "num_entries": n,
        "num_rounds": num_rounds,
        "slots": slots,
    })
}

// ── Sessions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct WebSession {
    user_id: u64,
    display_name: String,
    csrf_token: String,
    expires_at: f64,
}

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Turnier-Rollen-Prüfung — None = nicht prüfbar (Original-Fallback: durchlassen).
#[async_trait::async_trait]
pub trait RoleCheck: Send + Sync {
    async fn has_tournament_role(&self, _user_id: u64) -> Option<bool> {
        None
    }
}

pub struct NoRoleCheck;
#[async_trait::async_trait]
impl RoleCheck for NoRoleCheck {}

pub struct TurnierWeb {
    pub store: TournamentStore,
    pub guild_id: u64,
    pub steam_link_base_url: String,
    pub html_path: std::path::PathBuf,
    pub role_check: Arc<dyn RoleCheck>,
    sessions: tokio::sync::Mutex<HashMap<String, WebSession>>,
}

impl TurnierWeb {
    pub fn new(
        store: TournamentStore,
        guild_id: u64,
        steam_link_base_url: impl Into<String>,
        html_path: impl Into<std::path::PathBuf>,
        role_check: Arc<dyn RoleCheck>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            guild_id,
            steam_link_base_url: steam_link_base_url.into().trim_end_matches('/').to_string(),
            html_path: html_path.into(),
            role_check,
            sessions: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// ENV wie das Original (TURNIER_PUBLIC_GUILD_ID, PUBLIC_BASE_URL).
    pub fn from_env(store: TournamentStore, lookup: impl Fn(&str) -> Option<String>) -> Arc<Self> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Self::new(
            store,
            get("TURNIER_PUBLIC_GUILD_ID")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            get("PUBLIC_BASE_URL")
                .unwrap_or_else(|| "https://deutsche-deadlock-community.de/link".to_string()),
            get("TURNIER_PUBLIC_HTML_PATH").unwrap_or_else(|| DEFAULT_HTML_PATH.to_string()),
            Arc::new(NoRoleCheck),
        )
    }

    async fn create_session(&self, user_id: u64, display_name: String) -> (String, String) {
        let mut sessions = self.sessions.lock().await;
        let now = now_unix();
        sessions.retain(|_, s| s.expires_at >= now);
        let session_token = hex::encode(rand::random::<[u8; 32]>());
        let csrf_token = hex::encode(rand::random::<[u8; 16]>());
        sessions.insert(
            session_token.clone(),
            WebSession {
                user_id,
                display_name,
                csrf_token: csrf_token.clone(),
                expires_at: now + SESSION_TTL_SECONDS,
            },
        );
        (session_token, csrf_token)
    }

    /// Cookie validieren (64 Hex-Zeichen) + Sliding-TTL.
    async fn session_from_headers(&self, headers: &HeaderMap) -> Option<WebSession> {
        let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
        let token = cookie_header
            .split(';')
            .filter_map(|part| part.trim().split_once('='))
            .find(|(name, _)| *name == SESSION_COOKIE)
            .map(|(_, value)| value.trim().to_string())?;
        if token.len() != SESSION_TOKEN_HEX_LEN
            || !token
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return None;
        }
        let mut sessions = self.sessions.lock().await;
        let now = now_unix();
        let session = sessions.get_mut(&token)?;
        if session.expires_at < now {
            sessions.remove(&token);
            return None;
        }
        session.expires_at = now + SESSION_TTL_SECONDS;
        Some(session.clone())
    }

    fn check_csrf(&self, headers: &HeaderMap, session: &WebSession) -> bool {
        let header_token = headers
            .get("X-CSRF-Token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim();
        // Längen-Gleichheit + byteweiser Vergleich ohne Frühabbruch
        let expected = session.csrf_token.as_bytes();
        let provided = header_token.as_bytes();
        if provided.is_empty() || expected.is_empty() || provided.len() != expected.len() {
            return false;
        }
        provided
            .iter()
            .zip(expected)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

// ── Router + Handler ───────────────────────────────────────────────────────

pub fn router(web: Arc<TurnierWeb>) -> Router {
    Router::new()
        .route("/", get(handle_index))
        .route("/auth/login", get(handle_auth_login))
        .route("/auth/complete", get(handle_auth_complete))
        .route("/auth/logout", get(handle_auth_logout))
        .route("/api/overview", get(handle_overview))
        .route("/api/me", get(handle_me))
        .route("/api/signup", post(handle_signup))
        .route("/api/withdraw", post(handle_withdraw))
        .route("/api/team/create", post(handle_team_create))
        .route("/api/team/rename", post(handle_team_rename))
        .route("/api/team/kick", post(handle_team_kick))
        .route("/health", get(handle_health))
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(web)
}

async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().expect("static"));
    headers.insert("X-Content-Type-Options", "nosniff".parse().expect("static"));
    headers.insert(
        header::REFERRER_POLICY,
        "strict-origin-when-cross-origin".parse().expect("static"),
    );
    headers.insert("X-Frame-Options", "DENY".parse().expect("static"));
    headers.insert(
        "Permissions-Policy",
        "geolocation=(), microphone=(), camera=()"
            .parse()
            .expect("static"),
    );
    response
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn handle_health() -> Json<Value> {
    Json(json!({ "ok": true, "ts": now_unix() as i64 }))
}

async fn handle_index(State(web): State<Arc<TurnierWeb>>) -> Response {
    match std::fs::read_to_string(&web.html_path) {
        Ok(html) => axum::response::Html(html).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Seite nicht verfügbar.").into_response(),
    }
}

async fn handle_auth_login(State(web): State<Arc<TurnierWeb>>) -> Redirect {
    Redirect::to(&format!(
        "{}/discord/login?context=turnier",
        web.steam_link_base_url
    ))
}

async fn handle_auth_complete(
    State(web): State<Arc<TurnierWeb>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let token = params.get("token").map(|t| t.trim()).unwrap_or("");
    if token.is_empty() || token.len() > 256 || token.chars().any(char::is_whitespace) {
        return Redirect::to("/").into_response();
    }
    let Some((user_id, display_name)) = web.store.consume_auth_token(token).await else {
        return Redirect::to("/").into_response();
    };
    let (session_token, _csrf) = web.create_session(user_id, display_name).await;
    let cookie = format!(
        "{SESSION_COOKIE}={session_token}; HttpOnly; SameSite=Lax; Max-Age={}; Secure; Path=/",
        SESSION_TTL_SECONDS as i64
    );
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

async fn handle_auth_logout(State(web): State<Arc<TurnierWeb>>, headers: HeaderMap) -> Response {
    if let Some(session) = web.session_from_headers(&headers).await {
        let mut sessions = web.sessions.lock().await;
        sessions.retain(|_, s| s.csrf_token != session.csrf_token);
    }
    let cookie = format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Max-Age=0; Secure; Path=/");
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

async fn handle_overview(State(web): State<Arc<TurnierWeb>>) -> Response {
    if web.guild_id == 0 {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "guild not configured");
    }
    let teams = web.store.list_teams(web.guild_id).await;
    let signups = web.store.list_signups(web.guild_id).await;
    let period = web.store.active_period_json(web.guild_id).await;

    let mut members_by_team: HashMap<i64, Vec<Value>> = HashMap::new();
    let mut solo_signups: Vec<Value> = Vec::new();
    for signup in &signups {
        let entry = json!({
            "user_id": signup.user_id,
            "display_name": signup
                .display_name
                .clone()
                .unwrap_or_else(|| format!("User {}", signup.user_id)),
            "rank": signup.rank,
            "rank_label": rank_display(&signup.rank, signup.rank_subvalue),
            "rank_subvalue": signup.rank_subvalue,
        });
        match signup.team_id {
            Some(team_id) => members_by_team.entry(team_id).or_default().push(entry),
            None => solo_signups.push(entry),
        }
    }
    let teams_out: Vec<Value> = teams
        .iter()
        .map(|team| {
            json!({
                "id": team.id,
                "name": team.name,
                "created_by": team.created_by,
                "member_count": team.member_count,
                "members": members_by_team.get(&team.id).cloned().unwrap_or_default(),
            })
        })
        .collect();
    let bracket = generate_bracket(&signups, &teams);
    Json(json!({
        "guild_id": web.guild_id,
        "active_period": period,
        "teams": teams_out,
        "solo_signups": solo_signups,
        "bracket": bracket,
    }))
    .into_response()
}

async fn handle_me(State(web): State<Arc<TurnierWeb>>, headers: HeaderMap) -> Response {
    let Some(session) = web.session_from_headers(&headers).await else {
        return Json(json!({ "logged_in": false, "user": null })).into_response();
    };
    let signup = if web.guild_id > 0 {
        web.store.get_signup(web.guild_id, session.user_id).await
    } else {
        None
    };
    Json(json!({
        "logged_in": true,
        "user": {
            "id": session.user_id,
            "display_name": session.display_name,
            "csrf_token": session.csrf_token,
        },
        "signup": signup.map(signup_json),
    }))
    .into_response()
}

fn signup_json(signup: Signup) -> Value {
    json!({
        "user_id": signup.user_id,
        "registration_mode": signup.registration_mode,
        "rank": signup.rank,
        "rank_value": signup.rank_value,
        "rank_subvalue": signup.rank_subvalue,
        "display_name": signup.display_name,
        "team_id": signup.team_id,
        "team_name": signup.team_name,
        "assigned_by_admin": signup.assigned_by_admin,
        "status": if signup.status.is_empty() { Value::Null } else { json!(signup.status) },
    })
}

/// Auth + CSRF + Guild für die POST-Routen.
async fn authorize(web: &Arc<TurnierWeb>, headers: &HeaderMap) -> Result<WebSession, Response> {
    let Some(session) = web.session_from_headers(headers).await else {
        return Err(json_error(StatusCode::UNAUTHORIZED, "not logged in"));
    };
    if !web.check_csrf(headers, &session) {
        return Err(json_error(StatusCode::FORBIDDEN, "invalid csrf"));
    }
    if web.guild_id == 0 {
        return Err(json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "guild not configured",
        ));
    }
    Ok(session)
}

/// fromisoformat-Äquivalent (Datum oder Datum+Zeit, lokale Zeit).
fn parse_period_dt(raw: &str) -> Option<chrono::NaiveDateTime> {
    let raw = raw.trim();
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })
}

async fn handle_signup(
    State(web): State<Arc<TurnierWeb>>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let session = match authorize(&web, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };

    // Anmeldezeitraum offen?
    let Some(period_id) = web
        .store
        .active_period(web.guild_id)
        .await
        .map(|(id, _, _)| id)
    else {
        return json_error(StatusCode::BAD_REQUEST, "Kein aktiver Anmeldezeitraum.");
    };
    let window = web.store.period_window(web.guild_id, period_id).await;
    let Some((start_raw, end_raw)) = window else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Zeitraum-Fehler.");
    };
    let now = chrono::Local::now().naive_local();
    match (parse_period_dt(&start_raw), parse_period_dt(&end_raw)) {
        (Some(start), Some(end)) if start <= now && now <= end => {}
        (Some(_), Some(_)) => {
            return json_error(StatusCode::BAD_REQUEST, "Anmeldezeitraum ist nicht offen.")
        }
        _ => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Zeitraum-Fehler."),
    }

    // Turnier-Rolle (nur wenn prüfbar — Original-Fallback)
    if web.role_check.has_tournament_role(session.user_id).await == Some(false) {
        return json_error(
            StatusCode::FORBIDDEN,
            &format!("Du benötigst die Turnier-Rolle ({TURNIER_ROLE_ID})."),
        );
    }

    // Verifizierter Steam-Link + Rang
    let Some((rank_name, rank_sub)) = web.store.verified_steam_rank(session.user_id).await else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "Kein verifizierter Steam-Account verknüpft. Nutze /account_verknüpfen.",
        );
    };

    let body = body.map(|Json(value)| value).unwrap_or(Value::Null);
    let mode = match body.get("mode").and_then(Value::as_str) {
        Some("team") => "team",
        _ => "solo",
    };
    let mut team_id: Option<i64> = None;
    if mode == "team" {
        match body.get("team_id") {
            Some(raw) if !raw.is_null() => {
                let Some(parsed) = raw
                    .as_i64()
                    .or_else(|| raw.as_str().and_then(|s| s.parse().ok()))
                else {
                    return json_error(StatusCode::BAD_REQUEST, "Ungültige team_id.");
                };
                team_id = Some(parsed);
            }
            _ => {
                let team_name = body
                    .get("team_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if team_name.is_empty() {
                    return json_error(
                        StatusCode::BAD_REQUEST,
                        "team_name fehlt für Team-Anmeldung.",
                    );
                }
                match web
                    .store
                    .get_or_create_team(web.guild_id, team_name, Some(session.user_id))
                    .await
                {
                    Ok(team) => team_id = Some(team.id),
                    Err(message) => return json_error(StatusCode::BAD_REQUEST, &message),
                }
            }
        }
    }

    match web
        .store
        .upsert_signup(
            web.guild_id,
            session.user_id,
            mode,
            &rank_name.to_lowercase(),
            rank_sub,
            team_id,
            false,
            Some(session.display_name.clone()),
        )
        .await
    {
        Ok(signup) => Json(json!({ "ok": true, "signup": signup_json(signup) })).into_response(),
        Err(message) => json_error(StatusCode::BAD_REQUEST, &message),
    }
}

async fn handle_withdraw(State(web): State<Arc<TurnierWeb>>, headers: HeaderMap) -> Response {
    let session = match authorize(&web, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let removed = web.store.remove_signup(web.guild_id, session.user_id).await;
    Json(json!({ "ok": removed })).into_response()
}

async fn handle_team_create(
    State(web): State<Arc<TurnierWeb>>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let session = match authorize(&web, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(Json(body)) = body else {
        return json_error(StatusCode::BAD_REQUEST, "invalid json");
    };
    let team_name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if team_name.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "name fehlt.");
    }
    if web
        .store
        .get_signup(web.guild_id, session.user_id)
        .await
        .is_none()
    {
        return json_error(
            StatusCode::BAD_REQUEST,
            "Erst anmelden, dann Team erstellen.",
        );
    }
    let team = match web
        .store
        .get_or_create_team(web.guild_id, team_name, Some(session.user_id))
        .await
    {
        Ok(team) => team,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, &message),
    };
    if let Err(message) = web
        .store
        .assign_signup_team(web.guild_id, session.user_id, Some(team.id))
        .await
    {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &message);
    }
    Json(json!({ "ok": true, "team": {
        "id": team.id, "name": team.name, "created": team.created,
    }}))
    .into_response()
}

async fn handle_team_rename(
    State(web): State<Arc<TurnierWeb>>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let session = match authorize(&web, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(Json(body)) = body else {
        return json_error(StatusCode::BAD_REQUEST, "invalid json");
    };
    let team_id = body.get("team_id").and_then(Value::as_i64);
    let new_name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let (Some(team_id), false) = (team_id, new_name.is_empty()) else {
        return json_error(StatusCode::BAD_REQUEST, "team_id und name erforderlich.");
    };
    let Some(team) = web.store.get_team(web.guild_id, team_id).await else {
        return json_error(StatusCode::NOT_FOUND, "Team nicht gefunden.");
    };
    if team.created_by != Some(session.user_id) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Nur der Ersteller kann das Team umbenennen.",
        );
    }
    match web.store.rename_team(web.guild_id, team_id, new_name).await {
        Ok(renamed) => Json(json!({ "ok": renamed })).into_response(),
        Err(message) => json_error(StatusCode::BAD_REQUEST, &message),
    }
}

async fn handle_team_kick(
    State(web): State<Arc<TurnierWeb>>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let session = match authorize(&web, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(Json(body)) = body else {
        return json_error(StatusCode::BAD_REQUEST, "invalid json");
    };
    let team_id = body.get("team_id").and_then(Value::as_i64);
    let target_id = body.get("user_id").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    });
    let (Some(team_id), Some(target_id)) = (team_id, target_id) else {
        return json_error(StatusCode::BAD_REQUEST, "team_id und user_id erforderlich.");
    };
    let Some(team) = web.store.get_team(web.guild_id, team_id).await else {
        return json_error(StatusCode::NOT_FOUND, "Team nicht gefunden.");
    };
    if team.created_by != Some(session.user_id) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Nur der Ersteller kann Mitglieder entfernen.",
        );
    }
    if target_id == session.user_id {
        return json_error(
            StatusCode::BAD_REQUEST,
            "Nutze /api/withdraw um dich selbst abzumelden.",
        );
    }
    let removed = web
        .store
        .assign_signup_team(web.guild_id, target_id, None)
        .await
        .unwrap_or(false);
    Json(json!({ "ok": removed })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    async fn setup() -> (tempfile::TempDir, Arc<TurnierWeb>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dl_db::Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let store = TournamentStore::new(db);
        store.ensure_schema().await.expect("schema");
        // steam_links für den Signup-Pfad
        store
            .db
            .write(|c| {
                c.execute_batch(
                    "CREATE TABLE steam_links(user_id INTEGER, steam_id TEXT, verified INTEGER DEFAULT 0,
                       primary_account INTEGER DEFAULT 0, deadlock_rank INTEGER,
                       deadlock_rank_name TEXT, deadlock_subrank INTEGER,
                       deadlock_rank_updated_at DATETIME);",
                )
            })
            .await
            .expect("steam_links");
        let html = dir.path().join("index.html");
        std::fs::write(&html, "<html>Turnier</html>").expect("html");
        let web = TurnierWeb::new(store, 1, "https://link.test", html, Arc::new(NoRoleCheck));
        (dir, web)
    }

    #[test]
    fn bracket_wie_python() {
        // Referenz-JSON aus dem laufenden CPython-Original
        let signups = vec![
            Signup {
                user_id: 1,
                registration_mode: "team".into(),
                rank: "phantom".into(),
                rank_value: 9,
                rank_subvalue: 3,
                display_name: Some("A".into()),
                team_id: Some(1),
                team_name: None,
                assigned_by_admin: false,
                status: String::new(),
            },
            Signup {
                user_id: 2,
                registration_mode: "team".into(),
                rank: "oracle".into(),
                rank_value: 8,
                rank_subvalue: 0,
                display_name: Some("B".into()),
                team_id: Some(1),
                team_name: None,
                assigned_by_admin: false,
                status: String::new(),
            },
            Signup {
                user_id: 3,
                registration_mode: "solo".into(),
                rank: "eternus".into(),
                rank_value: 11,
                rank_subvalue: 6,
                display_name: Some("Solo1".into()),
                team_id: None,
                team_name: None,
                assigned_by_admin: false,
                status: String::new(),
            },
            Signup {
                user_id: 4,
                registration_mode: "solo".into(),
                rank: "seeker".into(),
                rank_value: 2,
                rank_subvalue: 1,
                display_name: None,
                team_id: None,
                team_name: None,
                assigned_by_admin: false,
                status: String::new(),
            },
            Signup {
                user_id: 5,
                registration_mode: "team".into(),
                rank: "ritualist".into(),
                rank_value: 5,
                rank_subvalue: 2,
                display_name: Some("C".into()),
                team_id: Some(7),
                team_name: None,
                assigned_by_admin: false,
                status: String::new(),
            },
        ];
        let teams = vec![
            TeamRow {
                id: 1,
                name: "Alpha".into(),
                created_by: None,
                member_count: 2,
            },
            TeamRow {
                id: 7,
                name: "Beta".into(),
                created_by: None,
                member_count: 1,
            },
        ];
        let bracket = generate_bracket(&signups, &teams);
        assert_eq!(bracket["num_entries"], 4);
        assert_eq!(bracket["num_rounds"], 2);
        assert_eq!(bracket["slots"], 4);
        // Sortierung: Solo1 72 > Alpha 54 > Beta 32 > User 4 13
        let names: Vec<&str> = bracket["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|e| e["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, vec!["Solo1", "Alpha", "Beta", "User 4"]);
        assert_eq!(bracket["entries"][1]["score"], 54.0);
        assert_eq!(bracket["entries"][3]["score"], 13);
        // Seeding 1-vs-4, 2-vs-3; Finale-Label
        assert_eq!(
            bracket["rounds"][0]["matches"][0]["entry_a"]["name"],
            "Solo1"
        );
        assert_eq!(
            bracket["rounds"][0]["matches"][0]["entry_b"]["name"],
            "User 4"
        );
        assert_eq!(bracket["rounds"][0]["label"], "🥊 Halbfinale");
        assert_eq!(bracket["rounds"][1]["label"], "🏆 Finale");
        // < 2 Einträge → Fehler-Form
        let empty = generate_bracket(&[], &[]);
        assert!(empty["error"].is_string());
    }

    #[tokio::test]
    async fn auth_und_signup_flow() {
        let (_dir, web) = setup().await;
        // Periode + Steam-Link vorbereiten
        web.store
            .create_period(1, "Cup", "2020-01-01", "2099-01-01", 6, None)
            .await
            .expect("period");
        web.store
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO steam_links(user_id, steam_id, verified, primary_account,
                       deadlock_rank, deadlock_rank_name, deadlock_subrank)
                     VALUES(100, 'x', 1, 1, 9, 'Phantom', 3)",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("steam");
        let token = web.store.create_auth_token(100, "Anna", 60.0).await;

        let app = router(web.clone());
        // auth/complete → Session-Cookie
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/auth/complete?token={token}"))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("cookie")
            .to_str()
            .expect("str")
            .to_string();
        let session_pair = cookie.split(';').next().expect("pair").to_string();

        // /api/me liefert csrf
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/me")
                    .header(header::COOKIE, &session_pair)
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body");
        let me: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(me["logged_in"], true);
        assert_eq!(me["user"]["display_name"], "Anna");
        let csrf = me["user"]["csrf_token"].as_str().expect("csrf").to_string();

        // Signup ohne CSRF → 403
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/signup")
                    .header(header::COOKIE, &session_pair)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        // Signup solo mit CSRF → ok, Rang aus steam_links
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/signup")
                    .header(header::COOKIE, &session_pair)
                    .header("X-CSRF-Token", &csrf)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from("{\"mode\":\"solo\"}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body");
        let result: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(result["ok"], true);
        assert_eq!(result["signup"]["rank"], "phantom");
        assert_eq!(result["signup"]["rank_subvalue"], 3);
        assert_eq!(result["signup"]["status"], "inserted");

        // Overview zeigt den Solo-Signup
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/overview")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body");
        let overview: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(overview["solo_signups"][0]["display_name"], "Anna");
        assert_eq!(overview["solo_signups"][0]["rank_label"], "Phantom 3");

        // Withdraw → ok=true
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/withdraw")
                    .header(header::COOKIE, &session_pair)
                    .header("X-CSRF-Token", &csrf)
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body");
        let result: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(result["ok"], true);
    }

    #[tokio::test]
    async fn abgelaufener_token_redirectet() {
        let (_dir, web) = setup().await;
        let token = web.store.create_auth_token(100, "Anna", -1.0).await;
        let response = router(web)
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/auth/complete?token={token}"))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }
}
