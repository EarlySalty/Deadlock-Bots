//! Lane-Router — Port von `cogs/tempvoice/router.py` +
//! `router_interface.py`.
//!
//! Wer den Router-VC betritt, wird nach seiner gespeicherten Präferenz
//! (ranked/casual/street_brawl + Auto-Join) in eine passende Lane gelotst:
//! bevorzugt eine Lane mit bekannten Mitspielern (Co-Spieler-Graph), sonst
//! die erste mit Platz (1–5 Mitglieder), sonst wird über die
//! TempVoice-Engine eine neue Lane im Modus erstellt.
//!
//! Panel-Buttons mit Original-custom_ids: `router_mode_{mode}` +
//! `router_autojoin_toggle`. Bewusste Lücke (dokumentiert): der
//! New-Player-Routing-Hook (eigene Anfänger-Kategorie) — das Original
//! behandelt einen fehlenden Hook identisch (kein Reroute).

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use dl_central_db::kv;
use dl_discord::{
    BridgeAttachment, BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler,
    InteractionRouter, VoiceEvent,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;

use crate::db::u64_to_i64;
use crate::tempvoice::store::DefaultPresetRecord;
use crate::tempvoice::TempVoiceEngine;

pub const ROUTER_VC_ID: u64 = 1513468587195633674;
pub const ROUTER_GUILD_ID: u64 = 1289721245281292288;
pub const ROUTER_TEXT_CHANNEL_ID: u64 = 1513468476365209670;
pub const RANKED_INFO_CHANNEL_ID: u64 = 1474827277610254570;
pub const ROUTER_CATEGORY_CHILL: u64 = 1289721245281292290;
pub const ROUTER_CATEGORY_RANKED_LEGACY: u64 = 1412804540994162789;
pub const ROUTER_CATEGORY_STREET_BRAWL_LEGACY: u64 = 1357422957017698478;
pub const MAX_LANE_MEMBERS: usize = 6;
pub const ROUTER_PANEL_KV_NS: &str = "tempvoice_router";
pub const ROUTER_PANEL_MESSAGE_KEY: &str = "components_v2_message_id";
pub const ROUTER_LEGACY_GUIDE_MESSAGE_KEY: &str = "guide_message_id";
pub const ROUTER_LEGACY_INTERFACE_MESSAGE_KEY: &str = "interface_message_id";
const ROUTER_LEGACY_MESSAGE_KEYS: [&str; 2] = [
    ROUTER_LEGACY_GUIDE_MESSAGE_KEY,
    ROUTER_LEGACY_INTERFACE_MESSAGE_KEY,
];
pub const ROUTER_PAYLOAD_FORMAT_KEY: &str = "payload_format";
pub const ROUTER_PAYLOAD_FORMAT: &str = "components_v2";
pub const ROUTER_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const ROUTER_ACCENT_GOLD: u64 = 0xC8A86B;
pub const ROUTER_BANNER_DIR: &str = "assets/welcome-banners";
pub const ROUTER_HERO_BANNER_FILENAME: &str = "router-hero.png";
pub const ROUTER_MANAGE_BANNER_FILENAME: &str = "divider-lane-verwalten.png";
pub const ROUTER_GUIDE_BANNER_FILENAME: &str = "divider-anleitung.png";
pub const ROUTER_FLOOD_WINDOW_SECS: u64 = 60;
pub const ROUTER_FLOOD_MAX_CREATES: usize = 4;
pub const ROUTER_SELECT_MODE_BEFORE_AUTOJOIN: &str = "Wähle zuerst einen Spielmodus.";
pub const ROUTER_PANEL_INTRO: &str =
    "Wähle deinen Modus — der Bot erstellt dir eine eigene Voice-Lane in der passenden Kategorie und zieht dich direkt rüber. Dafür musst du in einem Sprachkanal sitzen, zum Beispiel im Deadlock Router.";
pub const ROUTER_PANEL_MANAGE_INTRO: &str =
    "Die Buttons wirken auf die Lane, in der du gerade sitzt. Verwalten kann sie nur ihr Owner — ist der weg, holst du sie dir mit <:dl_crown:1522518265421631538> Owner übernehmen.";
pub const ROUTER_BUTTON_CASUAL: &str = "Casual";
pub const ROUTER_BUTTON_RANKED: &str = "Ranked";
pub const ROUTER_BUTTON_STREET_BRAWL: &str = "Street Brawl";
pub const ROUTER_BUTTON_CLAIM: &str = "Owner übernehmen";
pub const ROUTER_BUTTON_LIMIT: &str = "Limit setzen";
pub const ROUTER_BUTTON_RENAME: &str = "Umbenennen";
pub const ROUTER_BUTTON_RANK_GATE: &str = "Rang-Gate";
pub const ROUTER_BUTTON_KICK: &str = "Kick";
pub const ROUTER_BUTTON_BAN: &str = "Ban";
pub const ROUTER_BUTTON_UNBAN: &str = "Unban";
pub const ROUTER_BUTTON_MODE: &str = "Modus wechseln";
pub const VOICE_GUIDE_TITLE: &str = "**🎙️ So funktionieren unsere Voice-Lanes**";
pub const VOICE_GUIDE_BODY: &str = "Bei uns joinst du nicht in volle Kanäle — du bekommst deine eigene Lane:\n1. Join ➕Deadlock Router oder klick unten einen Modus-Button. Beim **ersten Mal** wählst du, was du spielen willst (Casual, Ranked, Street Brawl) — der Bot merkt sich das als deinen Standard.\n2. **Ab dann** geht's beim Router-Join sofort in deine eigene Lane — ohne Extra-Klick. Standard ändern? ⚙️ Voreinstellungen, jederzeit, auch ohne in einer Voice zu sein.\n3. Deine Lane gehört dir: Name, Limit, Kick — alles über „Lane verwalten\" steuerbar.\n4. Mitspieler findest du über „Mitspieler finden\" — oder lass dich mit 🔔 benachrichtigen, sobald ein passendes Gesuch reinkommt.";
pub const VOICE_GUIDE_DETAIL_BUTTON: &str = "📖 Ausführliche Anleitung";
pub const VOICE_PREFS_BUTTON: &str = "⚙️ Voreinstellungen";

// Server-Emojis im Brand-Look: gold getönte Lucide-Icons (gen_router_emojis.py),
// einmalig als Guild-Emojis hochgeladen — (Name, ID) sind stabil.
pub const ROUTER_EMOJI_CASUAL: (&str, &str) = ("dl_casual", "1522518264088100995");
pub const ROUTER_EMOJI_RANKED: (&str, &str) = ("dl_ranked", "1522518271306366996");
pub const ROUTER_EMOJI_BRAWL: (&str, &str) = ("dl_brawl", "1522518262708174928");
pub const ROUTER_EMOJI_CROWN: (&str, &str) = ("dl_crown", "1522518265421631538");
pub const ROUTER_EMOJI_LIMIT: (&str, &str) = ("dl_limit", "1522518268345192588");
pub const ROUTER_EMOJI_RENAME: (&str, &str) = ("dl_rename", "1522518272497418250");
pub const ROUTER_EMOJI_KICK: (&str, &str) = ("dl_kick", "1522518266298368073");
pub const ROUTER_EMOJI_BAN: (&str, &str) = ("dl_ban", "1522518261290369034");
pub const ROUTER_EMOJI_UNBAN: (&str, &str) = ("dl_unban", "1522518273827143751");
pub const ROUTER_EMOJI_MODE: (&str, &str) = ("dl_mode", "1522518269456547962");

pub const ROUTER_PANEL_MODE_HINT: &str = "-# Ranked ist offen; Rang-Gates setzt der Lane-Owner.";
pub const ROUTER_PANEL_LANE_CAPTION: &str = "-# Deine Lane";
pub const ROUTER_PANEL_MOD_CAPTION: &str = "-# Moderation";
pub const ROUTER_PANEL_GUIDE_CREATE: &str = "**Lane erstellen**\nKlick auf einen der drei Modus-Buttons — der Bot erstellt dir eine eigene Lane und zieht dich automatisch rüber. Du musst dafür in einem Sprachkanal sitzen; der Deadlock-Router-VC ist genau dafür da.";
pub const ROUTER_PANEL_GUIDE_OWNER: &str = "**Deine Lane gehört dir**\nWer die Lane erstellt, ist ihr Owner: Nur der Owner kann umbenennen, das Limit setzen oder Leute rauswerfen. Verlässt der Owner die Lane, holt sie sich jemand anderes mit <:dl_crown:1522518265421631538> Owner übernehmen. Leere Lanes räumt der Bot automatisch weg.";
pub const ROUTER_PANEL_GUIDE_BUTTONS: &str = "**Die Buttons im Detail**\n<:dl_crown:1522518265421631538> **Owner übernehmen** — macht dich zum Owner, wenn der bisherige weg ist\n<:dl_rename:1522518272497418250> **Umbenennen** — gibt deiner Lane einen eigenen Namen\n<:dl_limit:1522518268345192588> **Limit setzen** — legt fest, wie viele Leute in die Lane passen\n<:dl_mode:1522518269456547962> **Modus wechseln** — ändert den Modus deiner Lane (Casual / Ranked / Street Brawl / Off Topic)\n<:dl_kick:1522518266298368073> **Kick** / <:dl_ban:1522518261290369034> **Ban** / <:dl_unban:1522518273827143751> **Unban** — wirft Störer raus bzw. sperrt und entsperrt sie für deine Lane\n🔓 **Rang-Gate** *(nur Ranked, nur Owner)* — macht deine Lane exklusiv für verifizierte Ränge in einem Fenster (Standard: dein Rang ±1,5). Niemand fliegt raus, wirkt nur auf neue Joins; nochmal drücken schaltet es aus";
pub const ROUTER_REPLY_NOT_IN_VOICE: &str =
    "Du bist gerade in keinem Sprachkanal — geh zuerst in Voice, dann klappt's. Einstieg:";
pub const ROUTER_REPLY_UNKNOWN_MODE: &str =
    "Diesen Modus kennt der Bot nicht — nimm einen der Buttons im Panel.";
pub const ROUTER_REPLY_CREATED_PREFIX: &str = "Deine Lane steht:";
pub const ROUTER_REPLY_NOT_CREATED: &str =
    "Das hat gerade nicht geklappt — versuch es in ein paar Sekunden nochmal.";
pub const ROUTER_REPLY_ALREADY_OWN_LANE: &str =
    "Du hast schon eine eigene Lane. Modus oder Name änderst du über <:dl_mode:1522518269456547962> Modus wechseln und <:dl_rename:1522518272497418250> Umbenennen:";
pub const ROUTER_REPLY_FLOOD_GUARD: &str =
    "Ganz schön viele Lanes auf einmal 😄 — warte kurz, dann geht's weiter.";
pub const ROUTER_REPLY_DEFAULT_SAVED: &str = "Als dein Standard gespeichert — ändern über ⚙️";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouterMode {
    pub id: &'static str,
    pub label: &'static str,
    pub category_id: u64,
    pub staging_id: u64,
    pub style: u8,
    pub emoji: (&'static str, &'static str),
}

pub const ROUTER_MODES: [RouterMode; 3] = [
    RouterMode {
        id: "casual",
        label: ROUTER_BUTTON_CASUAL,
        category_id: ROUTER_CATEGORY_CHILL,
        staging_id: 1501089974093873232,
        style: 1,
        emoji: ROUTER_EMOJI_CASUAL,
    },
    RouterMode {
        id: "ranked",
        label: ROUTER_BUTTON_RANKED,
        category_id: ROUTER_CATEGORY_CHILL,
        staging_id: 1412804671432818890,
        style: 3,
        emoji: ROUTER_EMOJI_RANKED,
    },
    RouterMode {
        id: "street_brawl",
        label: ROUTER_BUTTON_STREET_BRAWL,
        category_id: ROUTER_CATEGORY_CHILL,
        staging_id: 1357422958544420944,
        style: 2,
        emoji: ROUTER_EMOJI_BRAWL,
    },
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RouterApplyOutput {
    pub guild_id: u64,
    pub channel_id: u64,
    pub dry_run: bool,
    pub payload_format: String,
    pub stored_payload_format: Option<String>,
    pub stored_message_id: Option<u64>,
    pub action: String,
    pub message_id: Option<u64>,
    pub warnings: Vec<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RouterPanelAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterPanelMessage {
    pub message_id: u64,
    pub has_embeds: bool,
    pub has_components: bool,
    pub custom_ids: Vec<String>,
}

/// Rollen, die verifizierten Ranked-Zugang belegen (Router- **und** LFG-Gate).
///
/// Der Server hat sein Rang-System auf granulare Subrang-Rollen umgebaut; die
/// frühere Liste der 11 „Haupt-Rang-Rollen" existiert im Server nicht mehr
/// (tote IDs) und blockte damit jede Ranked-Aktion. Verifiziert wird stattdessen
/// über den Steam-Verify-Marker, den der Steam-Bot bei jeder Verknüpfung vergibt
/// (`STEAM_VERIFIED_ROLE_ID`); die neue Discord-Linked-Role zählt vorwärts mit.
pub const VERIFIED_RANK_ROLE_IDS: [u64; 2] = [
    1419608095533043774, // Steam Verifiziert✅ OLD — bot-vergebener Marker (aktueller Bestand)
    1522540241951658024, // Steam Verifiziert✅ — Discord Linked Role (nach W3.3-Aktivierung)
];

pub fn router_modes() -> &'static [RouterMode] {
    &ROUTER_MODES
}

pub fn router_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn router_panel_attachments() -> Vec<RouterPanelAttachment> {
    [
        ROUTER_HERO_BANNER_FILENAME,
        ROUTER_MANAGE_BANNER_FILENAME,
        ROUTER_GUIDE_BANNER_FILENAME,
    ]
    .into_iter()
    .enumerate()
    .map(|(id, filename)| RouterPanelAttachment {
        id: u8::try_from(id).unwrap_or(u8::MAX),
        filename: filename.to_string(),
        relative_path: format!("{ROUTER_BANNER_DIR}/{filename}"),
    })
    .collect()
}

pub fn router_mode(mode: &str) -> Option<RouterMode> {
    ROUTER_MODES
        .iter()
        .copied()
        .find(|candidate| candidate.id == mode)
}

pub fn mode_to_category(mode: &str) -> u64 {
    router_mode(mode)
        .map(|mode| mode.category_id)
        .unwrap_or(ROUTER_MODES[0].category_id)
}

pub fn mode_to_staging(mode: &str) -> u64 {
    router_mode(mode)
        .map(|mode| mode.staging_id)
        .unwrap_or(ROUTER_MODES[0].staging_id)
}

pub fn default_preset_for_mode(user_id: u64, mode: &str) -> DefaultPresetRecord {
    let mode = router_mode(mode).map(|mode| mode.id).unwrap_or("casual");
    let (base_name, limit) = match mode {
        "ranked" => ("Ranked".to_string(), 6),
        "street_brawl" => ("Street Brawl".to_string(), 4),
        _ => ("Chill Lane".to_string(), 6),
    };
    DefaultPresetRecord {
        user_id,
        mode: mode.to_string(),
        base_name,
        limit,
        min_rank: "unknown".to_string(),
    }
}

const STAGING_IDS: [u64; 3] = [
    1501089974093873232,
    1412804671432818890,
    1357422958544420944,
];

/// Lane-Wahl wie `_find_suitable_lane`: 1–5 Mitglieder, keine Stagings;
/// Co-Spieler-Lane gewinnt, sonst die erste.
pub fn pick_lane(
    lanes: &[(u64, Vec<u64>)],
    co_player_ids: &std::collections::HashSet<u64>,
) -> Option<u64> {
    let suitable: Vec<&(u64, Vec<u64>)> = lanes
        .iter()
        .filter(|(channel_id, members)| {
            !STAGING_IDS.contains(channel_id)
                && !members.is_empty()
                && members.len() < MAX_LANE_MEMBERS
        })
        .collect();
    if !co_player_ids.is_empty() {
        if let Some((channel_id, _)) = suitable
            .iter()
            .find(|(_, members)| members.iter().any(|m| co_player_ids.contains(m)))
        {
            return Some(*channel_id);
        }
    }
    suitable.first().map(|(channel_id, _)| *channel_id)
}

pub fn router_panel_body() -> Map<String, Value> {
    let attachments = router_panel_attachments();
    router_panel_body_for_attachments(&attachments)
}

fn router_panel_body_for_attachments(attachments: &[RouterPanelAttachment]) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".to_string(), json!(ROUTER_COMPONENTS_V2_FLAG));
    body.insert(
        "allowed_mentions".to_string(),
        json!({ "parse": Vec::<String>::new() }),
    );
    body.insert(
        "components".to_string(),
        json!([router_container(vec![
            router_media_gallery(ROUTER_HERO_BANNER_FILENAME),
            router_text_display(ROUTER_PANEL_INTRO.to_string()),
            router_action_row(
                ROUTER_MODES
                    .iter()
                    .map(|mode| {
                        router_emoji_button(
                            mode.label,
                            mode.style,
                            &format!("router_spawn_{}", mode.id),
                            mode.emoji,
                        )
                    })
                    .collect(),
            ),
            router_text_display(ROUTER_PANEL_MODE_HINT.to_string()),
            router_separator(false, 2),
            router_media_gallery(ROUTER_MANAGE_BANNER_FILENAME),
            router_text_display(ROUTER_PANEL_MANAGE_INTRO.to_string()),
            router_text_display(ROUTER_PANEL_LANE_CAPTION.to_string()),
            router_action_row(vec![
                router_emoji_button(ROUTER_BUTTON_CLAIM, 3, "tv_owner_claim", ROUTER_EMOJI_CROWN),
                router_emoji_button(
                    ROUTER_BUTTON_RENAME,
                    2,
                    "tv_rename_btn",
                    ROUTER_EMOJI_RENAME
                ),
                router_emoji_button(ROUTER_BUTTON_LIMIT, 2, "tv_limit_btn", ROUTER_EMOJI_LIMIT),
                router_emoji_button(
                    ROUTER_BUTTON_MODE,
                    2,
                    "tv_mode_switch_btn",
                    ROUTER_EMOJI_MODE
                ),
                json!({
                    "type": 2,
                    "style": 2,
                    "label": ROUTER_BUTTON_RANK_GATE,
                    "custom_id": "tv_rank_gate",
                    "emoji": { "name": "🔓" },
                }),
            ]),
            router_text_display(ROUTER_PANEL_MOD_CAPTION.to_string()),
            router_action_row(vec![
                router_emoji_button(ROUTER_BUTTON_KICK, 4, "tv_kick", ROUTER_EMOJI_KICK),
                router_emoji_button(ROUTER_BUTTON_BAN, 4, "tv_ban", ROUTER_EMOJI_BAN),
                router_emoji_button(ROUTER_BUTTON_UNBAN, 2, "tv_unban", ROUTER_EMOJI_UNBAN),
            ]),
            router_separator(false, 2),
            router_media_gallery(ROUTER_GUIDE_BANNER_FILENAME),
            router_text_display(ROUTER_PANEL_GUIDE_CREATE.to_string()),
            router_separator(true, 1),
            router_text_display(ROUTER_PANEL_GUIDE_OWNER.to_string()),
            router_separator(true, 1),
            router_text_display(ROUTER_PANEL_GUIDE_BUTTONS.to_string()),
        ])]),
    );
    body.insert("attachments".to_string(), json!(attachments));
    body
}

fn router_separator(divider: bool, spacing: u8) -> Value {
    json!({
        "type": 14,
        "divider": divider,
        "spacing": spacing,
    })
}

fn router_container(components: Vec<Value>) -> Value {
    json!({
        "type": 17,
        "accent_color": ROUTER_ACCENT_GOLD,
        "components": components,
    })
}

fn router_text_display(content: String) -> Value {
    json!({
        "type": 10,
        "content": content,
    })
}

fn router_media_gallery(filename: &str) -> Value {
    json!({
        "type": 12,
        "items": [{
            "media": {
                "url": format!("attachment://{filename}"),
            },
        }],
    })
}

fn router_action_row(components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "components": components,
    })
}

