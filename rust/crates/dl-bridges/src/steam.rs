//! Steam-Bridge — Port von `cogs/steam_bridge.py`.
//!
//! Keine Business-Logik: Discord-Ereignisse gehen als JSON an den
//! Rust-Steam-Bot, dessen Antwort (`reply_text`/`reply_embed`/`ephemeral`/
//! `link_button`/`buttons`) wird als Discord-Antwort gerendert. Einzige
//! lokale UI: das Freundescode-Modal.

use std::sync::Arc;
use std::time::Duration;

use dl_discord::interactions::{ChannelMessage, ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, ChannelSender, CommandSpec, Dispatcher, InteractionHandler,
    InteractionRouter, MemberEvent,
};
use serde_json::{json, Map, Value};

pub const DEFAULT_API_URL: &str = "http://127.0.0.1:8783";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const RANKCHECK_FORWARD_TIMEOUT: Duration = Duration::from_secs(120);

/// custom_ids der persistenten Panels (inkl. Legacy-IDs alter Posts).
pub const PANEL_CUSTOM_IDS: [&str; 5] = [
    "steam_link_panel:open",
    "steam_link_panel:friend_code",
    "steam_link_panel:rankcheck",
    "linkpanel_friend_code",
    "linkpanel_rank_check",
];

pub const BETAINVITE_PANEL_CUSTOM_ID: &str = "betainvite:panel:start";

/// custom_ids, die lokal das Freundescode-Modal öffnen statt zu forwarden.
const FRIEND_CODE_MODAL_IDS: [&str; 2] = ["steam_link_panel:friend_code", "linkpanel_friend_code"];
const RANKCHECK_IDS: [&str; 2] = ["steam_link_panel:rankcheck", "linkpanel_rank_check"];
const FRIEND_CODE_MODAL_CUSTOM_ID: &str = "steam_bridge:friend_code_modal";
const FRIEND_CODE_SUBMIT_CUSTOM_ID: &str = "steam_link_panel:friend_code:submit";

fn component_forward_timeout(custom_id: &str) -> Duration {
    if RANKCHECK_IDS.contains(&custom_id) {
        RANKCHECK_FORWARD_TIMEOUT
    } else {
        DEFAULT_TIMEOUT
    }
}

const UNREACHABLE_MSG: &str =
    "⚠️ Steam-Bot ist gerade nicht erreichbar. Bitte versuche es in wenigen Sekunden erneut.";

/// Admin-Text-Kommandos (Prefix !), die an den Steam-Bot durchgereicht werden.
const ADMIN_COMMANDS: [&str; 6] = [
    "!steam_status",
    "!steam_friend_request",
    "!steam_lobby_convar",
    "!steam_lobby_apply",
    "!steam_lobby_event",
    "!steam_lobby_events",
];

// ── HTTP-Client ────────────────────────────────────────────────────────────

