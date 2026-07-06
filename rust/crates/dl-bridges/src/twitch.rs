//! Twitch-Live-Bridge — Port von `cogs/twitch/live_bridge.py`.
//!
//! Aufgaben:
//! - Klicks auf `twitch-live:*`-Buttons tracken (POST /live/link-click mit
//!   Idempotency-Key) und dem User ephemeral seinen Referral-Link geben.
//! - Beim Start aktive Live-Ankündigungen vom Twitch-Bot laden
//!   (GET /live/active-announcements), damit Klicks auf ältere Nachrichten
//!   weiter den richtigen Referral-Link kennen (ersetzt `bot.add_view`).
//! - Den `twitch_live_tracking`-view_spec des Brokers mit Daten versorgen
//!   (Registry-Eintrag beim Posten — löst die Phase-2-Kopplung auf).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use dl_changelog::{SpamLearningPayload, SpamLearningStore};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use reqwest::Url;
use serde_json::{json, Value};
use tokio::sync::RwLock;

pub const TWITCH_INTERNAL_API_BASE_PATH: &str = "/internal/twitch/v1";
pub const TRACKING_PREFIX: &str = "twitch-live:";
pub const SPAM_LEARNING_PREFIX: &str = "spam-learning:";
const DEFAULT_BUTTON_LABEL: &str = "Auf Twitch ansehen";
/// Backoff der Start-Rehydrierung (der Twitch-Bot kann später hochkommen).
const RESTORE_RETRY_DELAYS: [u64; 5] = [1, 2, 5, 10, 30];

#[derive(Debug, thiserror::Error)]
pub enum TwitchBridgeError {
    #[error("{0}")]
    Api(String),
    #[error("Twitch internal API request failed with status {0}")]
    Status(u16),
}

/// Eine aktive Live-Ankündigung (Schlüssel: custom_id des Buttons).
#[derive(Debug, Clone)]
pub struct Announcement {
    pub streamer_login: String,
    pub tracking_token: String,
    pub referral_url: String,
    pub button_label: String,
    pub channel_id: u64,
    pub message_id: u64,
}

/// custom_id wie TwitchLiveTrackingView.build_custom_id.
pub fn build_custom_id(streamer_login: &str, tracking_token: &str) -> String {
    let login_part: String = streamer_login
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(24)
        .collect();
    let login_part = if login_part.is_empty() {
        "stream".to_string()
    } else {
        login_part
    };
    let token_part: String = tracking_token.chars().take(32).collect();
    let token_part = if token_part.is_empty() {
        "track".to_string()
    } else {
        token_part
    };
    format!("{TRACKING_PREFIX}{login_part}:{token_part}")
}

// ── API-Client ─────────────────────────────────────────────────────────────

