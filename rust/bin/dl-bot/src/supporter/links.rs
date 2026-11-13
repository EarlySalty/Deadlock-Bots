use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{json, Value};

use dl_bridges::http::{internal_client, HttpLimits, HttpSetupError};
use dl_core::runtime_config::SUPPORTER_TWITCH_CHANNELS;

use super::{FundingSource, SupporterConfig};
use crate::serversync::support_publish::{
    format_whole_euro, join_alternatives, tier_button_id, SupportFunding, SupportPanelOptions,
    SUPPORT_BUTTON_BECOME, SUPPORT_BUTTON_MANAGE, SUPPORT_BUTTON_TIP, SUPPORT_BUTTON_TWITCH,
    SUPPORT_TIER_BUTTON_PREFIX,
};

const INTERNAL_TOKEN_HEADER: &str = "X-Internal-Token";
const CHECKOUT_PATH: &str = "/supporter/v1/checkout";
const TIP_PATH: &str = "/supporter/v1/tip";
const PORTAL_PATH: &str = "/supporter/v1/portal";
const FUNDING_PATH: &str = "/supporter/v1/funding";
const TWITCH_CONNECT_PATH: &str = "/internal/supporter/twitch-connect-link";
const PUBLIC_ORIGIN: &str = "https://deutsche-deadlock-community.de/";
const STRIPE_HOST_SUFFIX: &str = ".stripe.com";
const LINK_URL_MAX_LEN: usize = 512;
const MISSING_STEAM_TOKEN: &str = "interner Token für den Steam Bot fehlt";
const MISSING_TWITCH_TOKEN: &str = "interner Token für das Twitch-Dashboard fehlt";

pub const REPLY_FAILED: &str =
    "Das hat gerade nicht geklappt. Versuch es bitte in ein paar Minuten noch einmal.";
pub const REPLY_UNAVAILABLE: &str =
    "Das ist gerade noch nicht freigeschaltet. Schau bitte später noch mal vorbei.";
pub const REPLY_BECOME: &str = "Such dir aus, wie viel du im Monat geben möchtest. Danach bekommst du deinen persönlichen Zahlungslink.";
pub const REPLY_TIER: &str = "Hier geht es zur Zahlung. Der Link gehört zu deinem Discord-Account, gib ihn bitte nicht weiter.";
pub const REPLY_TIER_GONE: &str =
    "Diesen Betrag gibt es so nicht mehr. Wähl bitte im Panel neu aus, wie viel du geben möchtest.";
pub const REPLY_ALREADY_SUBSCRIBED: &str =
    "Du hast schon ein laufendes Abo. Ändern oder kündigen kannst du es über das Panel.";
pub const REPLY_TIP: &str = "Hier wählst du deinen Betrag selbst, den Mindestbetrag zeigt dir die Zahlungsseite. Der Link gehört zu deinem Discord-Account, gib ihn bitte nicht weiter.";
pub const REPLY_MANAGE: &str = "Hier kannst du dein Abo ändern oder kündigen.";
pub const REPLY_NO_SUBSCRIPTION: &str =
    "Zu deinem Discord-Account gibt es gerade kein laufendes Abo.";
const LABEL_TIP: &str = "Betrag wählen";
const LABEL_PAY: &str = "Weiter zur Zahlung";
const LABEL_MANAGE: &str = "Abo öffnen";
const LABEL_TWITCH: &str = "Weiter zu Twitch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoleNote {
    grant: bool,
    interval: Duration,
}

impl RoleNote {
    fn append(self, text: &str) -> String {
        if self.grant {
            format!(
                "{text}\n-# Die Rolle kommt nach der Zahlung beim nächsten Abgleich, der {} läuft.",
                cadence(self.interval)
            )
        } else {
            text.to_string()
        }
    }
}