fn router_emoji_button(label: &str, style: u8, custom_id: &str, emoji: (&str, &str)) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
        "emoji": { "name": emoji.0, "id": emoji.1 },
    })
}

pub fn voice_guide_detail_text() -> String {
    format!(
        "**📖 Voice-Lanes im Detail**\n{}\nRouter-VC-Join: erst Modus wählen, dann Verschiebung.\nRanked ist offen für alle — der Lane-Owner kann per 🔓 Rang-Gate optional nur verifizierte Ränge reinlassen (Steam verknüpfen in <#1398021105339334666>).\n{}\n{}\n**⚙️ Voreinstellungen** — Name, Limit (und Rang-Bereich) jederzeit festlegen, auch ohne in einer Lane zu sein — wird bei jeder neuen Lane automatisch angewandt. 💾 Presets sichern zusätzlich den Stand einer laufenden Lane.\n**Mitspieler finden** — Gesuch per Klick (Modus, Rang, Wann), erscheint in <#1522769149208821881>; 🔔 benachrichtigt dich bei passenden Gesuchen.",
        ROUTER_PANEL_GUIDE_CREATE,
        ROUTER_PANEL_GUIDE_OWNER,
        ROUTER_PANEL_GUIDE_BUTTONS,
    )
}

pub fn voice_guide_detail_reply() -> BridgeReply {
    let text = voice_guide_detail_text();
    let banner_path = router_repo_root()
        .join(ROUTER_BANNER_DIR)
        .join(ROUTER_HERO_BANNER_FILENAME);
    let banner = std::fs::read(banner_path)
        .ok()
        .map(|data| BridgeAttachment {
            filename: ROUTER_HERO_BANNER_FILENAME.to_string(),
            data,
        });
    let mut components = Vec::new();
    if banner.is_some() {
        components.push(router_media_gallery(ROUTER_HERO_BANNER_FILENAME));
    }
    components.push(router_text_display(text.clone()));
    let fallback = BridgeReply {
        content: Some(text.clone()),
        ephemeral: true,
        allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
        ..BridgeReply::default()
    };
    BridgeReply {
        components: Some(json!([router_container(components)])),
        message_flags: Some(64 | ROUTER_COMPONENTS_V2_FLAG),
        allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
        fallback: Some(Box::new(fallback)),
        attachments: banner.into_iter().collect(),
        ..BridgeReply::default()
    }
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait RouterPort: Send + Sync {
    /// Voice-Kanäle einer Kategorie mit ihren Mitglieder-IDs.
    async fn category_lanes(&self, guild_id: u64, category_id: u64) -> Vec<(u64, Vec<u64>)>;
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    async fn move_member(&self, guild_id: u64, user_id: u64, channel_id: u64)
        -> Result<(), String>;
    async fn send_dm(&self, user_id: u64, text: String);
}

#[async_trait::async_trait]
pub trait RouterInterfacePort: Send + Sync {
    async fn post_rich(
        &self,
        channel_id: u64,
        body: Map<String, Value>,
        attachments: &[RouterPanelAttachment],
    ) -> Result<u64, String>;
    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
        attachments: &[RouterPanelAttachment],
    ) -> Result<(), String>;
    async fn delete_message(
        &self,
        channel_id: u64,
        message_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<RouterPanelMessage>, String>;
}

pub struct RouterInterface {
    pool: PgPool,
    port: Arc<dyn RouterInterfacePort>,
}

impl RouterInterface {
    pub fn new(pool: PgPool, port: Arc<dyn RouterInterfacePort>) -> Arc<Self> {
        Arc::new(Self { pool, port })
    }

    pub async fn ensure_panel(&self) {
        if let Err(err) = self.apply_panel(true).await {
            tracing::warn!(%err, "RouterInterface: Panel konnte nicht angewendet werden");
        }
    }

    pub async fn apply_panel(&self, confirm: bool) -> Result<RouterApplyOutput, String> {
        let attachments = router_panel_attachments();
        let body = router_panel_body_for_attachments(&attachments);
        let recent = match self
            .port
            .recent_bot_messages(ROUTER_TEXT_CHANNEL_ID, 15)
            .await
        {
            Ok(messages) => Some(messages),
            Err(err) => {
                tracing::warn!(%err, "RouterInterface: History-Scan fehlgeschlagen");
                None
            }
        };
        let history_checked = recent.is_some();
        let history_message_id = recent.as_deref().and_then(find_existing_router_v2_panel);
        let kv_message_id = self.panel_message_id(ROUTER_PANEL_MESSAGE_KEY).await;
        let stored_payload_format = self.panel_payload_format().await;
        let planned_message_id = kv_message_id.or(history_message_id);
        let planned_action = if planned_message_id.is_some() {
            "planned_edit"
        } else {
            "planned_post"
        };
        let mut output = RouterApplyOutput {
            guild_id: self.guild_id(),
            channel_id: ROUTER_TEXT_CHANNEL_ID,
            dry_run: !confirm,
            payload_format: ROUTER_PAYLOAD_FORMAT.to_string(),
            stored_payload_format,
            stored_message_id: kv_message_id,
            action: planned_action.to_string(),
            message_id: planned_message_id,
            warnings: Vec::new(),
            payload: Value::Object(body.clone()),
        };
        if !confirm {
            if !history_checked {
                output.warnings.push(
                    "Router-Panel: History-Scan fehlgeschlagen; Dry-Run ohne Adoption.".to_string(),
                );
            }
            self.add_legacy_cleanup_dry_run_warnings(planned_message_id, &mut output)
                .await;
            return Ok(output);
        }

        validate_router_panel_attachments(&attachments)?;

        if let Some(message_id) = kv_message_id {
            if self
                .port
                .edit_rich(
                    ROUTER_TEXT_CHANNEL_ID,
                    message_id,
                    body.clone(),
                    &attachments,
                )
                .await
                .is_ok()
            {
                self.store_panel_metadata(message_id).await;
                output.dry_run = false;
                output.action = "edited".to_string();
                output.message_id = Some(message_id);
                self.cleanup_legacy_panels(message_id, &mut output).await;
                return Ok(output);
            }
            if history_message_id == Some(message_id) {
                return Err(format!(
                    "RouterInterface: vorhandenes Panel {message_id} konnte nicht editiert werden"
                ));
            }
        }

        if !history_checked {
            return Err(
                "RouterInterface: ohne erfolgreichen History-Scan wird kein neues Panel gepostet"
                    .to_string(),
            );
        }

        if let Some(message_id) = history_message_id {
            match self
                .port
                .edit_rich(
                    ROUTER_TEXT_CHANNEL_ID,
                    message_id,
                    body.clone(),
                    &attachments,
                )
                .await
            {
                Ok(()) => {
                    self.store_panel_metadata(message_id).await;
                    output.dry_run = false;
                    output.action = "adopted_edit".to_string();
                    output.message_id = Some(message_id);
                    self.cleanup_legacy_panels(message_id, &mut output).await;
                    return Ok(output);
                }
                Err(err) => {
                    return Err(format!(
                        "RouterInterface: adoptiertes Panel {message_id} konnte nicht editiert werden: {err}"
                    ));
                }
            }
        }

        match self
            .port
            .post_rich(ROUTER_TEXT_CHANNEL_ID, body, &attachments)
            .await
        {
            Ok(message_id) => {
                self.store_panel_metadata(message_id).await;
                output.dry_run = false;
                output.action = "posted".to_string();
                output.message_id = Some(message_id);
                self.cleanup_legacy_panels(message_id, &mut output).await;
                Ok(output)
            }
            Err(err) => Err(format!(
                "RouterInterface: Panel konnte nicht gepostet werden: {err}"
            )),
        }
    }

    async fn panel_message_id(&self, key: &str) -> Option<u64> {
        kv::get(&self.pool, ROUTER_PANEL_KV_NS, key)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
    }

    async fn panel_payload_format(&self) -> Option<String> {
        kv::get(&self.pool, ROUTER_PANEL_KV_NS, ROUTER_PAYLOAD_FORMAT_KEY)
            .await
            .ok()
            .flatten()
    }

    async fn store_panel_metadata(&self, message_id: u64) {
        self.store_panel_message_id(ROUTER_PANEL_MESSAGE_KEY, message_id)
            .await;
        if let Err(err) = kv::set(
            &self.pool,
            ROUTER_PANEL_KV_NS,
            ROUTER_PAYLOAD_FORMAT_KEY,
            ROUTER_PAYLOAD_FORMAT,
        )
        .await
        {
            tracing::warn!(%err, "RouterInterface: Payload-Format konnte nicht gespeichert werden");
        }
    }

    async fn store_panel_message_id(&self, key: &str, message_id: u64) {
        if let Err(err) =
            kv::set(&self.pool, ROUTER_PANEL_KV_NS, key, &message_id.to_string()).await
        {
            tracing::warn!(%err, key, "RouterInterface: Message-ID konnte nicht gespeichert werden");
        }
    }

    async fn legacy_panel_message_ids(&self) -> Vec<(&'static str, u64)> {
        let mut ids = Vec::new();
        for key in ROUTER_LEGACY_MESSAGE_KEYS {
            if let Some(message_id) = self.panel_message_id(key).await {
                ids.push((key, message_id));
            }
        }
        ids
    }

    async fn add_legacy_cleanup_dry_run_warnings(
        &self,
        current_message_id: Option<u64>,
        output: &mut RouterApplyOutput,
    ) {
        for (key, message_id) in self.legacy_panel_message_ids().await {
            if Some(message_id) == current_message_id {
                output.warnings.push(format!(
                    "Router-Panel: Legacy-KV-Key {key} zeigt auf aktuelles Panel {message_id}; Dry-Run würde nur den Key entfernen."
                ));
            } else {
                output.warnings.push(format!(
                    "Router-Panel: Legacy-Message {message_id} aus {key} würde gelöscht."
                ));
            }
        }
    }

    async fn cleanup_legacy_panels(&self, current_message_id: u64, output: &mut RouterApplyOutput) {
        for (key, message_id) in self.legacy_panel_message_ids().await {
            if message_id != current_message_id {
                if let Err(err) = self
                    .port
                    .delete_message(
                        ROUTER_TEXT_CHANNEL_ID,
                        message_id,
                        "Router: Legacy-Panel nach Components-V2-Cutover bereinigen",
                    )
                    .await
                {
                    output.warnings.push(format!(
                        "Router-Panel: Legacy-Message {message_id} aus {key} konnte nicht gelöscht werden: {err}"
                    ));
                }
            }
            if let Err(err) = kv::delete(&self.pool, ROUTER_PANEL_KV_NS, key).await {
                output.warnings.push(format!(
                    "Router-Panel: Legacy-KV-Key {key} konnte nicht entfernt werden: {err}"
                ));
            }
        }
    }

    fn guild_id(&self) -> u64 {
        ROUTER_GUILD_ID
    }
}

fn validate_router_panel_attachments(attachments: &[RouterPanelAttachment]) -> Result<(), String> {
    let repo_root = router_repo_root();
    for attachment in attachments {
        let path = repo_root.join(&attachment.relative_path);
        if !path.is_file() {
            return Err(format!(
                "Router-Banner `{}` fehlt; Router-Panel wird nicht gepostet/editiert",
                path.display()
            ));
        }
    }
    Ok(())
}

fn find_existing_router_v2_panel(messages: &[RouterPanelMessage]) -> Option<u64> {
    messages
        .iter()
        .find(|message| {
            !message.has_embeds
                && message.has_components
                && message
                    .custom_ids
                    .iter()
                    .any(|custom_id| custom_id.starts_with("router_spawn_"))
        })
        .map(|message| message.message_id)
}

pub struct LaneRouter {
    pub pool: PgPool,
    pub port: Arc<dyn RouterPort>,
    pub engine: Arc<TempVoiceEngine>,
    pub analyzer: Option<Arc<dl_activity::analyzer::ActivityAnalyzer>>,
    spawn_history: tokio::sync::Mutex<HashMap<u64, Vec<Instant>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterSpawnOutcome {
    Created { lane_id: u64 },
    AlreadyOwnLane { lane_id: u64 },
    FloodLimited,
    NotInVoice,
    NotCreated,
    UnknownMode,
}

impl LaneRouter {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn RouterPort>,
        engine: Arc<TempVoiceEngine>,
        analyzer: Option<Arc<dl_activity::analyzer::ActivityAnalyzer>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            engine,
            analyzer,
            spawn_history: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub async fn ensure_schema(&self) -> Result<(), crate::db::VoiceDbError> {
        // Zentraler Cutover: voice.router_user_prefs wird vom zentralen Migrator
        // bereitgestellt; lokales DDL ist hier absichtlich entfernt.
        Ok(())
    }

    pub async fn user_pref(&self, user_id: u64) -> Option<(String, bool)> {
        let user_id = u64_to_i64("router_user_prefs.user_id", user_id).ok()?;
        sqlx::query!(
            r#"
            SELECT mode, auto_join
              FROM voice.router_user_prefs
             WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| (row.mode, row.auto_join))
    }

    pub async fn set_user_pref(&self, user_id: u64, mode: &str, auto_join: bool) {
        let mode = mode.to_string();
        let result = async {
            let user_id = u64_to_i64("router_user_prefs.user_id", user_id)?;
            sqlx::query!(
                r#"
                INSERT INTO voice.router_user_prefs (user_id, mode, auto_join, updated_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (user_id) DO UPDATE SET
                    mode = EXCLUDED.mode,
                    auto_join = EXCLUDED.auto_join,
                    updated_at = NOW()
                "#,
                user_id,
                mode,
                auto_join,
            )
            .execute(&self.pool)
            .await?;
            Ok::<(), crate::db::VoiceDbError>(())
        }
        .await;
        if let Err(err) = result {
            tracing::warn!(%err, user_id, "Router: User-Pref konnte nicht gespeichert werden");
        }
    }

    pub async fn handle_event(self: &Arc<Self>, event: VoiceEvent) {
        let (guild_id, user_id, channel_id) = match event {
            VoiceEvent::Join {
                guild_id,
                user_id,
                channel_id,
            } => (guild_id, user_id, channel_id),
            VoiceEvent::Move {
                guild_id,
                user_id,
                to_channel_id,
                ..
            } => (guild_id, user_id, to_channel_id),
            _ => return,
        };
        if channel_id != ROUTER_VC_ID {
            return;
        }
        let Some(default) = self.default_preset(user_id).await else {
            return; // ohne Präferenz: User bleibt im Router-VC (Panel hilft)
        };
        if !self.flood_guard_allows(user_id).await {
            return;
        }
        if self
            .engine
            .create_router_lane(guild_id, user_id, &default.mode, ROUTER_VC_ID)
            .await
            .is_some()
        {
            self.mark_spawn_created(user_id).await;
        }
    }

    pub async fn spawn_lane_from_current_voice(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        mode: &str,
    ) -> RouterSpawnOutcome {
        self.spawn_lane_from_current_voice_with_role_ids(guild_id, user_id, mode, &[])
            .await
    }

    pub async fn spawn_lane_from_current_voice_with_role_ids(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        mode: &str,
        interaction_role_ids: &[u64],
    ) -> RouterSpawnOutcome {
        if router_mode(mode).is_none() {
            return RouterSpawnOutcome::UnknownMode;
        }
        if !self.flood_guard_allows(user_id).await {
            return RouterSpawnOutcome::FloodLimited;
        }
        let Some(current_channel_id) = self.port.member_voice_channel(guild_id, user_id).await
        else {
            return RouterSpawnOutcome::NotInVoice;
        };
        if self
            .user_owns_tempvoice_lane(guild_id, current_channel_id, user_id)
            .await
        {
            return RouterSpawnOutcome::AlreadyOwnLane {
                lane_id: current_channel_id,
            };
        }
        let _ = interaction_role_ids;
        match self
            .engine
            .create_router_lane(guild_id, user_id, mode, current_channel_id)
            .await
        {
            Some(lane_id) => {
                self.mark_spawn_created(user_id).await;
                RouterSpawnOutcome::Created { lane_id }
            }
            None => RouterSpawnOutcome::NotCreated,
        }
    }

    pub async fn cleanup_lfg_created_lane(
        self: &Arc<Self>,
        _guild_id: u64,
        lane_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.engine.cleanup_lane(lane_id, reason).await;
        Ok(())
    }

    async fn flood_guard_allows(&self, user_id: u64) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(ROUTER_FLOOD_WINDOW_SECS);
        let mut history = self.spawn_history.lock().await;
        let entries = history.entry(user_id).or_default();
        entries.retain(|created_at| now.saturating_duration_since(*created_at) < window);
        entries.len() < ROUTER_FLOOD_MAX_CREATES
    }

    async fn mark_spawn_created(&self, user_id: u64) {
        let now = Instant::now();
        let window = Duration::from_secs(ROUTER_FLOOD_WINDOW_SECS);
        let mut history = self.spawn_history.lock().await;
        let entries = history.entry(user_id).or_default();
        entries.retain(|created_at| now.saturating_duration_since(*created_at) < window);
        entries.push(now);
    }

    pub async fn default_preset(&self, user_id: u64) -> Option<DefaultPresetRecord> {
        self.engine
            .store
            .get_default_preset(user_id)
            .await
            .ok()
            .flatten()
    }

    pub async fn ensure_default_mode_if_missing(&self, user_id: u64, mode: &str) -> bool {
        if self.default_preset(user_id).await.is_some() {
            return false;
        }
        let Some(mode) = router_mode(mode) else {
            return false;
        };
        let record = default_preset_for_mode(user_id, mode.id);
        match self.engine.store.save_default_preset(record).await {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(%err, user_id, mode = mode.id, "Router: Default-Preset konnte nicht gespeichert werden");
                false
            }
        }
    }

    async fn user_owns_tempvoice_lane(&self, guild_id: u64, channel_id: u64, user_id: u64) -> bool {
        if let Some(owner_id) = self.engine.lane_owner(channel_id).await {
            return owner_id == user_id;
        }
        match self.engine.store.all_lanes().await {
            Ok(lanes) => lanes.iter().any(|lane| {
                lane.guild_id == guild_id
                    && lane.channel_id == channel_id
                    && lane.owner_id == user_id
            }),
            Err(err) => {
                tracing::warn!(%err, channel_id, user_id, "Router: TempVoice-Lane konnte nicht geprüft werden");
                false
            }
        }
    }

    /// Wie `_smart_route`: Ranked-Gate → Co-Spieler-Lane → erste passende
    /// → neue Lane über die TempVoice-Engine.
    pub async fn smart_route(self: &Arc<Self>, guild_id: u64, user_id: u64, mode: &str) {
        let category_id = mode_to_category(mode);
        let co_player_ids: std::collections::HashSet<u64> = match &self.analyzer {
            Some(analyzer) => analyzer
                .top_co_players(user_id, 20)
                .await
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
            None => std::collections::HashSet::new(),
        };
        let lanes = self.port.category_lanes(guild_id, category_id).await;
        if let Some(lane_id) = pick_lane(&lanes, &co_player_ids) {
            if let Err(err) = self.port.move_member(guild_id, user_id, lane_id).await {
                tracing::warn!(%err, user_id, lane_id, "Router: Move fehlgeschlagen");
            }
            return;
        }
        // Keine passende Lane → neue über die Engine (User steht im Router-VC)
        self.engine
            .create_router_lane(guild_id, user_id, mode, ROUTER_VC_ID)
            .await;
    }
}

fn router_reply_with_default_hint(base: String, default_saved: bool) -> String {
    if default_saved {
        format!("{base}\n{ROUTER_REPLY_DEFAULT_SAVED}")
    } else {
        base
    }
}

/// Panel-Buttons: router_mode_{mode} + router_autojoin_toggle.
struct RouterPanelHandler {
    router: Arc<LaneRouter>,
}

#[async_trait::async_trait]
impl InteractionHandler for RouterPanelHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.custom_id == "voice:guide:detail" {
            return voice_guide_detail_reply();
        }
        if let Some(mode) = interaction.custom_id.strip_prefix("router_spawn_") {
            let default_saved = self
                .router
                .ensure_default_mode_if_missing(interaction.user_id, mode)
                .await;
            return match self
                .router
                .spawn_lane_from_current_voice_with_role_ids(
                    interaction.guild_id,
                    interaction.user_id,
                    mode,
                    &interaction.role_ids,
                )
                .await
            {
                RouterSpawnOutcome::Created { lane_id } => {
                    BridgeReply::ephemeral_text(router_reply_with_default_hint(
                        format!("{ROUTER_REPLY_CREATED_PREFIX} <#{lane_id}>"),
                        default_saved,
                    ))
                }
                RouterSpawnOutcome::AlreadyOwnLane { lane_id } => {
                    BridgeReply::ephemeral_text(router_reply_with_default_hint(
                        format!("{ROUTER_REPLY_ALREADY_OWN_LANE} <#{lane_id}>"),
                        default_saved,
                    ))
                }
                RouterSpawnOutcome::FloodLimited => {
                    BridgeReply::ephemeral_text(ROUTER_REPLY_FLOOD_GUARD)
                }
                RouterSpawnOutcome::NotInVoice => {
                    BridgeReply::ephemeral_text(router_reply_with_default_hint(
                        format!("{ROUTER_REPLY_NOT_IN_VOICE} <#{ROUTER_VC_ID}>"),
                        default_saved,
                    ))
                }
                RouterSpawnOutcome::UnknownMode => {
                    BridgeReply::ephemeral_text(ROUTER_REPLY_UNKNOWN_MODE)
                }
                RouterSpawnOutcome::NotCreated => {
                    BridgeReply::ephemeral_text(ROUTER_REPLY_NOT_CREATED)
                }
            };
        }
        if interaction.custom_id == "router_autojoin_toggle" {
            let Some((mode, auto_join)) = self.router.user_pref(interaction.user_id).await else {
                return BridgeReply::ephemeral_text(ROUTER_SELECT_MODE_BEFORE_AUTOJOIN);
            };
            let new_auto = !auto_join;
            self.router
                .set_user_pref(interaction.user_id, &mode, new_auto)
                .await;
            return BridgeReply::ephemeral_text(if new_auto {
                "Auto-Join **an** — du wirst beim Betreten des Routers automatisch einsortiert."
            } else {
                "Auto-Join **aus** — wähle im Router künftig selbst per Knopf."
            });
        }
        let Some(mode) = interaction.custom_id.strip_prefix("router_mode_") else {
            return BridgeReply::ephemeral_text("Unbekannte Aktion.");
        };
        let mode = match router_mode(mode) {
            Some(mode) => mode.id.to_string(),
            None => return BridgeReply::ephemeral_text("Unbekannter Modus."),
        };
        let auto_join = self
            .router
            .user_pref(interaction.user_id)
            .await
            .map(|(_, auto)| auto)
            .unwrap_or(false);
        self.router
            .set_user_pref(interaction.user_id, &mode, auto_join)
            .await;
        // Steht der User gerade im Router-VC → sofort routen
        if self
            .router
            .port
            .member_voice_channel(interaction.guild_id, interaction.user_id)
            .await
            == Some(ROUTER_VC_ID)
        {
            self.router
                .smart_route(interaction.guild_id, interaction.user_id, &mode)
                .await;
            return BridgeReply::ephemeral_text(format!(
                "Modus **{mode}** gespeichert — ich sortiere dich ein."
            ));
        }
        BridgeReply::ephemeral_text(format!(
            "Modus **{mode}** gespeichert — gilt beim nächsten Router-Besuch."
        ))
    }
}