pub struct SteamBotClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl SteamBotClient {
    /// ENVs wie das Original: STEAM_BOT_API_URL + Token-Kette
    /// TWITCH_INTERNAL_API_TOKEN → MASTER_BROKER_TOKEN → MAIN_BOT_INTERNAL_TOKEN.
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Arc<Self> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let base_url = get("STEAM_BOT_API_URL").unwrap_or_else(|| DEFAULT_API_URL.to_string());
        let token = get("TWITCH_INTERNAL_API_TOKEN")
            .or_else(|| get("MASTER_BROKER_TOKEN"))
            .or_else(|| get("MAIN_BOT_INTERNAL_TOKEN"));
        Self::new(base_url, token)
    }

    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token,
        })
    }

    /// `POST {base}/events/discord` — `None` bei Nicht-200/Netzfehler (wie _post_event).
    pub async fn post_event(
        &self,
        kind: &str,
        data: Map<String, Value>,
        timeout: Duration,
    ) -> Option<Value> {
        let url = format!("{}/events/discord", self.base_url);
        let mut payload = Map::new();
        payload.insert("kind".into(), json!(kind));
        payload.extend(data);

        let mut request = self.http.post(&url).json(&payload).timeout(timeout);
        if let Some(token) = &self.token {
            request = request.header("X-Internal-Token", token);
        }
        match request.send().await {
            Ok(response) if response.status() == reqwest::StatusCode::OK => {
                response.json::<Value>().await.ok()
            }
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                tracing::warn!(%url, %status, body = %body.chars().take(200).collect::<String>(),
                    "steam-bridge: HTTP-Fehler");
                None
            }
            Err(err) => {
                tracing::warn!(%url, %err, "steam-bridge: Verbindungsfehler zum steam-bot");
                None
            }
        }
    }

    fn interaction_payload(interaction: &BridgeInteraction, custom_id: &str) -> Map<String, Value> {
        let mut inner = Map::new();
        inner.insert("custom_id".into(), json!(custom_id));
        inner.insert("user_id".into(), json!(interaction.user_id));
        inner.insert("guild_id".into(), json!(interaction.guild_id));
        inner.insert("channel_id".into(), json!(interaction.channel_id));
        if !interaction.values.is_empty() {
            inner.insert("values".into(), json!(interaction.values));
        }
        let mut data = Map::new();
        data.insert("interaction".into(), Value::Object(inner));
        data
    }

    /// Slash-Command-Wire-Objekt (Vertrag: data.name + data.options.<key>).
    fn slash_payload(
        interaction: &BridgeInteraction,
        name: &str,
        options: Option<Map<String, Value>>,
    ) -> Map<String, Value> {
        let mut command_data = Map::new();
        command_data.insert("name".into(), json!(name));
        if let Some(options) = options.filter(|o| !o.is_empty()) {
            command_data.insert("options".into(), Value::Object(options));
        }
        let mut inner = Map::new();
        inner.insert("custom_id".into(), json!(""));
        inner.insert("user_id".into(), json!(interaction.user_id));
        inner.insert("guild_id".into(), json!(interaction.guild_id));
        inner.insert("data".into(), Value::Object(command_data));
        let mut data = Map::new();
        data.insert("interaction".into(), Value::Object(inner));
        data
    }

    /// Frische Einmal-Login-URL — öffentlicher Helfer (voice_nudge nutzt ihn).
    pub async fn fetch_steam_link_url(&self, user_id: u64) -> Option<String> {
        let mut inner = Map::new();
        inner.insert("custom_id".into(), json!("steam_link_panel:open"));
        inner.insert("user_id".into(), json!(user_id));
        inner.insert("guild_id".into(), json!(0));
        inner.insert("channel_id".into(), json!(0));
        let mut data = Map::new();
        data.insert("interaction".into(), Value::Object(inner));
        let result = self
            .post_event("interaction", data, DEFAULT_TIMEOUT)
            .await?;
        result
            .get("link_button")
            .and_then(|b| b.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|url| !url.is_empty())
    }
}

// ── Antwort-Rendering ──────────────────────────────────────────────────────

/// Rust-Steam-Bot-Antwort → BridgeReply (wie _render_response).
fn render(result: Option<Value>) -> BridgeReply {
    let Some(result) = result else {
        return BridgeReply::ephemeral_text(UNREACHABLE_MSG);
    };

    let ephemeral = result
        .get("ephemeral")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let content = result
        .get("reply_text")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let embeds = result
        .get("reply_embed")
        .and_then(Value::as_object)
        .map(|embed| vec![Value::Object(embed.clone())])
        .unwrap_or_default();
    let components = build_components(&result);

    BridgeReply {
        content,
        embeds,
        components,
        ephemeral,
        ..BridgeReply::default()
    }
}

/// link_button + buttons → eine Action-Row (Discord-API-Format).
fn build_components(result: &Value) -> Option<Value> {
    let mut buttons: Vec<Value> = Vec::new();

    if let Some(link) = result.get("link_button").and_then(Value::as_object) {
        let url = link.get("url").and_then(Value::as_str).unwrap_or_default();
        if !url.is_empty() {
            let label = link
                .get("label")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("Öffnen");
            buttons.push(json!({ "type": 2, "style": 5, "label": label, "url": url }));
        }
    }

    if let Some(specs) = result.get("buttons").and_then(Value::as_array) {
        for spec in specs {
            let Some(spec) = spec.as_object() else {
                continue;
            };
            let custom_id = spec
                .get("custom_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if custom_id.is_empty() {
                continue;
            }
            let label = spec
                .get("label")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("\u{200b}");
            let style = match spec
                .get("style")
                .and_then(Value::as_str)
                .unwrap_or("secondary")
            {
                "primary" => 1,
                "success" => 3,
                "danger" => 4,
                _ => 2,
            };
            buttons.push(json!({
                "type": 2, "style": style, "label": label, "custom_id": custom_id,
            }));
        }
    }

    (!buttons.is_empty()).then(|| json!([{ "type": 1, "components": buttons }]))
}