pub struct TwitchApiClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl TwitchApiClient {
    /// ENVs wie das Original; `None` wenn kein TWITCH_INTERNAL_API_TOKEN gesetzt ist.
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Arc<Self>> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let token = get("TWITCH_INTERNAL_API_TOKEN")?;
        let base_url = get("TWITCH_INTERNAL_API_BASE_URL").unwrap_or_else(|| {
            let host = get("TWITCH_INTERNAL_API_HOST").unwrap_or_else(|| "127.0.0.1".to_string());
            let port = get("TWITCH_INTERNAL_API_PORT")
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(8776);
            format!("http://{host}:{port}")
        });
        let allow_non_loopback = get("TWITCH_INTERNAL_API_ALLOW_NON_LOOPBACK")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        let timeout = get("TWITCH_INTERNAL_API_TIMEOUT_SEC")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v.max(0.5))
            .unwrap_or(10.0);
        match Self::try_new(
            base_url,
            token,
            Duration::from_secs_f64(timeout),
            allow_non_loopback,
        ) {
            Ok(client) => Some(client),
            Err(err) => {
                tracing::warn!(%err, "Twitch internal API config ungueltig");
                None
            }
        }
    }

    pub fn try_new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        timeout: Duration,
        allow_non_loopback: bool,
    ) -> Result<Arc<Self>, TwitchBridgeError> {
        let base_url = normalize_internal_api_base_url(&base_url.into(), allow_non_loopback)
            .map_err(TwitchBridgeError::Api)?;
        Ok(Arc::new(Self {
            http: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
            base_url,
            token: token.into(),
        }))
    }

    #[cfg(test)]
    pub fn new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        timeout: Duration,
    ) -> Arc<Self> {
        Self::try_new(base_url, token, timeout, false).expect("valid Twitch API client")
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        payload: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, TwitchBridgeError> {
        let url = format!("{}{TWITCH_INTERNAL_API_BASE_PATH}{path}", self.base_url);
        let mut request = self
            .http
            .request(method, &url)
            .header("X-Internal-Token", &self.token);
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        if let Some(payload) = payload {
            request = request.json(payload);
        }
        let response = request
            .send()
            .await
            .map_err(|e| TwitchBridgeError::Api(e.to_string()))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(body)
        } else {
            // Fehlermeldung wie das Original aus error.message/message ziehen
            let message = body
                .get("error")
                .and_then(|e| e.get("message"))
                .or_else(|| body.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string);
            match message.filter(|m| !m.is_empty()) {
                Some(message) => Err(TwitchBridgeError::Api(message)),
                None => Err(TwitchBridgeError::Status(status.as_u16())),
            }
        }
    }

    /// Unverknüpfte Streamer für den Link-Matcher.
    pub async fn link_candidates(&self) -> Result<Vec<Value>, TwitchBridgeError> {
        let data = self
            .request(
                reqwest::Method::GET,
                "/streamers/link-candidates",
                None,
                None,
            )
            .await?;
        Ok(data
            .get("entries")
            .and_then(Value::as_array)
            .map(|entries| entries.iter().filter(|e| e.is_object()).cloned().collect())
            .unwrap_or_default())
    }

    /// Discord-Profil an einen Streamer hängen (Matcher: Auto-/Review-Link).
    pub async fn link_discord_profile(
        &self,
        login: &str,
        discord_user_id: u64,
        discord_display_name: &str,
    ) -> Result<Value, TwitchBridgeError> {
        let path = format!("/streamers/{}/discord-profile", login.to_lowercase());
        self.request(
            reqwest::Method::POST,
            &path,
            Some(&json!({
                "discord_user_id": discord_user_id.to_string(),
                "discord_display_name": discord_display_name,
                "mark_member": true,
            })),
            None,
        )
        .await
    }

    pub async fn active_announcements(&self) -> Result<Vec<Announcement>, TwitchBridgeError> {
        let payload = self
            .request(
                reqwest::Method::GET,
                "/live/active-announcements",
                None,
                None,
            )
            .await?;
        let Some(items) = payload.as_array() else {
            return Err(TwitchBridgeError::Api(
                "active live announcements payload is invalid".to_string(),
            ));
        };
        let mut entries = Vec::new();
        for item in items {
            let get_str = |key: &str| {
                item.get(key)
                    .map(|v| match v {
                        Value::String(s) => s.trim().to_string(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default()
            };
            let get_u64 = |key: &str| {
                item.get(key)
                    .and_then(|v| match v {
                        Value::Number(n) => n.as_u64(),
                        Value::String(s) => s.trim().parse::<u64>().ok(),
                        _ => None,
                    })
                    .unwrap_or(0)
            };
            let announcement = Announcement {
                streamer_login: get_str("streamer_login").to_lowercase(),
                tracking_token: get_str("tracking_token"),
                referral_url: get_str("referral_url"),
                button_label: {
                    let label = get_str("button_label");
                    if label.is_empty() {
                        DEFAULT_BUTTON_LABEL.to_string()
                    } else {
                        label.chars().take(80).collect()
                    }
                },
                channel_id: get_u64("channel_id"),
                message_id: get_u64("message_id"),
            };
            if announcement.streamer_login.is_empty()
                || announcement.tracking_token.is_empty()
                || announcement.message_id == 0
            {
                return Err(TwitchBridgeError::Api(
                    "active live announcement entry is invalid".to_string(),
                ));
            }
            if !is_valid_referral_url(&announcement.referral_url) {
                tracing::warn!(
                    streamer = %announcement.streamer_login,
                    "Twitch-Live-Ankuendigung mit ungueltiger referral_url uebersprungen"
                );
                continue;
            }
            entries.push(announcement);
        }
        Ok(entries)
    }

    /// Read-only Diagnose für Ticket-Auto-Hilfe:
    /// `GET /internal/twitch/v1/diagnose?discord_id=<id>`.
    pub async fn diagnose_discord_user(
        &self,
        discord_user_id: u64,
    ) -> Result<Value, TwitchBridgeError> {
        let path = format!("/diagnose?discord_id={discord_user_id}");
        self.request(reqwest::Method::GET, &path, None, None).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_link_click(
        &self,
        announcement: &Announcement,
        discord_user_id: u64,
        discord_username: &str,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        idempotency_key: &str,
    ) -> Result<(), TwitchBridgeError> {
        let payload = json!({
            "streamer_login": announcement.streamer_login,
            "tracking_token": announcement.tracking_token,
            "discord_user_id": discord_user_id.to_string(),
            "discord_username": discord_username,
            "guild_id": guild_id.filter(|v| *v > 0).map(|v| v.to_string()),
            "channel_id": channel_id.to_string(),
            "message_id": message_id.to_string(),
            "source_hint": "discord_button",
        });
        self.request(
            reqwest::Method::POST,
            "/live/link-click",
            Some(&payload),
            Some(idempotency_key),
        )
        .await
        .map(|_| ())
    }

    pub async fn learn_spam_pattern(
        &self,
        payload: &SpamLearningPayload,
        verdict: &str,
    ) -> Result<(), TwitchBridgeError> {
        self.request(
            reqwest::Method::POST,
            "/spam-learning",
            Some(&json!({
                "verdict": verdict,
                "pattern": &payload.pattern,
                "patternType": &payload.pattern_type,
                "sourceMessage": &payload.source_message,
                "sourceChannel": &payload.source_channel,
                "reason": &payload.reason,
            })),
            None,
        )
        .await
        .map(|_| ())
    }
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost" {
        return true;
    }
    normalized
        .parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn normalize_internal_api_base_url(
    value: &str,
    allow_non_loopback: bool,
) -> Result<String, String> {
    let mut raw = value.trim().to_string();
    if raw.is_empty() {
        return Err("base_url is required".to_string());
    }
    if !raw.contains("://") {
        raw = format!("http://{raw}");
    }
    let mut parsed = Url::parse(&raw).map_err(|_| "base_url is invalid".to_string())?;
    if parsed.scheme().is_empty() || parsed.host_str().is_none() {
        return Err("base_url is invalid".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("base_url must not contain credentials".to_string());
    }
    let host = parsed
        .host_str()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| "base_url is invalid".to_string())?;
    if !allow_non_loopback && !is_loopback_host(host) {
        return Err("base_url host must resolve to loopback unless explicitly allowed".to_string());
    }
    let mut path = parsed.path().trim_end_matches('/').to_string();
    let internal_base = TWITCH_INTERNAL_API_BASE_PATH.trim_end_matches('/');
    if path == internal_base {
        path.clear();
    } else if path.ends_with(internal_base) {
        let keep = path.len() - internal_base.len();
        path.truncate(keep);
        path = path.trim_end_matches('/').to_string();
    }
    parsed.set_path(&path);
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn is_valid_referral_url(value: &str) -> bool {
    let Ok(parsed) = Url::parse(value.trim()) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some()
}

// ── Registry (ersetzt bot.add_view-Rehydrierung) ───────────────────────────

#[derive(Default)]
pub struct TrackingRegistry {
    by_custom_id: RwLock<HashMap<String, Announcement>>,
}

impl TrackingRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn insert(&self, announcement: Announcement) {
        if !is_valid_referral_url(&announcement.referral_url) {
            return;
        }
        let custom_id = build_custom_id(&announcement.streamer_login, &announcement.tracking_token);
        self.by_custom_id
            .write()
            .await
            .insert(custom_id, announcement);
    }

    pub async fn get(&self, custom_id: &str) -> Option<Announcement> {
        self.by_custom_id.read().await.get(custom_id).cloned()
    }

    pub async fn replace_all(&self, announcements: Vec<Announcement>) {
        let mut map = HashMap::new();
        for announcement in announcements {
            if !is_valid_referral_url(&announcement.referral_url) {
                continue;
            }
            let custom_id =
                build_custom_id(&announcement.streamer_login, &announcement.tracking_token);
            map.insert(custom_id, announcement);
        }
        *self.by_custom_id.write().await = map;
    }
}

/// Start-Rehydrierung mit Backoff — wie `_restore_active_announcements_with_retry`.
pub fn spawn_restore(
    client: Arc<TwitchApiClient>,
    registry: Arc<TrackingRegistry>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut attempt = 0usize;
        loop {
            match client.active_announcements().await {
                Ok(announcements) => {
                    let count = announcements.len();
                    registry.replace_all(announcements).await;
                    tracing::info!(count, "Twitch-Live-Bridge: Ankündigungen rehydriert");
                    return;
                }
                Err(err) => {
                    let delay = RESTORE_RETRY_DELAYS[attempt.min(RESTORE_RETRY_DELAYS.len() - 1)];
                    attempt += 1;
                    tracing::warn!(%err, attempt, delay, "Twitch-Live-Rehydrierung fehlgeschlagen — Retry");
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                }
            }
        }
    })
}

// ── Klick-Handler ──────────────────────────────────────────────────────────

pub struct TrackingClickHandler {
    pub client: Arc<TwitchApiClient>,
    pub registry: Arc<TrackingRegistry>,
}

#[async_trait::async_trait]
impl InteractionHandler for TrackingClickHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        // Announcement auflösen; bei Miss einmal frisch vom Twitch-Bot laden
        // (robuster als Python, das nur registrierte Views kennt).
        let mut announcement = self.registry.get(&interaction.custom_id).await;
        if announcement.is_none() {
            if let Ok(fresh) = self.client.active_announcements().await {
                self.registry.replace_all(fresh).await;
                announcement = self.registry.get(&interaction.custom_id).await;
            }
        }
        let Some(announcement) = announcement else {
            tracing::warn!(custom_id = %interaction.custom_id,
                "Twitch-Live-Klick ohne bekannte Ankündigung");
            return BridgeReply::ephemeral_text("⚠️ Diese Live-Ankündigung ist nicht mehr aktiv.");
        };
        if !is_valid_referral_url(&announcement.referral_url) {
            tracing::warn!(
                streamer = %announcement.streamer_login,
                "Twitch-Live-Klick mit ungueltiger referral_url uebersprungen"
            );
            return BridgeReply::ephemeral_text("⚠️ Diese Live-Ankündigung ist nicht mehr aktiv.");
        }

        // Klick tracken (best effort — der User bekommt seinen Link immer)
        let channel_id = if interaction.channel_id > 0 {
            interaction.channel_id
        } else {
            announcement.channel_id
        };
        let message_id = interaction.message_id.unwrap_or(announcement.message_id);
        if channel_id > 0 && message_id > 0 {
            let idempotency_key = format!("twitch-live-click-{}", interaction.interaction_id);
            if let Err(err) = self
                .client
                .record_link_click(
                    &announcement,
                    interaction.user_id,
                    &interaction.author_name,
                    (interaction.guild_id > 0).then_some(interaction.guild_id),
                    channel_id,
                    message_id,
                    &idempotency_key,
                )
                .await
            {
                tracing::error!(%err, streamer = %announcement.streamer_login,
                    "Twitch-Live-Klick konnte nicht gespeichert werden");
            }
        }

        BridgeReply {
            content: Some(format!(
                "Hier ist dein Twitch-Link für **{}**.",
                announcement.streamer_login
            )),
            components: Some(json!([{ "type": 1, "components": [{
                "type": 2, "style": 5,
                "label": announcement.button_label,
                "url": announcement.referral_url,
            }]}])),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }
}

