//! Changelog-Empfänger — Rust-Port von `cogs/changelog_publisher.py`.
//!
//! Interner HTTP-Dienst auf 127.0.0.1:8899: nimmt Changelog-Posts entgegen
//! (einfach + mehrseitig), Highlight-Clips und eine lokale Message-Lese-API.
//! Auth: `token`-Feld im Body gegen CHANGELOG_API_TOKEN (wie das Original —
//! bewusst simpel, der Port ist loopback-only).

// Frühe HTTP-Fehlerantworten als Err(Response) — axum-idiomatisch.
#![allow(clippy::result_large_err)]

use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Map, Value};

pub const DEV_UPDATES_CHANNEL_ID: u64 = 1492910851483504821;
pub const TWITCH_BOT_CHANNEL_ID: u64 = 1318329964713611385;
pub const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;
const EMBED_COLOR: u32 = 0x5865F2;

#[derive(Debug, thiserror::Error)]
pub enum ChangelogError {
    #[error("channel {0} not found")]
    ChannelNotFound(u64),
    #[error("{0}")]
    Discord(String),
}

/// Discord-Zugriff des Changelog-Dienstes — implementiert von dl-discord.
#[async_trait::async_trait]
pub trait ChangelogDiscord: Send + Sync {
    /// → message_id
    async fn send(
        &self,
        channel_id: u64,
        content: Option<&str>,
        embeds: &[Value],
        mention_roles: bool,
    ) -> Result<u64, ChangelogError>;
    async fn delete_message(&self, channel_id: u64, message_id: u64) -> Result<(), ChangelogError>;
    async fn send_file(
        &self,
        channel_id: u64,
        content: &str,
        path: &Path,
    ) -> Result<(), ChangelogError>;
    async fn fetch_message(
        &self,
        channel_id: u64,
        message_id: u64,
    ) -> Result<Value, ChangelogError>;
    async fn fetch_history(
        &self,
        channel_id: u64,
        limit: u8,
        before_id: Option<u64>,
    ) -> Result<Vec<Value>, ChangelogError>;
}

pub struct ChangelogState {
    pub discord: Arc<dyn ChangelogDiscord>,
    pub token: String,
}

pub type SharedChangelog = Arc<ChangelogState>;

impl ChangelogState {
    /// Token wie das Original: CHANGELOG_API_TOKEN, Default "changeme-local".
    pub fn new(discord: Arc<dyn ChangelogDiscord>, token: Option<String>) -> SharedChangelog {
        Arc::new(Self {
            discord,
            token: token
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "changeme-local".to_string()),
        })
    }
}

pub fn router(state: SharedChangelog) -> Router {
    Router::new()
        .route("/changelog", post(handle_changelog))
        .route("/changelog/rich", post(handle_rich))
        .route("/highlight-clips", post(handle_highlights))
        .route("/discord/messages", post(handle_fetch_messages))
        .with_state(state)
}

fn err(status: u16, message: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(json!({ "ok": false, "error": message })),
    )
        .into_response()
}

fn parse_body(
    state: &ChangelogState,
    body: Option<&Json<Value>>,
) -> Result<Map<String, Value>, Response> {
    let data = body
        .and_then(|Json(v)| v.as_object().cloned())
        .ok_or_else(|| err(400, "invalid JSON"))?;
    let token = data
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if token != state.token {
        return Err(err(401, "unauthorized"));
    }
    Ok(data)
}

fn utc_now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn build_embed(title: &str, content: &str, target: &str) -> Value {
    json!({
        "title": format!("📋 {title}"),
        "description": content,
        "color": EMBED_COLOR,
        "timestamp": utc_now_iso(),
        "footer": { "text": if target == "twitch" { "Twitch Bot" } else { "Deadlock Bots" } },
    })
}

fn parse_channel_id(raw: Option<&Value>) -> Result<Option<u64>, Response> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.as_u64().unwrap_or(0) > 0 => Ok(n.as_u64()),
        Some(Value::String(s)) if !s.trim().is_empty() => s
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| err(400, "channel_id must be an integer")),
        Some(Value::Number(_)) => Err(err(400, "channel_id must be an integer")),
        // Falsy Werte (0, "", false) zählen wie in Python als "nicht gesetzt"
        Some(Value::Bool(false)) => Ok(None),
        Some(_) => Err(err(400, "channel_id must be an integer")),
    }
}