fn friend_code_modal() -> ModalSpec {
    ModalSpec {
        custom_id: FRIEND_CODE_MODAL_CUSTOM_ID.to_string(),
        title: "Steam-Freundescode".to_string(),
        fields: vec![ModalField {
            custom_id: "friend_code".to_string(),
            label: "Freundescode".to_string(),
            placeholder: "z. B. 820142646".to_string(),
            required: true,
            min_length: 1,
            max_length: 32,
            paragraph: false,
        }],
    }
}

// ── Handler ────────────────────────────────────────────────────────────────

/// Button-Klicks: Freundescode-IDs öffnen das Modal, alles andere wird
/// 1:1 an den Steam-Bot weitergeleitet.
struct ForwardComponent {
    client: Arc<SteamBotClient>,
}

#[async_trait::async_trait]
impl InteractionHandler for ForwardComponent {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if FRIEND_CODE_MODAL_IDS.contains(&interaction.custom_id.as_str()) {
            return BridgeReply {
                modal: Some(friend_code_modal()),
                ..BridgeReply::default()
            };
        }
        let payload = SteamBotClient::interaction_payload(&interaction, &interaction.custom_id);
        let timeout = component_forward_timeout(&interaction.custom_id);
        let result = self
            .client
            .post_event("interaction", payload, timeout)
            .await;
        render(result)
    }
}

/// Modal-Submit des Freundescodes → forward als friend_code:submit.
struct FriendCodeSubmit {
    client: Arc<SteamBotClient>,
}

#[async_trait::async_trait]
impl InteractionHandler for FriendCodeSubmit {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let code = interaction
            .options
            .get("friend_code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut with_value = interaction.clone();
        with_value.values = vec![code];
        let payload =
            SteamBotClient::interaction_payload(&with_value, FRIEND_CODE_SUBMIT_CUSTOM_ID);
        let result = self
            .client
            .post_event("interaction", payload, DEFAULT_TIMEOUT)
            .await;
        render(result)
    }
}

/// Generischer Slash-Command-Forwarder.
struct ForwardSlash {
    client: Arc<SteamBotClient>,
    /// Wire-Name auf der Rust-Steam-Bot-Seite (z. B. "steam_links").
    wire_name: &'static str,
    /// Optionen, die 1:1 aus der Interaction übernommen werden.
    passthrough_options: &'static [&'static str],
    timeout: Duration,
}

#[async_trait::async_trait]
impl InteractionHandler for ForwardSlash {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let mut options = Map::new();
        for key in self.passthrough_options {
            if let Some(value) = interaction.options.get(*key) {
                if !value.is_null() {
                    options.insert((*key).to_string(), value.clone());
                }
            }
        }
        let payload = SteamBotClient::slash_payload(&interaction, self.wire_name, Some(options));
        let result = self
            .client
            .post_event("slash_command", payload, self.timeout)
            .await;
        render(result)
    }
}

/// checkrank: Discord-User-Option → target_user_id + target_mention.
struct CheckRank {
    client: Arc<SteamBotClient>,
}

#[async_trait::async_trait]
impl InteractionHandler for CheckRank {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let target_id = interaction
            .options
            .get("user")
            .and_then(Value::as_u64)
            .unwrap_or(interaction.user_id);
        let mut options = Map::new();
        options.insert("target_user_id".into(), json!(target_id));
        options.insert("target_mention".into(), json!(format!("<@{target_id}>")));
        let payload = SteamBotClient::slash_payload(&interaction, "checkrank", Some(options));
        let result = self
            .client
            .post_event("slash_command", payload, Duration::from_secs(120))
            .await;
        render(result)
    }
}

/// publish_*: Rust liefert das Embed, wir posten das Panel ÖFFENTLICH in den
/// Kanal (mit persistenten Buttons) und bestätigen ephemeral.
struct PublishPanel {
    client: Arc<SteamBotClient>,
    wire_name: &'static str,
    panel_buttons: Value,
    confirmation: &'static str,
}

