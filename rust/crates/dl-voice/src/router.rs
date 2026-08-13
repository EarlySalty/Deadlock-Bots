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
/// Voice-Kanäle in den Lane-Kategorien, die KEINE Lane sind: der Router-Einstieg
/// selbst und die permanenten Kanäle. Sie dürfen nie Ziel eines Routings werden,
/// sonst „verschiebt" der Bot jemanden in den Kanal, in dem er schon sitzt.
/// Einzige Quelle für die Fixkanäle der TempVoice-Engine.
pub const NON_LANE_CHANNEL_IDS: [u64; 5] = [
    ROUTER_VC_ID,
    1493690350580138114, // permanenter Chill-Voice
    1411391356278018245, // Off-Topic-Anker
    1470126503252721845, // Neue-Spieler-Anker
    1505618194017161267,
];
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
pub const ROUTER_CREATE_BANNER_FILENAME: &str = "divider-lane-erstellen.png";
pub const ROUTER_MANAGE_BANNER_FILENAME: &str = "divider-lane-verwalten.png";
pub const ROUTER_GUIDE_BANNER_FILENAME: &str = "divider-anleitung.png";
/// Modus für die Lane, die ein Router-Join ohne gespeicherten Standard bekommt.
pub const ROUTER_FALLBACK_MODE: &str = "casual";
pub const ROUTER_FLOOD_WINDOW_SECS: u64 = 60;
pub const ROUTER_FLOOD_MAX_CREATES: usize = 4;
const ROUTER_INTRO_DM_TIMEOUT: Duration = Duration::from_secs(3);
const ROUTER_INTRO_DM_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
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
        style: 2,
        emoji: ROUTER_EMOJI_CASUAL,
    },
    RouterMode {
        id: "ranked",
        label: ROUTER_BUTTON_RANKED,
        category_id: ROUTER_CATEGORY_CHILL,
        staging_id: 1412804671432818890,
        style: 2,
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
    [ROUTER_CREATE_BANNER_FILENAME, ROUTER_MANAGE_BANNER_FILENAME]
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

pub fn has_voice_capacity(member_count: usize, user_limit: Option<u64>) -> bool {
    user_limit.is_none_or(|limit| limit == 0 || member_count < limit as usize)
}

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
                && !NON_LANE_CHANNEL_IDS.contains(channel_id)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoMoveDecision {
    SkipDisabled,
    SkipNotInRouter,
    SkipNotAlone,
    MoveExisting { lane_id: u64 },
    CreateCasual,
}

pub fn decide_auto_move(
    enabled: bool,
    user_id: u64,
    current_channel_id: Option<u64>,
    router_members: &[u64],
    casual_lanes: &[(u64, Vec<u64>)],
) -> AutoMoveDecision {
    if !enabled {
        return AutoMoveDecision::SkipDisabled;
    }
    if current_channel_id != Some(ROUTER_VC_ID) {
        return AutoMoveDecision::SkipNotInRouter;
    }
    if router_members != [user_id] {
        return AutoMoveDecision::SkipNotAlone;
    }
    pick_lane(casual_lanes, &Default::default())
        .map(|lane_id| AutoMoveDecision::MoveExisting { lane_id })
        .unwrap_or(AutoMoveDecision::CreateCasual)
}

fn log_auto_move_decision(
    guild_id: u64,
    user_id: u64,
    lane_id: Option<u64>,
    decision: &'static str,
    reason: &'static str,
) {
    tracing::info!(
        guild_id,
        user_id,
        ?lane_id,
        decision,
        reason,
        "Router-Auto-Move: Entscheidung"
    );
}

fn log_auto_move_error(
    guild_id: u64,
    user_id: u64,
    lane_id: Option<u64>,
    reason: &'static str,
    err: &str,
) {
    tracing::warn!(
        guild_id,
        user_id,
        ?lane_id,
        decision = "error",
        reason,
        error = err,
        "Router-Auto-Move: Entscheidung"
    );
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
            router_media_gallery(ROUTER_CREATE_BANNER_FILENAME),
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
        .join(ROUTER_GUIDE_BANNER_FILENAME);
    let banner = std::fs::read(banner_path)
        .ok()
        .map(|data| BridgeAttachment {
            filename: ROUTER_GUIDE_BANNER_FILENAME.to_string(),
            data,
        });
    let mut components = Vec::new();
    if banner.is_some() {
        components.push(router_media_gallery(ROUTER_GUIDE_BANNER_FILENAME));
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
    async fn channel_members(&self, guild_id: u64, channel_id: u64) -> Vec<u64>;
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    async fn move_member(&self, guild_id: u64, user_id: u64, channel_id: u64)
        -> Result<(), String>;
    async fn send_dm(&self, user_id: u64, text: String);
    /// Volle Components-V2-DM (flags + components) roh an den User senden.
    async fn send_dm_components(
        &self,
        user_id: u64,
        body: serde_json::Value,
    ) -> Result<(u64, u64), String>;
    async fn delete_dm_message(&self, channel_id: u64, message_id: u64) -> Result<(), String>;
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
        .rev()
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

#[derive(Debug, Clone, Copy)]
pub struct RouterAutoMoveConfig {
    pub enabled: bool,
    pub delay: Duration,
}

impl RouterAutoMoveConfig {
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            delay: Duration::from_secs(60),
        }
    }
}

pub struct LaneRouter {
    pub pool: PgPool,
    pub port: Arc<dyn RouterPort>,
    pub engine: Arc<TempVoiceEngine>,
    pub analyzer: Option<Arc<dl_activity::analyzer::ActivityAnalyzer>>,
    spawn_history: tokio::sync::Mutex<HashMap<u64, Vec<Instant>>>,
    auto_move: RouterAutoMoveConfig,
    auto_move_entries: tokio::sync::Mutex<HashMap<(u64, u64), Instant>>,
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
        Self::new_with_auto_move(
            pool,
            port,
            engine,
            analyzer,
            RouterAutoMoveConfig::disabled(),
        )
    }

    pub fn new_with_auto_move(
        pool: PgPool,
        port: Arc<dyn RouterPort>,
        engine: Arc<TempVoiceEngine>,
        analyzer: Option<Arc<dl_activity::analyzer::ActivityAnalyzer>>,
        auto_move: RouterAutoMoveConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            engine,
            analyzer,
            spawn_history: tokio::sync::Mutex::new(HashMap::new()),
            auto_move,
            auto_move_entries: tokio::sync::Mutex::new(HashMap::new()),
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
        self.update_auto_move_timer(&event).await;
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
        let default = match self.default_preset(user_id).await {
            Some(default) => default,
            None => {
                // Erst-Join ohne Standard: niemand bleibt im Router hängen.
                // Es gibt sofort eine Casual-Lane, die Onboarding-DM erklärt
                // danach, wie man den eigenen Standard setzt.
                self.spawn_fallback_lane_and_onboard(guild_id, user_id)
                    .await;
                return;
            }
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

    /// Router-Join ohne gespeicherten Standard: erst die Fallback-Lane bauen
    /// (Casual, damit der User nicht im Router sitzen bleibt), dann einmalig
    /// die Onboarding-DM mit dem Link auf genau diese Lane. Der Standard wird
    /// bewusst nicht gespeichert — den setzt der User selbst in der DM.
    async fn spawn_fallback_lane_and_onboard(self: &Arc<Self>, guild_id: u64, user_id: u64) {
        let lane_id = if self.flood_guard_allows(user_id).await {
            let lane_id = self
                .engine
                .create_router_lane(guild_id, user_id, ROUTER_FALLBACK_MODE, ROUTER_VC_ID)
                .await;
            if lane_id.is_some() {
                self.mark_spawn_created(user_id).await;
            } else {
                tracing::warn!(
                    user_id,
                    "Router: Fallback-Lane konnte nicht erstellt werden — DM bleibt der einzige Weg"
                );
            }
            lane_id
        } else {
            None
        };
        self.maybe_send_intro_dm(user_id, lane_id).await;
    }

    /// Auto-Move-Uhr für jemanden starten, der im Router-VC sitzt.
    async fn arm_auto_move(self: &Arc<Self>, guild_id: u64, user_id: u64) {
        if !self.auto_move.enabled {
            log_auto_move_decision(guild_id, user_id, None, "skipped", "disabled");
            return;
        }
        let entered_at = Instant::now();
        self.auto_move_entries
            .lock()
            .await
            .insert((guild_id, user_id), entered_at);
        let router = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(router.auto_move.delay).await;
            let is_current = router
                .auto_move_entries
                .lock()
                .await
                .get(&(guild_id, user_id))
                .is_some_and(|current| *current == entered_at);
            if !is_current {
                log_auto_move_decision(guild_id, user_id, None, "skipped", "left_or_rejoined");
                return;
            }
            router.run_auto_move(guild_id, user_id).await;
            let mut entries = router.auto_move_entries.lock().await;
            if entries
                .get(&(guild_id, user_id))
                .is_some_and(|current| *current == entered_at)
            {
                entries.remove(&(guild_id, user_id));
            }
        });
    }

    /// Nach einem Neustart gibt es für alle, die schon im Router-VC sitzen, nie
    /// wieder ein Join-Event. Ohne diesen Anstoß säßen sie dort bis zum
    /// nächsten eigenen Kanalwechsel fest.
    pub async fn prime_auto_move(self: &Arc<Self>, guild_id: u64) {
        let waiting = self.port.channel_members(guild_id, ROUTER_VC_ID).await;
        if waiting.is_empty() {
            return;
        }
        tracing::info!(
            guild_id,
            wartende = waiting.len(),
            "Router: Auto-Move nach Neustart nachgezogen"
        );
        for user_id in waiting {
            self.arm_auto_move(guild_id, user_id).await;
        }
    }

    async fn update_auto_move_timer(self: &Arc<Self>, event: &VoiceEvent) {
        let entered = match event {
            VoiceEvent::Join {
                guild_id,
                user_id,
                channel_id: ROUTER_VC_ID,
            }
            | VoiceEvent::Move {
                guild_id,
                user_id,
                to_channel_id: ROUTER_VC_ID,
                ..
            } => Some((*guild_id, *user_id)),
            _ => None,
        };
        if let Some((guild_id, user_id)) = entered {
            self.arm_auto_move(guild_id, user_id).await;
            return;
        }

        let left = match event {
            VoiceEvent::Leave {
                guild_id,
                user_id,
                channel_id: ROUTER_VC_ID,
            }
            | VoiceEvent::Move {
                guild_id,
                user_id,
                from_channel_id: ROUTER_VC_ID,
                ..
            } => Some((*guild_id, *user_id)),
            _ => None,
        };
        if let Some(key) = left {
            self.auto_move_entries.lock().await.remove(&key);
        }
    }

    async fn run_auto_move(&self, guild_id: u64, user_id: u64) {
        let current_channel = self.port.member_voice_channel(guild_id, user_id).await;
        let router_members = if current_channel == Some(ROUTER_VC_ID) {
            self.port.channel_members(guild_id, ROUTER_VC_ID).await
        } else {
            Vec::new()
        };
        let casual_lanes = if router_members == [user_id] {
            self.port
                .category_lanes(guild_id, ROUTER_CATEGORY_CHILL)
                .await
        } else {
            Vec::new()
        };

        match decide_auto_move(
            self.auto_move.enabled,
            user_id,
            current_channel,
            &router_members,
            &casual_lanes,
        ) {
            AutoMoveDecision::SkipDisabled => {
                log_auto_move_decision(guild_id, user_id, None, "skipped", "disabled")
            }
            AutoMoveDecision::SkipNotInRouter => {
                log_auto_move_decision(guild_id, user_id, None, "skipped", "left_router")
            }
            AutoMoveDecision::SkipNotAlone => {
                log_auto_move_decision(guild_id, user_id, None, "skipped", "not_alone")
            }
            AutoMoveDecision::MoveExisting { lane_id } => {
                if self.port.member_voice_channel(guild_id, user_id).await != Some(ROUTER_VC_ID)
                    || self.port.channel_members(guild_id, ROUTER_VC_ID).await != [user_id]
                {
                    log_auto_move_decision(
                        guild_id,
                        user_id,
                        None,
                        "skipped",
                        "state_changed_before_move",
                    );
                    return;
                }
                match self.port.move_member(guild_id, user_id, lane_id).await {
                    Ok(()) => log_auto_move_decision(
                        guild_id,
                        user_id,
                        Some(lane_id),
                        "moved",
                        "existing_casual_lane",
                    ),
                    Err(err) => log_auto_move_error(
                        guild_id,
                        user_id,
                        Some(lane_id),
                        "move_existing_failed",
                        &err,
                    ),
                }
            }
            AutoMoveDecision::CreateCasual => {
                match self
                    .engine
                    .create_router_lane_if_alone(guild_id, user_id, "casual", ROUTER_VC_ID)
                    .await
                {
                    Ok(Some(lane_id)) => log_auto_move_decision(
                        guild_id,
                        user_id,
                        Some(lane_id),
                        "moved",
                        "new_casual_lane",
                    ),
                    Ok(None) => log_auto_move_decision(
                        guild_id,
                        user_id,
                        None,
                        "skipped",
                        "state_changed_or_create_busy",
                    ),
                    Err(err) => {
                        log_auto_move_error(guild_id, user_id, None, "create_casual_failed", &err)
                    }
                }
            }
        }
    }

    /// Einmalige Router-Onboarding-DM beim Erst-Join ohne gespeicherten Standard.
    /// `lane_id` ist die eben gebaute Fallback-Lane (None, wenn das nicht klappte).
    /// Jede Entscheidung wird geloggt (Privacy / gesendet / schon gesendet / Marker-Fehler).
    async fn maybe_send_intro_dm(&self, user_id: u64, lane_id: Option<u64>) {
        let db_user_id = match u64_to_i64("core.user_privacy.user_id", user_id) {
            Ok(user_id) => user_id,
            Err(err) => {
                tracing::warn!(%err, user_id, "Router: User-ID nicht speicherbar — überspringe Intro-DM");
                return;
            }
        };
        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(err) => {
                tracing::warn!(%err, user_id, "Router: Privacy-Transaktion nicht startbar — überspringe Intro-DM");
                return;
            }
        };
        if let Err(err) = dl_community::privacy::lock_user_privacy(&mut tx, db_user_id).await {
            tracing::warn!(%err, user_id, "Router: Privacy-Lock nicht setzbar — überspringe Intro-DM");
            return;
        }
        let opted_out = match sqlx::query_scalar::<_, bool>(
            "SELECT opted_out FROM core.user_privacy WHERE user_id = $1",
        )
        .bind(db_user_id)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(opted_out) => opted_out.unwrap_or(false),
            Err(err) => {
                tracing::warn!(%err, user_id, "Router: Privacy-Status nicht lesbar — überspringe Intro-DM");
                return;
            }
        };
        if opted_out {
            tracing::debug!(user_id, "Router: Intro-DM übersprungen (Privacy-Opt-out)");
            return;
        }
        let already_sent = match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM voice.router_intro_dm WHERE user_id = $1)",
        )
        .bind(db_user_id)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(sent) => sent,
            Err(err) => {
                tracing::warn!(%err, user_id, "Router: Intro-DM-Marker nicht lesbar — überspringe");
                return;
            }
        };
        match intro_dm_decision(already_sent) {
            IntroDmDecision::SkipAlreadySent => {
                tracing::debug!(user_id, "Router: Intro-DM übersprungen (schon gesendet)");
            }
            IntroDmDecision::Send => {
                let delivery = match tokio::time::timeout(
                    ROUTER_INTRO_DM_TIMEOUT,
                    self.port
                        .send_dm_components(user_id, router_intro_dm_body(lane_id)),
                )
                .await
                {
                    Ok(Ok(delivery)) => Some(delivery),
                    Ok(Err(err)) => {
                        tracing::warn!(%err, user_id, "Router: Intro-DM-Transportfehler — Zustellung unsicher, Wiederholung wird gesperrt");
                        None
                    }
                    Err(_) => {
                        tracing::warn!(
                            user_id,
                            timeout_secs = ROUTER_INTRO_DM_TIMEOUT.as_secs(),
                            "Router: Intro-DM-Timeout — Zustellung unsicher, Wiederholung wird gesperrt"
                        );
                        None
                    }
                };
                // Bei bestätigtem Versand markiert die Zeile den Erfolg. Nach einem Timeout ist
                // sie der dauerhafte Unsicherheitsmarker und verhindert automatische Doppel-DMs.
                if let Err(err) = sqlx::query(
                    "INSERT INTO voice.router_intro_dm (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING",
                )
                .bind(db_user_id)
                .execute(&mut *tx)
                .await
                {
                    tracing::warn!(%err, user_id, "Router: Intro-DM-Marker nicht setzbar");
                    drop(tx);
                    let cleaned = match delivery {
                        Some(delivery) => self.discard_intro_dm(user_id, delivery).await,
                        None => false,
                    };
                    if !cleaned {
                        self.persist_intro_uncertain(user_id, db_user_id).await;
                    }
                    return;
                }
                if let Err(err) = tx.commit().await {
                    tracing::warn!(%err, user_id, "Router: Intro-DM-Marker nicht commitbar");
                    let marker_committed = self.reconcile_intro_marker(user_id, db_user_id).await;
                    if marker_committed == Some(true) {
                        tracing::warn!(user_id, "Router: Intro-DM-Marker trotz verlorener Commit-Bestaetigung verifiziert");
                        return;
                    }
                    if marker_committed.is_none() {
                        tracing::error!(user_id, "Router: Intro-DM-Commit nicht sicher verifizierbar; bestaetigte DM wird nicht destruktiv entfernt");
                        self.persist_intro_uncertain(user_id, db_user_id).await;
                        return;
                    }
                    let cleaned = match delivery {
                        Some(delivery) => self.discard_intro_dm(user_id, delivery).await,
                        None => false,
                    };
                    if !cleaned {
                        self.persist_intro_uncertain(user_id, db_user_id).await;
                    }
                    return;
                }
                if delivery.is_some() {
                    tracing::info!(
                        user_id,
                        "Router: Intro-DM gesendet (Erst-Join ohne Standard)"
                    );
                } else {
                    tracing::warn!(
                        user_id,
                        "Router: Intro-DM-Zustellung dauerhaft als unsicher markiert"
                    );
                }
            }
        }
    }

    async fn reconcile_intro_marker(&self, user_id: u64, db_user_id: i64) -> Option<bool> {
        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(err) => {
                tracing::error!(%err, user_id, "Router: Intro-DM-Commit-Reconcile nicht startbar");
                return None;
            }
        };
        if let Err(err) = dl_community::privacy::lock_user_privacy(&mut tx, db_user_id).await {
            tracing::error!(%err, user_id, "Router: Privacy-Lock fuer Intro-DM-Reconcile fehlgeschlagen");
            return None;
        }
        let marker = match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM voice.router_intro_dm WHERE user_id = $1)",
        )
        .bind(db_user_id)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(marker) => marker,
            Err(err) => {
                tracing::error!(%err, user_id, "Router: Intro-DM-Marker nach Commit-Fehler nicht lesbar");
                return None;
            }
        };
        if let Err(err) = tx.commit().await {
            tracing::error!(%err, user_id, "Router: Intro-DM-Reconcile nicht commitbar");
            return None;
        }
        Some(marker)
    }

    async fn discard_intro_dm(&self, user_id: u64, (channel_id, message_id): (u64, u64)) -> bool {
        match tokio::time::timeout(
            ROUTER_INTRO_DM_CLEANUP_TIMEOUT,
            self.port.delete_dm_message(channel_id, message_id),
        )
        .await
        {
            Ok(Ok(())) => true,
            Ok(Err(err)) => {
                tracing::error!(%err, user_id, channel_id, message_id, "Router: Intro-DM konnte nach Markerfehler nicht entfernt werden");
                false
            }
            Err(_) => {
                tracing::error!(
                    user_id,
                    channel_id,
                    message_id,
                    timeout_secs = ROUTER_INTRO_DM_CLEANUP_TIMEOUT.as_secs(),
                    "Router: Intro-DM-Cleanup hat Zeitlimit ueberschritten"
                );
                false
            }
        }
    }

    async fn persist_intro_uncertain(&self, user_id: u64, db_user_id: i64) {
        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(err) => {
                tracing::error!(%err, user_id, "Router: Unsichere Intro-DM konnte nicht dauerhaft gesperrt werden");
                return;
            }
        };
        if let Err(err) = dl_community::privacy::lock_user_privacy(&mut tx, db_user_id).await {
            tracing::error!(%err, user_id, "Router: Privacy-Lock fuer unsichere Intro-DM fehlgeschlagen");
            return;
        }
        let opted_out = match sqlx::query_scalar::<_, bool>(
            "SELECT opted_out FROM core.user_privacy WHERE user_id = $1",
        )
        .bind(db_user_id)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(value) => value.unwrap_or(false),
            Err(err) => {
                tracing::error!(%err, user_id, "Router: Privacy-Status fuer unsichere Intro-DM nicht lesbar");
                return;
            }
        };
        if opted_out {
            return;
        }
        if let Err(err) = sqlx::query(
            "INSERT INTO voice.router_intro_dm (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING",
        )
        .bind(db_user_id)
        .execute(&mut *tx)
        .await
        {
            tracing::error!(%err, user_id, "Router: Unsicherheitsmarker nicht schreibbar");
            return;
        }
        if let Err(err) = tx.commit().await {
            tracing::error!(%err, user_id, "Router: Unsicherheitsmarker nicht commitbar");
            return;
        }
        tracing::warn!(
            user_id,
            "Router: Intro-DM nach Cleanupfehler dauerhaft als unsicher markiert"
        );
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

