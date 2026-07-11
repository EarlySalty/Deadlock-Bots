//! Steam-Bridge — Port von `cogs/steam_bridge.py`.
//!
//! Keine Business-Logik: Discord-Ereignisse gehen als JSON an den
//! Rust-Steam-Bot, dessen Antwort (`reply_text`/`reply_embed`/`ephemeral`/
//! `link_button`/`buttons`) wird als Discord-Antwort gerendert. Einzige
//! lokale UI: das Freundescode-Modal.

use std::sync::Arc;
use std::time::Duration;

use dl_central_db::kv;
use dl_discord::interactions::{ChannelMessage, ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, ChannelSender, CommandSpec, Dispatcher, InteractionHandler,
    InteractionRouter, MemberEvent, ResponseMessageHook,
};
use serde_json::{json, Map, Value};
use sqlx::PgPool;

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

/// custom_ids, die lokal das Freundescode-Modal öffnen statt zu forwarden.
const FRIEND_CODE_MODAL_IDS: [&str; 2] = ["steam_link_panel:friend_code", "linkpanel_friend_code"];
const RANKCHECK_IDS: [&str; 2] = ["steam_link_panel:rankcheck", "linkpanel_rank_check"];
const FRIEND_CODE_MODAL_CUSTOM_ID: &str = "steam_bridge:friend_code_modal";
const FRIEND_CODE_SUBMIT_CUSTOM_ID: &str = "steam_link_panel:friend_code:submit";
pub const STEAM_PANEL_KV_NS: &str = "steam_link_panel";
pub const STEAM_PANEL_KV_KEY: &str = "panel_ref";
pub const BRD09_STEAM_PANEL_POSTED_MSG: &str = "✅ Steam-Panel gepostet.";
pub const BRD09_STEAM_PANEL_MESSAGE_ID_OPTION_DESC: &str =
    "ID einer bestehenden Message, die editiert werden soll (optional)";

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
        let discord_name = if interaction.author_display_name.trim().is_empty() {
            &interaction.author_name
        } else {
            &interaction.author_display_name
        };
        inner.insert("data".into(), json!({ "discord_name": discord_name }));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SteamPanelRef {
    channel_id: u64,
    message_id: u64,
}

#[derive(Clone)]
struct SteamPanelStore {
    pool: PgPool,
}

impl SteamPanelStore {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn get(&self) -> Option<SteamPanelRef> {
        let raw = kv::get(&self.pool, STEAM_PANEL_KV_NS, STEAM_PANEL_KV_KEY)
            .await
            .ok()
            .flatten()?;
        let value = serde_json::from_str::<Value>(&raw).ok()?;
        let channel_id = value.get("channel_id").and_then(Value::as_u64)?;
        let message_id = value.get("message_id").and_then(Value::as_u64)?;
        (channel_id > 0 && message_id > 0).then_some(SteamPanelRef {
            channel_id,
            message_id,
        })
    }

    async fn set(&self, channel_id: u64, message_id: u64) {
        let payload = json!({
            "channel_id": channel_id,
            "message_id": message_id,
        })
        .to_string();
        if let Err(err) = kv::set(&self.pool, STEAM_PANEL_KV_NS, STEAM_PANEL_KV_KEY, &payload).await
        {
            tracing::warn!(%err, "Steam-Panel-Referenz konnte nicht gespeichert werden");
        }
    }

    async fn clear(&self) {
        if let Err(err) = kv::delete(&self.pool, STEAM_PANEL_KV_NS, STEAM_PANEL_KV_KEY).await {
            tracing::debug!(%err, "Steam-Panel-Referenz konnte nicht geloescht werden");
        }
    }
}

struct PersistSteamPanelRef {
    store: SteamPanelStore,
    channel_id: u64,
}

#[async_trait::async_trait]
impl ResponseMessageHook for PersistSteamPanelRef {
    async fn on_response_message(&self, message_id: u64) {
        self.store.set(self.channel_id, message_id).await;
    }
}

async fn fetch_panel_result(
    client: &Arc<SteamBotClient>,
    wire_name: &str,
    user_id: u64,
    guild_id: u64,
) -> Option<Value> {
    let interaction = BridgeInteraction {
        user_id,
        guild_id,
        ..BridgeInteraction::default()
    };
    let payload = SteamBotClient::slash_payload(&interaction, wire_name, None);
    client
        .post_event("slash_command", payload, DEFAULT_TIMEOUT)
        .await
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
    panel_store: Option<SteamPanelStore>,
}

