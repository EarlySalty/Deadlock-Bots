use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{json, Value};

use super::{FundingSource, SupporterConfig};
use crate::serversync::support_publish::{
    SupportFunding, SUPPORT_BUTTON_BECOME, SUPPORT_BUTTON_MANAGE, SUPPORT_BUTTON_TIP,
    SUPPORT_BUTTON_TWITCH,
};

const INTERNAL_TOKEN_HEADER: &str = "X-Internal-Token";
const CHECKOUT_PATH: &str = "/supporter/v1/checkout";
const TIP_PATH: &str = "/supporter/v1/tip";
const PORTAL_PATH: &str = "/supporter/v1/portal";
const FUNDING_PATH: &str = "/supporter/v1/funding";
const TWITCH_CONNECT_PATH: &str = "/internal/supporter/twitch-connect-link";
const PUBLIC_ORIGIN: &str = "https://deutsche-deadlock-community.de/";
const STRIPE_HOST_SUFFIX: &str = ".stripe.com";
const MISSING_TIMEOUT: &str = "supporter_request_timeout_ms fehlt oder ist ungültig";
const MISSING_STEAM_TOKEN: &str = "interner Token für den Steam Bot fehlt";
const MISSING_TWITCH_TOKEN: &str = "interner Token für das Twitch-Dashboard fehlt";
pub const TIERS: [u32; 3] = [5, 10, 20];

pub const REPLY_FAILED: &str =
    "Das hat gerade nicht geklappt. Versuch es bitte in ein paar Minuten noch einmal.";
pub const REPLY_UNAVAILABLE: &str =
    "Das ist gerade noch nicht freigeschaltet. Schau bitte später noch mal vorbei.";
pub const REPLY_BECOME: &str = "Such dir aus, wie viel du im Monat geben möchtest. Die Links gehören zu deinem Discord-Account, gib sie bitte nicht weiter.\n-# Die Rolle kommt automatisch ein paar Minuten nach der Zahlung.";
pub const REPLY_ALREADY_SUBSCRIBED: &str = "Du hast schon ein laufendes Abo. Ändern oder kündigen kannst du es über „Abo verwalten oder kündigen“.";
pub const REPLY_TIP: &str = "Hier wählst du deinen Betrag selbst, ab 5 €. Dafür bekommst du die Vorteile für 31 Tage. Der Link gehört zu deinem Discord-Account, gib ihn bitte nicht weiter.";
pub const REPLY_MANAGE: &str = "Hier kannst du dein Abo ändern oder kündigen.";
pub const REPLY_NO_SUBSCRIPTION: &str =
    "Zu deinem Discord-Account gibt es gerade kein laufendes Abo.";
