use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{json, Value};

use super::{FundingSource, SupporterConfig};
use crate::serversync::support_publish::{
    tier_button_id, SupportFunding, SupportPanelOptions, SUPPORT_BUTTON_BECOME,
    SUPPORT_BUTTON_MANAGE, SUPPORT_BUTTON_TIP, SUPPORT_BUTTON_TWITCH, SUPPORT_TIER_BUTTON_PREFIX,
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

pub const REPLY_FAILED: &str =
    "Das hat gerade nicht geklappt. Versuch es bitte in ein paar Minuten noch einmal.";
pub const REPLY_UNAVAILABLE: &str =
    "Das ist gerade noch nicht freigeschaltet. Schau bitte später noch mal vorbei.";
pub const REPLY_BECOME: &str = "Such dir aus, wie viel du im Monat geben möchtest. Danach bekommst du deinen persönlichen Zahlungslink.";
pub const REPLY_TIER: &str = "Hier geht es zur Zahlung. Der Link gehört zu deinem Discord-Account, gib ihn bitte nicht weiter.\n-# Die Rolle kommt automatisch ein paar Minuten nach der Zahlung.";
pub const REPLY_TIER_GONE: &str =
    "Diesen Betrag gibt es so nicht mehr. Klick bitte noch einmal auf „Supporter werden“.";
pub const REPLY_ALREADY_SUBSCRIBED: &str = "Du hast schon ein laufendes Abo. Ändern oder kündigen kannst du es über „Abo verwalten oder kündigen“.";
pub const REPLY_TIP: &str = "Hier wählst du deinen Betrag selbst, ab 5 €. Dafür bekommst du die Vorteile für 31 Tage. Der Link gehört zu deinem Discord-Account, gib ihn bitte nicht weiter.";
pub const REPLY_MANAGE: &str = "Hier kannst du dein Abo ändern oder kündigen.";
pub const REPLY_NO_SUBSCRIPTION: &str =
    "Zu deinem Discord-Account gibt es gerade kein laufendes Abo.";
pub const REPLY_TWITCH: &str = "Verbinde hier deinen Twitch-Account. Danach prüft der Bot automatisch, ob du bei earlysalty, dach_lock oder einem passenden Streamer abonniert hast.\n-# Der Link gilt nur kurz und nur für dich.";
const LABEL_TIP: &str = "Betrag wählen";
const LABEL_PAY: &str = "Weiter zur Zahlung";
const LABEL_MANAGE: &str = "Abo öffnen";
const LABEL_TWITCH: &str = "Twitch verbinden";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    Url(String),
    NoSubscription,
    AlreadySubscribed,
    Unavailable,
    UnknownTier,
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
    panel: SupportPanelOptions,
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
            panel: config.panel_options(),
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
        let body = match response.json::<Value>().await {
            Ok(body) => body,
            Err(error) if status.is_success() => {
                return Err(LinkError::Failed(format!(
                    "{path} antwortet {status} mit unlesbarem Inhalt: {error}"
                )))
            }
            Err(_) => Value::Null,
        };
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
            let code = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|body| {
                    body.get("error")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            return Err(format!("{FUNDING_PATH} antwortet {status} {code}"));
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

    fn panel_options(&self) -> SupportPanelOptions {
        self.panel.clone()
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
    match (status, code) {
        (StatusCode::NOT_FOUND, "no_subscription") => return Ok(LinkOutcome::NoSubscription),
        (StatusCode::CONFLICT, "already_subscribed") => return Ok(LinkOutcome::AlreadySubscribed),
        (StatusCode::BAD_REQUEST, "unknown_tier") => {
            tracing::error!(
                path,
                "Unterstützer-Stufe fehlt im Steam Bot: supporter_tiers_eur und die Stripe-Preise passen nicht zusammen"
            );
            return Ok(LinkOutcome::UnknownTier);
        }
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "supporter_not_configured" | "tip_not_configured" | "prices_unverified",
        ) => {
            tracing::warn!(
                path,
                code,
                "Unterstützer-Link: Steam Bot meldet nicht eingerichtet"
            );
            return Ok(LinkOutcome::Unavailable);
        }
        (StatusCode::UNAUTHORIZED, _)
        | (StatusCode::SERVICE_UNAVAILABLE, "auth_not_configured") => {
            tracing::error!(
                path,
                %status,
                code,
                "Unterstützer-Link: interner Token wird nicht angenommen"
            );
        }
        _ => {}
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
    Tier,
    Tip,
    Manage,
    Twitch,
}

impl SupportButton {
    fn name(self) -> &'static str {
        match self {
            Self::Become => "become",
            Self::Tier => "checkout",
            Self::Tip => "tip",
            Self::Manage => "portal",
            Self::Twitch => "twitch",
        }
    }
}

struct SupportButtonHandler {
    links: Arc<dyn SupporterLinks>,
    button: SupportButton,
    tiers_eur: Arc<[u32]>,
}

#[async_trait::async_trait]
impl dl_discord::InteractionHandler for SupportButtonHandler {
    async fn handle(&self, interaction: dl_discord::BridgeInteraction) -> dl_discord::BridgeReply {
        let user_id = interaction.user_id;
        let result = match self.button {
            SupportButton::Become => Ok(tier_picker(&self.tiers_eur)),
            SupportButton::Tier => {
                tier_reply(
                    self.links.as_ref(),
                    user_id,
                    &interaction.custom_id,
                    &self.tiers_eur,
                )
                .await
            }
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
                tracing::warn!(
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

fn tier_picker(tiers_eur: &[u32]) -> dl_discord::BridgeReply {
    let buttons = tiers_eur
        .iter()
        .map(|eur| {
            json!({
                "type": 2,
                "style": 1,
                "label": format!("{eur} € im Monat"),
                "custom_id": tier_button_id(*eur),
            })
        })
        .collect();
    button_reply(REPLY_BECOME, buttons)
}

fn tier_from_custom_id(custom_id: &str, tiers_eur: &[u32]) -> Option<u32> {
    custom_id
        .strip_prefix(SUPPORT_TIER_BUTTON_PREFIX)?
        .parse::<u32>()
        .ok()
        .filter(|eur| tiers_eur.contains(eur))
}

async fn tier_reply(
    links: &dyn SupporterLinks,
    user_id: u64,
    custom_id: &str,
    tiers_eur: &[u32],
) -> Result<dl_discord::BridgeReply, LinkError> {
    let Some(eur) = tier_from_custom_id(custom_id, tiers_eur) else {
        tracing::warn!(
            user_id,
            custom_id,
            "Unterstützer-Stufe nicht mehr angeboten, alter Knopf"
        );
        return Ok(dl_discord::BridgeReply::ephemeral_text(REPLY_TIER_GONE));
    };
    let label = format!("{LABEL_PAY} ({eur} € im Monat)");
    single_reply(links.checkout(user_id, eur).await, REPLY_TIER, &label)
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
        LinkOutcome::UnknownTier => REPLY_TIER_GONE,
    };
    Ok(dl_discord::BridgeReply::ephemeral_text(reply))
}

fn link_button(label: &str, url: &str) -> Value {
    json!({ "type": 2, "style": 5, "label": label, "url": url })
}

fn link_reply(text: &str, buttons: Vec<Value>) -> dl_discord::BridgeReply {
    button_reply(text, buttons)
}

fn button_reply(text: &str, buttons: Vec<Value>) -> dl_discord::BridgeReply {
    dl_discord::BridgeReply {
        content: Some(text.to_string()),
        components: Some(json!([{ "type": 1, "components": buttons }])),
        ephemeral: true,
        allowed_mentions: Some(json!({ "parse": [] })),
        ..dl_discord::BridgeReply::default()
    }
}

pub fn register(
    router: &mut dl_discord::InteractionRouter,
    links: Arc<dyn SupporterLinks>,
    tiers_eur: &[u32],
) {
    let tiers_eur: Arc<[u32]> = Arc::from(tiers_eur);
    let handler = |button| {
        Arc::new(SupportButtonHandler {
            links: links.clone(),
            button,
            tiers_eur: tiers_eur.clone(),
        })
    };
    for (custom_id, button) in [
        (SUPPORT_BUTTON_BECOME, SupportButton::Become),
        (SUPPORT_BUTTON_TIP, SupportButton::Tip),
        (SUPPORT_BUTTON_MANAGE, SupportButton::Manage),
        (SUPPORT_BUTTON_TWITCH, SupportButton::Twitch),
    ] {
        router.on_custom_id(custom_id, handler(button));
    }
    router.on_prefix(SUPPORT_TIER_BUTTON_PREFIX, handler(SupportButton::Tier));
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::InteractionHandler;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixed {
        checkout: fn(u32) -> LinkResult,
        other: LinkResult,
        checkouts: AtomicUsize,
    }

    fn fixed(checkout: fn(u32) -> LinkResult, other: LinkResult) -> Fixed {
        Fixed {
            checkout,
            other,
            checkouts: AtomicUsize::new(0),
        }
    }

    #[async_trait::async_trait]
    impl SupporterLinks for Fixed {
        async fn checkout(&self, _discord_id: u64, tier: u32) -> LinkResult {
            self.checkouts.fetch_add(1, Ordering::SeqCst);
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

    const TIERS: [u32; 4] = [5, 10, 20, 50];

    fn failure() -> LinkError {
        LinkError::Failed("down".to_string())
    }

    fn stripe(tier: u32) -> LinkResult {
        Ok(LinkOutcome::Url(format!(
            "https://checkout.stripe.com/{tier}"
        )))
    }

    async fn click(
        links: Arc<dyn SupporterLinks>,
        button: SupportButton,
        custom_id: &str,
    ) -> dl_discord::BridgeReply {
        let handler = SupportButtonHandler {
            links,
            button,
            tiers_eur: Arc::from(&TIERS[..]),
        };
        handler
            .handle(dl_discord::BridgeInteraction {
                user_id: 42,
                custom_id: custom_id.to_string(),
                ..dl_discord::BridgeInteraction::default()
            })
            .await
    }

    fn reply_buttons(reply: &dl_discord::BridgeReply) -> Vec<Value> {
        reply
            .components
            .as_ref()
            .and_then(|rows| rows[0]["components"].as_array().cloned())
            .unwrap_or_default()
    }

    fn reply_urls(reply: &dl_discord::BridgeReply) -> Vec<String> {
        reply_buttons(reply)
            .iter()
            .filter_map(|button| button["url"].as_str().map(str::to_string))
            .collect()
    }

    #[tokio::test]
    async fn supporter_werden_zeigt_alle_stufen_ohne_checkout() {
        let links = Arc::new(fixed(stripe, Err(failure())));
        let reply = click(links.clone(), SupportButton::Become, SUPPORT_BUTTON_BECOME).await;
        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(REPLY_BECOME));
        let ids: Vec<String> = reply_buttons(&reply)
            .iter()
            .filter_map(|button| button["custom_id"].as_str().map(str::to_string))
            .collect();
        assert_eq!(
            ids,
            vec![
                "support:tier:5",
                "support:tier:10",
                "support:tier:20",
                "support:tier:50"
            ]
        );
        assert_eq!(links.checkouts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stufenklick_holt_genau_einen_link() {
        let links = Arc::new(fixed(stripe, Err(failure())));
        let reply = click(links.clone(), SupportButton::Tier, "support:tier:50").await;
        assert_eq!(reply.content.as_deref(), Some(REPLY_TIER));
        assert_eq!(
            reply_urls(&reply),
            vec!["https://checkout.stripe.com/50".to_string()]
        );
        assert_eq!(links.checkouts.load(Ordering::SeqCst), 1);
        for stale in [
            "support:tier:3",
            "support:tier:",
            "support:tier:x",
            "support:tier:-5",
        ] {
            let gone = click(links.clone(), SupportButton::Tier, stale).await;
            assert_eq!(gone.content.as_deref(), Some(REPLY_TIER_GONE), "{stale}");
        }
        assert_eq!(links.checkouts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stufenklick_beantwortet_jede_antwort_des_steam_bots() {
        type Checkout = fn(u32) -> LinkResult;
        let cases: [(Checkout, &str); 4] = [
            (|_| Err(LinkError::Failed("down".to_string())), REPLY_FAILED),
            (
                |_| Ok(LinkOutcome::AlreadySubscribed),
                REPLY_ALREADY_SUBSCRIBED,
            ),
            (|_| Ok(LinkOutcome::UnknownTier), REPLY_TIER_GONE),
            (|_| Ok(LinkOutcome::Unavailable), REPLY_UNAVAILABLE),
        ];
        for (checkout, expected) in cases {
            let reply = click(
                Arc::new(fixed(checkout, Err(failure()))),
                SupportButton::Tier,
                "support:tier:10",
            )
            .await;
            assert_eq!(reply.content.as_deref(), Some(expected));
            assert!(reply.ephemeral);
        }
    }

    #[tokio::test]
    async fn kein_abo_und_twitch_werden_freundlich_beantwortet() {
        let none = click(
            Arc::new(fixed(stripe, Ok(LinkOutcome::NoSubscription))),
            SupportButton::Manage,
            SUPPORT_BUTTON_MANAGE,
        )
        .await;
        assert_eq!(none.content.as_deref(), Some(REPLY_NO_SUBSCRIPTION));
        let twitch = click(
            Arc::new(fixed(
                stripe,
                Ok(LinkOutcome::Url(
                    "https://deutsche-deadlock-community.de/twitch/x".to_string(),
                )),
            )),
            SupportButton::Twitch,
            SUPPORT_BUTTON_TWITCH,
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
        assert!(!config.panel_options().twitch);
        assert_eq!(config.invalid_keys, vec!["DL_SUPPORTER_REQUEST_TIMEOUT_MS"]);
        let links = HttpSupporterLinks::new(&config);
        assert_eq!(
            links.checkout(42, 5).await,
            Err(LinkError::NotConfigured(MISSING_TIMEOUT))
        );
        assert_eq!(links.missing_config(), Some(MISSING_TIMEOUT));
        assert_eq!(links.twitch_connect(42).await, Ok(LinkOutcome::Unavailable));
        let tier = click(
            HttpSupporterLinks::new(&config),
            SupportButton::Tier,
            "support:tier:5",
        )
        .await;
        assert_eq!(tier.content.as_deref(), Some(REPLY_FAILED));
        let twitch = click(
            HttpSupporterLinks::new(&config),
            SupportButton::Twitch,
            SUPPORT_BUTTON_TWITCH,
        )
        .await;
        assert_eq!(twitch.content.as_deref(), Some(REPLY_UNAVAILABLE));
        assert!(twitch.ephemeral);
    }

    #[test]
    fn panelwerte_kommen_aus_der_konfiguration() {
        let defaults = SupporterConfig::from_lookup(|_| None);
        assert_eq!(defaults.goal_cents, 25_000);
        assert_eq!(defaults.tiers_eur, vec![5, 10, 20, 50]);
        assert_eq!(defaults.plus_monthly_cents, None);
        assert!(defaults.invalid_keys.is_empty());
        let set = SupporterConfig::from_lookup(|key| match key {
            "DL_SUPPORTER_FUNDING_GOAL_CENTS" => Some("30000".to_string()),
            "DL_SUPPORTER_TIERS_EUR" => Some("5, 15".to_string()),
            "DL_SUPPORTER_PLUS_MONTHLY_CENTS" => Some("300".to_string()),
            "DL_SUPPORTER_TWITCH_API_BASE" => Some("http://127.0.0.1:8769".to_string()),
            "TWITCH_INTERNAL_API_TOKEN" => Some("internal".to_string()),
            _ => None,
        });
        assert_eq!(
            set.panel_options(),
            SupportPanelOptions {
                tiers_eur: vec![5, 15],
                plus_monthly_cents: Some(300),
                twitch: true,
            }
        );
        assert_eq!(set.goal_cents, 30_000);
        for (key, raw) in [
            ("DL_SUPPORTER_TIERS_EUR", "10,5"),
            ("DL_SUPPORTER_TIERS_EUR", "5,5"),
            ("DL_SUPPORTER_TIERS_EUR", "4"),
            ("DL_SUPPORTER_TIERS_EUR", "5,,10"),
            ("DL_SUPPORTER_TIERS_EUR", "5,10,20,50,100,200"),
            ("DL_SUPPORTER_PLUS_MONTHLY_CENTS", "0"),
            ("DL_SUPPORTER_FUNDING_GOAL_CENTS", "abc"),
        ] {
            let config =
                SupporterConfig::from_lookup(|asked| (asked == key).then(|| raw.to_string()));
            assert_eq!(config.invalid_keys, vec![key], "{key}={raw}");
            assert_eq!(config.tiers_eur, vec![5, 10, 20, 50]);
            assert_eq!(config.plus_monthly_cents, None);
            assert_eq!(config.goal_cents, 25_000);
        }
    }

    #[test]
    fn tokens_kommen_aus_genau_einer_quelle() {
        let config = SupporterConfig::from_lookup(|key| match key {
            "MASTER_BROKER_TOKEN" => Some("broker".to_string()),
            _ => None,
        });
        assert_eq!(config.twitch_token, None);
        assert_eq!(config.steam_token.as_deref(), Some("broker"));
        assert!(!config.role_grant_apply);
        let config = SupporterConfig::from_lookup(|key| match key {
            "MASTER_BROKER_TOKEN" => Some("broker".to_string()),
            "TWITCH_INTERNAL_API_TOKEN" => Some("internal".to_string()),
            "DL_SUPPORTER_ROLE_GRANT_APPLY" => Some("1".to_string()),
            _ => None,
        });
        assert_eq!(config.twitch_token.as_deref(), Some("internal"));
        assert_eq!(config.steam_token.as_deref(), Some("internal"));
        assert!(config.role_grant_apply);
        assert!(!config.role_sync_apply);
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
        let coded = |status: StatusCode, code: &str| {
            link_outcome("p", status, &json!({ "error": code }), is_stripe_url)
        };
        assert_eq!(
            coded(StatusCode::NOT_FOUND, "no_subscription"),
            Ok(LinkOutcome::NoSubscription)
        );
        assert_eq!(
            coded(StatusCode::CONFLICT, "already_subscribed"),
            Ok(LinkOutcome::AlreadySubscribed)
        );
        assert_eq!(
            coded(StatusCode::BAD_REQUEST, "unknown_tier"),
            Ok(LinkOutcome::UnknownTier)
        );
        for code in [
            "prices_unverified",
            "tip_not_configured",
            "supporter_not_configured",
        ] {
            assert_eq!(
                coded(StatusCode::SERVICE_UNAVAILABLE, code),
                Ok(LinkOutcome::Unavailable),
                "{code}"
            );
        }
        for (status, code) in [
            (StatusCode::UNAUTHORIZED, "unauthorized"),
            (StatusCode::SERVICE_UNAVAILABLE, "auth_not_configured"),
            (StatusCode::SERVICE_UNAVAILABLE, "checkout_busy"),
            (StatusCode::BAD_GATEWAY, "stripe_error"),
            (StatusCode::INTERNAL_SERVER_ERROR, "database_error"),
            (StatusCode::NOT_FOUND, ""),
            (StatusCode::CONFLICT, "other"),
        ] {
            assert!(coded(status, code).is_err(), "{status} {code}");
        }
        assert_eq!(
            parse_funding(
                &json!({ "monthly_cents_active": 1500, "supporters": 3 }),
                25_000
            ),
            Ok(SupportFunding {
                raised_cents: 1500,
                goal_cents: 25_000
            })
        );
        assert!(parse_funding(&json!({}), 25_000).is_err());
        assert!(parse_funding(&json!({ "monthly_cents_active": -1 }), 25_000).is_err());
        assert!(parse_funding(&json!({ "monthly_cents_active": null }), 25_000).is_err());
    }
}