#[async_trait::async_trait]
impl InteractionHandler for PublishPanel {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let Some(result) = fetch_panel_result(
            &self.client,
            self.wire_name,
            interaction.user_id,
            interaction.guild_id,
        )
        .await
        else {
            return BridgeReply::ephemeral_text(
                "⚠️ Steam-Bot ist gerade nicht erreichbar. Bitte erneut versuchen.",
            );
        };
        let target_channel_id = interaction
            .options
            .get("channel")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .unwrap_or(interaction.channel_id);
        let explicit_message_id = interaction
            .options
            .get("message_id")
            .and_then(|value| match value {
                Value::Number(n) => n.as_u64(),
                Value::String(s) => s.trim().parse::<u64>().ok(),
                _ => None,
            })
            .filter(|id| *id > 0);
        let stored_ref = match &self.panel_store {
            Some(store) if self.wire_name == "publish_steam_panel" => store.get().await,
            _ => None,
        };
        let edit_message_id = explicit_message_id.or_else(|| {
            stored_ref
                .filter(|stored| stored.channel_id == target_channel_id)
                .map(|stored| stored.message_id)
        });
        let response_message_hook = self.panel_store.as_ref().and_then(|store| {
            (self.wire_name == "publish_steam_panel").then(|| {
                Arc::new(PersistSteamPanelRef {
                    store: store.clone(),
                    channel_id: target_channel_id,
                }) as Arc<dyn ResponseMessageHook>
            })
        });
        let content = result
            .get("reply_text")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let embeds = result
            .get("reply_embed")
            .and_then(Value::as_object)
            .map(|e| vec![Value::Object(e.clone())])
            .unwrap_or_default();
        BridgeReply {
            channel_message: Some(ChannelMessage {
                target_channel_id: Some(target_channel_id),
                content,
                embeds,
                components: Some(self.panel_buttons.clone()),
                edit_message_id,
                confirmation: self.confirmation.to_string(),
                response_message_hook,
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

// ── Registrierung ──────────────────────────────────────────────────────────

/// Registriert alle Steam-Bridge-Routen am InteractionRouter.
pub fn register(router: &mut InteractionRouter, client: Arc<SteamBotClient>) {
    register_inner(router, client, None);
}

pub fn register_with_pool(
    router: &mut InteractionRouter,
    client: Arc<SteamBotClient>,
    pool: PgPool,
) {
    register_inner(router, client, Some(SteamPanelStore::new(pool)));
}

pub fn register_with_db(router: &mut InteractionRouter, client: Arc<SteamBotClient>, pool: PgPool) {
    register_with_pool(router, client, pool);
}

fn register_inner(
    router: &mut InteractionRouter,
    client: Arc<SteamBotClient>,
    panel_store: Option<SteamPanelStore>,
) {
    // Persistente Panel-Buttons
    let forward = Arc::new(ForwardComponent {
        client: client.clone(),
    });
    for id in PANEL_CUSTOM_IDS {
        router.on_custom_id(id, forward.clone());
    }
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
        "invite",
        spec(json!({
            "name": "invite",
            "description": "Lädt jemanden per Steam-Freundescode zum Playtest ein.",
            "default_member_permissions": "8",
            "options": [
                {"type": 3, "name": "freundescode", "description": "Steam-Freundescode, nur Ziffern, zum Beispiel 1852752823", "required": true},
                {"type": 6, "name": "user", "description": "Das Discord-Mitglied dazu, damit die Einladung in der Historie steht", "required": false}
            ],
        })),
        forward_slash("invite", &["freundescode", "user"], 30),
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
            "options": [{
                "type": 3,
                "name": "message_id",
                "description": BRD09_STEAM_PANEL_MESSAGE_ID_OPTION_DESC,
                "required": false
            }],
        })),
        Arc::new(PublishPanel {
            client,
            wire_name: "publish_steam_panel",
            panel_buttons: steam_panel_components(),
            confirmation: BRD09_STEAM_PANEL_POSTED_MSG,
            panel_store,
        }),
    );
}

pub fn spawn_panel_restore(
    client: Arc<SteamBotClient>,
    pool: PgPool,
    adapter: Arc<dl_discord::DiscordAdapter>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let store = SteamPanelStore::new(pool);
        let Some(panel_ref) = store.get().await else {
            return;
        };
        let Some(result) = fetch_panel_result(&client, "publish_steam_panel", 0, 0).await else {
            return;
        };
        let Some(embed) = result
            .get("reply_embed")
            .and_then(Value::as_object)
            .map(|e| Value::Object(e.clone()))
        else {
            return;
        };
        let mut body = Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), steam_panel_components());
        match adapter
            .edit_raw_public(panel_ref.channel_id, panel_ref.message_id, &body)
            .await
        {
            Ok(()) => tracing::info!(message_id = panel_ref.message_id, "Steam-Panel restored"),
            Err(err) => {
                tracing::warn!(%err, message_id = panel_ref.message_id, "Steam-Panel-Restore fehlgeschlagen");
                if should_clear_panel_ref_after_restore_error(&err) {
                    store.clear().await;
                }
            }
        }
    })
}

fn should_clear_panel_ref_after_restore_error(err: &serenity::Error) -> bool {
    should_clear_panel_ref_after_restore_status(dl_discord::dispatch::panel_edit_status_code(err))
}