pub const REPLY_TWITCH: &str = "Verbinde hier deinen Twitch-Account. Danach prüft der Bot automatisch, ob du bei earlysalty, dach_lock oder einem passenden Streamer abonniert hast.\n-# Der Link gilt nur kurz und nur für dich.";
const LABEL_TIP: &str = "Betrag wählen";
const LABEL_MANAGE: &str = "Abo öffnen";
const LABEL_TWITCH: &str = "Twitch verbinden";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    Url(String),
    NoSubscription,
    AlreadySubscribed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    NotConfigured(&'static str),
    Failed(String),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured(missing) => write!(f, "nicht eingerichtet: {missing}"),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

type LinkResult = Result<LinkOutcome, LinkError>;

#[async_trait::async_trait]
pub trait SupporterLinks: Send + Sync {
    async fn checkout(&self, discord_id: u64, tier: u32) -> LinkResult;
    async fn tip(&self, discord_id: u64) -> LinkResult;
    async fn portal(&self, discord_id: u64) -> LinkResult;
    async fn twitch_connect(&self, discord_id: u64) -> LinkResult;
}

pub struct HttpSupporterLinks {
    http: Option<reqwest::Client>,
    steam_base: String,
    steam_token: Option<String>,
    twitch_base: Option<String>,
    twitch_token: Option<String>,
    goal_cents: i64,
}

impl HttpSupporterLinks {
    pub fn new(config: &SupporterConfig) -> Arc<Self> {
        let links = Self {
            http: http_client(config.request_timeout),
            steam_base: config.steam_api_base.trim_end_matches('/').to_string(),
            steam_token: config.steam_token.clone(),
            twitch_base: config
                .twitch_api_base
                .as_deref()
                .map(|base| base.trim_end_matches('/').to_string()),
            twitch_token: config.twitch_token.clone(),
            goal_cents: config.goal_cents,
        };
        links.log_setup();
        Arc::new(links)
    }

    fn log_setup(&self) {
        tracing::info!(
            steam_base = %self.steam_base,
            twitch_base = ?self.twitch_base,
            "Server-Unterstützer: Knöpfe eingerichtet"
        );
        for (absent, missing) in [
            (self.http.is_none(), MISSING_TIMEOUT),
            (self.steam_token.is_none(), MISSING_STEAM_TOKEN),
            (self.twitch_token.is_none(), MISSING_TWITCH_TOKEN),
        ] {
            if absent {
                tracing::warn!(
                    missing,
                    "Server-Unterstützer: betroffene Knöpfe antworten nur mit einer Fehlermeldung"
                );
            }
        }
        if self.twitch_base.is_none() {
            tracing::warn!(
                "Server-Unterstützer: DL_SUPPORTER_TWITCH_API_BASE fehlt, „Twitch verbinden“ meldet „noch nicht freigeschaltet“"
            );
        }
    }

    fn client(&self) -> Result<&reqwest::Client, LinkError> {
        self.http
            .as_ref()
            .ok_or(LinkError::NotConfigured(MISSING_TIMEOUT))
    }

    async fn post(
        &self,
        base: &str,
        token: &str,
        path: &str,
        body: Value,
    ) -> Result<(StatusCode, Value), LinkError> {
        let response = self
            .client()?
            .post(format!("{base}{path}"))
            .header(INTERNAL_TOKEN_HEADER, token)
            .json(&body)
            .send()
            .await
            .map_err(|error| LinkError::Failed(format!("{path} nicht erreichbar: {error}")))?;
        let status = response.status();
        let body = response.json::<Value>().await.unwrap_or(Value::Null);
        Ok((status, body))
    }

    async fn steam_link(&self, path: &str, body: Value) -> LinkResult {
        let token = required(self.steam_token.as_deref(), MISSING_STEAM_TOKEN)?;
        let (status, body) = self.post(&self.steam_base, token, path, body).await?;
        link_outcome(path, status, &body, is_stripe_url)
    }
}

fn required<'a>(value: Option<&'a str>, missing: &'static str) -> Result<&'a str, LinkError> {
    value.ok_or(LinkError::NotConfigured(missing))
}

fn http_client(timeout: Option<Duration>) -> Option<reqwest::Client> {
    let limit = timeout?;
    reqwest::Client::builder()
        .timeout(limit)
        .build()
        .map_err(|error| {
            tracing::warn!(%error, "Server-Unterstützer: HTTP-Client nicht erstellt");
        })
        .ok()
}

#[async_trait::async_trait]
impl SupporterLinks for HttpSupporterLinks {
    async fn checkout(&self, discord_id: u64, tier: u32) -> LinkResult {
        self.steam_link(
            CHECKOUT_PATH,
            json!({ "discord_id": discord_id, "tier": tier }),
        )
        .await
    }

    async fn tip(&self, discord_id: u64) -> LinkResult {
        self.steam_link(TIP_PATH, json!({ "discord_id": discord_id }))
            .await
    }

    async fn portal(&self, discord_id: u64) -> LinkResult {
        self.steam_link(PORTAL_PATH, json!({ "discord_id": discord_id }))
            .await
    }

    async fn twitch_connect(&self, discord_id: u64) -> LinkResult {
        let Some(base) = self.twitch_base.as_deref() else {
            return Ok(LinkOutcome::Unavailable);
        };
        let token = required(self.twitch_token.as_deref(), MISSING_TWITCH_TOKEN)?;
        let (status, body) = self
            .post(
                base,
                token,
                TWITCH_CONNECT_PATH,
                json!({ "discord_id": discord_id }),
            )
            .await?;
        link_outcome(TWITCH_CONNECT_PATH, status, &body, is_public_url)
    }
}

#[async_trait::async_trait]
impl FundingSource for HttpSupporterLinks {
    async fn funding(&self) -> Result<SupportFunding, String> {
        let token = required(self.steam_token.as_deref(), MISSING_STEAM_TOKEN)
            .map_err(|error| error.to_string())?;
        let response = self
            .client()
            .map_err(|error| error.to_string())?
            .get(format!("{}{FUNDING_PATH}", self.steam_base))
            .header(INTERNAL_TOKEN_HEADER, token)
            .send()
            .await
            .map_err(|error| format!("{FUNDING_PATH} nicht erreichbar: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("{FUNDING_PATH} antwortet {status}"));
        }
        let body = response
            .json::<Value>()
            .await
            .map_err(|error| format!("{FUNDING_PATH} Antwort unlesbar: {error}"))?;
        parse_funding(&body, self.goal_cents)
    }

    fn missing_config(&self) -> Option<&'static str> {
        if self.http.is_none() {
            return Some(MISSING_TIMEOUT);
        }
        self.steam_token.is_none().then_some(MISSING_STEAM_TOKEN)
    }

    fn goal_cents(&self) -> i64 {
        self.goal_cents
    }
}

fn parse_funding(body: &Value, goal_cents: i64) -> Result<SupportFunding, String> {
    let raised_cents = body
        .get("monthly_cents_active")
        .and_then(Value::as_i64)
        .filter(|cents| *cents >= 0)
        .ok_or_else(|| format!("{FUNDING_PATH} ohne monthly_cents_active"))?;
    Ok(SupportFunding {
        raised_cents,
        goal_cents,
    })
}

fn link_outcome(
    path: &str,
    status: StatusCode,
    body: &Value,
    accept: fn(&str) -> bool,
) -> LinkResult {
    let code = body.get("error").and_then(Value::as_str).unwrap_or("");
    if status == StatusCode::NOT_FOUND && code == "no_subscription" {
        return Ok(LinkOutcome::NoSubscription);
    }
    if status == StatusCode::CONFLICT && code == "already_subscribed" {
        return Ok(LinkOutcome::AlreadySubscribed);
    }
    if !status.is_success() {
        return Err(LinkError::Failed(format!(
            "{path} antwortet {status} {code}"
        )));
    }
    body.get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| accept(url))
        .map(|url| LinkOutcome::Url(url.to_string()))
        .ok_or_else(|| LinkError::Failed(format!("{path} liefert keinen gültigen Link")))
}