/// Entscheidung über die Router-Erst-DM. Rein testbar; Seiteneffekte (DB,
/// Senden) liegen im Aufrufer (siehe [`LaneRouter::maybe_send_intro_dm`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntroDmDecision {
    Send,
    SkipAlreadySent,
}

pub fn intro_dm_decision(already_sent: bool) -> IntroDmDecision {
    if already_sent {
        IntroDmDecision::SkipAlreadySent
    } else {
        IntroDmDecision::Send
    }
}

/// Components-V2-Onboarding-DM für den Erst-Join ohne Standard. Bettet das
/// echte Voreinstellungen-Panel (`tv_prefs_*`) plus den `router_dm_done`-Fertig-
/// Button ein, damit der User seinen Modus direkt in der DM setzen kann.
///
/// `lane_id` ist die beim Join automatisch gebaute Casual-Lane. Ohne sie (die
/// Erstellung ist fehlgeschlagen) erklärt die DM stattdessen den Weg über den
/// Fertig-Button.
pub fn router_intro_dm_body(lane_id: Option<u64>) -> Value {
    let (headline, situation) = match lane_id {
        Some(lane_id) => (
            "## <:dl_casual:1522518264088100995> Deine Lane steht".to_string(),
            format!(
                "Schön, dass du da bist. Deinen Lieblingsmodus kenne ich noch nicht, also hab ich dir kurzerhand eine **Casual**-Lane gebaut und dich reingezogen: <#{lane_id}>. Häng dich rein, der Rest hier dauert keine Minute."
            ),
        ),
        None => (
            "## <:dl_mode:1522518269456547962> Willkommen im Deadlock Router".to_string(),
            format!(
                "Schön, dass du da bist. Mit der Lane hat es gerade leider nicht geklappt. Sag mir unten, was du spielen willst, und klick auf **Fertig**: dann baue ich sie dir sofort, solange du in einem Sprachkanal sitzt. Einstieg ist immer <#{ROUTER_VC_ID}>."
            ),
        ),
    };
    let closing = "Dein Preset kannst du jederzeit hier ändern.".to_string();
    let step_two = if lane_id.is_some() {
        "### 2. Feinschliff, wenn du magst\nName, Limit und Rang für alle künftigen Lanes. Ein Klick auf **Fertig** stellt deine Lane direkt auf den gewählten Modus um."
    } else {
        "### 2. Feinschliff, wenn du magst\nName, Limit und Rang für alle künftigen Lanes. Ein Klick auf **Fertig** baut dir deine Lane im gewählten Modus."
    };
    json!({
        "flags": ROUTER_COMPONENTS_V2_FLAG,
        "allowed_mentions": { "parse": [] },
        "components": [{
            "type": 17,
            "accent_color": ROUTER_ACCENT_GOLD,
            "components": [
                { "type": 10, "content": headline },
                { "type": 14, "divider": true, "spacing": 2 },
                { "type": 10, "content": situation },
                { "type": 14, "divider": true, "spacing": 2 },
                { "type": 10, "content": "### 1. Was spielst du am liebsten?\nSag es mir einmal, dann merke ich es mir. Ab dem nächsten Join bekommst du sofort eine Lane im richtigen Modus, ganz ohne Nachfrage." },
                { "type": 1, "components": [
                    { "type": 2, "style": 2, "label": "Casual", "custom_id": "tv_prefs_mode_casual", "emoji": { "name": "dl_casual", "id": "1522518264088100995" } },
                    { "type": 2, "style": 2, "label": "Ranked", "custom_id": "tv_prefs_mode_ranked", "emoji": { "name": "dl_ranked", "id": "1522518271306366996" } },
                    { "type": 2, "style": 2, "label": "Street Brawl", "custom_id": "tv_prefs_mode_street_brawl", "emoji": { "name": "dl_brawl", "id": "1522518262708174928" } }
                ]},
                { "type": 14, "divider": false, "spacing": 2 },
                { "type": 10, "content": step_two },
                { "type": 1, "components": [
                    { "type": 2, "style": 2, "label": "Name+Limit ändern", "custom_id": "tv_prefs_name_limit" },
                    { "type": 2, "style": 2, "label": "Rang ändern", "custom_id": "tv_prefs_rank" },
                    { "type": 2, "style": 3, "label": "Fertig", "custom_id": "router_dm_done", "emoji": { "name": "dl_crown", "id": "1522518265421631538" } }
                ]},
                { "type": 14, "divider": true, "spacing": 2 },
                { "type": 10, "content": closing }
            ]
        }]
    })
}