pub struct SpamLearningHandler {
    pub client: Arc<TwitchApiClient>,
    pub store: SpamLearningStore,
}

fn parse_spam_learning_custom_id(custom_id: &str) -> Option<(&str, &str)> {
    let rest = custom_id.strip_prefix(SPAM_LEARNING_PREFIX)?;
    let (verdict, token) = rest.split_once(':')?;
    match verdict {
        "spam" | "safe" if !token.trim().is_empty() => Some((verdict, token)),
        _ => None,
    }
}

fn learned_components(token: &str) -> Value {
    json!([{
        "type": 1,
        "components": [
            {
                "type": 2,
                "style": 2,
                "label": "Gelernt",
                "custom_id": format!("spam-learning:done-spam:{token}"),
                "disabled": true,
            },
            {
                "type": 2,
                "style": 2,
                "label": "Gelernt",
                "custom_id": format!("spam-learning:done-safe:{token}"),
                "disabled": true,
            },
        ],
    }])
}

#[async_trait::async_trait]
impl InteractionHandler for SpamLearningHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !interaction.author_can_manage_messages && !interaction.author_can_manage_guild {
            return BridgeReply::ephemeral_text("Keine Berechtigung.");
        }

        let Some((verdict, token)) = parse_spam_learning_custom_id(&interaction.custom_id) else {
            return BridgeReply::ephemeral_text("Lernen fehlgeschlagen.");
        };
        let Some(payload) = self.store.take(token) else {
            return BridgeReply::ephemeral_text("Dieser Lernfall ist nicht mehr aktiv.");
        };

        if let Err(err) = self.client.learn_spam_pattern(&payload, verdict).await {
            tracing::error!(%err, verdict, "Twitch-Spam-Lernen fehlgeschlagen");
            return BridgeReply::ephemeral_text("Lernen fehlgeschlagen.");
        }

        BridgeReply {
            components: Some(learned_components(token)),
            update_message: true,
            ..BridgeReply::default()
        }
    }
}