fn cadence(interval: Duration) -> String {
    let seconds = interval.as_secs().max(1);
    let (count, one, many) = if seconds % 3600 == 0 {
        (seconds / 3600, "Stunde", "Stunden")
    } else if seconds % 60 == 0 {
        (seconds / 60, "Minute", "Minuten")
    } else {
        (seconds, "Sekunde", "Sekunden")
    };
    if count == 1 {
        format!("jede {one}")
    } else {
        format!("alle {count} {many}")
    }
}

pub fn reply_twitch() -> String {
    format!(
        "Verbinde hier deinen Twitch-Account. Danach prüft der Bot automatisch, ob du bei {} abonniert hast.\n-# Der Link gilt nur kurz und nur für dich.",
        join_alternatives(
            SUPPORTER_TWITCH_CHANNELS
                .iter()
                .map(|channel| channel.to_string())
                .chain(std::iter::once("einem passenden Streamer".to_string()))
                .collect()
        )
    )
}

fn tier_label(eur: u64) -> String {
    format!("{} im Monat", format_whole_euro(eur))
}

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
    async fn checkout(&self, discord_id: u64, tier: u64) -> LinkResult;
    async fn tip(&self, discord_id: u64) -> LinkResult;
    async fn portal(&self, discord_id: u64) -> LinkResult;
    async fn twitch_connect(&self, discord_id: u64) -> LinkResult;
}

pub struct HttpSupporterLinks {
    http: reqwest::Client,
    steam_base: String,
    steam_token: Option<String>,
    twitch_base: Option<String>,
    twitch_token: Option<String>,
    goal_cents: i64,
    panel: SupportPanelOptions,
}