fn should_clear_panel_ref_after_restore_status(status: Option<u16>) -> bool {
    dl_discord::dispatch::is_panel_edit_not_found_status(status)
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
                Ok(MemberEvent::Remove {
                    guild_id, user_id, ..
                }) => {
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
                | Ok(MemberEvent::ScreeningCompleted { .. })
                | Ok(MemberEvent::NativeOnboardingCompleted { .. }) => {}
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
            author_name: "Nani".to_string(),
            author_display_name: "Nani".to_string(),
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
            "buttons": [{ "custom_id": "steam_link_panel:rankcheck", "label": "Weiter", "style": "primary" }],
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
        assert_eq!(row[1]["custom_id"], "steam_link_panel:rankcheck");

        let sent = received.lock().expect("lock");
        assert_eq!(sent[0]["kind"], "interaction");
        assert_eq!(sent[0]["interaction"]["custom_id"], "steam_link_panel:open");
        assert_eq!(sent[0]["interaction"]["user_id"], 42);
        assert_eq!(sent[0]["interaction"]["data"]["discord_name"], "Nani");
        server.abort();
    }

    #[tokio::test]
    async fn rankcheck_forward_sendet_display_name_statt_username() {
        let (url, received, server) = mock_steam_bot(json!({ "reply_text": "ok" })).await;
        let client = SteamBotClient::new(url, None);
        let handler = ForwardComponent { client };
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "steam_link_panel:rankcheck".to_string(),
                user_id: 42,
                author_name: "discorduser#1234".to_string(),
                author_display_name: "Server Nick".to_string(),
                guild_id: 7,
                channel_id: 9,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(reply.content.as_deref(), Some("ok"));
        let sent = received.lock().expect("lock");
        assert_eq!(
            sent[0]["interaction"]["data"]["discord_name"],
            "Server Nick"
        );
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

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn steam_panel_editiert_gespeicherte_message_im_selben_kanal() {
        let (url, _received, server) = mock_steam_bot(json!({
            "reply_embed": { "title": "Steam" },
        }))
        .await;
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("central test db");
        kv::set(
            db.pool(),
            STEAM_PANEL_KV_NS,
            STEAM_PANEL_KV_KEY,
            r#"{"channel_id":9,"message_id":555}"#,
        )
        .await
        .expect("set");

        let handler = PublishPanel {
            client: SteamBotClient::new(url, None),
            wire_name: "publish_steam_panel",
            panel_buttons: steam_panel_components(),
            confirmation: BRD09_STEAM_PANEL_POSTED_MSG,
            panel_store: Some(SteamPanelStore::new(db.pool().clone())),
        };
        let reply = handler.handle(interaction("")).await;
        let panel = reply.channel_message.expect("panel");

        assert_eq!(panel.target_channel_id, Some(9));
        assert_eq!(panel.edit_message_id, Some(555));
        assert!(panel.response_message_hook.is_some());
        server.abort();
    }

    #[test]
    fn panel_restore_cleart_ref_nur_bei_not_found() {
        assert!(should_clear_panel_ref_after_restore_status(Some(404)));
        assert!(!should_clear_panel_ref_after_restore_status(Some(403)));
        assert!(!should_clear_panel_ref_after_restore_status(None));
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
        // Panels + Legacy + Modal
        for id in PANEL_CUSTOM_IDS {
            assert!(router.resolve_component(id).is_some(), "{id}");
        }
        let removed_funnel_id = ["beta", "invite:intent:community"].concat();
        assert!(router.resolve_component(&removed_funnel_id).is_none());
        assert!(router
            .resolve_component(FRIEND_CODE_MODAL_CUSTOM_ID)
            .is_some());
        // Slash-Commands
        for name in [
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
            "invite",
        ] {
            assert!(router.resolve_command(name).is_some(), "{name}");
        }
        // 12 Routen, aber nur 9 Top-Level-Definitionen (steam-Gruppe dedupliziert)
        assert_eq!(router.command_definitions().len(), 9);
    }

    #[test]
    fn invite_command_ist_admin_command_mit_freundescode_und_user_option() {
        let mut router = InteractionRouter::new();
        register(&mut router, SteamBotClient::new("http://x", None));

        assert!(router.resolve_command("invite").is_some());
        let definitions = router.command_definitions();
        let invite = definitions
            .iter()
            .find(|definition| definition["name"] == "invite")
            .expect("invite command definition");

        assert_eq!(invite["default_member_permissions"], json!("8"));

        let options = invite["options"].as_array().expect("invite options");
        let shape: Vec<_> = options
            .iter()
            .map(|option| (&option["type"], &option["name"], &option["required"]))
            .collect();
        assert_eq!(
            shape,
            vec![
                (&json!(3), &json!("freundescode"), &json!(true)),
                (&json!(6), &json!("user"), &json!(false)),
            ]
        );

        // Discord zeigt jede description im Command-Picker. Ein durchgerutschter
        // Platzhalter waere damit user-sichtbar, der Test haelt das auf.
        for text in std::iter::once(&invite["description"])
            .chain(options.iter().map(|option| &option["description"]))
        {
            let text = text.as_str().expect("description ist ein String");
            assert_ne!(text, "Platzhalter");
            assert!(!text.is_empty());
            // Discord zaehlt Zeichen, nicht Bytes: len() waere bei Umlauten zu streng.
            assert!(text.chars().count() <= 100, "Discord-Limit: {text}");
        }
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