/// Registriert die Twitch-Live-Routen am Router.
pub fn register(
    router: &mut InteractionRouter,
    client: Arc<TwitchApiClient>,
    registry: Arc<TrackingRegistry>,
) {
    router.on_prefix(
        TRACKING_PREFIX,
        Arc::new(TrackingClickHandler { client, registry }),
    );
}

pub fn register_spam_learning(
    router: &mut InteractionRouter,
    client: Arc<TwitchApiClient>,
    store: SpamLearningStore,
) {
    router.on_prefix(
        SPAM_LEARNING_PREFIX,
        Arc::new(SpamLearningHandler { client, store }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Mutex;

    async fn mock_twitch_bot(
        announcements: Value,
        click_response: Value,
    ) -> (
        String,
        Arc<Mutex<Vec<(String, Value)>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let received: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let rec_get = received.clone();
        let rec_post = received.clone();
        let app = axum::Router::new()
            .route(
                "/internal/twitch/v1/live/active-announcements",
                axum::routing::get(move || {
                    let announcements = announcements.clone();
                    let received = rec_get.clone();
                    async move {
                        received
                            .lock()
                            .expect("lock")
                            .push(("GET".into(), Value::Null));
                        axum::Json(announcements)
                    }
                }),
            )
            .route(
                "/internal/twitch/v1/live/link-click",
                axum::routing::post(
                    move |headers: axum::http::HeaderMap, body: axum::Json<Value>| {
                        let received = rec_post.clone();
                        let click_response = click_response.clone();
                        async move {
                            let idem = headers
                                .get("Idempotency-Key")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or_default()
                                .to_string();
                            received
                                .lock()
                                .expect("lock")
                                .push((format!("POST:{idem}"), body.0));
                            axum::Json(click_response)
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), received, handle)
    }

    async fn mock_spam_learning_bot() -> (
        String,
        Arc<Mutex<Vec<(String, Value)>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let received: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let rec_post = received.clone();
        let app = axum::Router::new().route(
            "/internal/twitch/v1/spam-learning",
            axum::routing::post(move |body: axum::Json<Value>| {
                let received = rec_post.clone();
                async move {
                    received.lock().expect("lock").push(("POST".into(), body.0));
                    axum::Json(json!({"ok": true}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), received, handle)
    }

    fn announcement_json() -> Value {
        json!([{
            "streamer_login": "DragSkope",
            "message_id": "555",
            "tracking_token": "tok123",
            "referral_url": "https://twitch.tv/dragskope?ref=x",
            "button_label": "",
            "channel_id": "777",
        }])
    }

    #[test]
    fn custom_id_wie_python() {
        assert_eq!(
            build_custom_id("DragSkope", "tok123"),
            "twitch-live:dragskope:tok123"
        );
        // Sonderzeichen raus, Kürzung, Fallbacks
        assert_eq!(build_custom_id("", ""), "twitch-live:stream:track");
        assert_eq!(
            build_custom_id("abc-def_ghi", &"x".repeat(40)),
            format!("twitch-live:abcdefghi:{}", "x".repeat(32))
        );
    }

    #[test]
    fn spam_learning_custom_id_validierung() {
        assert_eq!(
            parse_spam_learning_custom_id("spam-learning:spam:abc"),
            Some(("spam", "abc"))
        );
        assert_eq!(
            parse_spam_learning_custom_id("spam-learning:safe:abc"),
            Some(("safe", "abc"))
        );
        assert!(parse_spam_learning_custom_id("spam-learning:done-spam:abc").is_none());
        assert!(parse_spam_learning_custom_id("spam-learning:spam:").is_none());
    }

    #[test]
    fn internal_api_base_url_wird_wie_python_gehaertet() {
        assert_eq!(
            normalize_internal_api_base_url("127.0.0.1:8776/internal/twitch/v1", false)
                .expect("normalized"),
            "http://127.0.0.1:8776"
        );
        assert!(normalize_internal_api_base_url("https://u:p@127.0.0.1:8776", false).is_err());
        assert!(normalize_internal_api_base_url("https://example.com", false).is_err());
        assert_eq!(
            normalize_internal_api_base_url("https://example.com/internal/twitch/v1", true)
                .expect("override"),
            "https://example.com"
        );
    }

    #[tokio::test]
    async fn active_announcements_ueberspringt_defekte_referral_urls() {
        let (url, _received, server) = mock_twitch_bot(
            json!([
                {
                    "streamer_login": "bad",
                    "message_id": "1",
                    "tracking_token": "bad",
                    "referral_url": "javascript:alert(1)",
                    "channel_id": "7"
                },
                {
                    "streamer_login": "good",
                    "message_id": "2",
                    "tracking_token": "good",
                    "referral_url": "https://twitch.tv/good",
                    "channel_id": "7"
                }
            ]),
            json!({"ok": true}),
        )
        .await;
        let client = TwitchApiClient::new(url, "tok", Duration::from_secs(5));
        let restored = client.active_announcements().await.expect("announcements");

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].streamer_login, "good");
        server.abort();
    }

    #[tokio::test]
    async fn klick_trackt_und_liefert_referral_link() {
        let (url, received, server) =
            mock_twitch_bot(announcement_json(), json!({"ok": true})).await;
        let client = TwitchApiClient::new(url, "tok", Duration::from_secs(5));
        let registry = TrackingRegistry::new();
        registry
            .replace_all(client.active_announcements().await.expect("announcements"))
            .await;

        let handler = TrackingClickHandler { client, registry };
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "twitch-live:dragskope:tok123".to_string(),
                interaction_id: 999,
                user_id: 42,
                author_name: "tester".to_string(),
                guild_id: 1,
                channel_id: 777,
                message_id: Some(555),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.content.expect("text").contains("dragskope"));
        assert!(reply.ephemeral);
        let components = reply.components.expect("components");
        assert_eq!(
            components[0]["components"][0]["url"],
            "https://twitch.tv/dragskope?ref=x"
        );
        // Default-Label, weil button_label leer war
        assert_eq!(
            components[0]["components"][0]["label"],
            DEFAULT_BUTTON_LABEL
        );

        let sent = received.lock().expect("lock");
        let click = sent
            .iter()
            .find(|(m, _)| m.starts_with("POST:"))
            .expect("click request");
        assert_eq!(click.0, "POST:twitch-live-click-999");
        assert_eq!(click.1["streamer_login"], "dragskope");
        assert_eq!(click.1["discord_user_id"], "42");
        assert_eq!(click.1["message_id"], "555");
        server.abort();
    }

    #[tokio::test]
    async fn spam_learning_klick_postet_verdict_und_deaktiviert_buttons() {
        let (url, received, server) = mock_spam_learning_bot().await;
        let client = TwitchApiClient::new(url, "tok", Duration::from_secs(5));
        let store = SpamLearningStore::new();
        let token = store.insert(SpamLearningPayload {
            pattern: "viewer bot pitch".to_string(),
            pattern_type: "phrase".to_string(),
            source_message: "@demo viewer bot pitch".to_string(),
            source_channel: "demo".to_string(),
            reason: "Score 1".to_string(),
        });

        let handler = SpamLearningHandler { client, store };
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: format!("spam-learning:spam:{token}"),
                author_can_manage_messages: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.update_message);
        let components = reply.components.expect("components");
        assert_eq!(components[0]["components"][0]["label"], "Gelernt");
        assert_eq!(components[0]["components"][0]["disabled"], true);
        let sent = received.lock().expect("lock");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1["verdict"], "spam");
        assert_eq!(sent[0].1["pattern"], "viewer bot pitch");
        assert_eq!(sent[0].1["patternType"], "phrase");
        server.abort();
    }

    #[tokio::test]
    async fn unbekannter_klick_laedt_frisch_nach() {
        let (url, _received, server) =
            mock_twitch_bot(announcement_json(), json!({"ok": true})).await;
        let client = TwitchApiClient::new(url, "tok", Duration::from_secs(5));
        let registry = TrackingRegistry::new(); // leer!
        let handler = TrackingClickHandler { client, registry };
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "twitch-live:dragskope:tok123".to_string(),
                interaction_id: 1,
                user_id: 2,
                channel_id: 777,
                message_id: Some(555),
                ..BridgeInteraction::default()
            })
            .await;
        // Nach Refresh gefunden → Link kommt
        assert!(reply.content.expect("text").contains("dragskope"));
        server.abort();
    }

    #[tokio::test]
    async fn wirklich_unbekannt_gibt_hinweis() {
        let (url, _received, server) = mock_twitch_bot(json!([]), json!({"ok": true})).await;
        let client = TwitchApiClient::new(url, "tok", Duration::from_secs(5));
        let handler = TrackingClickHandler {
            client,
            registry: TrackingRegistry::new(),
        };
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "twitch-live:wer:auchimmer".to_string(),
                ..BridgeInteraction::default()
            })
            .await;
        assert!(reply.content.expect("text").contains("nicht mehr aktiv"));
        server.abort();
    }

    #[tokio::test]
    async fn klick_mit_defekter_referral_url_liefert_keinen_button() {
        let client = TwitchApiClient::new("http://127.0.0.1:9", "tok", Duration::from_secs(1));
        let registry = TrackingRegistry::new();
        registry
            .insert(Announcement {
                streamer_login: "bad".to_string(),
                tracking_token: "tok".to_string(),
                referral_url: "notaurl".to_string(),
                button_label: DEFAULT_BUTTON_LABEL.to_string(),
                channel_id: 1,
                message_id: 2,
            })
            .await;
        let handler = TrackingClickHandler { client, registry };
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "twitch-live:bad:tok".to_string(),
                interaction_id: 1,
                user_id: 2,
                channel_id: 1,
                message_id: Some(2),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.content.expect("text").contains("nicht mehr aktiv"));
        assert!(reply.components.is_none());
    }
}