fn url_host(url: &str) -> Option<&str> {
    if url.chars().any(|ch| ch.is_whitespace() || ch.is_control()) || url.len() > 512 {
        return None;
    }
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.contains('@') && !host.contains(':')).then_some(host)
}

fn is_stripe_url(url: &str) -> bool {
    url_host(url).is_some_and(|host| host.ends_with(STRIPE_HOST_SUFFIX))
}

fn is_public_url(url: &str) -> bool {
    url_host(url).is_some() && url.starts_with(PUBLIC_ORIGIN)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportButton {
    Become,
    Tip,
    Manage,
    Twitch,
}

impl SupportButton {
    fn name(self) -> &'static str {
        match self {
            Self::Become => "checkout",
            Self::Tip => "tip",
            Self::Manage => "portal",
            Self::Twitch => "twitch",
        }
    }
}

struct SupportButtonHandler {
    links: Arc<dyn SupporterLinks>,
    button: SupportButton,
}

#[async_trait::async_trait]
impl dl_discord::InteractionHandler for SupportButtonHandler {
    async fn handle(&self, interaction: dl_discord::BridgeInteraction) -> dl_discord::BridgeReply {
        let user_id = interaction.user_id;
        let result = match self.button {
            SupportButton::Become => become_reply(self.links.as_ref(), user_id).await,
            SupportButton::Tip => single_reply(self.links.tip(user_id).await, REPLY_TIP, LABEL_TIP),
            SupportButton::Manage => {
                single_reply(self.links.portal(user_id).await, REPLY_MANAGE, LABEL_MANAGE)
            }
            SupportButton::Twitch => single_reply(
                self.links.twitch_connect(user_id).await,
                REPLY_TWITCH,
                LABEL_TWITCH,
            ),
        };
        match result {
            Ok(reply) => {
                tracing::info!(
                    user_id,
                    button = self.button.name(),
                    "Unterstützer-Knopf beantwortet"
                );
                reply
            }
            Err(LinkError::NotConfigured(missing)) => {
                tracing::debug!(
                    user_id,
                    button = self.button.name(),
                    missing,
                    "Unterstützer-Knopf: nicht eingerichtet"
                );
                dl_discord::BridgeReply::ephemeral_text(REPLY_FAILED)
            }
            Err(LinkError::Failed(error)) => {
                tracing::warn!(user_id, button = self.button.name(), %error, "Unterstützer-Knopf: Link nicht erzeugt");
                dl_discord::BridgeReply::ephemeral_text(REPLY_FAILED)
            }
        }
    }
}

