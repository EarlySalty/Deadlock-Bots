//! Button "Twitch verknüpfen" im Verify-Panel (#deadlock-rang).
//!
//! Ein Klick holt beim Dashboard (:8766, loopback, gleicher interner Token wie
//! der Broker) einen frischen Discord-Link für den delegierten Flow mit Scope
//! `identify connections` und schickt ihn nur dem Klickenden. Den Rest
//! (Callback, Speichern, Bestätigungsseite) erledigt das Dashboard; der Bot
//! speichert nichts und sieht kein Token.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::rang_guide_publish::TWITCH_LINK_OPEN_CUSTOM_ID;

const INITIATE_PATH: &str = "/internal/v1/discord/twitch-link/initiate";
const INTERNAL_TOKEN_HEADER: &str = "X-Internal-Token";
/// Discord erwartet die Interaction-Antwort binnen 3 Sekunden.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(2_500);

pub const TWITCH_LINK_INTRO: &str = "Hier verknüpfst du dein Twitch-Konto mit dem Server. Discord fragt dich gleich, ob wir sehen dürfen, welche Konten du mit Discord verbunden hast. Wir merken uns davon nur dein Twitch-Konto, damit dich der Bot in Partner-Streams erkennt.\n-# Der Link ist ein paar Stunden gültig.";
pub const TWITCH_LINK_FAILED: &str =
    "Das hat gerade nicht geklappt. Versuch es bitte in einer Minute noch einmal.";
pub const TWITCH_LINK_BUTTON_LABEL: &str = "Twitch verknüpfen";

/// Liefert den Discord-Link für die Twitch-Verknüpfung.
#[async_trait::async_trait]
pub trait TwitchLinkUrlSource: Send + Sync {
    async fn authorize_url(&self) -> Result<String, String>;
}

/// Holt den Link per `POST /internal/v1/discord/twitch-link/initiate`.
pub struct DashboardTwitchLinkClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl DashboardTwitchLinkClient {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty()),
        }
    }
}