/// `POST /changelog` — {token, title, content, target|channel_id}.
async fn handle_changelog(
    State(state): State<SharedChangelog>,
    body: Option<Json<Value>>,
) -> Response {
    let data = match parse_body(&state, body.as_ref()) {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    let title = data
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let content = data
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let target = data
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("all")
        .trim();
    if title.is_empty() || content.is_empty() {
        return err(400, "title and content required");
    }

    let direct = match parse_channel_id(data.get("channel_id")) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let channel_id = match direct {
        Some(id) => id,
        None => {
            if target != "all" && target != "twitch" {
                return err(400, "target must be one of {'all', 'twitch'}");
            }
            if target == "twitch" {
                TWITCH_BOT_CHANNEL_ID
            } else {
                DEV_UPDATES_CHANNEL_ID
            }
        }
    };

    let embed = build_embed(title, content, target);
    match state.discord.send(channel_id, None, &[embed], false).await {
        Ok(_) => Json(json!({ "ok": true, "channel_id": channel_id })).into_response(),
        Err(e) => {
            tracing::warn!(%e, channel_id, "Changelog-Post fehlgeschlagen");
            err(500, &e.to_string())
        }
    }
}

/// `POST /changelog/rich` — mehrere Embeds in EINER Nachricht.
async fn handle_rich(State(state): State<SharedChangelog>, body: Option<Json<Value>>) -> Response {
    let data = match parse_body(&state, body.as_ref()) {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    let Some(sections) = data
        .get("sections")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
    else {
        return err(400, "sections required");
    };
    if sections.len() > 10 {
        return err(400, "max 10 sections");
    }
    let target = data
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("all")
        .trim();
    let channel_id = match parse_channel_id(data.get("channel_id")) {
        Ok(Some(id)) => id,
        Ok(None) => {
            if target == "twitch" {
                TWITCH_BOT_CHANNEL_ID
            } else {
                DEV_UPDATES_CHANNEL_ID
            }
        }
        Err(resp) => return resp,
    };

    // Alte Nachricht ersetzen (best effort, wie das Original)
    if let Some(replace_id) = data
        .get("replace_message_id")
        .and_then(|v| match v {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        })
        .filter(|v| *v > 0)
    {
        if let Err(e) = state.discord.delete_message(channel_id, replace_id).await {
            tracing::warn!(%e, replace_id, "rich-changelog: alte Nachricht nicht löschbar");
        }
    }

    let footer = if target == "twitch" {
        "Twitch Bot"
    } else {
        "Deadlock Bots"
    };
    let mut embeds = Vec::new();
    let valid_sections: Vec<(&str, String)> = sections
        .iter()
        .filter_map(|sec| {
            let title = sec
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let content = sec
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            (!content.is_empty()).then_some((title, content))
        })
        .collect();
    for (i, (title, content)) in valid_sections.iter().enumerate() {
        let mut embed = Map::new();
        if !title.is_empty() {
            embed.insert("title".into(), json!(title));
        }
        let description: String = content.chars().take(4096).collect();
        embed.insert("description".into(), json!(description));
        embed.insert("color".into(), json!(EMBED_COLOR));
        if i == valid_sections.len() - 1 {
            embed.insert("footer".into(), json!({ "text": footer }));
            embed.insert("timestamp".into(), json!(utc_now_iso()));
        }
        embeds.push(Value::Object(embed));
    }
    if embeds.is_empty() {
        return err(400, "no valid sections");
    }

    let role_ping = data
        .get("role_ping_id")
        .and_then(|v| match v {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        })
        .filter(|v| *v > 0);
    let content = role_ping.map(|id| format!("<@&{id}>"));

    match state
        .discord
        .send(channel_id, content.as_deref(), &embeds, role_ping.is_some())
        .await
    {
        Ok(message_id) => Json(json!({
            "ok": true,
            "channel_id": channel_id,
            "message_id": message_id,
        }))
        .into_response(),
        Err(ChangelogError::ChannelNotFound(id)) => err(404, &format!("channel {id} not found")),
        Err(e) => {
            tracing::warn!(%e, channel_id, "rich-changelog fehlgeschlagen");
            err(500, &e.to_string())
        }
    }
}

/// `POST /highlight-clips` — Übersicht-Embed + Clips als Datei-Anhänge.
async fn handle_highlights(
    State(state): State<SharedChangelog>,
    body: Option<Json<Value>>,
) -> Response {
    let data = match parse_body(&state, body.as_ref()) {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    let channel_id = data
        .get("channel_id")
        .and_then(|v| match v {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        })
        .unwrap_or(0);
    let streamer = data
        .get("streamer_login")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let match_id = data
        .get("match_id")
        .and_then(|v| match v {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        })
        .unwrap_or(0);
    if channel_id == 0 || streamer.is_empty() || match_id == 0 {
        return err(400, "channel_id, streamer_login, match_id required");
    }
    let empty = Vec::new();
    let events = data
        .get("events")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let clip_paths = data
        .get("clip_paths")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let header = json!({
        "title": format!("Highlights — {streamer} (Match #{match_id})"),
        "description": format!("{} Clip(s)", clip_paths.len()),
        "color": 0xE67E22,
        "timestamp": utc_now_iso(),
    });
    if let Err(e) = state.discord.send(channel_id, None, &[header], false).await {
        if matches!(e, ChangelogError::ChannelNotFound(_)) {
            return err(404, &format!("channel {channel_id} not found"));
        }
        return err(500, &e.to_string());
    }

    for (event, clip_path) in events.iter().zip(clip_paths.iter()) {
        let Some(path_str) = clip_path.as_str() else {
            continue;
        };
        let path = Path::new(path_str);
        let size_ok = std::fs::metadata(path)
            .map(|m| m.len() <= MAX_FILE_BYTES)
            .unwrap_or(false);
        if !size_ok {
            continue;
        }
        let label = event
            .get("label")
            .or_else(|| event.get("event_type"))
            .and_then(Value::as_str)
            .unwrap_or("Clip");
        if let Err(e) = state
            .discord
            .send_file(channel_id, &format!("**{streamer}** — {label}"), path)
            .await
        {
            tracing::warn!(%e, ?path, "Highlight-Clip-Upload fehlgeschlagen");
        }
    }

    Json(json!({ "ok": true, "clips_sent": clip_paths.len() })).into_response()
}

/// `POST /discord/messages` — lokale Lese-API.
async fn handle_fetch_messages(
    State(state): State<SharedChangelog>,
    body: Option<Json<Value>>,
) -> Response {
    let data = match parse_body(&state, body.as_ref()) {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    let channel_id = match data.get("channel_id") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => match s.trim().parse::<u64>() {
            Ok(v) => v,
            Err(_) => return err(400, "channel_id must be an integer"),
        },
        _ => 0,
    };
    if channel_id == 0 {
        return err(400, "channel_id required");
    }

    let message_id = data.get("message_id").and_then(|v| match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    });

    let result = if let Some(message_id) = message_id {
        state
            .discord
            .fetch_message(channel_id, message_id)
            .await
            .map(|m| vec![m])
    } else {
        let limit = data
            .get("limit")
            .and_then(|v| match v {
                Value::Number(n) => n.as_u64(),
                Value::String(s) => s.trim().parse::<u64>().ok(),
                _ => None,
            })
            .unwrap_or(1)
            .min(100) as u8;
        let before_id = data.get("before_id").and_then(|v| match v {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        });
        state
            .discord
            .fetch_history(channel_id, limit, before_id)
            .await
    };

    match result {
        Ok(messages) => Json(json!({ "ok": true, "messages": messages })).into_response(),
        Err(ChangelogError::ChannelNotFound(id)) => err(404, &format!("channel {id} not found")),
        Err(e) => err(500, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Mutex;
    use tower::ServiceExt;

    /// Mock: zeichnet Sends auf, simuliert unbekannte Kanäle.
    struct MockDiscord {
        sent: Mutex<Vec<(u64, Option<String>, usize)>>,
    }

    #[async_trait::async_trait]
    impl ChangelogDiscord for MockDiscord {
        async fn send(
            &self,
            channel_id: u64,
            content: Option<&str>,
            embeds: &[Value],
            _mention_roles: bool,
        ) -> Result<u64, ChangelogError> {
            if channel_id == 404 {
                return Err(ChangelogError::ChannelNotFound(channel_id));
            }
            self.sent.lock().expect("mock lock").push((
                channel_id,
                content.map(str::to_string),
                embeds.len(),
            ));
            Ok(999)
        }
        async fn delete_message(&self, _c: u64, _m: u64) -> Result<(), ChangelogError> {
            Ok(())
        }
        async fn send_file(&self, _c: u64, _t: &str, _p: &Path) -> Result<(), ChangelogError> {
            Ok(())
        }
        async fn fetch_message(&self, _c: u64, m: u64) -> Result<Value, ChangelogError> {
            Ok(json!({ "id": m }))
        }
        async fn fetch_history(
            &self,
            _c: u64,
            limit: u8,
            _b: Option<u64>,
        ) -> Result<Vec<Value>, ChangelogError> {
            Ok(vec![json!({ "id": 1 }); limit as usize])
        }
    }

    fn test_app() -> (Router, Arc<MockDiscord>) {
        let mock = Arc::new(MockDiscord {
            sent: Mutex::new(Vec::new()),
        });
        let state = ChangelogState::new(mock.clone(), Some("test-token".to_string()));
        (router(state), mock)
    }

    async fn post_json(app: Router, path: &str, body: Value) -> (u16, Value) {
        let response = app
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn token_pflicht() {
        let (app, _) = test_app();
        let (status, body) = post_json(
            app,
            "/changelog",
            json!({"token": "falsch", "title": "T", "content": "C"}),
        )
        .await;
        assert_eq!(status, 401);
        assert_eq!(body["error"], "unauthorized");
    }

    #[tokio::test]
    async fn einfacher_changelog_landet_im_dev_kanal() {
        let (app, mock) = test_app();
        let (status, body) = post_json(
            app,
            "/changelog",
            json!({"token": "test-token", "title": "T", "content": "C"}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["channel_id"], DEV_UPDATES_CHANNEL_ID);
        let sent = mock.sent.lock().expect("lock");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, DEV_UPDATES_CHANNEL_ID);
    }

    #[tokio::test]
    async fn twitch_target_und_validierung() {
        let (app, _) = test_app();
        let (status, body) = post_json(
            app.clone(),
            "/changelog",
            json!({"token": "test-token", "title": "T", "content": "C", "target": "twitch"}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["channel_id"], TWITCH_BOT_CHANNEL_ID);

        let (status, _) = post_json(
            app.clone(),
            "/changelog",
            json!({"token": "test-token", "title": "T", "content": "C", "target": "quatsch"}),
        )
        .await;
        assert_eq!(status, 400);

        let (status, _) = post_json(
            app,
            "/changelog",
            json!({"token": "test-token", "title": "", "content": "C"}),
        )
        .await;
        assert_eq!(status, 400);
    }

    #[tokio::test]
    async fn rich_changelog_mit_sections() {
        let (app, mock) = test_app();
        let (status, body) = post_json(
            app,
            "/changelog/rich",
            json!({
                "token": "test-token",
                "sections": [
                    {"title": "A", "content": "Inhalt A"},
                    {"title": "B", "content": ""},
                    {"title": "C", "content": "Inhalt C"},
                ],
                "role_ping_id": 42,
            }),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["message_id"], 999);
        let sent = mock.sent.lock().expect("lock");
        // Leere Section B übersprungen → 2 Embeds, Role-Ping im Content
        assert_eq!(sent[0].2, 2);
        assert_eq!(sent[0].1.as_deref(), Some("<@&42>"));
    }

    #[tokio::test]
    async fn rich_kanal_nicht_gefunden() {
        let (app, _) = test_app();
        let (status, _) = post_json(
            app,
            "/changelog/rich",
            json!({
                "token": "test-token",
                "channel_id": 404,
                "sections": [{"title": "A", "content": "x"}],
            }),
        )
        .await;
        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn fetch_messages_einzeln_und_history() {
        let (app, _) = test_app();
        let (status, body) = post_json(
            app.clone(),
            "/discord/messages",
            json!({"token": "test-token", "channel_id": 7, "message_id": 123}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["messages"][0]["id"], 123);

        let (status, body) = post_json(
            app,
            "/discord/messages",
            json!({"token": "test-token", "channel_id": 7, "limit": 3}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(3));
    }
}