#[async_trait::async_trait]
impl InteractionHandler for PublishPanel {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let payload = SteamBotClient::slash_payload(&interaction, self.wire_name, None);
        let Some(result) = self
            .client
            .post_event("slash_command", payload, DEFAULT_TIMEOUT)
            .await
        else {
            return BridgeReply::ephemeral_text(
                "⚠️ Steam-Bot ist gerade nicht erreichbar. Bitte erneut versuchen.",
            );
        };
        let embeds = result
            .get("reply_embed")
            .and_then(Value::as_object)
            .map(|e| vec![Value::Object(e.clone())])
            .unwrap_or_default();
        BridgeReply {
            channel_message: Some(ChannelMessage {
                embeds,
                components: Some(self.panel_buttons.clone()),
                confirmation: self.confirmation.to_string(),
            }),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }
}

/// Panel-Komponenten (Labels aus dem Original-Panel-Posting).
fn steam_panel_components() -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 1, "label": "🔗 Steam verknüpfen", "custom_id": "steam_link_panel:open" },
        { "type": 2, "style": 2, "label": "🔢 Freundescode eingeben", "custom_id": "steam_link_panel:friend_code" },
        { "type": 2, "style": 2, "label": "📊 Rang prüfen", "custom_id": "steam_link_panel:rankcheck" },
    ]}])
}

fn betainvite_panel_components() -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 1, "label": "🎟️ Einladung starten", "custom_id": BETAINVITE_PANEL_CUSTOM_ID },
    ]}])
}

// ── Registrierung ──────────────────────────────────────────────────────────