pub fn register(router_panel: &mut InteractionRouter, router: Arc<LaneRouter>) {
    let handler = Arc::new(RouterPanelHandler { router });
    router_panel.on_prefix("router_spawn_", handler.clone());
    router_panel.on_prefix("router_mode_", handler.clone());
    router_panel.on_custom_id("router_autojoin_toggle", handler.clone());
    router_panel.on_custom_id("voice:guide:detail", handler);
}

pub fn spawn(router: Arc<LaneRouter>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_voice();
    tokio::spawn(async move {
        if let Err(err) = router.ensure_schema().await {
            tracing::warn!(%err, "Router: Schema-Anlage fehlgeschlagen");
        }
        loop {
            match events.recv().await {
                Ok(event) => router.handle_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock, clippy::type_complexity)]

    use super::*;
    use serde_json::Map;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex as StdMutex;

    #[test]
    fn lane_wahl_wie_python() {
        let lanes = vec![
            (1501089974093873232u64, vec![1, 2]), // Staging → nie
            (10, vec![]),                         // leer → nie
            (11, vec![1, 2, 3, 4, 5, 6]),         // voll → nie
            (12, vec![7, 8]),
            (13, vec![9]),
        ];
        // ohne Co-Spieler: erste passende (12)
        assert_eq!(pick_lane(&lanes, &Default::default()), Some(12));
        // Co-Spieler 9 sitzt in 13 → 13 gewinnt
        let co: std::collections::HashSet<u64> = [9].into();
        assert_eq!(pick_lane(&lanes, &co), Some(13));
        // nichts passendes
        assert_eq!(pick_lane(&[(10, vec![])], &Default::default()), None);
    }

    #[test]
    fn modus_zuordnung() {
        assert_eq!(mode_to_category("ranked"), ROUTER_CATEGORY_CHILL);
        assert_eq!(mode_to_category("street_brawl"), ROUTER_CATEGORY_CHILL);
        assert_eq!(mode_to_category("casual"), ROUTER_CATEGORY_CHILL);
        assert_eq!(mode_to_category("quatsch"), ROUTER_CATEGORY_CHILL);
        assert_eq!(mode_to_staging("ranked"), 1412804671432818890);
    }

    #[test]
    fn router_panel_body_ist_components_v2_mit_router_und_tv_buttons() {
        let body = router_panel_body();
        assert_eq!(body.get("flags"), Some(&json!(ROUTER_COMPONENTS_V2_FLAG)));
        assert!(body
            .get("allowed_mentions")
            .and_then(|value| value["parse"].as_array())
            .expect("allowed mentions")
            .is_empty());
        assert!(body.get("embeds").is_none());
        let container = &body
            .get("components")
            .and_then(serde_json::Value::as_array)
            .expect("components")[0];
        assert_eq!(container["type"], 17);
        assert_eq!(container["accent_color"], ROUTER_ACCENT_GOLD);
        let container_components = container["components"]
            .as_array()
            .expect("container components");
        let media_urls: Vec<&str> = container_components
            .iter()
            .filter(|component| component["type"] == 12)
            .filter_map(|component| component["items"][0]["media"]["url"].as_str())
            .collect();
        assert_eq!(
            media_urls,
            vec![
                "attachment://router-hero.png",
                "attachment://divider-lane-verwalten.png",
                "attachment://divider-anleitung.png",
            ]
        );
        let text_displays: Vec<&str> = container_components
            .iter()
            .filter(|component| component["type"] == 10)
            .filter_map(|component| component["content"].as_str())
            .collect();
        assert_eq!(
            text_displays,
            vec![
                ROUTER_PANEL_INTRO,
                ROUTER_PANEL_MODE_HINT,
                ROUTER_PANEL_MANAGE_INTRO,
                ROUTER_PANEL_LANE_CAPTION,
                ROUTER_PANEL_MOD_CAPTION,
                ROUTER_PANEL_GUIDE_CREATE,
                ROUTER_PANEL_GUIDE_OWNER,
                ROUTER_PANEL_GUIDE_BUTTONS,
            ]
        );
        assert_eq!(
            container_components
                .iter()
                .filter(|component| component["type"] == 14)
                .count(),
            4
        );
        assert_eq!(
            body.get("attachments").expect("attachments"),
            &json!([
                {"id": 0, "filename": "router-hero.png"},
                {"id": 1, "filename": "divider-lane-verwalten.png"},
                {"id": 2, "filename": "divider-anleitung.png"},
            ])
        );
        let custom_ids: Vec<&str> = container_components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert!(!custom_ids.contains(&"tv_tag_filter"));
        assert_eq!(
            custom_ids,
            vec![
                "router_spawn_casual",
                "router_spawn_ranked",
                "router_spawn_street_brawl",
                "tv_owner_claim",
                "tv_rename_btn",
                "tv_limit_btn",
                "tv_mode_switch_btn",
                "tv_rank_gate",
                "tv_kick",
                "tv_ban",
                "tv_unban",
            ]
        );
    }

    #[test]
    fn voice_guide_detail_reply_ist_v2_mit_banner() {
        let reply = voice_guide_detail_reply();
        assert_eq!(reply.message_flags, Some(64 | ROUTER_COMPONENTS_V2_FLAG));
        assert_eq!(reply.attachments.len(), 1);
        assert_eq!(reply.attachments[0].filename, ROUTER_HERO_BANNER_FILENAME);
        let container = &reply
            .components
            .as_ref()
            .and_then(serde_json::Value::as_array)
            .expect("components")[0];
        let children = container["components"]
            .as_array()
            .expect("container children");
        assert_eq!(
            children[0]["items"][0]["media"]["url"],
            format!("attachment://{ROUTER_HERO_BANNER_FILENAME}")
        );
        assert!(children
            .iter()
            .filter_map(|component| component["content"].as_str())
            .any(|content| content.contains(ROUTER_PANEL_GUIDE_BUTTONS)));
        assert!(reply.fallback.is_some());
    }

    #[derive(Default)]
    struct MockRouterInterfacePort {
        posts: StdMutex<
            Vec<(
                u64,
                Map<String, serde_json::Value>,
                Vec<RouterPanelAttachment>,
            )>,
        >,
        edits: StdMutex<
            Vec<(
                u64,
                u64,
                Map<String, serde_json::Value>,
                Vec<RouterPanelAttachment>,
            )>,
        >,
        deletes: StdMutex<Vec<(u64, u64, String)>>,
        fail_edits: StdMutex<bool>,
        fail_recent: StdMutex<bool>,
        recent: StdMutex<Vec<RouterPanelMessage>>,
    }

    #[async_trait::async_trait]
    impl RouterInterfacePort for MockRouterInterfacePort {
        async fn post_rich(
            &self,
            channel_id: u64,
            body: Map<String, serde_json::Value>,
            attachments: &[RouterPanelAttachment],
        ) -> Result<u64, String> {
            self.posts
                .lock()
                .expect("posts")
                .push((channel_id, body, attachments.to_vec()));
            Ok(9000 + self.posts.lock().expect("posts").len() as u64)
        }

        async fn edit_rich(
            &self,
            channel_id: u64,
            message_id: u64,
            body: Map<String, serde_json::Value>,
            attachments: &[RouterPanelAttachment],
        ) -> Result<(), String> {
            if *self.fail_edits.lock().expect("fail") {
                return Err("missing".to_string());
            }
            self.edits.lock().expect("edits").push((
                channel_id,
                message_id,
                body,
                attachments.to_vec(),
            ));
            Ok(())
        }

        async fn delete_message(
            &self,
            channel_id: u64,
            message_id: u64,
            reason: &str,
        ) -> Result<(), String> {
            self.deletes.lock().expect("deletes").push((
                channel_id,
                message_id,
                reason.to_string(),
            ));
            Ok(())
        }

        async fn recent_bot_messages(
            &self,
            _channel_id: u64,
            _limit: u8,
        ) -> Result<Vec<RouterPanelMessage>, String> {
            if *self.fail_recent.lock().expect("fail recent") {
                return Err("history failed".to_string());
            }
            Ok(self.recent.lock().expect("recent").clone())
        }
    }

    struct NoopRouterPort;

    #[async_trait::async_trait]
    impl RouterPort for NoopRouterPort {
        async fn category_lanes(&self, _guild_id: u64, _category_id: u64) -> Vec<(u64, Vec<u64>)> {
            Vec::new()
        }
        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            Vec::new()
        }
        async fn member_voice_channel(&self, _guild_id: u64, _user_id: u64) -> Option<u64> {
            None
        }
        async fn move_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _channel_id: u64,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn send_dm(&self, _user_id: u64, _text: String) {}
    }

    #[derive(Default)]
    struct StaticRouterPort {
        voice_channel: StdMutex<Option<u64>>,
        role_ids: StdMutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl RouterPort for StaticRouterPort {
        async fn category_lanes(&self, _guild_id: u64, _category_id: u64) -> Vec<(u64, Vec<u64>)> {
            Vec::new()
        }

        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            self.role_ids.lock().expect("roles").clone()
        }

        async fn member_voice_channel(&self, _guild_id: u64, _user_id: u64) -> Option<u64> {
            *self.voice_channel.lock().expect("voice")
        }

        async fn move_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _channel_id: u64,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn send_dm(&self, _user_id: u64, _text: String) {}
    }

    struct NoopLanePort;

    #[async_trait::async_trait]
    impl crate::tempvoice::LanePort for NoopLanePort {
        async fn create_voice_channel(
            &self,
            _guild_id: u64,
            _category_id: Option<u64>,
            _name: &str,
            _user_limit: i64,
        ) -> Result<u64, String> {
            Ok(1)
        }
        async fn delete_channel(&self, _channel_id: u64, _reason: &str) -> Result<(), String> {
            Ok(())
        }
        async fn move_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _channel_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn rename_channel(
            &self,
            _channel_id: u64,
            _name: &str,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn set_member_connect(
            &self,
            _channel_id: u64,
            _user_id: u64,
            _connect: Option<bool>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn apply_member_connect_batch(
            &self,
            _channel_id: u64,
            _denied_user_ids: &HashSet<u64>,
            _clear_user_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn set_role_connect(
            &self,
            _channel_id: u64,
            _role_id: u64,
            _connect: Option<bool>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn apply_role_connect_batch(
            &self,
            _guild_id: u64,
            _channel_id: u64,
            _allowed_role_ids: &HashSet<u64>,
            _clear_role_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn apply_role_connect_overwrites(
            &self,
            _guild_id: u64,
            _channel_id: u64,
            _allowed_role_ids: &HashSet<u64>,
            _denied_role_ids: &HashSet<u64>,
            _clear_role_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn set_user_limit(
            &self,
            _channel_id: u64,
            _limit: i64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn disconnect_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn member_display_name(&self, _guild_id: u64, _user_id: u64) -> Option<String> {
            None
        }
        async fn add_role(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _role_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn remove_role(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _role_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn set_nick(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _nick: Option<&str>,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn member_nick(&self, _guild_id: u64, _user_id: u64) -> Option<String> {
            None
        }
        async fn channel_user_limit(&self, _guild_id: u64, _channel_id: u64) -> Option<i64> {
            None
        }
        async fn guild_role_names(&self, _guild_id: u64) -> Vec<(u64, String)> {
            Vec::new()
        }
        async fn set_channel_category(
            &self,
            _channel_id: u64,
            _category_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn member_voice_channel(&self, _guild_id: u64, _user_id: u64) -> Option<u64> {
            None
        }
        async fn member_role_names(&self, _guild_id: u64, _user_id: u64) -> Vec<String> {
            Vec::new()
        }
        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            Vec::new()
        }
        async fn channel_members(&self, _guild_id: u64, _channel_id: u64) -> Vec<u64> {
            Vec::new()
        }
        async fn channel_name(&self, _guild_id: u64, _channel_id: u64) -> Option<String> {
            None
        }
        async fn channel_category(&self, _guild_id: u64, _channel_id: u64) -> Option<u64> {
            None
        }
        async fn category_voice_channel_names(
            &self,
            _guild_id: u64,
            _category_id: u64,
        ) -> Vec<String> {
            Vec::new()
        }
        async fn category_voice_channels(
            &self,
            _guild_id: u64,
            _category_id: u64,
        ) -> Vec<(u64, String)> {
            Vec::new()
        }
        async fn channel_created_at(&self, _channel_id: u64) -> Option<i64> {
            None
        }
    }

    fn test_engine(pool: sqlx::PgPool) -> Arc<TempVoiceEngine> {
        TempVoiceEngine::new(
            crate::tempvoice::TempVoiceConfig {
                guild_id_hint: 1,
                staging_channels: HashSet::new(),
                fixed_lane_ids: HashSet::new(),
                tempvoice_categories: HashSet::new(),
                minrank_categories: HashSet::new(),
                ranked_category_id: 0,
                staging_rules: HashMap::new(),
            },
            crate::tempvoice::TempVoiceStore::new(pool),
            Arc::new(NoopLanePort),
        )
    }

    #[tokio::test]
    async fn router_interface_panel_wird_idempotent_ueber_kv_gepflegt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockRouterInterfacePort::default());
        let interface = RouterInterface::new(pool.clone(), port.clone());

        interface.ensure_panel().await;
        {
            let posts = port.posts.lock().expect("posts");
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0].2, router_panel_attachments());
        }
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9001")
        );
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_PAYLOAD_FORMAT_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some(ROUTER_PAYLOAD_FORMAT)
        );

        interface.ensure_panel().await;
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        {
            let edits = port.edits.lock().expect("edits");
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].3, router_panel_attachments());
        }

        *port.fail_edits.lock().expect("fail") = true;
        interface.ensure_panel().await;
        assert_eq!(port.posts.lock().expect("posts").len(), 2);
    }

    #[tokio::test]
    async fn router_interface_adoptiert_v2_panel_aus_history_bei_leerer_kv() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockRouterInterfacePort::default());
        *port.recent.lock().expect("recent") = vec![RouterPanelMessage {
            message_id: 7002,
            has_embeds: false,
            has_components: true,
            custom_ids: vec!["router_spawn_casual".to_string()],
        }];
        let interface = RouterInterface::new(pool.clone(), port.clone());

        interface.ensure_panel().await;

        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        let edits = port.edits.lock().expect("edits");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].1, 7002);
        assert_eq!(edits[0].3, router_panel_attachments());
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("7002")
        );
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_PAYLOAD_FORMAT_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some(ROUTER_PAYLOAD_FORMAT)
        );
    }

    #[tokio::test]
    async fn router_interface_adoptiert_nur_panel_mit_router_spawn_custom_id() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockRouterInterfacePort::default());
        *port.recent.lock().expect("recent") = vec![
            RouterPanelMessage {
                message_id: 7002,
                has_embeds: false,
                has_components: true,
                custom_ids: vec!["tv_owner_claim".to_string()],
            },
            RouterPanelMessage {
                message_id: 7003,
                has_embeds: true,
                has_components: true,
                custom_ids: vec!["router_mode_casual".to_string()],
            },
        ];
        let interface = RouterInterface::new(pool.clone(), port.clone());

        interface.ensure_panel().await;

        assert_eq!(port.edits.lock().expect("edits").len(), 0);
        {
            let posts = port.posts.lock().expect("posts");
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0].2, router_panel_attachments());
        }
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9001")
        );
    }

    #[tokio::test]
    async fn router_interface_cleanup_loescht_legacy_panels_und_kv_nach_apply() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        dl_central_db::kv::set(
            &pool,
            ROUTER_PANEL_KV_NS,
            ROUTER_LEGACY_GUIDE_MESSAGE_KEY,
            "7101",
        )
        .await
        .expect("guide kv");
        dl_central_db::kv::set(
            &pool,
            ROUTER_PANEL_KV_NS,
            ROUTER_LEGACY_INTERFACE_MESSAGE_KEY,
            "7102",
        )
        .await
        .expect("interface kv");
        let port = Arc::new(MockRouterInterfacePort::default());
        let interface = RouterInterface::new(pool.clone(), port.clone());

        let output = interface.apply_panel(true).await.expect("apply");

        assert_eq!(output.action, "posted");
        assert!(output.warnings.is_empty());
        assert_eq!(
            port.deletes.lock().expect("deletes").clone(),
            vec![
                (
                    ROUTER_TEXT_CHANNEL_ID,
                    7101,
                    "Router: Legacy-Panel nach Components-V2-Cutover bereinigen".to_string(),
                ),
                (
                    ROUTER_TEXT_CHANNEL_ID,
                    7102,
                    "Router: Legacy-Panel nach Components-V2-Cutover bereinigen".to_string(),
                ),
            ]
        );
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_LEGACY_GUIDE_MESSAGE_KEY)
                .await
                .expect("guide kv"),
            None
        );
        assert_eq!(
            dl_central_db::kv::get(
                &pool,
                ROUTER_PANEL_KV_NS,
                ROUTER_LEGACY_INTERFACE_MESSAGE_KEY,
            )
            .await
            .expect("interface kv"),
            None
        );
    }

    #[tokio::test]
    async fn router_interface_dry_run_kuendigt_legacy_cleanup_an_ohne_zu_loeschen() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        dl_central_db::kv::set(
            &pool,
            ROUTER_PANEL_KV_NS,
            ROUTER_LEGACY_GUIDE_MESSAGE_KEY,
            "7201",
        )
        .await
        .expect("guide kv");
        dl_central_db::kv::set(
            &pool,
            ROUTER_PANEL_KV_NS,
            ROUTER_LEGACY_INTERFACE_MESSAGE_KEY,
            "7202",
        )
        .await
        .expect("interface kv");
        let port = Arc::new(MockRouterInterfacePort::default());
        let interface = RouterInterface::new(pool.clone(), port.clone());

        let output = interface.apply_panel(false).await.expect("dry run");

        assert!(output.dry_run);
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
        assert_eq!(port.deletes.lock().expect("deletes").len(), 0);
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("Legacy-Message 7201")));
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("Legacy-Message 7202")));
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_LEGACY_GUIDE_MESSAGE_KEY)
                .await
                .expect("guide kv")
                .as_deref(),
            Some("7201")
        );
        assert_eq!(
            dl_central_db::kv::get(
                &pool,
                ROUTER_PANEL_KV_NS,
                ROUTER_LEGACY_INTERFACE_MESSAGE_KEY,
            )
            .await
            .expect("interface kv")
            .as_deref(),
            Some("7202")
        );
    }

    #[tokio::test]
    async fn router_interface_postet_nicht_blind_wenn_history_scan_fehlgeschlagen_ist() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockRouterInterfacePort::default());
        *port.fail_recent.lock().expect("fail recent") = true;
        let interface = RouterInterface::new(db.pool().clone(), port.clone());

        interface.ensure_panel().await;

        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
    }

    #[test]
    fn router_panel_attachment_validation_meldet_fehlende_banner_datei() {
        let err = validate_router_panel_attachments(&[RouterPanelAttachment {
            id: 0,
            filename: "fehlt.png".to_string(),
            relative_path: format!("{ROUTER_BANNER_DIR}/fehlt.png"),
        }])
        .expect_err("missing banner");

        assert!(err.contains("Router-Banner"));
        assert!(err.contains("fehlt.png"));
        assert!(err.contains("nicht gepostet/editiert"));
    }

    #[tokio::test]
    async fn router_user_pref_roundtrip() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let router = LaneRouter::new(
            pool.clone(),
            Arc::new(NoopRouterPort),
            test_engine(pool),
            None,
        );

        assert_eq!(router.user_pref(42).await, None);
        router.set_user_pref(42, "ranked", true).await;
        assert_eq!(
            router.user_pref(42).await,
            Some(("ranked".to_string(), true))
        );
        router.set_user_pref(42, "casual", false).await;
        assert_eq!(
            router.user_pref(42).await,
            Some(("casual".to_string(), false))
        );
    }

    #[tokio::test]
    async fn router_spawn_blockt_wenn_user_bereits_owner_der_aktuellen_lane_ist() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(StaticRouterPort::default());
        *port.voice_channel.lock().expect("voice") = Some(777);
        let engine = test_engine(pool.clone());
        engine
            .store
            .upsert_lane(crate::tempvoice::store::LaneRecord {
                channel_id: 777,
                guild_id: 1,
                owner_id: 42,
                initial_owner_id: Some(42),
                base_name: "Chill Lane".to_string(),
                category_id: mode_to_category("casual"),
                source_staging_id: Some(ROUTER_VC_ID),
            })
            .await
            .expect("lane");
        let router = LaneRouter::new(pool, port, engine.clone(), None);

        let outcome = router.spawn_lane_from_current_voice(1, 42, "casual").await;

        assert_eq!(outcome, RouterSpawnOutcome::AlreadyOwnLane { lane_id: 777 });
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
    }

    #[tokio::test]
    async fn router_spawn_bremst_erst_ab_fuenfter_erstellung_in_60s() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(StaticRouterPort::default());
        *port.voice_channel.lock().expect("voice") = Some(555);
        let engine = test_engine(pool.clone());
        let router = LaneRouter::new(pool, port, engine.clone(), None);

        for _ in 0..ROUTER_FLOOD_MAX_CREATES {
            assert_eq!(
                router.spawn_lane_from_current_voice(1, 42, "casual").await,
                RouterSpawnOutcome::Created { lane_id: 1 }
            );
        }
        let blocked = router.spawn_lane_from_current_voice(1, 42, "casual").await;

        assert_eq!(blocked, RouterSpawnOutcome::FloodLimited);
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
    }

    #[tokio::test]
    async fn router_spawn_handler_user_ohne_voice_liefert_ephemeren_link() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let router = LaneRouter::new(
            pool.clone(),
            Arc::new(StaticRouterPort::default()),
            test_engine(pool),
            None,
        );
        let handler = RouterPanelHandler { router };

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "router_spawn_casual".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        let content = reply.content.expect("content");
        assert!(content.contains(ROUTER_REPLY_NOT_IN_VOICE));
        assert!(content.contains(&format!("<#{ROUTER_VC_ID}>")));
        assert!(content.contains(ROUTER_REPLY_DEFAULT_SAVED));
        assert_eq!(
            handler
                .router
                .default_preset(42)
                .await
                .expect("default")
                .mode,
            "casual"
        );
    }

    #[tokio::test]
    async fn router_spawn_handler_happy_path_erstellt_lane_ueber_tempvoice() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(StaticRouterPort::default());
        *port.voice_channel.lock().expect("voice") = Some(555);
        let engine = test_engine(pool.clone());
        let router = LaneRouter::new(pool, port, engine.clone(), None);
        let handler = RouterPanelHandler { router };

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "router_spawn_casual".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        let content = reply.content.expect("content");
        assert!(content.contains(&format!("{ROUTER_REPLY_CREATED_PREFIX} <#1>")));
        assert!(content.contains(ROUTER_REPLY_DEFAULT_SAVED));
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].channel_id, 1);
        assert_eq!(lanes[0].owner_id, 42);
        assert_eq!(
            engine
                .store
                .get_default_preset(42)
                .await
                .expect("default query")
                .expect("default")
                .mode,
            "casual"
        );
    }

    #[tokio::test]
    async fn router_spawn_handler_ranked_nutzt_interaction_roles_bei_leerem_cache() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(StaticRouterPort::default());
        *port.voice_channel.lock().expect("voice") = Some(555);
        let engine = test_engine(pool.clone());
        let router = LaneRouter::new(pool, port, engine.clone(), None);
        let handler = RouterPanelHandler { router };

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "router_spawn_ranked".to_string(),
                guild_id: 1,
                user_id: 42,
                role_ids: vec![VERIFIED_RANK_ROLE_IDS[0]],
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        let content = reply.content.expect("content");
        assert!(content.contains(&format!("{ROUTER_REPLY_CREATED_PREFIX} <#1>")));
        assert!(content.contains(ROUTER_REPLY_DEFAULT_SAVED));
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].category_id, mode_to_category("ranked"));
    }

    #[tokio::test]
    async fn router_join_ohne_default_erstellt_keine_lane() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        let router = LaneRouter::new(
            pool,
            Arc::new(StaticRouterPort::default()),
            engine.clone(),
            None,
        );

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;

        assert!(engine.store.all_lanes().await.expect("lanes").is_empty());
    }

    #[tokio::test]
    async fn router_join_mit_default_erstellt_lane_im_default_modus() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        engine
            .store
            .save_default_preset(default_preset_for_mode(42, "street_brawl"))
            .await
            .expect("default");
        let router = LaneRouter::new(
            pool,
            Arc::new(StaticRouterPort::default()),
            engine.clone(),
            None,
        );

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;

        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].category_id, mode_to_category("street_brawl"));
    }

    #[tokio::test]
    async fn router_join_ranked_ohne_verify_erstellt_lane() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        engine
            .store
            .save_default_preset(default_preset_for_mode(42, "ranked"))
            .await
            .expect("default");
        let router = LaneRouter::new(
            pool,
            Arc::new(StaticRouterPort::default()),
            engine.clone(),
            None,
        );

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;

        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].category_id, ROUTER_CATEGORY_CHILL);
        assert_eq!(
            engine
                .store
                .get_default_preset(42)
                .await
                .expect("default query")
                .expect("default")
                .mode,
            "ranked"
        );
    }
}