impl HttpSupporterLinks {
    pub fn new(config: &SupporterConfig, limits: HttpLimits) -> Result<Arc<Self>, HttpSetupError> {
        let links = Self {
            http: internal_client(limits)?,
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
        Ok(Arc::new(links))
    }

    fn log_setup(&self) {
        tracing::info!(
            steam_base = %self.steam_base,
            twitch_base = ?self.twitch_base,
            twitch_button = self.panel.twitch,
            "Server-Unterstützer: Knöpfe eingerichtet"
        );
        if let Some(missing) = FundingSource::missing_config(self) {
            tracing::warn!(
                missing,
                "Server-Unterstützer: Abo, Einmalig und Verwalten antworten „noch nicht freigeschaltet“, das Monatsziel wird nicht abgeglichen"
            );
        }
        if self.twitch_base.is_some() && self.twitch_token.is_none() {
            tracing::warn!(
                missing = MISSING_TWITCH_TOKEN,
                "Server-Unterstützer: Twitch-Knopf bleibt im Panel ausgeblendet"
            );
        }
    }

    async fn post(
        &self,
        base: &str,
        token: &str,
        path: &str,
        body: Value,
    ) -> Result<(StatusCode, Value), LinkError> {
        let response = self
            .http
            .post(format!("{base}{path}"))
            .header(INTERNAL_TOKEN_HEADER, token)
            .json(&body)
            .send()
            .await
            .map_err(|error| LinkError::Failed(format!("{path} nicht erreichbar: {error}")))?;
        read_reply(path, response).await.map_err(LinkError::Failed)
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

#[async_trait::async_trait]
impl SupporterLinks for HttpSupporterLinks {
    async fn checkout(&self, discord_id: u64, tier: u64) -> LinkResult {
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
            .http
            .get(format!("{}{FUNDING_PATH}", self.steam_base))
            .header(INTERNAL_TOKEN_HEADER, token)
            .send()
            .await
            .map_err(|error| format!("{FUNDING_PATH} nicht erreichbar: {error}"))?;
        let (status, body) = read_reply(FUNDING_PATH, response).await?;
        if !status.is_success() {
            let code = error_code(&body);
            log_token_rejection(FUNDING_PATH, status, code);
            return Err(format!("{FUNDING_PATH} antwortet {status} {code}"));
        }
        parse_funding(&body, self.goal_cents)
    }

    fn missing_config(&self) -> Option<&'static str> {
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

async fn read_reply(
    path: &str,
    response: reqwest::Response,
) -> Result<(StatusCode, Value), String> {
    let status = response.status();
    match response.json::<Value>().await {
        Ok(body) => Ok((status, body)),
        Err(error) if status.is_success() => Err(format!(
            "{path} antwortet {status} mit unlesbarem Inhalt: {error}"
        )),
        Err(_) => Ok((status, Value::Null)),
    }
}

fn error_code(body: &Value) -> &str {
    body.get("error").and_then(Value::as_str).unwrap_or("")
}

fn link_outcome(
    path: &str,
    status: StatusCode,
    body: &Value,
    accept: fn(&str) -> bool,
) -> LinkResult {
    let code = error_code(body);
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
        _ => log_token_rejection(path, status, code),
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

fn log_token_rejection(path: &str, status: StatusCode, code: &str) {
    if status == StatusCode::UNAUTHORIZED
        || (status == StatusCode::SERVICE_UNAVAILABLE && code == "auth_not_configured")
    {
        tracing::error!(
            path,
            %status,
            code,
            "Server-Unterstützer: interner Token wird nicht angenommen"
        );
    }
}

fn url_host(url: &str) -> Option<&str> {
    if url
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '\\')
        || url.len() > LINK_URL_MAX_LEN
    {
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
    tiers_eur: Arc<[u64]>,
    role_note: RoleNote,
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
                    self.role_note,
                )
                .await
            }
            SupportButton::Tip => single_reply(
                self.links.tip(user_id).await,
                &self.role_note.append(REPLY_TIP),
                LABEL_TIP,
            ),
            SupportButton::Manage => {
                single_reply(self.links.portal(user_id).await, REPLY_MANAGE, LABEL_MANAGE)
            }
            SupportButton::Twitch => single_reply(
                self.links.twitch_connect(user_id).await,
                &reply_twitch(),
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
                dl_discord::BridgeReply::ephemeral_text(REPLY_UNAVAILABLE)
            }
            Err(LinkError::Failed(error)) => {
                tracing::warn!(user_id, button = self.button.name(), %error, "Unterstützer-Knopf: Link nicht erzeugt");
                dl_discord::BridgeReply::ephemeral_text(REPLY_FAILED)
            }
        }
    }
}

fn tier_picker(tiers_eur: &[u64]) -> dl_discord::BridgeReply {
    let buttons = tiers_eur
        .iter()
        .map(|eur| {
            json!({
                "type": 2,
                "style": 1,
                "label": tier_label(*eur),
                "custom_id": tier_button_id(*eur),
            })
        })
        .collect();
    button_reply(REPLY_BECOME, buttons)
}

fn tier_from_custom_id(custom_id: &str, tiers_eur: &[u64]) -> Option<u64> {
    custom_id
        .strip_prefix(SUPPORT_TIER_BUTTON_PREFIX)?
        .parse::<u64>()
        .ok()
        .filter(|eur| tiers_eur.contains(eur))
}

async fn tier_reply(
    links: &dyn SupporterLinks,
    user_id: u64,
    custom_id: &str,
    tiers_eur: &[u64],
    role_note: RoleNote,
) -> Result<dl_discord::BridgeReply, LinkError> {
    let Some(eur) = tier_from_custom_id(custom_id, tiers_eur) else {
        tracing::warn!(
            user_id,
            custom_id,
            "Unterstützer-Stufe nicht mehr angeboten, alter Knopf"
        );
        return Ok(dl_discord::BridgeReply::ephemeral_text(REPLY_TIER_GONE));
    };
    let label = format!("{LABEL_PAY} ({})", tier_label(eur));
    single_reply(
        links.checkout(user_id, eur).await,
        &role_note.append(REPLY_TIER),
        &label,
    )
}

fn single_reply(
    outcome: LinkResult,
    text: &str,
    label: &str,
) -> Result<dl_discord::BridgeReply, LinkError> {
    let reply = match outcome? {
        LinkOutcome::Url(url) => return Ok(button_reply(text, vec![link_button(label, &url)])),
        LinkOutcome::NoSubscription => REPLY_NO_SUBSCRIPTION,
        LinkOutcome::AlreadySubscribed => REPLY_ALREADY_SUBSCRIBED,
        LinkOutcome::Unavailable | LinkOutcome::UnknownTier => REPLY_UNAVAILABLE,
    };
    Ok(dl_discord::BridgeReply::ephemeral_text(reply))
}

fn link_button(label: &str, url: &str) -> Value {
    json!({ "type": 2, "style": 5, "label": label, "url": url })
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
    config: &SupporterConfig,
) {
    let tiers_eur: Arc<[u64]> = Arc::from(config.tiers_eur.as_slice());
    let role_note = RoleNote {
        grant: config.role_grant_apply,
        interval: config.interval,
    };
    let handler = |button| {
        Arc::new(SupportButtonHandler {
            links: links.clone(),
            button,
            tiers_eur: tiers_eur.clone(),
            role_note,
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
        checkout: fn(u64) -> LinkResult,
        other: LinkResult,
        checkouts: AtomicUsize,
    }

    fn fixed(checkout: fn(u64) -> LinkResult, other: LinkResult) -> Fixed {
        Fixed {
            checkout,
            other,
            checkouts: AtomicUsize::new(0),
        }
    }

    #[async_trait::async_trait]
    impl SupporterLinks for Fixed {
        async fn checkout(&self, _discord_id: u64, tier: u64) -> LinkResult {
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

    const TIERS: [u64; 5] = [5, 10, 20, 50, 1000];
    const TEST_INTERVAL: Duration = Duration::from_secs(300);

    fn failure() -> LinkError {
        LinkError::Failed("down".to_string())
    }

    fn stripe(tier: u64) -> LinkResult {
        Ok(LinkOutcome::Url(format!(
            "https://checkout.stripe.com/{tier}"
        )))
    }

    async fn click(
        links: Arc<dyn SupporterLinks>,
        button: SupportButton,
        custom_id: &str,
    ) -> dl_discord::BridgeReply {
        click_with_grant(links, button, custom_id, true).await
    }

    async fn click_with_grant(
        links: Arc<dyn SupporterLinks>,
        button: SupportButton,
        custom_id: &str,
        role_grant: bool,
    ) -> dl_discord::BridgeReply {
        let handler = SupportButtonHandler {
            links,
            button,
            tiers_eur: Arc::from(&TIERS[..]),
            role_note: RoleNote {
                grant: role_grant,
                interval: TEST_INTERVAL,
            },
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
                "support:tier:50",
                "support:tier:1000"
            ]
        );
        let labels: Vec<String> = reply_buttons(&reply)
            .iter()
            .filter_map(|button| button["label"].as_str().map(str::to_string))
            .collect();
        assert_eq!(labels[4], "1.000 € im Monat");
        assert_eq!(links.checkouts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stufenklick_holt_genau_einen_link() {
        let links = Arc::new(fixed(stripe, Err(failure())));
        let reply = click(links.clone(), SupportButton::Tier, "support:tier:50").await;
        assert_eq!(
            reply.content.as_deref(),
            Some(
                "Hier geht es zur Zahlung. Der Link gehört zu deinem Discord-Account, gib ihn bitte nicht weiter.\n-# Die Rolle kommt nach der Zahlung beim nächsten Abgleich, der alle 5 Minuten läuft."
            )
        );
        let manual =
            click_with_grant(links.clone(), SupportButton::Tier, "support:tier:50", false).await;
        assert_eq!(manual.content.as_deref(), Some(REPLY_TIER));
        assert!(!REPLY_TIER.contains("Rolle"));
        assert_eq!(
            reply_urls(&reply),
            vec!["https://checkout.stripe.com/50".to_string()]
        );
        assert_eq!(links.checkouts.load(Ordering::SeqCst), 2);
        for stale in [
            "support:tier:3",
            "support:tier:",
            "support:tier:x",
            "support:tier:-5",
        ] {
            let gone = click(links.clone(), SupportButton::Tier, stale).await;
            assert_eq!(gone.content.as_deref(), Some(REPLY_TIER_GONE), "{stale}");
        }
        assert_eq!(links.checkouts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stufenklick_beantwortet_jede_antwort_des_steam_bots() {
        type Checkout = fn(u64) -> LinkResult;
        let cases: [(Checkout, &str); 4] = [
            (|_| Err(LinkError::Failed("down".to_string())), REPLY_FAILED),
            (
                |_| Ok(LinkOutcome::AlreadySubscribed),
                REPLY_ALREADY_SUBSCRIBED,
            ),
            (|_| Ok(LinkOutcome::UnknownTier), REPLY_UNAVAILABLE),
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

    fn config_from(
        community: dl_core::runtime_config::CommunityOptions,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> SupporterConfig {
        SupporterConfig::new(&community, "http://127.0.0.1:8783", lookup)
    }

    fn test_links(config: &SupporterConfig) -> Arc<HttpSupporterLinks> {
        HttpSupporterLinks::new(
            config,
            HttpLimits::from_bridges(&dl_core::runtime_config::BridgeOptions::default()),
        )
        .expect("Testclient")
    }

    #[tokio::test]
    async fn fehlende_einrichtung_bleibt_ohne_anfrage_und_ohne_falschen_host() {
        let config = config_from(Default::default(), |_| None);
        assert_eq!(config.steam_token, None);
        assert_eq!(config.twitch_api_base, None);
        assert!(!config.panel_options().twitch);
        let links = test_links(&config);
        for outcome in [
            links.checkout(42, 5).await,
            links.tip(42).await,
            links.portal(42).await,
        ] {
            assert_eq!(outcome, Err(LinkError::NotConfigured(MISSING_STEAM_TOKEN)));
        }
        assert_eq!(links.missing_config(), Some(MISSING_STEAM_TOKEN));
        assert!(links
            .funding()
            .await
            .is_err_and(|error| error.contains(MISSING_STEAM_TOKEN)));
        assert_eq!(links.twitch_connect(42).await, Ok(LinkOutcome::Unavailable));
        for (button, custom_id) in [
            (SupportButton::Tier, "support:tier:5"),
            (SupportButton::Tip, SUPPORT_BUTTON_TIP),
            (SupportButton::Manage, SUPPORT_BUTTON_MANAGE),
            (SupportButton::Twitch, SUPPORT_BUTTON_TWITCH),
        ] {
            let reply = click(test_links(&config), button, custom_id).await;
            assert_eq!(
                reply.content.as_deref(),
                Some(REPLY_UNAVAILABLE),
                "{custom_id}"
            );
            assert!(reply.ephemeral);
        }
    }

    #[test]
    fn panelwerte_kommen_aus_der_konfiguration() {
        let defaults = config_from(Default::default(), |_| None);
        assert_eq!(
            defaults.goal_cents,
            dl_core::runtime_config::SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS
        );
        assert_eq!(
            defaults.tiers_eur,
            dl_core::runtime_config::SUPPORTER_DEFAULT_TIERS_EUR
        );
        assert!(!defaults.panel_options().role_grant);
        let set = config_from(
            dl_core::runtime_config::CommunityOptions {
                supporter_funding_goal_cents: Some(30_000),
                supporter_tiers_eur: Some(vec![5, 15]),
                supporter_twitch_api_base: Some("http://127.0.0.1:8769".to_string()),
                supporter_role_grant_apply: Some(true),
                ..Default::default()
            },
            |key| (key == "TWITCH_INTERNAL_API_TOKEN").then(|| "internal".to_string()),
        );
        assert_eq!(
            set.panel_options(),
            SupportPanelOptions {
                tiers_eur: vec![5, 15],
                twitch: true,
                role_grant: true,
            }
        );
        assert_eq!(set.goal_cents, 30_000);
        assert_eq!(set.steam_api_base, "http://127.0.0.1:8783");
    }

    #[test]
    fn tokens_kommen_aus_genau_einer_quelle() {
        let config = config_from(Default::default(), |key| {
            (key == "MASTER_BROKER_TOKEN").then(|| "broker".to_string())
        });
        assert_eq!(config.twitch_token, None);
        assert_eq!(config.steam_token, None);
        let config = config_from(Default::default(), |key| match key {
            "MASTER_BROKER_TOKEN" => Some("broker".to_string()),
            "STEAM_INTERNAL_API_TOKEN" => Some("steam-wert-7d1".to_string()),
            _ => None,
        });
        assert_eq!(config.steam_token.as_deref(), Some("steam-wert-7d1"));
        assert_eq!(config.twitch_token, None);
        let debug = format!("{config:?}");
        assert!(!debug.contains("steam-wert-7d1"), "{debug}");
        let config = config_from(Default::default(), |key| match key {
            "STEAM_INTERNAL_API_TOKEN" => Some("steam-wert-7d1".to_string()),
            "TWITCH_INTERNAL_API_TOKEN" => Some("twitch-wert-4c9".to_string()),
            _ => None,
        });
        assert_eq!(config.twitch_token.as_deref(), Some("twitch-wert-4c9"));
        assert_eq!(config.steam_token.as_deref(), Some("twitch-wert-4c9"));
        let debug = format!("{config:?}");
        assert!(!debug.contains("twitch-wert-4c9"), "{debug}");
    }

    #[test]
    fn antworttexte_nennen_werte_aus_dem_code_und_keine_panel_knoepfe() {
        assert!(!REPLY_TIP.chars().any(|ch| ch.is_ascii_digit()));
        let alternatives: Vec<String> = SUPPORTER_TWITCH_CHANNELS
            .iter()
            .map(|channel| channel.to_string())
            .chain(std::iter::once("einem passenden Streamer".to_string()))
            .collect();
        assert!(reply_twitch().contains(&format!(
            "bei {} abonniert",
            join_alternatives(alternatives)
        )));
        for (interval, expected) in [
            (60, "jede Minute"),
            (300, "alle 5 Minuten"),
            (3600, "jede Stunde"),
            (7200, "alle 2 Stunden"),
            (90, "alle 90 Sekunden"),
        ] {
            assert_eq!(cadence(Duration::from_secs(interval)), expected);
        }
        let off = RoleNote {
            grant: false,
            interval: TEST_INTERVAL,
        };
        assert_eq!(off.append(REPLY_TIP), REPLY_TIP);
        for text in [REPLY_TIER_GONE, REPLY_ALREADY_SUBSCRIBED] {
            for label in [
                crate::serversync::support_publish::SUPPORT_BECOME_BUTTON_LABEL,
                crate::serversync::support_publish::SUPPORT_MANAGE_BUTTON_LABEL,
            ] {
                assert!(!text.contains(label), "{text}");
            }
        }
        assert_ne!(
            LABEL_TWITCH,
            crate::serversync::support_publish::SUPPORT_TWITCH_BUTTON_LABEL
        );
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
        for bypass in [
            "https://evil.example\\.stripe.com/c",
            "https://evil.example\\@checkout.stripe.com/c",
        ] {
            assert!(!is_stripe_url(bypass), "{bypass}");
        }
        let long = format!(
            "https://checkout.stripe.com/{}",
            "a".repeat(LINK_URL_MAX_LEN)
        );
        assert!(!is_stripe_url(&long));
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