/// Registriert alle Steam-Bridge-Routen am InteractionRouter.
pub fn register(router: &mut InteractionRouter, client: Arc<SteamBotClient>) {
    // Persistente Buttons: Panels + kompletter Betainvite-Funnel via Präfix
    let forward = Arc::new(ForwardComponent {
        client: client.clone(),
    });
    for id in PANEL_CUSTOM_IDS {
        router.on_custom_id(id, forward.clone());
    }
    router.on_prefix("betainvite:", forward.clone());
    router.on_custom_id(
        FRIEND_CODE_MODAL_CUSTOM_ID,
        Arc::new(FriendCodeSubmit {
            client: client.clone(),
        }),
    );

    let spec = |definition: Value| CommandSpec { definition };
    let forward_slash =
        |wire_name: &'static str, passthrough: &'static [&'static str], secs: u64| {
            Arc::new(ForwardSlash {
                client: client.clone(),
                wire_name,
                passthrough_options: passthrough,
                timeout: Duration::from_secs(secs),
            })
        };

    // Öffentliche Commands
    router.on_command(
        "betainvite",
        spec(json!({
            "name": "betainvite",
            "description": "Starte den Deadlock-Playtest-Invite-Flow.",
        })),
        forward_slash("betainvite", &[], 15),
    );
    router.on_command(
        "account_verknüpfen",
        spec(json!({
            "name": "account_verknüpfen",
            "description": "Zeigt den Steam-OpenID-Link zur Account-Verknüpfung.",
        })),
        forward_slash("account_verknüpfen", &[], 15),
    );
    router.on_command(
        "steam_rank",
        spec(json!({
            "name": "steam_rank",
            "description": "Fragt den Deadlock-Rang über die Steam PlayerCard ab.",
            "options": [{
                "type": 3, "name": "target",
                "description": "SteamID64/Vanity/Link (leer = dein eigener Account)",
                "required": false,
            }],
        })),
        forward_slash("steam_rank", &["target"], 60),
    );
    router.on_command(
        "checkrank",
        spec(json!({
            "name": "checkrank",
            "description": "Prüft den Deadlock-Rang eines Discord-Users per @Mention.",
            "options": [{
                "type": 6, "name": "user",
                "description": "Discord-User (leer = du selbst)",
                "required": false,
            }],
        })),
        Arc::new(CheckRank {
            client: client.clone(),
        }),
    );

    // steam-Gruppe
    let steam_group = json!({
        "name": "steam",
        "description": "Steam-Links verwalten",
        "options": [
            { "type": 1, "name": "links", "description": "Zeigt deine gespeicherten Steam-Links." },
            { "type": 1, "name": "whoami",
              "description": "Prüft ID/Vanity/Profil-Link und zeigt Persona + SteamID.",
              "options": [{ "type": 3, "name": "steam",
                "description": "SteamID64, Vanity oder steamcommunity-Link", "required": true }] },
            { "type": 1, "name": "setprimary",
              "description": "Markiert einen verknüpften Steam-Account als Primär.",
              "options": [
                { "type": 3, "name": "steam",
                  "description": "SteamID64, Vanity oder steamcommunity-Link", "required": true },
                { "type": 3, "name": "name", "description": "Optionaler Anzeigename", "required": false },
              ] },
            { "type": 1, "name": "unlink", "description": "Entfernt einen Steam-Link.",
              "options": [{ "type": 3, "name": "steam",
                "description": "SteamID64, Vanity oder steamcommunity-Link", "required": true }] },
        ],
    });
    router.on_command(
        "steam links",
        spec(steam_group.clone()),
        forward_slash("steam_links", &[], 15),
    );
    router.on_command(
        "steam whoami",
        spec(steam_group.clone()),
        forward_slash("steam_whoami", &["steam"], 15),
    );
    router.on_command(
        "steam setprimary",
        spec(steam_group.clone()),
        forward_slash("steam_setprimary", &["steam", "name"], 15),
    );
    router.on_command(
        "steam unlink",
        spec(steam_group),
        forward_slash("steam_unlink", &["steam"], 15),
    );

    // Admin-Commands (default_member_permissions: 32 = Manage Guild, 8 = Administrator)
    router.on_command(
        "publish_betainvite_panel",
        spec(json!({
            "name": "publish_betainvite_panel",
            "description": "Veröffentlicht das Invite-Panel mit dem Einstiegs-Button (nur Admins).",
            "default_member_permissions": "32",
        })),
        Arc::new(PublishPanel {
            client: client.clone(),
            wire_name: "publish_betainvite_panel",
            panel_buttons: betainvite_panel_components(),
            confirmation: "✅ Invite-Panel gepostet.",
        }),
    );
    router.on_command(
        "betainvite_stats",
        spec(json!({
            "name": "betainvite_stats",
            "description": "Zeigt Funnel-Metriken des Playtest-Invite-Systems (nur Admins).",
            "default_member_permissions": "32",
        })),
        forward_slash("betainvite_stats", &[], 15),
    );
    for (name, description) in [
        (
            "steam_rank_sync",
            "(Admin) Synchronisiert Friend-Ranks und Rang-Rollen.",
        ),
        (
            "subrank_sync",
            "(Admin) Startet sofort den Deadlock Subrank-Auto-Sync.",
        ),
        (
            "sync_steam_friends",
            "(Admin) Synchronisiert die Steam-Freundesliste + Verified-Rollen.",
        ),
    ] {
        router.on_command(
            name,
            spec(json!({
                "name": name,
                "description": description,
                "default_member_permissions": "8",
            })),
            match name {
                "steam_rank_sync" => forward_slash("steam_rank_sync", &[], 180),
                "subrank_sync" => forward_slash("subrank_sync", &[], 180),
                _ => forward_slash("sync_steam_friends", &[], 180),
            },
        );
    }
    router.on_command(
        "publish_steam_panel",
        spec(json!({
            "name": "publish_steam_panel",
            "description": "(Admin) Steam-Verknüpfen-Panel in diesem Channel posten.",
            "default_member_permissions": "8",
        })),
        Arc::new(PublishPanel {
            client,
            wire_name: "publish_steam_panel",
            panel_buttons: steam_panel_components(),
            confirmation: "✅ Steam-Panel gepostet.",
        }),
    );
}

// ── Listener (member_remove + !steam_*-Admin-Kommandos) ───────────────────