async fn become_reply(
    links: &dyn SupporterLinks,
    user_id: u64,
) -> Result<dl_discord::BridgeReply, LinkError> {
    let (five, ten, twenty) = tokio::join!(
        links.checkout(user_id, TIERS[0]),
        links.checkout(user_id, TIERS[1]),
        links.checkout(user_id, TIERS[2]),
    );
    let mut buttons = Vec::new();
    let mut errors = Vec::new();
    for (tier, outcome) in TIERS.iter().zip([five, ten, twenty]) {
        match outcome {
            Ok(LinkOutcome::Url(url)) => {
                buttons.push(link_button(&format!("{tier} € im Monat"), &url))
            }
            Ok(LinkOutcome::AlreadySubscribed) => {
                return Ok(dl_discord::BridgeReply::ephemeral_text(
                    REPLY_ALREADY_SUBSCRIBED,
                ))
            }
            Ok(other) => errors.push(LinkError::Failed(format!("{tier}: {other:?}"))),
            Err(LinkError::Failed(error)) => {
                errors.push(LinkError::Failed(format!("{tier}: {error}")))
            }
            Err(not_configured) => errors.push(not_configured),
        }
    }
    if buttons.is_empty() {
        return Err(combine_errors(errors));
    }
    if !errors.is_empty() {
        tracing::warn!(user_id, errors = ?errors, "Unterstützer-Abo: nicht alle Stufen verfügbar");
    }
    Ok(link_reply(REPLY_BECOME, buttons))
}

fn combine_errors(errors: Vec<LinkError>) -> LinkError {
    let failed: Vec<String> = errors
        .iter()
        .filter_map(|error| match error {
            LinkError::Failed(message) => Some(message.clone()),
            LinkError::NotConfigured(_) => None,
        })
        .collect();
    if !failed.is_empty() {
        return LinkError::Failed(failed.join("; "));
    }
    errors
        .into_iter()
        .next()
        .unwrap_or_else(|| LinkError::Failed("keine Abo-Stufe beantwortet".to_string()))
}