/// In-place-Update der Router-DM (bleibt Components-V2, kein `content`-Feld).
fn router_dm_reply(text: impl Into<String>) -> BridgeReply {
    BridgeReply {
        components: Some(json!([{
            "type": 17,
            "accent_color": ROUTER_ACCENT_GOLD,
            "components": [{ "type": 10, "content": text.into() }],
        }])),
        update_message: true,
        message_flags: Some(ROUTER_COMPONENTS_V2_FLAG),
        allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
        ..BridgeReply::default()
    }
}

/// Hinweis als NEUE DM-Nachricht, ohne das Panel zu ersetzen — die Buttons
/// (inkl. Fertig) bleiben stehen, damit der User es gleich nochmal versuchen kann.
fn router_dm_hint(text: impl Into<String>) -> BridgeReply {
    BridgeReply {
        content: Some(text.into()),
        allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
        ..BridgeReply::default()
    }
}

/// Mappt das Spawn-Ergebnis des Fertig-Buttons auf die DM-Antwort (rein testbar).
fn router_dm_done_reply(outcome: &RouterSpawnOutcome) -> BridgeReply {
    match outcome {
        RouterSpawnOutcome::Created { lane_id } => {
            router_dm_reply(format!("Fertig, du bist in deiner Lane <#{lane_id}>. Viel Spaß."))
        }
        RouterSpawnOutcome::AlreadyOwnLane { lane_id } => {
            router_dm_reply(format!("Du bist schon in deiner Lane <#{lane_id}>."))
        }
        RouterSpawnOutcome::NotInVoice => router_dm_hint(format!(
            "Geh in den <#{ROUTER_VC_ID}>, dann bau ich dir deine Lane. Dein Standard ist gespeichert."
        )),
        RouterSpawnOutcome::FloodLimited => router_dm_hint(ROUTER_REPLY_FLOOD_GUARD),
        RouterSpawnOutcome::UnknownMode => router_dm_hint(ROUTER_REPLY_UNKNOWN_MODE),
        RouterSpawnOutcome::NotCreated => router_dm_hint(ROUTER_REPLY_NOT_CREATED),
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
        if interaction.custom_id == "router_dm_done" {
            // Fertig aus der Onboarding-DM: DM hat guild_id 0 → auf die Guild
            // auflösen und mit dem gespeicherten Standard eine Lane spawnen.
            let guild_id = if interaction.guild_id == 0 {
                ROUTER_GUILD_ID
            } else {
                interaction.guild_id
            };
            let Some(default) = self.router.default_preset(interaction.user_id).await else {
                return router_dm_hint("Wähl oben zuerst einen Modus, dann klappt Fertig.");
            };
            let outcome = self
                .router
                .spawn_lane_from_current_voice_with_role_ids(
                    guild_id,
                    interaction.user_id,
                    &default.mode,
                    &interaction.role_ids,
                )
                .await;
            // Der User sitzt nach dem Router-Join schon in seiner Fallback-Lane.
            // Fertig baut dann keine zweite, sondern stellt diese auf den
            // gewählten Modus um.
            if let RouterSpawnOutcome::AlreadyOwnLane { lane_id } = outcome {
                let switch_error = self
                    .router
                    .engine
                    .switch_lane_mode(guild_id, lane_id, interaction.user_id, &default.mode)
                    .await;
                if switch_error.is_none() {
                    let label = router_mode(&default.mode)
                        .map(|mode| mode.label)
                        .unwrap_or(ROUTER_BUTTON_CASUAL);
                    return router_dm_reply(format!(
                        "Passt: <#{lane_id}> läuft jetzt als **{label}**, und das bleibt dein Standard. Viel Spaß."
                    ));
                }
                tracing::warn!(
                    user_id = interaction.user_id,
                    lane_id,
                    mode = %default.mode,
                    "Router: Modus-Umstellung der Fallback-Lane fehlgeschlagen"
                );
            }
            return router_dm_done_reply(&outcome);
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
    router_panel.on_custom_id("router_dm_done", handler.clone());
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
    #[ignore = "Hilfsausgabe: druckt die echte Onboarding-DM zum Gegenlesen"]
    fn dump_intro_dm() {
        println!(
            "{}",
            serde_json::to_string(&router_intro_dm_body(Some(1523272810825252944))).expect("json")
        );
    }

    #[test]
    fn intro_dm_decision_sendet_nur_beim_ersten_mal() {
        assert_eq!(intro_dm_decision(false), IntroDmDecision::Send);
        assert_eq!(intro_dm_decision(true), IntroDmDecision::SkipAlreadySent);
    }

    #[test]
    fn intro_dm_body_ist_components_v2_mit_prefs_panel() {
        let body = router_intro_dm_body(Some(4242));
        assert_eq!(body["flags"], json!(32768));
        let text = serde_json::to_string(&body).expect("json");
        // Enthält die echten Voreinstellungen-Buttons + den Fertig-Button.
        assert!(text.contains("tv_prefs_mode_casual"));
        assert!(text.contains("tv_prefs_mode_ranked"));
        assert!(text.contains("tv_prefs_mode_street_brawl"));
        assert!(text.contains("tv_prefs_name_limit"));
        assert!(text.contains("tv_prefs_rank"));
        assert!(text.contains("router_dm_done"));
        // Kernbotschaft: die Lane steht schon, der Standard fehlt noch.
        assert!(text.contains("<#4242>"));
        assert!(text.contains("Deine Lane steht"));
        assert!(text.contains("Was spielst du am liebsten"));
        // Der Abschluss ist ein Satz, keine Feature-Liste.
        assert!(text.contains("Preset kannst du jederzeit hier ändern"));
    }

    #[test]
    fn intro_dm_body_ohne_lane_erklaert_den_fertig_weg() {
        let text = serde_json::to_string(&router_intro_dm_body(None)).expect("json");
        assert!(text.contains("Willkommen im Deadlock Router"));
        assert!(text.contains("nicht geklappt"));
        assert!(text.contains("router_dm_done"));
    }

    #[test]
    fn router_dm_done_reply_updatet_die_dm_in_place() {
        let created = router_dm_done_reply(&RouterSpawnOutcome::Created { lane_id: 42 });
        assert!(
            created.update_message,
            "Fertig muss die DM in-place updaten"
        );
        assert!(!created.ephemeral, "DM-Antwort darf nicht ephemeral sein");
        assert_eq!(created.message_flags, Some(ROUTER_COMPONENTS_V2_FLAG));
        let text = serde_json::to_string(&created.components).expect("json");
        assert!(text.contains("Fertig"));
        assert!(text.contains("42"));

        // NotInVoice ist ein Hinweis, der das Panel STEHEN lässt (kein Update).
        let not_in_voice = router_dm_done_reply(&RouterSpawnOutcome::NotInVoice);
        assert!(
            !not_in_voice.update_message,
            "Hinweis darf das Panel nicht ersetzen"
        );
        let text = not_in_voice.content.clone().expect("content");
        assert!(text.contains(&ROUTER_VC_ID.to_string()));
        assert!(text.contains("Standard ist gespeichert"));
    }

    #[test]
    fn lane_wahl_ignoriert_den_router_vc_und_fixkanaele() {
        // Der Router-VC liegt selbst in der Chill-Kategorie und kam damit als
        // „passende Lane" zurück: der Bot verschob Leute in den Kanal, in dem
        // sie schon saßen, und der Auto-Move lief ins Leere.
        let lanes = vec![(ROUTER_VC_ID, vec![7]), (NON_LANE_CHANNEL_IDS[1], vec![8])];
        assert_eq!(pick_lane(&lanes, &Default::default()), None);

        let co: std::collections::HashSet<u64> = [7].into_iter().collect();
        assert_eq!(
            pick_lane(&lanes, &co),
            None,
            "auch ein Co-Spieler im Router-VC macht ihn nicht zur Lane"
        );

        let mit_echter_lane = vec![(ROUTER_VC_ID, vec![7]), (42, vec![8])];
        assert_eq!(pick_lane(&mit_echter_lane, &Default::default()), Some(42));
    }

    #[test]
    fn auto_move_baut_eine_lane_statt_in_den_router_zu_schieben() {
        // Genau der Live-Fall: allein im Router, und die einzige „Lane" in der
        // Kategorie ist der Router-VC selbst.
        assert_eq!(
            decide_auto_move(
                true,
                7,
                Some(ROUTER_VC_ID),
                &[7],
                &[(ROUTER_VC_ID, vec![7])]
            ),
            AutoMoveDecision::CreateCasual
        );
    }

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
    fn voice_capacity_respektiert_individuelles_discord_limit() {
        assert!(has_voice_capacity(2, None));
        assert!(has_voice_capacity(2, Some(0)));
        assert!(has_voice_capacity(1, Some(2)));
        assert!(!has_voice_capacity(2, Some(2)));
    }

    #[test]
    fn auto_move_ist_per_flag_abschaltbar() {
        assert_eq!(
            decide_auto_move(false, 7, Some(ROUTER_VC_ID), &[7], &[]),
            AutoMoveDecision::SkipDisabled
        );
    }

    #[test]
    fn auto_move_skippt_wenn_user_den_router_verlassen_hat() {
        assert_eq!(
            decide_auto_move(true, 7, None, &[], &[]),
            AutoMoveDecision::SkipNotInRouter
        );
        assert_eq!(
            decide_auto_move(true, 7, Some(99), &[7], &[]),
            AutoMoveDecision::SkipNotInRouter
        );
    }

    #[test]
    fn auto_move_skippt_wenn_user_nicht_mehr_allein_ist() {
        assert_eq!(
            decide_auto_move(true, 7, Some(ROUTER_VC_ID), &[7, 8], &[]),
            AutoMoveDecision::SkipNotAlone
        );
        assert_eq!(
            decide_auto_move(true, 7, Some(ROUTER_VC_ID), &[8], &[]),
            AutoMoveDecision::SkipNotAlone
        );
    }

    #[test]
    fn auto_move_nutzt_belegte_lane_sonst_neue_casual_lane() {
        let lanes = vec![(10, vec![]), (11, vec![1, 2, 3, 4, 5, 6]), (12, vec![1, 2])];
        assert_eq!(
            decide_auto_move(true, 7, Some(ROUTER_VC_ID), &[7], &lanes),
            AutoMoveDecision::MoveExisting { lane_id: 12 }
        );
        assert_eq!(
            decide_auto_move(true, 7, Some(ROUTER_VC_ID), &[7], &[]),
            AutoMoveDecision::CreateCasual
        );
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
                "attachment://divider-lane-erstellen.png",
                "attachment://divider-lane-verwalten.png",
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
            ]
        );
        assert_eq!(
            container_components
                .iter()
                .filter(|component| component["type"] == 14)
                .count(),
            1
        );
        assert_eq!(
            body.get("attachments").expect("attachments"),
            &json!([
                {"id": 0, "filename": "divider-lane-erstellen.png"},
                {"id": 1, "filename": "divider-lane-verwalten.png"},
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
        let mode_styles: Vec<u64> = container_components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter(|component| {
                component["custom_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("router_spawn_"))
            })
            .filter_map(|component| component["style"].as_u64())
            .collect();
        assert_eq!(mode_styles, vec![2, 2, 2]);
    }

    #[test]
    fn voice_guide_detail_reply_ist_v2_mit_banner() {
        let reply = voice_guide_detail_reply();
        assert_eq!(reply.message_flags, Some(64 | ROUTER_COMPONENTS_V2_FLAG));
        assert_eq!(reply.attachments.len(), 1);
        assert_eq!(reply.attachments[0].filename, ROUTER_GUIDE_BANNER_FILENAME);
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
            format!("attachment://{ROUTER_GUIDE_BANNER_FILENAME}")
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
        async fn channel_members(&self, _guild_id: u64, _channel_id: u64) -> Vec<u64> {
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
        async fn send_dm_components(
            &self,
            _user_id: u64,
            _body: serde_json::Value,
        ) -> Result<(u64, u64), String> {
            Ok((700, 800))
        }
        async fn delete_dm_message(
            &self,
            _channel_id: u64,
            _message_id: u64,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct StaticRouterPort {
        voice_channel: StdMutex<Option<u64>>,
        role_ids: StdMutex<Vec<u64>>,
        dm_components: StdMutex<Vec<serde_json::Value>>,
        dm_deletes: StdMutex<Vec<(u64, u64)>>,
        dm_started: StdMutex<Option<Arc<tokio::sync::Notify>>>,
        dm_release: StdMutex<Option<Arc<tokio::sync::Notify>>>,
        dm_record_before_release: StdMutex<bool>,
        dm_error: StdMutex<bool>,
        dm_delete_error: StdMutex<bool>,
        dm_delete_started: StdMutex<Option<Arc<tokio::sync::Notify>>>,
        dm_delete_release: StdMutex<Option<Arc<tokio::sync::Notify>>>,
    }

    #[async_trait::async_trait]
    impl RouterPort for StaticRouterPort {
        async fn category_lanes(&self, _guild_id: u64, _category_id: u64) -> Vec<(u64, Vec<u64>)> {
            Vec::new()
        }

        async fn channel_members(&self, _guild_id: u64, _channel_id: u64) -> Vec<u64> {
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
        async fn send_dm_components(
            &self,
            _user_id: u64,
            body: serde_json::Value,
        ) -> Result<(u64, u64), String> {
            let started = self.dm_started.lock().expect("started").clone();
            let release = self.dm_release.lock().expect("release").clone();
            let record_before_release = *self
                .dm_record_before_release
                .lock()
                .expect("record before release");
            if let Some(started) = started {
                started.notify_one();
            }
            if record_before_release {
                self.dm_components.lock().expect("dm").push(body.clone());
            }
            if let Some(release) = release {
                release.notified().await;
            }
            if *self.dm_error.lock().expect("dm error") {
                return Err("dm failed".to_string());
            }
            if !record_before_release {
                self.dm_components.lock().expect("dm").push(body);
            }
            Ok((700, 800))
        }

        async fn delete_dm_message(&self, channel_id: u64, message_id: u64) -> Result<(), String> {
            self.dm_deletes
                .lock()
                .expect("dm deletes")
                .push((channel_id, message_id));
            let started = self
                .dm_delete_started
                .lock()
                .expect("delete started")
                .clone();
            let release = self
                .dm_delete_release
                .lock()
                .expect("delete release")
                .clone();
            if let Some(started) = started {
                started.notify_one();
            }
            if let Some(release) = release {
                release.notified().await;
            }
            if *self.dm_delete_error.lock().expect("dm delete error") {
                return Err("dm delete failed".to_string());
            }
            Ok(())
        }
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
        async fn create_restricted_voice_channel(
            &self,
            _guild_id: u64,
            _category_id: u64,
            _name: &str,
            _connect_user_ids: &[u64],
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

    async fn wait_for_db_lock(pool: &PgPool, query_fragment: &str, wait_event: Option<&str>) {
        let query_pattern = format!("%{query_fragment}%");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let waiting = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(
                        SELECT 1
                          FROM pg_stat_activity
                         WHERE datname = current_database()
                           AND pid <> pg_backend_pid()
                           AND state = 'active'
                           AND wait_event_type = 'Lock'
                           AND query LIKE $1
                           AND ($2::TEXT IS NULL OR wait_event = $2)
                    )",
                )
                .bind(&query_pattern)
                .bind(wait_event)
                .fetch_one(pool)
                .await
                .expect("pg_stat_activity");
                if waiting {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("DB-Lock-Wait fuer {query_fragment} nicht sichtbar"));
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
        *port.recent.lock().expect("recent") = vec![
            RouterPanelMessage {
                message_id: 7002,
                has_embeds: false,
                has_components: true,
                custom_ids: vec!["router_spawn_casual".to_string()],
            },
            RouterPanelMessage {
                message_id: 7004,
                has_embeds: false,
                has_components: true,
                custom_ids: vec!["router_spawn_ranked".to_string()],
            },
        ];
        let interface = RouterInterface::new(pool.clone(), port.clone());

        interface.ensure_panel().await;

        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        let edits = port.edits.lock().expect("edits");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].1, 7004);
        assert_eq!(edits[0].3, router_panel_attachments());
        assert_eq!(
            dl_central_db::kv::get(&pool, ROUTER_PANEL_KV_NS, ROUTER_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("7004")
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
    async fn router_join_ohne_default_baut_casual_lane_und_dm_genau_einmal() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        let router = LaneRouter::new(pool, port.clone(), engine.clone(), None);

        for _ in 0..2 {
            router
                .handle_event(VoiceEvent::Join {
                    guild_id: 1,
                    user_id: 42,
                    channel_id: ROUTER_VC_ID,
                })
                .await;
        }

        // Niemand bleibt im Router hängen: der Join baut sofort eine Casual-Lane.
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert!(!lanes.is_empty(), "Fallback-Lane muss entstehen");
        assert_eq!(lanes[0].category_id, mode_to_category(ROUTER_FALLBACK_MODE));
        // Die Erklär-DM kommt trotzdem nur einmal.
        assert_eq!(port.dm_components.lock().expect("dm").len(), 1);
        assert!(engine.store.router_intro_dm_sent(42).await.expect("marker"));
        // Der Standard bleibt leer, den setzt der User selbst in der DM.
        assert!(router.default_preset(42).await.is_none());
    }

    #[tokio::test]
    async fn router_intro_dm_transportfehler_setzt_unsicheren_marker() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        *port.dm_error.lock().expect("dm error") = true;
        let router = LaneRouter::new(pool, port.clone(), engine.clone(), None);

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;

        assert!(port.dm_components.lock().expect("dm").is_empty());
        assert!(engine.store.router_intro_dm_sent(42).await.expect("marker"));
    }

    #[tokio::test]
    async fn router_intro_dm_markerfehler_gibt_privacy_lock_vor_cleanup_frei() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::raw_sql(
            r#"
            CREATE FUNCTION fail_router_intro_insert() RETURNS trigger
            LANGUAGE plpgsql AS $$
            BEGIN
                RAISE EXCEPTION 'forced router intro insert failure';
            END;
            $$;
            CREATE TRIGGER fail_router_intro_insert_trigger
            BEFORE INSERT ON voice.router_intro_dm
            FOR EACH ROW EXECUTE FUNCTION fail_router_intro_insert();
            "#,
        )
        .execute(&pool)
        .await
        .expect("immediate marker failure trigger");
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        let delete_started = Arc::new(tokio::sync::Notify::new());
        let delete_release = Arc::new(tokio::sync::Notify::new());
        *port.dm_delete_started.lock().expect("delete started") = Some(delete_started.clone());
        *port.dm_delete_release.lock().expect("delete release") = Some(delete_release.clone());
        let router = LaneRouter::new(pool.clone(), port, engine, None);
        let blocker_one = pool.acquire().await.expect("pool blocker one");
        let blocker_two = pool.acquire().await.expect("pool blocker two");
        let blocker_three = pool.acquire().await.expect("pool blocker three");

        let router_task = tokio::spawn(async move {
            router
                .handle_event(VoiceEvent::Join {
                    guild_id: 1,
                    user_id: 42,
                    channel_id: ROUTER_VC_ID,
                })
                .await;
        });
        tokio::time::timeout(Duration::from_secs(5), delete_started.notified())
            .await
            .expect("DM-Cleanup wurde nicht gestartet");

        let connection_released =
            tokio::time::timeout(Duration::from_millis(500), pool.acquire()).await;
        assert!(
            connection_released.is_ok(),
            "Discord-Cleanup darf die Pool-Verbindung nicht halten"
        );
        drop(connection_released);
        drop((blocker_one, blocker_two, blocker_three));
        delete_release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), router_task)
            .await
            .expect("router task deadline")
            .expect("router task");
    }

    #[tokio::test]
    async fn router_intro_dm_cleanup_ist_zeitlich_begrenzt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::raw_sql(
            r#"
            CREATE FUNCTION fail_router_intro_insert() RETURNS trigger
            LANGUAGE plpgsql AS $$
            BEGIN
                RAISE EXCEPTION 'forced router intro insert failure';
            END;
            $$;
            CREATE TRIGGER fail_router_intro_insert_trigger
            BEFORE INSERT ON voice.router_intro_dm
            FOR EACH ROW EXECUTE FUNCTION fail_router_intro_insert();
            "#,
        )
        .execute(&pool)
        .await
        .expect("immediate marker failure trigger");
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        *port.dm_delete_release.lock().expect("delete release") =
            Some(Arc::new(tokio::sync::Notify::new()));
        let router = LaneRouter::new(pool, port, engine, None);

        let completed = tokio::time::timeout(
            ROUTER_INTRO_DM_TIMEOUT + Duration::from_secs(2),
            router.handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            }),
        )
        .await;

        assert!(
            completed.is_ok(),
            "Discord-Cleanup darf nicht dauerhaft haengen"
        );
    }

    #[tokio::test]
    async fn router_intro_dm_timeout_setzt_unsicheren_marker_und_verhindert_duplikat() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        *port.dm_release.lock().expect("release") = Some(Arc::new(tokio::sync::Notify::new()));
        *port
            .dm_record_before_release
            .lock()
            .expect("record before release") = true;
        let router = LaneRouter::new(pool, port.clone(), engine.clone(), None);

        let completed = tokio::time::timeout(
            ROUTER_INTRO_DM_TIMEOUT + Duration::from_secs(5),
            router.handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            }),
        )
        .await;

        assert!(completed.is_ok(), "Router muss die blockierte DM begrenzen");
        assert!(engine.store.router_intro_dm_sent(42).await.expect("marker"));
        *port.dm_release.lock().expect("release") = None;
        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;
        assert_eq!(port.dm_components.lock().expect("dm").len(), 1);
    }

    #[tokio::test]
    async fn router_intro_dm_commitfehler_entfernt_bestaetigte_dm() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::raw_sql(
            r#"
            CREATE FUNCTION fail_router_intro_commit() RETURNS trigger
            LANGUAGE plpgsql AS $$
            BEGIN
                RAISE EXCEPTION 'forced router intro commit failure';
            END;
            $$;
            CREATE CONSTRAINT TRIGGER fail_router_intro_commit_trigger
            AFTER INSERT ON voice.router_intro_dm
            DEFERRABLE INITIALLY DEFERRED
            FOR EACH ROW EXECUTE FUNCTION fail_router_intro_commit();
            "#,
        )
        .execute(&pool)
        .await
        .expect("deferred failure trigger");
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        let router = LaneRouter::new(pool, port.clone(), engine.clone(), None);

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;

        assert!(!engine.store.router_intro_dm_sent(42).await.expect("marker"));
        assert_eq!(
            *port.dm_deletes.lock().expect("dm deletes"),
            vec![(700, 800)]
        );
    }

    #[tokio::test]
    async fn router_intro_dm_cleanupfehler_setzt_dauerhaften_unsicherheitsmarker() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::raw_sql(
            r#"
            CREATE SEQUENCE fail_router_intro_commit_once_seq;
            CREATE FUNCTION fail_router_intro_commit_once() RETURNS trigger
            LANGUAGE plpgsql AS $$
            BEGIN
                IF nextval('fail_router_intro_commit_once_seq') = 1 THEN
                    RAISE EXCEPTION 'forced one-shot router intro commit failure';
                END IF;
                RETURN NEW;
            END;
            $$;
            CREATE CONSTRAINT TRIGGER fail_router_intro_commit_once_trigger
            AFTER INSERT ON voice.router_intro_dm
            DEFERRABLE INITIALLY DEFERRED
            FOR EACH ROW EXECUTE FUNCTION fail_router_intro_commit_once();
            "#,
        )
        .execute(&pool)
        .await
        .expect("one-shot deferred failure trigger");
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        *port.dm_delete_error.lock().expect("dm delete error") = true;
        let router = LaneRouter::new(pool, port.clone(), engine.clone(), None);

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;
        assert!(engine.store.router_intro_dm_sent(42).await.expect("marker"));

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;
        assert_eq!(port.dm_components.lock().expect("dm").len(), 1);
    }

    #[tokio::test]
    async fn router_intro_dm_bleibt_nach_privacy_erase_geloescht() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        engine
            .store
            .mark_router_intro_dm_sent(42)
            .await
            .expect("intro marker");
        dl_community::privacy::delete_user_data(&pool, 42, "test".to_string(), 1_000)
            .await
            .expect("privacy erase");
        assert!(!engine
            .store
            .router_intro_dm_sent(42)
            .await
            .expect("marker deleted"));
        let port = Arc::new(StaticRouterPort::default());
        let router = LaneRouter::new(pool, port.clone(), engine.clone(), None);

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;

        assert!(port.dm_components.lock().expect("dm").is_empty());
        assert!(!engine
            .store
            .router_intro_dm_sent(42)
            .await
            .expect("marker stays deleted"));
    }

    #[tokio::test]
    async fn router_intro_dm_blockiert_bei_privacy_db_fehler() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::query("ALTER TABLE core.user_privacy RENAME TO user_privacy_unavailable")
            .execute(&pool)
            .await
            .expect("privacy table unavailable");
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        let router = LaneRouter::new(pool, port.clone(), engine.clone(), None);

        router
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 42,
                channel_id: ROUTER_VC_ID,
            })
            .await;

        assert!(port.dm_components.lock().expect("dm").is_empty());
        assert!(!engine
            .store
            .router_intro_dm_sent(42)
            .await
            .expect("marker stays absent"));
    }

    #[tokio::test]
    async fn privacy_erase_wartet_db_seitig_auf_laufende_router_intro_dm() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        let port = Arc::new(StaticRouterPort::default());
        let dm_started = Arc::new(tokio::sync::Notify::new());
        let dm_release = Arc::new(tokio::sync::Notify::new());
        *port.dm_started.lock().expect("started") = Some(dm_started.clone());
        *port.dm_release.lock().expect("release") = Some(dm_release.clone());
        let router = LaneRouter::new(pool.clone(), port.clone(), engine.clone(), None);
        let router_task = tokio::spawn(async move {
            router
                .handle_event(VoiceEvent::Join {
                    guild_id: 1,
                    user_id: 42,
                    channel_id: ROUTER_VC_ID,
                })
                .await;
        });
        tokio::time::timeout(Duration::from_secs(5), dm_started.notified())
            .await
            .expect("Intro-DM wurde nicht gestartet");

        let erase_pool = pool.clone();
        let erase_task = tokio::spawn(async move {
            dl_community::privacy::delete_user_data(&erase_pool, 42, "test".to_string(), 1_000)
                .await
        });
        wait_for_db_lock(&pool, "pg_advisory_xact_lock", Some("advisory")).await;
        dm_release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), router_task)
            .await
            .expect("router task deadline")
            .expect("router task");
        tokio::time::timeout(Duration::from_secs(10), erase_task)
            .await
            .expect("erase task deadline")
            .expect("erase task")
            .expect("privacy erase");

        assert_eq!(port.dm_components.lock().expect("dm").len(), 1);
        assert!(!engine
            .store
            .router_intro_dm_sent(42)
            .await
            .expect("marker stays deleted"));
    }

    #[tokio::test]
    async fn router_intro_dm_wartet_hinter_bereits_laufendem_privacy_erase() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let engine = test_engine(pool.clone());
        engine
            .store
            .mark_router_intro_dm_sent(42)
            .await
            .expect("intro marker");
        let mut marker_blocker = pool.begin().await.expect("blocker tx");
        sqlx::query_scalar::<_, i64>(
            "SELECT user_id FROM voice.router_intro_dm WHERE user_id = $1 FOR UPDATE",
        )
        .bind(42_i64)
        .fetch_one(&mut *marker_blocker)
        .await
        .expect("lock marker row");

        let erase_pool = pool.clone();
        let erase_task = tokio::spawn(async move {
            dl_community::privacy::delete_user_data(&erase_pool, 42, "test".to_string(), 1_000)
                .await
        });
        wait_for_db_lock(
            &pool,
            "DELETE FROM voice.router_intro_dm",
            Some("transactionid"),
        )
        .await;

        let port = Arc::new(StaticRouterPort::default());
        let router = LaneRouter::new(pool.clone(), port.clone(), engine.clone(), None);
        let router_task = tokio::spawn(async move {
            router
                .handle_event(VoiceEvent::Join {
                    guild_id: 1,
                    user_id: 42,
                    channel_id: ROUTER_VC_ID,
                })
                .await;
        });
        wait_for_db_lock(&pool, "pg_advisory_xact_lock", Some("advisory")).await;
        marker_blocker.commit().await.expect("release marker row");

        tokio::time::timeout(Duration::from_secs(10), erase_task)
            .await
            .expect("erase task deadline")
            .expect("erase task")
            .expect("privacy erase");
        tokio::time::timeout(Duration::from_secs(5), router_task)
            .await
            .expect("router task deadline")
            .expect("router task");

        assert!(port.dm_components.lock().expect("dm").is_empty());
        assert!(!engine
            .store
            .router_intro_dm_sent(42)
            .await
            .expect("marker stays deleted"));
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