/// Meldet Server-Austritte an den Steam-Bot (on_member_remove).
pub fn spawn_member_remove_listener(
    dispatcher: &Dispatcher,
    client: Arc<SteamBotClient>,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_members();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(MemberEvent::Remove { guild_id, user_id }) => {
                    let mut inner = Map::new();
                    inner.insert("guild_id".into(), json!(guild_id));
                    inner.insert("user_id".into(), json!(user_id));
                    let mut data = Map::new();
                    data.insert("member_remove".into(), Value::Object(inner));
                    let _ = client
                        .post_event("member_remove", data, DEFAULT_TIMEOUT)
                        .await;
                }
                Ok(MemberEvent::Join { .. })
                | Ok(MemberEvent::Ban { .. })
                | Ok(MemberEvent::Unban { .. })
                | Ok(MemberEvent::ScreeningCompleted { .. }) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "steam-bridge: Member-Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Fängt !steam_*-Admin-Kommandos ab und leitet sie weiter.
pub fn spawn_admin_command_listener(
    dispatcher: &Dispatcher,
    client: Arc<SteamBotClient>,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "steam-bridge: Message-Events verpasst");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if event.guild_id.is_none() || !event.content.trim().starts_with("!steam_") {
                continue;
            }
            let content = event.content.trim();
            let mut parts = content.split_whitespace();
            let command = parts.next().unwrap_or_default().to_lowercase();
            if !ADMIN_COMMANDS.contains(&command.as_str()) || !event.author_is_admin {
                continue;
            }
            let args = parts.collect::<Vec<_>>().join(" ");

            let mut inner = Map::new();
            inner.insert(
                "name".into(),
                json!(command.trim_start_matches('!').to_string()),
            );
            inner.insert("args".into(), json!(args));
            inner.insert("invoker_id".into(), json!(event.author_id));
            let mut data = Map::new();
            data.insert("admin_command".into(), Value::Object(inner));

            let result = client
                .post_event("admin_command", data, DEFAULT_TIMEOUT)
                .await;
            let (content, embeds) = match &result {
                None => (
                    Some("⚠️ Steam-Bot ist gerade nicht erreichbar.".to_string()),
                    vec![],
                ),
                Some(result) => (
                    result
                        .get("reply_text")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    result
                        .get("reply_embed")
                        .and_then(Value::as_object)
                        .map(|e| vec![Value::Object(e.clone())])
                        .unwrap_or_default(),
                ),
            };
            if let Err(err) = sender
                .send_to_channel(event.channel_id, content.as_deref(), &embeds)
                .await
            {
                tracing::warn!(%err, "steam-bridge: Admin-Antwort senden fehlgeschlagen");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Mutex;

    /// Mock-Steam-Bot: zeichnet Requests auf, antwortet vorgegeben.
    async fn mock_steam_bot(
        response: Value,
    ) -> (String, Arc<Mutex<Vec<Value>>>, tokio::task::JoinHandle<()>) {
        let received: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();
        let app = axum::Router::new().route(
            "/events/discord",
            axum::routing::post(move |body: axum::Json<Value>| {
                let received = received_clone.clone();
                let response = response.clone();
                async move {
                    received.lock().expect("mock lock").push(body.0);
                    axum::Json(response)
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

    fn interaction(custom_id: &str) -> BridgeInteraction {
        BridgeInteraction {
            custom_id: custom_id.to_string(),
            user_id: 42,
            guild_id: 7,
            channel_id: 9,
            ..BridgeInteraction::default()
        }
    }

    #[tokio::test]
    async fn button_forward_wire_format_und_rendering() {
        let (url, received, server) = mock_steam_bot(json!({
            "reply_text": "Hallo!",
            "ephemeral": false,
            "link_button": { "label": "Login", "url": "https://example.com/x" },
            "buttons": [{ "custom_id": "betainvite:link:continue", "label": "Weiter", "style": "primary" }],
        }))
        .await;
        let client = SteamBotClient::new(url, Some("tok".to_string()));
        let handler = ForwardComponent {
            client: client.clone(),
        };

        let reply = handler.handle(interaction("steam_link_panel:open")).await;
        assert_eq!(reply.content.as_deref(), Some("Hallo!"));
        assert!(!reply.ephemeral);
        let components = reply.components.expect("components");
        let row = &components[0]["components"];
        assert_eq!(row[0]["style"], 5); // Link-Button zuerst
        assert_eq!(row[1]["custom_id"], "betainvite:link:continue");

        let sent = received.lock().expect("lock");
        assert_eq!(sent[0]["kind"], "interaction");
        assert_eq!(sent[0]["interaction"]["custom_id"], "steam_link_panel:open");
        assert_eq!(sent[0]["interaction"]["user_id"], 42);
        server.abort();
    }

    #[tokio::test]
    async fn friend_code_button_oeffnet_modal_lokal() {
        // Kein Server nötig — der Klick darf NICHT geforwarded werden.
        let client = SteamBotClient::new("http://127.0.0.1:9", None);
        let handler = ForwardComponent { client };
        let reply = handler.handle(interaction("linkpanel_friend_code")).await;
        let modal = reply.modal.expect("modal");
        assert_eq!(modal.custom_id, FRIEND_CODE_MODAL_CUSTOM_ID);
        assert_eq!(modal.fields[0].max_length, 32);
    }

    #[tokio::test]
    async fn modal_submit_forwardet_friend_code() {
        let (url, received, server) = mock_steam_bot(json!({ "reply_text": "ok" })).await;
        let client = SteamBotClient::new(url, None);
        let handler = FriendCodeSubmit { client };
        let mut itx = interaction(FRIEND_CODE_MODAL_CUSTOM_ID);
        itx.options
            .insert("friend_code".to_string(), json!("820142646"));
        let reply = handler.handle(itx).await;
        assert_eq!(reply.content.as_deref(), Some("ok"));
        let sent = received.lock().expect("lock");
        assert_eq!(
            sent[0]["interaction"]["custom_id"],
            FRIEND_CODE_SUBMIT_CUSTOM_ID
        );
        assert_eq!(sent[0]["interaction"]["values"][0], "820142646");
        server.abort();
    }

    #[tokio::test]
    async fn slash_command_wire_format() {
        let (url, received, server) = mock_steam_bot(json!({ "reply_text": "ok" })).await;
        let client = SteamBotClient::new(url, None);
        let handler = ForwardSlash {
            client,
            wire_name: "steam_whoami",
            passthrough_options: &["steam"],
            timeout: DEFAULT_TIMEOUT,
        };
        let mut itx = interaction("");
        itx.options.insert("steam".to_string(), json!("gabelogan"));
        let _ = handler.handle(itx).await;
        let sent = received.lock().expect("lock");
        assert_eq!(sent[0]["kind"], "slash_command");
        assert_eq!(sent[0]["interaction"]["data"]["name"], "steam_whoami");
        assert_eq!(
            sent[0]["interaction"]["data"]["options"]["steam"],
            "gabelogan"
        );
        server.abort();
    }

    #[tokio::test]
    async fn steam_bot_nicht_erreichbar() {
        let client = SteamBotClient::new("http://127.0.0.1:9", None); // tote Adresse
        let handler = ForwardComponent { client };
        let reply = handler
            .handle(interaction("steam_link_panel:rankcheck"))
            .await;
        assert!(reply.content.expect("text").contains("nicht erreichbar"));
        assert!(reply.ephemeral);
    }

    #[test]
    fn router_registrierung_vollstaendig() {
        let mut router = InteractionRouter::new();
        register(&mut router, SteamBotClient::new("http://x", None));
        // Panels + Legacy + Betainvite-Präfix + Modal
        for id in PANEL_CUSTOM_IDS {
            assert!(router.resolve_component(id).is_some(), "{id}");
        }
        assert!(router
            .resolve_component("betainvite:intent:community")
            .is_some());
        assert!(router
            .resolve_component(FRIEND_CODE_MODAL_CUSTOM_ID)
            .is_some());
        // Slash-Commands
        for name in [
            "betainvite",
            "account_verknüpfen",
            "steam links",
            "steam whoami",
            "steam setprimary",
            "steam unlink",
            "steam_rank",
            "checkrank",
            "steam_rank_sync",
            "subrank_sync",
            "sync_steam_friends",
            "publish_steam_panel",
            "publish_betainvite_panel",
            "betainvite_stats",
        ] {
            assert!(router.resolve_command(name).is_some(), "{name}");
        }
        // 14 Routen, aber nur 11 Top-Level-Definitionen (steam-Gruppe dedupliziert)
        assert_eq!(router.command_definitions().len(), 11);
    }

    #[test]
    fn rankcheck_buttons_nutzen_langzeit_timeout() {
        assert_eq!(
            component_forward_timeout("steam_link_panel:rankcheck"),
            Duration::from_secs(120)
        );
        assert_eq!(
            component_forward_timeout("linkpanel_rank_check"),
            Duration::from_secs(120)
        );
        assert_eq!(
            component_forward_timeout("steam_link_panel:open"),
            DEFAULT_TIMEOUT
        );
    }
}