fn single_reply(
    outcome: LinkResult,
    text: &str,
    label: &str,
) -> Result<dl_discord::BridgeReply, LinkError> {
    let reply = match outcome? {
        LinkOutcome::Url(url) => return Ok(link_reply(text, vec![link_button(label, &url)])),
        LinkOutcome::NoSubscription => REPLY_NO_SUBSCRIPTION,
        LinkOutcome::AlreadySubscribed => REPLY_ALREADY_SUBSCRIBED,
        LinkOutcome::Unavailable => REPLY_UNAVAILABLE,
    };
    Ok(dl_discord::BridgeReply::ephemeral_text(reply))
}

fn link_button(label: &str, url: &str) -> Value {
    json!({ "type": 2, "style": 5, "label": label, "url": url })
}

fn link_reply(text: &str, buttons: Vec<Value>) -> dl_discord::BridgeReply {
    dl_discord::BridgeReply {
        content: Some(text.to_string()),
        components: Some(json!([{ "type": 1, "components": buttons }])),
        ephemeral: true,
        allowed_mentions: Some(json!({ "parse": [] })),
        ..dl_discord::BridgeReply::default()
    }
}

pub fn register(router: &mut dl_discord::InteractionRouter, links: Arc<dyn SupporterLinks>) {
    for (custom_id, button) in [
        (SUPPORT_BUTTON_BECOME, SupportButton::Become),
        (SUPPORT_BUTTON_TIP, SupportButton::Tip),
        (SUPPORT_BUTTON_MANAGE, SupportButton::Manage),
        (SUPPORT_BUTTON_TWITCH, SupportButton::Twitch),
    ] {
        router.on_custom_id(
            custom_id,
            Arc::new(SupportButtonHandler {
                links: links.clone(),
                button,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::InteractionHandler;

    struct Fixed {
        checkout: fn(u32) -> LinkResult,
        other: LinkResult,
    }

    #[async_trait::async_trait]
    impl SupporterLinks for Fixed {
        async fn checkout(&self, _discord_id: u64, tier: u32) -> LinkResult {
            (self.checkout)(tier)
        }
        async fn tip(&self, _discord_id: u64) -> LinkResult {
            self.other.clone()
        }
        async fn portal(&self, _discord_id: u64) -> LinkResult {
            self.other.clone()
        }
        async fn twitch_connect(&self, _discord_id: u64) -> LinkResult {
            self.other.clone()
        }
    }

    fn failure() -> LinkError {
        LinkError::Failed("down".to_string())
    }

    async fn click(links: Fixed, button: SupportButton) -> dl_discord::BridgeReply {
        click_shared(Arc::new(links), button).await
    }

    async fn click_shared(
        links: Arc<dyn SupporterLinks>,
        button: SupportButton,
    ) -> dl_discord::BridgeReply {
        let handler = SupportButtonHandler { links, button };
        handler
            .handle(dl_discord::BridgeInteraction {
                user_id: 42,
                ..dl_discord::BridgeInteraction::default()
            })
            .await
    }

    fn reply_urls(reply: &dl_discord::BridgeReply) -> Vec<String> {
        reply
            .components
            .as_ref()
            .and_then(|rows| rows[0]["components"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|button| button["url"].as_str().map(str::to_string))
            .collect()
    }

    #[tokio::test]
    async fn supporter_werden_zeigt_nur_erreichbare_stufen() {
        let links = Fixed {
            checkout: |tier| match tier {
                10 => Err(failure()),
                _ => Ok(LinkOutcome::Url(format!(
                    "https://checkout.stripe.com/{tier}"
                ))),
            },
            other: Err(failure()),
        };
        let reply = click(links, SupportButton::Become).await;
        assert!(reply.ephemeral);
        assert_eq!(
            reply_urls(&reply),
            vec![
                "https://checkout.stripe.com/5".to_string(),
                "https://checkout.stripe.com/20".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn fehler_und_kein_abo_werden_freundlich_beantwortet() {
        let down = click(
            Fixed {
                checkout: |_| Err(failure()),
                other: Err(failure()),
            },
            SupportButton::Become,
        )
        .await;
        assert_eq!(down.content.as_deref(), Some(REPLY_FAILED));
        assert!(down.ephemeral);
        let none = click(
            Fixed {
                checkout: |_| Err(failure()),
                other: Ok(LinkOutcome::NoSubscription),
            },
            SupportButton::Manage,
        )
        .await;
        assert_eq!(none.content.as_deref(), Some(REPLY_NO_SUBSCRIPTION));
        let twitch = click(
            Fixed {
                checkout: |_| Err(failure()),
                other: Ok(LinkOutcome::Url(
                    "https://deutsche-deadlock-community.de/twitch/x".to_string(),
                )),
            },
            SupportButton::Twitch,
        )
        .await;
        assert_eq!(
            reply_urls(&twitch),
            vec!["https://deutsche-deadlock-community.de/twitch/x".to_string()]
        );
    }

    #[tokio::test]
    async fn fehlende_einrichtung_bleibt_ohne_anfrage_und_ohne_falschen_host() {
        let config = SupporterConfig::from_lookup(|key| match key {
            "TWITCH_INTERNAL_API_TOKEN" => Some("token".to_string()),
            "DL_SUPPORTER_REQUEST_TIMEOUT_MS" => Some("0".to_string()),
            _ => None,
        });
        assert_eq!(config.request_timeout, None);
        assert_eq!(config.twitch_api_base, None);
        let links = HttpSupporterLinks::new(&config);
        assert_eq!(
            links.checkout(42, 5).await,
            Err(LinkError::NotConfigured(MISSING_TIMEOUT))
        );
        assert_eq!(links.missing_config(), Some(MISSING_TIMEOUT));
        assert_eq!(links.twitch_connect(42).await, Ok(LinkOutcome::Unavailable));
        let become_reply =
            click_shared(HttpSupporterLinks::new(&config), SupportButton::Become).await;
        assert_eq!(become_reply.content.as_deref(), Some(REPLY_FAILED));
        let twitch = click_shared(HttpSupporterLinks::new(&config), SupportButton::Twitch).await;
        assert_eq!(twitch.content.as_deref(), Some(REPLY_UNAVAILABLE));
        assert!(twitch.ephemeral);
    }

    #[test]
    fn antworten_der_dienste_werden_streng_gelesen() {
        let ok = json!({ "url": "https://checkout.stripe.com/c/pay/x" });
        assert_eq!(
            link_outcome("p", StatusCode::OK, &ok, is_stripe_url),
            Ok(LinkOutcome::Url(
                "https://checkout.stripe.com/c/pay/x".to_string()
            ))
        );
        let foreign = json!({ "url": "https://evil.example/stripe.com" });
        assert!(link_outcome("p", StatusCode::OK, &foreign, is_stripe_url).is_err());
        let admin = json!({ "url": "https://admin.deutsche-deadlock-community.de/twitch/x" });
        assert!(link_outcome("p", StatusCode::OK, &admin, is_public_url).is_err());
        let none = json!({ "error": "no_subscription" });
        assert_eq!(
            link_outcome("p", StatusCode::NOT_FOUND, &none, is_stripe_url),
            Ok(LinkOutcome::NoSubscription)
        );
        let taken = json!({ "error": "already_subscribed" });
        assert_eq!(
            link_outcome("p", StatusCode::CONFLICT, &taken, is_stripe_url),
            Ok(LinkOutcome::AlreadySubscribed)
        );
        assert!(link_outcome("p", StatusCode::BAD_GATEWAY, &json!({}), is_stripe_url).is_err());
        assert_eq!(
            parse_funding(
                &json!({ "monthly_cents_active": 1500, "supporters": 3 }),
                5000
            ),
            Ok(SupportFunding {
                raised_cents: 1500,
                goal_cents: 5000
            })
        );
        assert!(parse_funding(&json!({}), 5000).is_err());
    }
}