#[async_trait::async_trait]
impl TwitchLinkUrlSource for DashboardTwitchLinkClient {
    async fn authorize_url(&self) -> Result<String, String> {
        let Some(token) = self.token.as_deref() else {
            return Err("kein interner Token fuer das Dashboard".to_string());
        };
        let response = self
            .http
            .post(format!("{}{INITIATE_PATH}", self.base_url))
            .header(INTERNAL_TOKEN_HEADER, token)
            .json(&json!({}))
            .send()
            .await
            .map_err(|err| format!("Dashboard nicht erreichbar: {err}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("Dashboard antwortet {status}"));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|err| format!("Dashboard-Antwort unlesbar: {err}"))?;
        parse_authorize_url(&body)
    }
}

fn parse_authorize_url(body: &Value) -> Result<String, String> {
    body.get("authorize_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| url.starts_with("https://"))
        .map(str::to_string)
        .ok_or_else(|| "Dashboard-Antwort ohne gueltigen Link".to_string())
}

/// Nur für den Klickenden sichtbare Antwort mit Link-Button.
pub fn twitch_link_reply(authorize_url: &str) -> dl_discord::BridgeReply {
    dl_discord::BridgeReply {
        content: Some(TWITCH_LINK_INTRO.to_string()),
        components: Some(json!([{
            "type": 1,
            "components": [{
                "type": 2,
                "style": 5,
                "label": TWITCH_LINK_BUTTON_LABEL,
                "url": authorize_url,
            }],
        }])),
        ephemeral: true,
        allowed_mentions: Some(json!({ "parse": [] })),
        ..dl_discord::BridgeReply::default()
    }
}

pub struct TwitchLinkButtonHandler {
    source: Arc<dyn TwitchLinkUrlSource>,
}

impl TwitchLinkButtonHandler {
    pub fn new(source: Arc<dyn TwitchLinkUrlSource>) -> Self {
        Self { source }
    }
}

#[async_trait::async_trait]
impl dl_discord::InteractionHandler for TwitchLinkButtonHandler {
    async fn handle(&self, interaction: dl_discord::BridgeInteraction) -> dl_discord::BridgeReply {
        match self.source.authorize_url().await {
            Ok(url) => {
                tracing::info!(
                    user_id = interaction.user_id,
                    "Twitch-Verknuepfung: Link ausgegeben"
                );
                twitch_link_reply(&url)
            }
            Err(error) => {
                tracing::warn!(
                    user_id = interaction.user_id,
                    %error,
                    "Twitch-Verknuepfung: Link konnte nicht erzeugt werden"
                );
                dl_discord::BridgeReply::ephemeral_text(TWITCH_LINK_FAILED)
            }
        }
    }
}

pub fn register(router: &mut dl_discord::InteractionRouter, source: Arc<dyn TwitchLinkUrlSource>) {
    router.on_custom_id(
        TWITCH_LINK_OPEN_CUSTOM_ID,
        Arc::new(TwitchLinkButtonHandler::new(source)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Result<String, String>);

    #[async_trait::async_trait]
    impl TwitchLinkUrlSource for Fixed {
        async fn authorize_url(&self) -> Result<String, String> {
            self.0.clone()
        }
    }

    fn click() -> dl_discord::BridgeInteraction {
        dl_discord::BridgeInteraction {
            custom_id: TWITCH_LINK_OPEN_CUSTOM_ID.to_string(),
            user_id: 42,
            ..dl_discord::BridgeInteraction::default()
        }
    }

    #[tokio::test]
    async fn klick_liefert_ephemeren_link_button() {
        let url = "https://discord.com/api/v10/oauth2/authorize?scope=identify+connections";
        let handler = TwitchLinkButtonHandler::new(Arc::new(Fixed(Ok(url.to_string()))));
        let reply = dl_discord::InteractionHandler::handle(&handler, click()).await;
        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(TWITCH_LINK_INTRO));
        let components = reply.components.expect("components");
        assert_eq!(components[0]["components"][0]["style"], json!(5));
        assert_eq!(components[0]["components"][0]["url"], url);
        assert_eq!(reply.allowed_mentions, Some(json!({"parse": []})));
    }

    #[tokio::test]
    async fn fehler_liefert_kurzen_ephemeren_hinweis() {
        let handler = TwitchLinkButtonHandler::new(Arc::new(Fixed(Err("weg".to_string()))));
        let reply = dl_discord::InteractionHandler::handle(&handler, click()).await;
        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(TWITCH_LINK_FAILED));
        assert!(reply.components.is_none());
    }

    #[test]
    fn texte_ohne_fachwoerter_und_gedankenstriche() {
        for text in [
            TWITCH_LINK_INTRO,
            TWITCH_LINK_FAILED,
            TWITCH_LINK_BUTTON_LABEL,
        ] {
            assert!(!text.contains('—') && !text.contains('–'), "{text}");
            for word in ["OAuth", "Scope", "Connection", "Register", "Token"] {
                assert!(!text.contains(word), "{word} in {text}");
            }
        }
    }

    #[test]
    fn link_aus_dashboard_antwort() {
        assert_eq!(
            parse_authorize_url(&json!({"authorize_url": "https://discord.com/x"})),
            Ok("https://discord.com/x".to_string())
        );
        assert!(parse_authorize_url(&json!({"authorize_url": "javascript:alert(1)"})).is_err());
        assert!(parse_authorize_url(&json!({})).is_err());
    }

    #[test]
    fn klick_ist_im_router_registriert() {
        let mut router = dl_discord::InteractionRouter::new();
        register(&mut router, Arc::new(Fixed(Err(String::new()))));
        assert!(router
            .resolve_component(TWITCH_LINK_OPEN_CUSTOM_ID)
            .is_some());
    }

    #[tokio::test]
    async fn client_ohne_token_fragt_nicht_an() {
        let client = DashboardTwitchLinkClient::new("http://127.0.0.1:9", None);
        assert!(client.authorize_url().await.is_err());
    }
}
