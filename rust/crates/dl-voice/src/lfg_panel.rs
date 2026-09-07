use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use crate::lfg_watch::{insert_or_replace_watch, LfgWatchWindow};
use chrono::{DateTime, Utc};
use dl_central_db::kv;
use dl_discord::{
    BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler, InteractionRouter, VoiceEvent,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tokio::sync::{Mutex, Notify, RwLock};

pub const LFG_GUILD_ID: u64 = crate::router::ROUTER_GUILD_ID;
pub const LFG_PANEL_KV_NS: &str = "lfg_panel";
pub const LFG_USER_PREF_KV_NS: &str = "lfg_user_pref";
pub const LFG_PANEL_MESSAGE_KEY: &str = "components_v2_message_id";
pub const LFG_PAYLOAD_FORMAT_KEY: &str = "payload_format";
pub const LFG_PAYLOAD_FORMAT: &str = "components_v2";
pub const LFG_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const LFG_ACCENT_GOLD: u64 = crate::router::ROUTER_ACCENT_GOLD;
pub const LFG_BANNER_DIR: &str = "assets/welcome-banners";
pub const LFG_PANEL_BANNER_FILENAME: &str = "router-hero.png";
pub const LFG_CREATE_START_CUSTOM_ID: &str = "lfg:create:start";
pub const LFG_CREATE_MODE_PREFIX: &str = "lfg:create:mode:";
pub const LFG_WATCH_START_CUSTOM_ID: &str = "lfg:watch:start";
pub const LFG_WATCH_MODE_PREFIX: &str = "lfg:watch:mode:";
pub const LFG_WATCH_RANK_FROM_SELECT_CUSTOM_ID: &str = "lfg:watch:rank_von";
pub const LFG_WATCH_RANK_TO_SELECT_CUSTOM_ID: &str = "lfg:watch:rank_bis";
pub const LFG_WATCH_WINDOW_SELECT_CUSTOM_ID: &str = "lfg:watch:window";
pub const LFG_WATCH_ACTIVATE_CUSTOM_ID: &str = "lfg:watch:activate";
pub const LFG_RANK_FROM_SELECT_CUSTOM_ID: &str = "lfg:rank_von";
pub const LFG_RANK_TO_SELECT_CUSTOM_ID: &str = "lfg:rank_bis";
pub const LFG_SLOTS_SELECT_CUSTOM_ID: &str = "lfg:slots";
pub const LFG_WHEN_SELECT_CUSTOM_ID: &str = "lfg:when";
pub const LFG_POST_PREFIX: &str = "lfg:post:";
pub const LFG_OPEN_LANE_PREFIX: &str = "lfg:open_lane:";
pub const LFG_JOIN_PREFIX: &str = "lfg:join:";
pub const LFG_PUBLISH_LANE_CUSTOM_ID: &str = "lfg:publish_lane";
pub const LFG_EXPIRY_HOURS: i64 = 24;
pub const LFG_CREATING_STALE_MINUTES: i64 = 5;
pub const LFG_RECONCILE_INTERVAL_SECONDS: u64 = 60;
pub const LFG_EDIT_MIN_INTERVAL_SECONDS: i64 = 5;
pub const LFG_EDIT_429_BACKOFF_SECONDS: f64 = 1.0;
pub const LFG_STREET_BRAWL_CAP: i64 = 4;
pub const LFG_FORUM_MAX_APPLIED_TAGS: usize = 5;
pub const LFG_FORUM_TAG_MODE_CASUAL: u64 = 1522799471594045510;
pub const LFG_FORUM_TAG_MODE_RANKED: u64 = 1522799471594045511;
pub const LFG_FORUM_TAG_MODE_STREET_BRAWL: u64 = 1522799471594045512;
pub const LFG_FORUM_TAG_RANK_BEGINNER: u64 = 1522799471594045513;
pub const LFG_FORUM_TAG_RANK_ADVANCED: u64 = 1522799471594045514;
pub const LFG_FORUM_TAG_RANK_EXPERIENCED: u64 = 1522799471594045515;
pub const LFG_FORUM_TAG_RANK_ELITE: u64 = 1522799471594045516;
pub const LFG_FORUM_TAG_RANK_ANY: u64 = 1522799471594045517;
pub const LFG_FORUM_TAG_STATUS_ACTIVE: u64 = 1522799471594045518;
pub const LFG_FORUM_TAG_STATUS_LOOKING: u64 = 1522799471594045519;

pub const LFG_PANEL_BODY: &str = "**Mitspieler finden**\nModus wählen, Rang-Bereich und Plätze angeben — fertig ist dein Gesuch als eigener Post. Der Post zeigt live, wie viele Plätze in der Lane frei sind, und mit **Beitreten** landest du direkt im Voice.\n\nGesuche räumen sich selbst weg, sobald die Lane schließt.\nWer regelmäßig dabei ist, taucht im [Rank-Leaderboard](https://deutsche-deadlock-community.de/aktivitaet/#rank-leaderboard-card) der Community auf.";
pub const LFG_PANEL_BUTTON: &str = "Mitspieler suchen";
pub const LFG_WATCH_PANEL_BUTTON: &str = "🔔 Benachrichtige mich";
pub const LFG_MODE_PROMPT: &str = "Wofür suchst du Leute?";
pub const LFG_MODE_BUTTON_CASUAL: &str = "Normale Lane";
pub const LFG_MODE_BUTTON_RANKED: &str = "Ranked";
pub const LFG_MODE_BUTTON_STREET_BRAWL: &str = "Street Brawl";

// Panel-Button-Emojis im Brand-Look (gold getönte Lucide-Icons): Modus recycelt
// die Router-Emojis, "Mitspieler suchen" nutzt das Gold-Such-Emoji (gen_lfg_tag_emojis.py).
pub const LFG_EMOJI_SEARCH: (&str, &str) = ("dl_lfg_sucht", "1522801046509064263");
pub const LFG_RANK_FROM_PLACEHOLDER: &str = "Von welchem Rang? (optional)";
pub const LFG_RANK_TO_PLACEHOLDER: &str = "Bis welchem Rang? (optional)";
pub const LFG_SLOTS_PLACEHOLDER: &str = "Wie viele Plätze frei?";
pub const LFG_RANK_ANY_LABEL: &str = "Rang egal";
pub const LFG_RANK_ANY_VALUE: &str = "egal";
pub const LFG_BTN_POSTEN: &str = "Suche veröffentlichen";
pub const LFG_ERR_KEIN_RANKED_RANG: &str = "Für Ranked brauchst du einen verifizierten Rang. Verknüpf dein Steam-Konto in <#1398021105339334666>, dann geht's hier weiter.";
pub const LFG_ERR_RANG_UNBEKANNT: &str =
    "Wähl deinen Rang oben aus der Liste — oder lass ihn auf „Rang egal“.";
pub const LFG_ERR_PLAETZE_UNGUELTIG: &str =
    "Wähl oben aus, wie viele Plätze frei sind, dann klick auf „Suche veröffentlichen“.";
pub const LFG_ERR_SCHON_AKTIVE_SUCHE: &str =
    "Du hast schon ein laufendes Gesuch. Schließ das erst, bevor du ein neues aufmachst.";
pub const LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN: &str =
    "Da ist was schiefgelaufen — dein Gesuch wurde nicht erstellt. Probier's gleich nochmal.";
pub const LFG_ERFOLG_POST_ERSTELLT: &str = "Dein Gesuch ist online.";
pub const LFG_ERFOLG_LANE_AUFGEMACHT: &str =
    "Lane ist offen und mit deinem Gesuch verknüpft — der Post zählt jetzt live mit.";
pub const LFG_ERFOLG_JOIN_MOVED: &str = "Ab in die Lane mit dir — viel Spaß!";
pub const LFG_BTN_LANE_AUFMACHEN: &str = "🚀 Lane gleich aufmachen";
pub const LFG_POST_TITEL_SCHEMA: &str = "LFG:";
pub const LFG_POST_BODY_HEADER: &str = "## Mitspieler gesucht";
pub const LFG_POST_BODY_VON: &str = "Suche von";
pub const LFG_POST_BODY_MODUS: &str = "Modus";
pub const LFG_POST_BODY_RANG: &str = "Rang";
pub const LFG_POST_BODY_PLAETZE: &str = "Plätze frei";
pub const LFG_POST_RANG_EGAL: &str = "Alle Ränge";
pub const LFG_POST_STATUS_OFFEN: &str =
    "Klick auf **Beitreten**, dann ziehen wir dich direkt in die Lane.";
pub const LFG_POST_STATUS_VOLL: &str =
    "Aktuell voll — schau später nochmal rein oder mach ein eigenes Gesuch auf.";
pub const LFG_BTN_BEITRETEN: &str = "➡️ Beitreten";
pub const LFG_ERR_JOIN_LANE_TOT: &str =
    "Die Lane gibt's nicht mehr — das Gesuch wird gerade geschlossen.";
pub const LFG_ERR_JOIN_LANE_VOLL: &str = "Die Lane ist gerade voll.";
pub const LFG_ERR_JOIN_KEIN_RANG: &str = "Für Ranked-Lanes brauchst du einen verifizierten Rang. Verknüpf dein Steam-Konto in <#1398021105339334666>.";
pub const LFG_ERR_JOIN_NICHT_IN_VOICE: &str =
    "Geh erst in irgendeinen Voice-Kanal — dann können wir dich direkt rüberziehen.";
pub const LFG_ERR_JOIN_EIGENER_POST: &str =
    "Das ist dein eigenes Gesuch — nutz den Lane-Button aus der Bestätigung.";
pub const LFG_ERR_JOIN_SCHON_DRIN: &str = "Du bist schon in der Lane.";
pub const LFG_ERR_JOIN_MOVE_FEHLGESCHLAGEN: &str =
    "Konnte dich nicht verschieben — probier's nochmal oder join die Lane manuell.";
pub const LFG_ERR_OPEN_NICHT_DEIN_POST: &str =
    "Nur wer das Gesuch erstellt hat, kann die Lane dazu aufmachen.";
pub const LFG_ERR_OPEN_NICHT_IN_VOICE: &str =
    "Geh erst in einen Voice-Kanal (z. B. über den Router), dann machen wir deine Lane auf.";
pub const LFG_ERR_OPEN_LANE_SCHON_VERKNUEPFT: &str = "Zu dem Gesuch läuft schon eine Lane.";
pub const LFG_BTN_PUBLISH_LANE: &str = "🔎 Mitspieler suchen";
pub const LFG_ERR_PUBLISH_LANE_SCHON_VEROEFFENTLICHT: &str =
    "Für diese Lane läuft schon ein Gesuch.";
pub const LFG_PRESETS_SUBMENU_TEXT: &str =
    "Speichern merkt sich Name, Plätze und Rang der Lane, in der du sitzt.\nLaden legt genau diesen Stand später wieder drauf.";
pub const LFG_PRESETS_BTN_SAVE: &str = "💾 Preset speichern";
pub const LFG_PRESETS_BTN_LOAD: &str = "📂 Preset laden";
pub const LFG_WATCH_BUILDER_TEXT: &str = "🔔 **Ich sag dir Bescheid.** Sag mir, wonach du suchst — ich schicke dir eine DM, sobald ein passendes Gesuch auftaucht.";
pub const LFG_WATCH_ERR_UNVOLLSTAENDIG: &str = "Wähl noch Modus und Zeitfenster aus.";
pub const LFG_WATCH_BTN_AKTIVIEREN: &str = "🔔 Aktivieren";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfgMode {
    Casual,
    Ranked,
    StreetBrawl,
}

impl LfgMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Casual => "casual",
            Self::Ranked => "ranked",
            Self::StreetBrawl => "street_brawl",
        }
    }

    fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "casual" => Some(Self::Casual),
            "ranked" => Some(Self::Ranked),
            "street_brawl" => Some(Self::StreetBrawl),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Casual => "Normale Lane",
            Self::Ranked => "Ranked",
            Self::StreetBrawl => "Street Brawl",
        }
    }

    fn mode_custom_id(self) -> String {
        format!("{LFG_CREATE_MODE_PREFIX}{}", self.as_str())
    }

    fn watch_mode_custom_id(self) -> String {
        format!("{LFG_WATCH_MODE_PREFIX}{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfgPlayWindow {
    Jetzt,
    HeuteAbend,
    Wochenende,
    Flexibel,
}

impl LfgPlayWindow {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Jetzt => "jetzt",
            Self::HeuteAbend => "heute_abend",
            Self::Wochenende => "wochenende",
            Self::Flexibel => "flexibel",
        }
    }

    pub(crate) fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "jetzt" => Some(Self::Jetzt),
            "heute_abend" => Some(Self::HeuteAbend),
            "wochenende" => Some(Self::Wochenende),
            "flexibel" => Some(Self::Flexibel),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Jetzt => "Jetzt / bin bereit",
            Self::HeuteAbend => "Heute Abend",
            Self::Wochenende => "Wochenende",
            Self::Flexibel => "Flexibel",
        }
    }

    pub(crate) fn emoji(self) -> &'static str {
        match self {
            Self::Jetzt => "⚡",
            Self::HeuteAbend => "🌙",
            Self::Wochenende => "📅",
            Self::Flexibel => "🕒",
        }
    }
}

pub fn play_window_dm_suffix(raw: &str) -> Option<String> {
    LfgPlayWindow::from_str(raw).map(|window| format!(" · {} {}", window.emoji(), window.label()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LfgRankRange {
    pub min: Option<i32>,
    pub max: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LfgDraft {
    mode: LfgMode,
    rank_from: Option<String>,
    rank_to: Option<String>,
    slots: Option<i32>,
    play_window: Option<LfgPlayWindow>,
    lane_id: Option<u64>,
}

impl LfgDraft {
    fn new(mode: LfgMode, lane_id: Option<u64>) -> Self {
        Self {
            mode,
            rank_from: None,
            rank_to: None,
            slots: None,
            play_window: None,
            lane_id,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LfgWatchDraft {
    mode: Option<LfgMode>,
    rank_from: Option<String>,
    rank_to: Option<String>,
    window: Option<LfgWatchWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LfgUserPreset {
    rank_from: Option<String>,
    rank_to: Option<String>,
    slots: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    play_window: Option<String>,
}

impl LfgUserPreset {
    fn from_draft(draft: &LfgDraft) -> Self {
        Self {
            rank_from: draft.rank_from.clone(),
            rank_to: draft.rank_to.clone(),
            slots: draft.slots,
            play_window: draft.play_window.map(|window| window.as_str().to_string()),
        }
    }

    fn into_draft(self, mode: LfgMode) -> Option<LfgDraft> {
        let rank_from = normalize_preset_rank(self.rank_from)?;
        let rank_to = normalize_preset_rank(self.rank_to)?;
        let slots = self
            .slots
            .map(|slots| slots.clamp(1, slot_select_cap(mode)));
        Some(LfgDraft {
            mode,
            rank_from,
            rank_to,
            slots,
            play_window: self
                .play_window
                .as_deref()
                .and_then(LfgPlayWindow::from_str),
            lane_id: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LfgRankSelectOption {
    label: &'static str,
    value: &'static str,
    emoji_name: &'static str,
    emoji_id: &'static str,
}

const LFG_RANK_SELECT_OPTIONS: [LfgRankSelectOption; 11] = [
    LfgRankSelectOption {
        label: "Initiate",
        value: "initiate",
        emoji_name: "initiate",
        emoji_id: "1316457822518775869",
    },
    LfgRankSelectOption {
        label: "Seeker",
        value: "seeker",
        emoji_name: "seeker",
        emoji_id: "1316458138886475876",
    },
    LfgRankSelectOption {
        label: "Acolyte",
        value: "acolyte",
        emoji_name: "acolyte",
        emoji_id: "1316455291629342750",
    },
    LfgRankSelectOption {
        label: "Sentinel",
        value: "sentinel",
        emoji_name: "sentinel",
        emoji_id: "1316455305315352587",
    },
    LfgRankSelectOption {
        label: "Mystic",
        value: "mystic",
        emoji_name: "mystic",
        emoji_id: "1316458203298660533",
    },
    LfgRankSelectOption {
        label: "Ritualist",
        value: "ritualist",
        emoji_name: "ritualist",
        emoji_id: "1316457650367496306",
    },
    LfgRankSelectOption {
        label: "Emissary",
        value: "emissary",
        emoji_name: "emissary",
        emoji_id: "1397687455313952918",
    },
    LfgRankSelectOption {
        label: "Oracle",
        value: "oracle",
        emoji_name: "oracle",
        emoji_id: "1316457885743579317",
    },
    LfgRankSelectOption {
        label: "Phantom",
        value: "phantom",
        emoji_name: "phantom",
        emoji_id: "1316457982363701278",
    },
    LfgRankSelectOption {
        label: "Ascendant",
        value: "ascendant",
        emoji_name: "ascendant",
        emoji_id: "1316457367818338385",
    },
    LfgRankSelectOption {
        label: "Eternus",
        value: "eternus",
        emoji_name: "eternus",
        emoji_id: "1316457737621868574",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfgForumPostDraft {
    pub post_id: i64,
    pub title: String,
    pub body: String,
    pub applied_tags: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LfgCreatedForumPost {
    pub thread_id: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfgPanelApplyOutput {
    pub guild_id: u64,
    pub channel_id: Option<u64>,
    pub dry_run: bool,
    pub payload_format: String,
    pub stored_payload_format: Option<String>,
    pub stored_message_id: Option<u64>,
    pub action: String,
    pub message_id: Option<u64>,
    pub blocked_reason: Option<String>,
    pub warnings: Vec<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfgPanelAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfgPanelMessage {
    pub message_id: u64,
    pub has_embeds: bool,
    pub has_components: bool,
    pub custom_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LfgPanelChannelKind {
    Text,
    News,
    Forum,
    Other(String),
}

impl LfgPanelChannelKind {
    fn supports_regular_messages(&self) -> bool {
        matches!(self, Self::Text | Self::News)
    }

    fn as_reason_fragment(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::News => "news",
            Self::Forum => "forum",
            Self::Other(kind) => kind.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LfgLaneOccupancy {
    pub member_count: i64,
    pub user_limit: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LfgEditError {
    RateLimited { retry_after_seconds: f64 },
    NotFound(String),
    Other(String),
}

impl LfgEditError {
    pub fn rate_limited(retry_after_seconds: f64) -> Self {
        Self::RateLimited {
            retry_after_seconds,
        }
    }

    fn retry_after_seconds(&self) -> Option<f64> {
        match self {
            Self::RateLimited {
                retry_after_seconds,
            } => Some(*retry_after_seconds),
            Self::NotFound(_) | Self::Other(_) => None,
        }
    }
}

impl fmt::Display for LfgEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited {
                retry_after_seconds,
            } => write!(f, "HTTP 429 (retry_after={retry_after_seconds})"),
            Self::NotFound(err) | Self::Other(err) => f.write_str(err),
        }
    }
}

#[async_trait::async_trait]
pub trait LfgPanelPort: Send + Sync {
    async fn post_rich(
        &self,
        channel_id: u64,
        body: Map<String, Value>,
        attachments: &[LfgPanelAttachment],
    ) -> Result<u64, String>;

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
        attachments: &[LfgPanelAttachment],
    ) -> Result<(), String>;

    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<LfgPanelMessage>, String>;

    async fn panel_channel_kind(
        &self,
        _guild_id: u64,
        _channel_id: u64,
    ) -> Option<LfgPanelChannelKind> {
        None
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;

    async fn create_forum_post(
        &self,
        forum_channel_id: u64,
        draft: LfgForumPostDraft,
    ) -> Result<LfgCreatedForumPost, String>;

    async fn first_thread_message_id(&self, thread_id: u64) -> Result<Option<u64>, String>;

    async fn archive_and_lock_thread(&self, thread_id: u64) -> Result<(), String>;

    async fn edit_forum_starter_message(
        &self,
        thread_id: u64,
        starter_message_id: u64,
        body: String,
    ) -> Result<(), LfgEditError>;

    async fn edit_forum_post_tags(
        &self,
        thread_id: u64,
        applied_tags: Vec<u64>,
    ) -> Result<(), LfgEditError>;

    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;

    /// Schickt dem User eine Direktnachricht. Fehler werden vom Aufrufer nur geloggt.
    async fn send_dm(&self, user_id: u64, content: String) -> Result<(), String>;

    async fn move_member(&self, guild_id: u64, user_id: u64, lane_id: u64) -> Result<(), String>;

    async fn lane_occupancy(&self, guild_id: u64, lane_id: u64) -> Option<LfgLaneOccupancy>;

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64>;
}

#[async_trait::async_trait]
pub trait LfgLaneSpawner: Send + Sync {
    async fn spawn_lane_from_current_voice(
        &self,
        guild_id: u64,
        user_id: u64,
        mode: &str,
        interaction_role_ids: &[u64],
    ) -> crate::router::RouterSpawnOutcome;

    async fn cleanup_created_lane(
        &self,
        guild_id: u64,
        lane_id: u64,
        reason: &str,
    ) -> Result<(), String>;
}

pub struct RouterLfgLaneSpawner {
    router: Arc<crate::router::LaneRouter>,
}

impl RouterLfgLaneSpawner {
    pub fn new(router: Arc<crate::router::LaneRouter>) -> Arc<Self> {
        Arc::new(Self { router })
    }
}

#[async_trait::async_trait]
impl LfgLaneSpawner for RouterLfgLaneSpawner {
    async fn spawn_lane_from_current_voice(
        &self,
        guild_id: u64,
        user_id: u64,
        mode: &str,
        interaction_role_ids: &[u64],
    ) -> crate::router::RouterSpawnOutcome {
        self.router
            .spawn_lane_from_current_voice_with_role_ids(
                guild_id,
                user_id,
                mode,
                interaction_role_ids,
            )
            .await
    }

    async fn cleanup_created_lane(
        &self,
        guild_id: u64,
        lane_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.router
            .cleanup_lfg_created_lane(guild_id, lane_id, reason)
            .await
    }
}

#[derive(Default)]
struct LfgEditQueue {
    pending: Mutex<HashSet<i64>>,
    notify: Notify,
}

pub struct LfgPanelInterface {
    pool: PgPool,
    port: Arc<dyn LfgPanelPort>,
    panel_channel_id: Option<u64>,
    missing_panel_channel_reason: Option<String>,
    forum_channel_id: Option<u64>,
    missing_forum_channel_reason: Option<String>,
    cutover_active: bool,
    lane_spawner: RwLock<Option<Arc<dyn LfgLaneSpawner>>>,
    edit_queue: LfgEditQueue,
    pending_drafts: Mutex<HashMap<u64, LfgDraft>>,
    pending_watch_drafts: Mutex<HashMap<u64, LfgWatchDraft>>,
}

impl LfgPanelInterface {
    pub fn new(pool: PgPool, port: Arc<dyn LfgPanelPort>, channel_id: Option<u64>) -> Arc<Self> {
        Self::new_with_split_channel_config(
            pool,
            port,
            channel_id,
            channel_id
                .is_none()
                .then(|| "DL_LFG_PANEL_CHANNEL_ID fehlt".to_string()),
            channel_id,
            channel_id
                .is_none()
                .then(|| "DL_LFG_FORUM_CHANNEL_ID fehlt".to_string()),
            true,
        )
    }

    pub fn new_with_channel_config(
        pool: PgPool,
        port: Arc<dyn LfgPanelPort>,
        channel_id: Option<u64>,
        missing_channel_reason: Option<String>,
        cutover_active: bool,
    ) -> Arc<Self> {
        Self::new_with_split_channel_config(
            pool,
            port,
            channel_id,
            missing_channel_reason.clone(),
            channel_id,
            channel_id
                .is_none()
                .then(|| "DL_LFG_FORUM_CHANNEL_ID fehlt".to_string()),
            cutover_active,
        )
    }

    pub fn new_with_split_channel_config(
        pool: PgPool,
        port: Arc<dyn LfgPanelPort>,
        panel_channel_id: Option<u64>,
        missing_panel_channel_reason: Option<String>,
        forum_channel_id: Option<u64>,
        missing_forum_channel_reason: Option<String>,
        cutover_active: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            panel_channel_id,
            missing_panel_channel_reason,
            forum_channel_id,
            missing_forum_channel_reason,
            cutover_active,
            lane_spawner: RwLock::new(None),
            edit_queue: LfgEditQueue::default(),
            pending_drafts: Mutex::new(HashMap::new()),
            pending_watch_drafts: Mutex::new(HashMap::new()),
        })
    }

    pub async fn set_lane_spawner(&self, spawner: Arc<dyn LfgLaneSpawner>) {
        *self.lane_spawner.write().await = Some(spawner);
    }

    pub async fn ensure_panel(&self) {
        if let Err(err) = self.apply_panel(true).await {
            tracing::warn!(%err, "LfgPanelInterface: Panel konnte nicht angewendet werden");
        }
    }

    pub fn target_channel_id(&self) -> Option<u64> {
        self.panel_channel_id
    }

    pub fn cutover_active(&self) -> bool {
        self.cutover_active
    }

    pub async fn apply_panel(&self, confirm: bool) -> Result<LfgPanelApplyOutput, String> {
        let attachments = lfg_panel_attachments();
        let body = lfg_panel_body_for_attachments(&attachments);
        let stored_message_id = self.panel_message_id().await;
        let stored_payload_format = self.panel_payload_format().await;
        if !self.cutover_active {
            let mut output = LfgPanelApplyOutput {
                guild_id: LFG_GUILD_ID,
                channel_id: self.panel_channel_id,
                dry_run: true,
                payload_format: LFG_PAYLOAD_FORMAT.to_string(),
                stored_payload_format,
                stored_message_id,
                action: "blocked_cutover_disabled".to_string(),
                message_id: stored_message_id,
                blocked_reason: Some("cutover_disabled".to_string()),
                warnings: Vec::new(),
                payload: Value::Object(body),
            };
            output
                .warnings
                .push("LFG-Panel: Cutover ist nicht aktiv; confirm wird blockiert.".to_string());
            if let Some(reason) = self.missing_forum_channel_reason.as_deref() {
                output
                    .warnings
                    .push(format!("LFG-Forum: Zielkanal fehlt ({reason})."));
            }
            return Ok(output);
        }
        if let Some(reason) = self.panel_channel_blocked_reason().await {
            let action = if self.panel_channel_id.is_some() {
                "blocked_invalid_channel_type"
            } else {
                "blocked_missing_channel"
            };
            return Ok(LfgPanelApplyOutput {
                guild_id: LFG_GUILD_ID,
                channel_id: self.panel_channel_id,
                dry_run: true,
                payload_format: LFG_PAYLOAD_FORMAT.to_string(),
                stored_payload_format,
                stored_message_id,
                action: action.to_string(),
                message_id: stored_message_id,
                blocked_reason: Some(reason.clone()),
                warnings: vec![format!("LFG-Panel: {reason}")],
                payload: Value::Object(body),
            });
        }
        let recent = match self.panel_channel_id {
            Some(channel_id) => match self.port.recent_bot_messages(channel_id, 15).await {
                Ok(messages) => Some(messages),
                Err(err) => {
                    tracing::warn!(%err, "LfgPanelInterface: History-Scan fehlgeschlagen");
                    None
                }
            },
            None => None,
        };
        let history_checked = self.panel_channel_id.is_none() || recent.is_some();
        let history_message_id = recent.as_deref().and_then(find_existing_lfg_v2_panel);
        let planned_message_id = stored_message_id.or(history_message_id);
        let action = match (self.panel_channel_id, planned_message_id) {
            (None, _) => "blocked_missing_channel",
            (Some(_), Some(_)) => "planned_edit",
            (Some(_), None) => "planned_post",
        };
        let mut output = LfgPanelApplyOutput {
            guild_id: LFG_GUILD_ID,
            channel_id: self.panel_channel_id,
            dry_run: !confirm,
            payload_format: LFG_PAYLOAD_FORMAT.to_string(),
            stored_payload_format,
            stored_message_id,
            action: action.to_string(),
            message_id: planned_message_id,
            blocked_reason: self
                .panel_channel_id
                .is_none()
                .then(|| self.missing_panel_channel_reason.clone())
                .flatten(),
            warnings: Vec::new(),
            payload: Value::Object(body.clone()),
        };
        if self.panel_channel_id.is_none() {
            let reason = self
                .missing_panel_channel_reason
                .as_deref()
                .unwrap_or("DL_LFG_PANEL_CHANNEL_ID fehlt");
            output.warnings.push(format!(
                "LFG-Panel: Zielkanal fehlt ({reason}); DL_LFG_PANEL_CHANNEL_ID muss auf einen Text-Kanal zeigen."
            ));
            output.dry_run = true;
            return Ok(output);
        }
        if !confirm {
            if self.panel_channel_id.is_some() && !history_checked {
                output.warnings.push(
                    "LFG-Panel: History-Scan fehlgeschlagen; Dry-Run ohne Adoption.".to_string(),
                );
            }
            return Ok(output);
        }

        validate_lfg_panel_attachments(&attachments)?;
        let channel_id = self.panel_channel_id.expect("checked channel_id");
        let mut ignored_history_message_id = None;
        if let Some(message_id) = stored_message_id {
            match self
                .port
                .edit_rich(channel_id, message_id, body.clone(), &attachments)
                .await
            {
                Ok(()) => {
                    self.store_panel_metadata(message_id).await;
                    output.dry_run = false;
                    output.action = "edited".to_string();
                    output.message_id = Some(message_id);
                    return Ok(output);
                }
                Err(err) if is_not_found_error(&err) => {
                    self.delete_panel_message_id().await;
                    ignored_history_message_id = Some(message_id);
                    output.warnings.push(format!(
                        "LFG-Panel: gespeicherte Message-ID {message_id} ist stale (404); KV wurde geloescht."
                    ));
                }
                Err(err) => return Err(err),
            }
        }

        if !history_checked {
            return Err(
                "LfgPanelInterface: ohne erfolgreichen History-Scan wird kein neues Panel gepostet"
                    .to_string(),
            );
        }

        if let Some(message_id) =
            history_message_id.filter(|message_id| Some(*message_id) != ignored_history_message_id)
        {
            self.port
                .edit_rich(channel_id, message_id, body.clone(), &attachments)
                .await
                .map_err(|err| {
                    format!(
                        "LfgPanelInterface: adoptiertes Panel {message_id} konnte nicht editiert werden: {err}"
                    )
                })?;
            self.store_panel_metadata(message_id).await;
            output.dry_run = false;
            output.action = "adopted_edit".to_string();
            output.message_id = Some(message_id);
            return Ok(output);
        }

        let message_id = self.port.post_rich(channel_id, body, &attachments).await?;
        self.store_panel_metadata(message_id).await;
        output.dry_run = false;
        output.action = "posted".to_string();
        output.message_id = Some(message_id);
        Ok(output)
    }

    async fn panel_channel_blocked_reason(&self) -> Option<String> {
        let Some(panel_channel_id) = self.panel_channel_id else {
            return Some(
                self.missing_panel_channel_reason
                    .clone()
                    .unwrap_or_else(|| "DL_LFG_PANEL_CHANNEL_ID fehlt".to_string()),
            );
        };
        let kind = self
            .port
            .panel_channel_kind(LFG_GUILD_ID, panel_channel_id)
            .await?;
        if kind.supports_regular_messages() {
            return None;
        }
        Some(format!(
            "DL_LFG_PANEL_CHANNEL_ID zeigt auf {}-Kanal; Panel-Ziel muss ein Text-Kanal sein",
            kind.as_reason_fragment()
        ))
    }

    async fn panel_message_id(&self) -> Option<u64> {
        kv::get(&self.pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
    }

    async fn panel_payload_format(&self) -> Option<String> {
        kv::get(&self.pool, LFG_PANEL_KV_NS, LFG_PAYLOAD_FORMAT_KEY)
            .await
            .ok()
            .flatten()
    }

    async fn store_panel_metadata(&self, message_id: u64) {
        if let Err(err) = kv::set(
            &self.pool,
            LFG_PANEL_KV_NS,
            LFG_PANEL_MESSAGE_KEY,
            &message_id.to_string(),
        )
        .await
        {
            tracing::warn!(%err, "LfgPanelInterface: Message-ID konnte nicht gespeichert werden");
        }
        if let Err(err) = kv::set(
            &self.pool,
            LFG_PANEL_KV_NS,
            LFG_PAYLOAD_FORMAT_KEY,
            LFG_PAYLOAD_FORMAT,
        )
        .await
        {
            tracing::warn!(%err, "LfgPanelInterface: Payload-Format konnte nicht gespeichert werden");
        }
    }

    async fn delete_panel_message_id(&self) {
        if let Err(err) = kv::delete(&self.pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY).await {
            tracing::warn!(%err, "LfgPanelInterface: stale Message-ID konnte nicht geloescht werden");
        }
    }
}

pub fn lfg_repo_root() -> PathBuf {
    crate::router::router_repo_root()
}

pub fn lfg_panel_attachments() -> Vec<LfgPanelAttachment> {
    vec![LfgPanelAttachment {
        id: 0,
        filename: LFG_PANEL_BANNER_FILENAME.to_string(),
        relative_path: format!("{LFG_BANNER_DIR}/{LFG_PANEL_BANNER_FILENAME}"),
    }]
}

pub fn lfg_panel_body() -> Map<String, Value> {
    let attachments = lfg_panel_attachments();
    lfg_panel_body_for_attachments(&attachments)
}

fn lfg_panel_body_for_attachments(attachments: &[LfgPanelAttachment]) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".to_string(), json!(LFG_COMPONENTS_V2_FLAG));
    body.insert(
        "allowed_mentions".to_string(),
        json!({ "parse": Vec::<String>::new() }),
    );
    let mut components = Vec::new();
    if !attachments.is_empty() {
        components.push(lfg_media_gallery(&attachments[0].filename));
    }
    components.push(lfg_text_display(LFG_PANEL_BODY.to_string()));
    components.push(lfg_action_row(vec![
        lfg_emoji_button(
            LFG_PANEL_BUTTON,
            1,
            LFG_CREATE_START_CUSTOM_ID,
            LFG_EMOJI_SEARCH,
        ),
        lfg_button(LFG_WATCH_PANEL_BUTTON, 2, LFG_WATCH_START_CUSTOM_ID),
    ]));
    body.insert(
        "components".to_string(),
        json!([{
            "type": 17,
            "accent_color": LFG_ACCENT_GOLD,
            "components": components,
        }]),
    );
    body.insert("attachments".to_string(), json!(attachments));
    body
}

fn lfg_media_gallery(filename: &str) -> Value {
    json!({
        "type": 12,
        "items": [{
            "media": { "url": format!("attachment://{filename}") },
        }],
    })
}

fn lfg_text_display(content: String) -> Value {
    json!({
        "type": 10,
        "content": content,
    })
}

fn lfg_action_row(components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "components": components,
    })
}

fn lfg_button(label: &str, style: u8, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
    })
}

fn lfg_emoji_button(label: &str, style: u8, custom_id: &str, emoji: (&str, &str)) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
        "emoji": { "name": emoji.0, "id": emoji.1 },
    })
}

fn validate_lfg_panel_attachments(attachments: &[LfgPanelAttachment]) -> Result<(), String> {
    let repo_root = lfg_repo_root();
    for attachment in attachments {
        let path = repo_root.join(&attachment.relative_path);
        if !path.is_file() {
            return Err(format!(
                "LFG-Banner `{}` fehlt; LFG-Panel wird nicht gepostet/editiert",
                path.display()
            ));
        }
    }
    Ok(())
}

fn find_existing_lfg_v2_panel(messages: &[LfgPanelMessage]) -> Option<u64> {
    messages
        .iter()
        .rev()
        .find(|message| {
            !message.has_embeds
                && message.has_components
                && message
                    .custom_ids
                    .iter()
                    .any(|custom_id| custom_id.starts_with("lfg:create:"))
        })
        .map(|message| message.message_id)
}

fn is_not_found_error(err: &str) -> bool {
    err.contains("HTTP 404") || err.contains("404")
}

fn lfg_mode_selection_components() -> Value {
    json!([{ "type": 1, "components": [
        lfg_emoji_button(LFG_MODE_BUTTON_CASUAL, 2, &LfgMode::Casual.mode_custom_id(), crate::router::ROUTER_EMOJI_CASUAL),
        lfg_emoji_button(LFG_MODE_BUTTON_RANKED, 1, &LfgMode::Ranked.mode_custom_id(), crate::router::ROUTER_EMOJI_RANKED),
        lfg_emoji_button(LFG_MODE_BUTTON_STREET_BRAWL, 2, &LfgMode::StreetBrawl.mode_custom_id(), crate::router::ROUTER_EMOJI_BRAWL),
    ]}])
}

fn lfg_watch_mode_components() -> Value {
    json!({ "type": 1, "components": [
        lfg_emoji_button(LFG_MODE_BUTTON_CASUAL, 2, &LfgMode::Casual.watch_mode_custom_id(), crate::router::ROUTER_EMOJI_CASUAL),
        lfg_emoji_button(LFG_MODE_BUTTON_RANKED, 1, &LfgMode::Ranked.watch_mode_custom_id(), crate::router::ROUTER_EMOJI_RANKED),
        lfg_emoji_button(LFG_MODE_BUTTON_STREET_BRAWL, 2, &LfgMode::StreetBrawl.watch_mode_custom_id(), crate::router::ROUTER_EMOJI_BRAWL),
    ]})
}

fn lfg_post_custom_id(mode: LfgMode) -> String {
    format!("{LFG_POST_PREFIX}{}", mode.as_str())
}

fn lfg_open_lane_custom_id(post_id: i64) -> String {
    format!("{LFG_OPEN_LANE_PREFIX}{post_id}")
}

pub fn lfg_join_custom_id(post_id: i64) -> String {
    format!("{LFG_JOIN_PREFIX}{post_id}")
}

fn lfg_success_components(post_id: i64) -> Value {
    json!([{ "type": 1, "components": [
        lfg_button(LFG_BTN_LANE_AUFMACHEN, 1, &lfg_open_lane_custom_id(post_id))
    ]}])
}

fn lfg_rank_select_option_json(
    label: &str,
    value: &str,
    emoji_name: &str,
    emoji_id: &str,
    selected: bool,
) -> Value {
    json!({
        "label": label,
        "value": value,
        "default": selected,
        "emoji": { "name": emoji_name, "id": emoji_id },
    })
}

fn lfg_rank_select(custom_id: &str, placeholder: &str, current: Option<&str>) -> Value {
    let mut options = Vec::with_capacity(LFG_RANK_SELECT_OPTIONS.len() + 1);
    options.push(lfg_rank_select_option_json(
        LFG_RANK_ANY_LABEL,
        LFG_RANK_ANY_VALUE,
        "dl_rang_egal",
        "1522801043803472064",
        false,
    ));
    options.extend(LFG_RANK_SELECT_OPTIONS.iter().map(|rank| {
        lfg_rank_select_option_json(
            rank.label,
            rank.value,
            rank.emoji_name,
            rank.emoji_id,
            current == Some(rank.value),
        )
    }));
    json!({
        "type": 1,
        "components": [{
            "type": 3,
            "custom_id": custom_id,
            "placeholder": placeholder,
            "min_values": 1,
            "max_values": 1,
            "options": options,
        }],
    })
}

fn slot_select_cap(mode: LfgMode) -> i32 {
    mode_default_capacity(mode).clamp(1, 5) as i32
}

fn lfg_slots_select(mode: LfgMode, current: Option<i32>) -> Value {
    let options = (1..=slot_select_cap(mode))
        .map(|slots| {
            let label = match slots {
                1 => "1 Platz".to_string(),
                2 => "2 Plätze".to_string(),
                3 => "3 Plätze".to_string(),
                4 => "4 Plätze".to_string(),
                5 => "5 Plätze".to_string(),
                other => format!("{other} Plätze"),
            };
            json!({
                "label": label,
                "value": slots.to_string(),
                "default": current == Some(slots),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": 1,
        "components": [{
            "type": 3,
            "custom_id": LFG_SLOTS_SELECT_CUSTOM_ID,
            "placeholder": LFG_SLOTS_PLACEHOLDER,
            "min_values": 1,
            "max_values": 1,
            "options": options,
        }],
    })
}

fn lfg_when_select(current: Option<LfgPlayWindow>) -> Value {
    let options = [
        LfgPlayWindow::Jetzt,
        LfgPlayWindow::HeuteAbend,
        LfgPlayWindow::Wochenende,
        LfgPlayWindow::Flexibel,
    ]
    .into_iter()
    .map(|window| {
        json!({
            "label": format!("{} {}", window.emoji(), window.label()),
            "value": window.as_str(),
            "default": current == Some(window),
        })
    })
    .collect::<Vec<_>>();
    json!({
        "type": 1,
        "components": [{
            "type": 3,
            "custom_id": LFG_WHEN_SELECT_CUSTOM_ID,
            "placeholder": "Wann willst du spielen? (optional)",
            "min_values": 1,
            "max_values": 1,
            "options": options,
        }],
    })
}

fn lfg_watch_window_select(current: Option<LfgWatchWindow>) -> Value {
    let options = [
        (LfgWatchWindow::Now3h, "⚡ Die nächsten 3 Stunden"),
        (LfgWatchWindow::Today, "🌙 Heute"),
        (LfgWatchWindow::Weekend, "📅 Dieses Wochenende"),
        (LfgWatchWindow::Week, "🗓️ Diese Woche"),
    ]
    .into_iter()
    .map(|(window, label)| {
        json!({
            "label": label,
            "value": window.as_str(),
            "default": current == Some(window),
        })
    })
    .collect::<Vec<_>>();
    json!({
        "type": 1,
        "components": [{
            "type": 3,
            "custom_id": LFG_WATCH_WINDOW_SELECT_CUSTOM_ID,
            "placeholder": "Wie lange soll ich Ausschau halten?",
            "min_values": 1,
            "max_values": 1,
            "options": options,
        }],
    })
}

fn lfg_draft_components(draft: &LfgDraft) -> Value {
    json!([
        lfg_rank_select(
            LFG_RANK_FROM_SELECT_CUSTOM_ID,
            LFG_RANK_FROM_PLACEHOLDER,
            draft.rank_from.as_deref(),
        ),
        lfg_rank_select(
            LFG_RANK_TO_SELECT_CUSTOM_ID,
            LFG_RANK_TO_PLACEHOLDER,
            draft.rank_to.as_deref(),
        ),
        lfg_slots_select(draft.mode, draft.slots),
        lfg_when_select(draft.play_window),
        { "type": 1, "components": [
            lfg_button(LFG_BTN_POSTEN, 1, &lfg_post_custom_id(draft.mode))
        ]},
    ])
}

fn lfg_watch_components(draft: &LfgWatchDraft) -> Value {
    json!([
        lfg_watch_mode_components(),
        lfg_rank_select(
            LFG_WATCH_RANK_FROM_SELECT_CUSTOM_ID,
            LFG_RANK_FROM_PLACEHOLDER,
            draft.rank_from.as_deref(),
        ),
        lfg_rank_select(
            LFG_WATCH_RANK_TO_SELECT_CUSTOM_ID,
            LFG_RANK_TO_PLACEHOLDER,
            draft.rank_to.as_deref(),
        ),
        lfg_watch_window_select(draft.window),
        { "type": 1, "components": [
            lfg_button(LFG_WATCH_BTN_AKTIVIEREN, 1, LFG_WATCH_ACTIVATE_CUSTOM_ID)
        ]},
    ])
}

fn lfg_rank_label_from_index(index: i32) -> String {
    LFG_RANK_SELECT_OPTIONS
        .iter()
        .find(|rank| {
            i32::try_from(crate::tempvoice::logic::rank_index(rank.value)).ok() == Some(index)
        })
        .map(|rank| rank.label.to_string())
        .unwrap_or_else(|| rank_name(index))
}

fn lfg_draft_rank_summary(draft: &LfgDraft) -> String {
    let Some(range) = lfg_rank_range_from_draft(draft) else {
        return LFG_RANK_ANY_LABEL.to_string();
    };
    match (draft.rank_from.as_deref(), draft.rank_to.as_deref()) {
        (None, None) => LFG_RANK_ANY_LABEL.to_string(),
        (Some(_), None) => match range.min {
            Some(min) => format!("ab {}", lfg_rank_label_from_index(min)),
            None => LFG_RANK_ANY_LABEL.to_string(),
        },
        (None, Some(_)) => match range.max {
            Some(max) => format!("bis {}", lfg_rank_label_from_index(max)),
            None => LFG_RANK_ANY_LABEL.to_string(),
        },
        (Some(_), Some(_)) => match (range.min, range.max) {
            (Some(min), Some(max)) => format!(
                "{} → {}",
                lfg_rank_label_from_index(min),
                lfg_rank_label_from_index(max)
            ),
            (Some(min), None) => format!("ab {}", lfg_rank_label_from_index(min)),
            (None, Some(max)) => format!("bis {}", lfg_rank_label_from_index(max)),
            (None, None) => LFG_RANK_ANY_LABEL.to_string(),
        },
    }
}

fn lfg_draft_slots_summary(slots: Option<i32>) -> String {
    match slots {
        None => "Plätze offen".to_string(),
        Some(1) => "1 Platz".to_string(),
        Some(slots) => format!("{slots} Plätze"),
    }
}

fn lfg_draft_content(draft: &LfgDraft) -> String {
    let when = match draft.play_window {
        Some(window) => format!(" · {} {}", window.emoji(), window.label()),
        None => String::new(),
    };
    format!(
        "**{}** · {} · {}{}\nWähl Rang-Bereich und Plätze, dann **Suche veröffentlichen**.",
        draft.mode.display_name(),
        lfg_draft_rank_summary(draft),
        lfg_draft_slots_summary(draft.slots),
        when
    )
}

fn lfg_draft_reply(draft: &LfgDraft, update_message: bool) -> BridgeReply {
    BridgeReply {
        content: Some(lfg_draft_content(draft)),
        components: Some(lfg_draft_components(draft)),
        ephemeral: !update_message,
        update_message,
        ..BridgeReply::default()
    }
}

fn lfg_watch_reply(draft: &LfgWatchDraft, update_message: bool) -> BridgeReply {
    BridgeReply {
        content: Some(LFG_WATCH_BUILDER_TEXT.to_string()),
        components: Some(lfg_watch_components(draft)),
        ephemeral: !update_message,
        update_message,
        ..BridgeReply::default()
    }
}

fn lfg_watch_expires_at_sql(now_expr: &str, window: LfgWatchWindow) -> String {
    let target = match window {
        LfgWatchWindow::Now3h => format!("({now_expr}) + interval '3 hours'"),
        LfgWatchWindow::Today => {
            format!(
                "GREATEST(
                    ({now_expr}) + interval '1 hour',
                    (((({now_expr}) AT TIME ZONE 'Europe/Berlin')::date + 1) + time '02:00')
                        AT TIME ZONE 'Europe/Berlin'
                )"
            )
        }
        LfgWatchWindow::Weekend => {
            format!(
                "((((({now_expr}) AT TIME ZONE 'Europe/Berlin')::date
                    + (8 - EXTRACT(ISODOW FROM ({now_expr}) AT TIME ZONE 'Europe/Berlin')::int))
                    + time '00:00') AT TIME ZONE 'Europe/Berlin')"
            )
        }
        LfgWatchWindow::Week => format!("({now_expr}) + interval '7 days'"),
    };
    format!("SELECT GREATEST(({target}), ({now_expr}) + interval '1 minute')")
}

async fn lfg_watch_expires_at(
    pool: &PgPool,
    window: LfgWatchWindow,
) -> Result<DateTime<Utc>, sqlx::Error> {
    let sql = lfg_watch_expires_at_sql("now()", window);
    sqlx::query_scalar(&sql).fetch_one(pool).await
}

fn parse_requested_slots(raw: &str, mode: LfgMode) -> Option<i32> {
    raw.trim()
        .parse::<i32>()
        .ok()
        .filter(|slots| (1..=slot_select_cap(mode)).contains(slots))
}

fn normalize_lfg_rank_value(raw: &str) -> Option<Option<String>> {
    let normalized = raw.trim().to_lowercase();
    if normalized == LFG_RANK_ANY_VALUE {
        return Some(None);
    }
    (crate::tempvoice::logic::rank_index(&normalized) > 0).then_some(Some(normalized))
}

fn normalize_preset_rank(raw: Option<String>) -> Option<Option<String>> {
    match raw {
        Some(raw) => normalize_lfg_rank_value(&raw),
        None => Some(None),
    }
}

fn lfg_rank_value_index(value: Option<&str>) -> Option<Option<i32>> {
    let Some(value) = value else {
        return Some(None);
    };
    if value == LFG_RANK_ANY_VALUE {
        return Some(None);
    }
    let idx = crate::tempvoice::logic::rank_index(value);
    if idx == 0 {
        return None;
    }
    let Ok(idx) = i32::try_from(idx) else {
        return None;
    };
    Some(Some(idx))
}

fn lfg_rank_range_from_values(
    rank_from: Option<&str>,
    rank_to: Option<&str>,
) -> Option<LfgRankRange> {
    let mut min = lfg_rank_value_index(rank_from)?;
    let mut max = lfg_rank_value_index(rank_to)?;
    if let (Some(left), Some(right)) = (min, max) {
        if left > right {
            min = Some(right);
            max = Some(left);
        }
    }
    Some(LfgRankRange { min, max })
}

fn lfg_rank_range_from_draft(draft: &LfgDraft) -> Option<LfgRankRange> {
    lfg_rank_range_from_values(draft.rank_from.as_deref(), draft.rank_to.as_deref())
}

fn lfg_rank_range_from_watch_draft(draft: &LfgWatchDraft) -> Option<LfgRankRange> {
    lfg_rank_range_from_values(draft.rank_from.as_deref(), draft.rank_to.as_deref())
}

#[cfg(test)]
fn parse_rank_range(raw: &str) -> Option<LfgRankRange> {
    let normalized = raw.trim().to_lowercase();
    if normalized.is_empty() {
        return Some(LfgRankRange {
            min: None,
            max: None,
        });
    }

    fn rank_token(token: &str) -> Option<i32> {
        let idx = crate::tempvoice::logic::rank_index(token.trim());
        (idx > 0).then(|| i32::try_from(idx).expect("rank index fits i32"))
    }

    fn range(min: i32, max: i32) -> Option<LfgRankRange> {
        (min <= max).then_some(LfgRankRange {
            min: Some(min),
            max: Some(max),
        })
    }

    if normalized.contains('-') {
        let parts = normalized.split('-').map(str::trim).collect::<Vec<_>>();
        if let [min, max] = parts.as_slice() {
            return range(rank_token(min)?, rank_token(max)?);
        }
        return None;
    }

    match normalized.split_whitespace().collect::<Vec<_>>().as_slice() {
        [rank] => {
            let rank = rank_token(rank)?;
            Some(LfgRankRange {
                min: Some(rank),
                max: Some(rank),
            })
        }
        [min, "bis", max] => range(rank_token(min)?, rank_token(max)?),
        _ => None,
    }
}

fn rank_name(index: i32) -> String {
    crate::tempvoice::logic::RANK_ORDER
        .get(index as usize)
        .map(|rank| crate::tempvoice::logic::capitalize(rank))
        .unwrap_or_else(|| LFG_ERR_RANG_UNBEKANNT.to_string())
}

fn rank_range_label(range: LfgRankRange) -> String {
    match (range.min, range.max) {
        (Some(min), Some(max)) if min == max => rank_name(min),
        (Some(min), Some(max)) => format!("{} bis {}", rank_name(min), rank_name(max)),
        (Some(min), None) => format!("{} bis {}", rank_name(min), rank_name(11)),
        (None, Some(max)) => format!("{} bis {}", rank_name(1), rank_name(max)),
        _ => LFG_POST_RANG_EGAL.to_string(),
    }
}

#[derive(Debug, Clone, Copy)]
struct LfgForumRankBracket {
    min: i32,
    max: i32,
    tag_id: u64,
}

const LFG_FORUM_RANK_BRACKETS: [LfgForumRankBracket; 4] = [
    LfgForumRankBracket {
        min: 1,
        max: 2,
        tag_id: LFG_FORUM_TAG_RANK_BEGINNER,
    },
    LfgForumRankBracket {
        min: 3,
        max: 5,
        tag_id: LFG_FORUM_TAG_RANK_ADVANCED,
    },
    LfgForumRankBracket {
        min: 6,
        max: 8,
        tag_id: LFG_FORUM_TAG_RANK_EXPERIENCED,
    },
    LfgForumRankBracket {
        min: 9,
        max: 11,
        tag_id: LFG_FORUM_TAG_RANK_ELITE,
    },
];

fn ranges_overlap(left_min: i32, left_max: i32, right_min: i32, right_max: i32) -> bool {
    left_min <= right_max && right_min <= left_max
}

fn derive_lfg_forum_tag_ids(
    mode: LfgMode,
    rank_range: LfgRankRange,
    lane_attached: bool,
) -> Vec<u64> {
    let mode_tag = match mode {
        LfgMode::Casual => LFG_FORUM_TAG_MODE_CASUAL,
        LfgMode::Ranked => LFG_FORUM_TAG_MODE_RANKED,
        LfgMode::StreetBrawl => LFG_FORUM_TAG_MODE_STREET_BRAWL,
    };
    let status_tag = if lane_attached {
        LFG_FORUM_TAG_STATUS_ACTIVE
    } else {
        LFG_FORUM_TAG_STATUS_LOOKING
    };
    let mut rank_tags = match (mode, rank_range.min, rank_range.max) {
        (LfgMode::Ranked, Some(min), Some(max)) if min <= max => LFG_FORUM_RANK_BRACKETS
            .iter()
            .filter_map(|bracket| {
                ranges_overlap(min, max, bracket.min, bracket.max).then_some(bracket.tag_id)
            })
            .collect::<Vec<_>>(),
        (LfgMode::Ranked, Some(min), None) => LFG_FORUM_RANK_BRACKETS
            .iter()
            .filter_map(|bracket| {
                ranges_overlap(min, 11, bracket.min, bracket.max).then_some(bracket.tag_id)
            })
            .collect::<Vec<_>>(),
        (LfgMode::Ranked, None, Some(max)) => LFG_FORUM_RANK_BRACKETS
            .iter()
            .filter_map(|bracket| {
                ranges_overlap(1, max, bracket.min, bracket.max).then_some(bracket.tag_id)
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    if rank_tags.is_empty() {
        rank_tags.push(LFG_FORUM_TAG_RANK_ANY);
    }

    let rank_slot_count = LFG_FORUM_MAX_APPLIED_TAGS.saturating_sub(2);
    let mut tags = Vec::with_capacity(LFG_FORUM_MAX_APPLIED_TAGS);
    tags.push(mode_tag);
    tags.extend(rank_tags.into_iter().take(rank_slot_count));
    tags.push(status_tag);
    tags.truncate(LFG_FORUM_MAX_APPLIED_TAGS);
    tags
}

fn mode_default_capacity(mode: LfgMode) -> i64 {
    match mode {
        LfgMode::Ranked => crate::tempvoice::logic::DEFAULT_RANKED_CAP,
        LfgMode::StreetBrawl => LFG_STREET_BRAWL_CAP,
        LfgMode::Casual => crate::tempvoice::logic::DEFAULT_CASUAL_CAP,
    }
}

fn mode_from_category_id(category_id: u64) -> Option<LfgMode> {
    if category_id == crate::router::ROUTER_CATEGORY_RANKED_LEGACY {
        Some(LfgMode::Ranked)
    } else if category_id == crate::router::ROUTER_CATEGORY_STREET_BRAWL_LEGACY {
        Some(LfgMode::StreetBrawl)
    } else if category_id == crate::router::ROUTER_CATEGORY_CHILL {
        Some(LfgMode::Casual)
    } else {
        None
    }
}

fn mode_from_staging_id(staging_id: u64) -> Option<LfgMode> {
    if staging_id == crate::router::mode_to_staging("ranked") {
        Some(LfgMode::Ranked)
    } else if staging_id == crate::router::mode_to_staging("street_brawl") {
        Some(LfgMode::StreetBrawl)
    } else if staging_id == crate::router::mode_to_staging("casual") {
        Some(LfgMode::Casual)
    } else {
        None
    }
}

fn mode_from_lane_base(base_name: &str) -> Option<LfgMode> {
    let lower = base_name.trim().to_ascii_lowercase();
    if lower.starts_with("ranked") {
        Some(LfgMode::Ranked)
    } else if lower.starts_with("street brawl") {
        Some(LfgMode::StreetBrawl)
    } else if lower.starts_with("chill lane") {
        Some(LfgMode::Casual)
    } else {
        None
    }
}

fn mode_from_lane_record(
    category_id: u64,
    source_staging_id: Option<u64>,
    base_name: &str,
) -> Option<LfgMode> {
    source_staging_id
        .and_then(mode_from_staging_id)
        .or_else(|| mode_from_lane_base(base_name))
        .or_else(|| mode_from_category_id(category_id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LfgRenderedCapacity {
    free: i64,
    total: i64,
    full: bool,
}

fn rendered_capacity(
    mode: LfgMode,
    requested_slots: i32,
    occupancy: Option<LfgLaneOccupancy>,
) -> LfgRenderedCapacity {
    let (occupied, total) = match occupancy {
        Some(occupancy) => {
            let total = if occupancy.user_limit > 0 {
                occupancy.user_limit
            } else {
                mode_default_capacity(mode)
            };
            (occupancy.member_count.max(0), total.max(0))
        }
        None => (0, i64::from(requested_slots).max(0)),
    };
    let free = total.saturating_sub(occupied);
    LfgRenderedCapacity {
        free,
        total,
        full: total > 0 && free == 0,
    }
}

fn lfg_post_body(
    owner_id: u64,
    mode: LfgMode,
    rank_range: LfgRankRange,
    requested_slots: i32,
    occupancy: Option<LfgLaneOccupancy>,
    play_window: Option<LfgPlayWindow>,
) -> String {
    let rank_label = rank_range_label(rank_range);
    let capacity = rendered_capacity(mode, requested_slots, occupancy);
    let mut lines = vec![
        LFG_POST_BODY_HEADER.to_string(),
        format!("{LFG_POST_BODY_VON}: <@{owner_id}>"),
        format!("{LFG_POST_BODY_MODUS}: {}", mode.display_name()),
        format!("{LFG_POST_BODY_RANG}: {rank_label}"),
    ];
    if let Some(window) = play_window {
        lines.push(format!("🕒 Wann: {} {}", window.emoji(), window.label()));
    }
    lines.extend([
        format!(
            "{LFG_POST_BODY_PLAETZE}: {}/{}",
            capacity.free, capacity.total
        ),
        if capacity.full {
            LFG_POST_STATUS_VOLL.to_string()
        } else {
            LFG_POST_STATUS_OFFEN.to_string()
        },
    ]);
    lines.join("\n")
}

fn render_hash(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[allow(clippy::too_many_arguments)]
fn lfg_post_draft(
    post_id: i64,
    owner_id: u64,
    mode: LfgMode,
    rank_range: LfgRankRange,
    requested_slots: i32,
    occupancy: Option<LfgLaneOccupancy>,
    lane_attached: bool,
    play_window: Option<LfgPlayWindow>,
) -> LfgForumPostDraft {
    let rank_label = rank_range_label(rank_range);
    let title = format!(
        "{LFG_POST_TITEL_SCHEMA} {} · {rank_label} · sucht {requested_slots}",
        mode.display_name()
    );
    let body = lfg_post_body(
        owner_id,
        mode,
        rank_range,
        requested_slots,
        occupancy,
        play_window,
    );
    let applied_tags = derive_lfg_forum_tag_ids(mode, rank_range, lane_attached);
    LfgForumPostDraft {
        post_id,
        title,
        body,
        applied_tags,
    }
}

fn discord_id_i64(id: u64, field: &str) -> Result<i64, String> {
    i64::try_from(id).map_err(|_| format!("{field} ist keine gueltige BIGINT-Discord-ID"))
}

fn db_i64_to_u64(value: i64, field: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("{field} ist keine gueltige Discord-ID"))
}

fn parse_i64_suffix(custom_id: &str, prefix: &str) -> Option<i64> {
    custom_id.strip_prefix(prefix)?.parse::<i64>().ok()
}

fn lfg_edit_delay_from_last(now: DateTime<Utc>, last: Option<DateTime<Utc>>) -> Option<Duration> {
    let last = last?;
    let elapsed = now.signed_duration_since(last);
    let min = chrono::Duration::seconds(LFG_EDIT_MIN_INTERVAL_SECONDS);
    (elapsed < min).then(|| {
        let remaining = (min - elapsed)
            .to_std()
            .unwrap_or_else(|_| Duration::from_secs(0));
        remaining.max(Duration::from_millis(100))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LfgReservationError {
    AlreadyOpen,
    Db(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LfgPostCreateError {
    AlreadyOpen,
    Failed,
}

#[derive(Debug, Clone)]
struct LfgPostRecord {
    id: i64,
    guild_id: u64,
    thread_id: Option<u64>,
    starter_message_id: Option<u64>,
    lane_id: Option<u64>,
    owner_id: u64,
    mode: LfgMode,
    rank_range: LfgRankRange,
    requested_slots: i32,
    play_window: Option<LfgPlayWindow>,
    status: String,
    last_render_hash: Option<String>,
    last_post_edit_at: Option<DateTime<Utc>>,
}

impl LfgPostRecord {
    fn body_with(&self, occupancy: Option<LfgLaneOccupancy>) -> String {
        lfg_post_body(
            self.owner_id,
            self.mode,
            self.rank_range,
            self.requested_slots,
            occupancy,
            self.play_window,
        )
    }

    fn forum_tag_ids(&self, lane_attached: bool) -> Vec<u64> {
        derive_lfg_forum_tag_ids(self.mode, self.rank_range, lane_attached)
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db_err| db_err.code())
        .is_some_and(|code| code == "23505")
}

impl LfgPanelInterface {
    async fn ranked_allowed(
        &self,
        interaction: &BridgeInteraction,
        guild_id: u64,
        mode: LfgMode,
    ) -> bool {
        let _ = (interaction, guild_id, mode);
        true
    }

    async fn handle_start(&self) -> BridgeReply {
        BridgeReply {
            content: Some(LFG_MODE_PROMPT.to_string()),
            components: Some(lfg_mode_selection_components()),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    async fn handle_watch_start(&self, interaction: BridgeInteraction) -> BridgeReply {
        let draft = LfgWatchDraft::default();
        self.pending_watch_drafts
            .lock()
            .await
            .insert(interaction.user_id, draft.clone());
        lfg_watch_reply(&draft, false)
    }

    async fn handle_watch_select(
        &self,
        interaction: BridgeInteraction,
        selected_mode: Option<LfgMode>,
    ) -> BridgeReply {
        let guild_id = if interaction.guild_id == 0 {
            LFG_GUILD_ID
        } else {
            interaction.guild_id
        };
        if let Some(mode) = selected_mode {
            if !self.ranked_allowed(&interaction, guild_id, mode).await {
                return BridgeReply::ephemeral_text(LFG_ERR_KEIN_RANKED_RANG);
            }
            let mut drafts = self.pending_watch_drafts.lock().await;
            let Some(draft) = drafts.get_mut(&interaction.user_id) else {
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            };
            draft.mode = Some(mode);
            let updated = draft.clone();
            drop(drafts);
            return lfg_watch_reply(&updated, true);
        }

        let Some(selected) = interaction.values.first().map(String::as_str) else {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        };
        let mut drafts = self.pending_watch_drafts.lock().await;
        let Some(draft) = drafts.get_mut(&interaction.user_id) else {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        };
        match interaction.custom_id.as_str() {
            LFG_WATCH_RANK_FROM_SELECT_CUSTOM_ID => {
                let Some(rank) = normalize_lfg_rank_value(selected) else {
                    return BridgeReply::ephemeral_text(LFG_ERR_RANG_UNBEKANNT);
                };
                draft.rank_from = rank;
            }
            LFG_WATCH_RANK_TO_SELECT_CUSTOM_ID => {
                let Some(rank) = normalize_lfg_rank_value(selected) else {
                    return BridgeReply::ephemeral_text(LFG_ERR_RANG_UNBEKANNT);
                };
                draft.rank_to = rank;
            }
            LFG_WATCH_WINDOW_SELECT_CUSTOM_ID => {
                let Some(window) = LfgWatchWindow::from_str(selected) else {
                    return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
                };
                draft.window = Some(window);
            }
            _ => {
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            }
        }
        let updated = draft.clone();
        drop(drafts);
        lfg_watch_reply(&updated, true)
    }

    async fn handle_watch_activate(&self, interaction: BridgeInteraction) -> BridgeReply {
        let draft = {
            let drafts = self.pending_watch_drafts.lock().await;
            let Some(draft) = drafts.get(&interaction.user_id).cloned() else {
                return BridgeReply::ephemeral_text(LFG_WATCH_ERR_UNVOLLSTAENDIG);
            };
            draft
        };
        let (Some(mode), Some(window)) = (draft.mode, draft.window) else {
            return BridgeReply::ephemeral_text(LFG_WATCH_ERR_UNVOLLSTAENDIG);
        };
        let guild_id = if interaction.guild_id == 0 {
            LFG_GUILD_ID
        } else {
            interaction.guild_id
        };
        if !self.ranked_allowed(&interaction, guild_id, mode).await {
            return BridgeReply::ephemeral_text(LFG_ERR_KEIN_RANKED_RANG);
        }
        let Some(range) = lfg_rank_range_from_watch_draft(&draft) else {
            return BridgeReply::ephemeral_text(LFG_ERR_RANG_UNBEKANNT);
        };
        let expires_at = match lfg_watch_expires_at(&self.pool, window).await {
            Ok(expires_at) => expires_at,
            Err(err) => {
                tracing::error!(%err, user_id = interaction.user_id, "LFG-Watch: expires_at konnte nicht berechnet werden");
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            }
        };
        if let Err(err) = insert_or_replace_watch(
            &self.pool,
            guild_id,
            interaction.user_id,
            mode.as_str(),
            range,
            window,
            expires_at,
        )
        .await
        {
            tracing::error!(%err, user_id = interaction.user_id, "LFG-Watch konnte nicht gespeichert werden");
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        }
        self.pending_watch_drafts
            .lock()
            .await
            .remove(&interaction.user_id);
        BridgeReply {
            content: Some(format!(
                "🔔 Alles klar — ich halte Ausschau nach **{}** · {} und schicke dir eine DM, sobald was Passendes auftaucht. (Einmalig; danach kannst du dich wieder eintragen.)",
                mode.display_name(),
                rank_range_label(range)
            )),
            update_message: true,
            ..BridgeReply::default()
        }
    }

    async fn load_user_preset_draft(&self, user_id: u64, mode: LfgMode) -> LfgDraft {
        let key = user_id.to_string();
        let raw = match kv::get(&self.pool, LFG_USER_PREF_KV_NS, &key).await {
            Ok(Some(raw)) => raw,
            Ok(None) => return LfgDraft::new(mode, None),
            Err(err) => {
                tracing::warn!(%err, user_id, "LFG-Preset konnte nicht geladen werden");
                return LfgDraft::new(mode, None);
            }
        };
        match serde_json::from_str::<LfgUserPreset>(&raw)
            .ok()
            .and_then(|preset| preset.into_draft(mode))
        {
            Some(draft) => draft,
            None => {
                tracing::warn!(user_id, "LFG-Preset konnte nicht geparst werden");
                LfgDraft::new(mode, None)
            }
        }
    }

    async fn save_user_preset(&self, user_id: u64, draft: &LfgDraft) {
        let preset = LfgUserPreset::from_draft(draft);
        let value = match serde_json::to_string(&preset) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(%err, user_id, "LFG-Preset konnte nicht serialisiert werden");
                return;
            }
        };
        if let Err(err) = kv::set(
            &self.pool,
            LFG_USER_PREF_KV_NS,
            &user_id.to_string(),
            &value,
        )
        .await
        {
            tracing::warn!(%err, user_id, "LFG-Preset konnte nicht gespeichert werden");
        }
    }

    async fn handle_mode(&self, interaction: BridgeInteraction, mode: LfgMode) -> BridgeReply {
        let guild_id = if interaction.guild_id == 0 {
            LFG_GUILD_ID
        } else {
            interaction.guild_id
        };
        if !self.ranked_allowed(&interaction, guild_id, mode).await {
            return BridgeReply::ephemeral_text(LFG_ERR_KEIN_RANKED_RANG);
        }
        let draft = self.load_user_preset_draft(interaction.user_id, mode).await;
        self.pending_drafts
            .lock()
            .await
            .insert(interaction.user_id, draft.clone());
        lfg_draft_reply(&draft, true)
    }

    async fn handle_draft_select(&self, interaction: BridgeInteraction) -> BridgeReply {
        let Some(selected) = interaction.values.first().map(String::as_str) else {
            return match interaction.custom_id.as_str() {
                LFG_RANK_FROM_SELECT_CUSTOM_ID | LFG_RANK_TO_SELECT_CUSTOM_ID => {
                    BridgeReply::ephemeral_text(LFG_ERR_RANG_UNBEKANNT)
                }
                LFG_SLOTS_SELECT_CUSTOM_ID => {
                    BridgeReply::ephemeral_text(LFG_ERR_PLAETZE_UNGUELTIG)
                }
                LFG_WHEN_SELECT_CUSTOM_ID => {
                    BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN)
                }
                _ => BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN),
            };
        };
        let mut drafts = self.pending_drafts.lock().await;
        let Some(draft) = drafts.get_mut(&interaction.user_id) else {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        };
        match interaction.custom_id.as_str() {
            LFG_RANK_FROM_SELECT_CUSTOM_ID => {
                let Some(rank) = normalize_lfg_rank_value(selected) else {
                    return BridgeReply::ephemeral_text(LFG_ERR_RANG_UNBEKANNT);
                };
                draft.rank_from = rank;
            }
            LFG_RANK_TO_SELECT_CUSTOM_ID => {
                let Some(rank) = normalize_lfg_rank_value(selected) else {
                    return BridgeReply::ephemeral_text(LFG_ERR_RANG_UNBEKANNT);
                };
                draft.rank_to = rank;
            }
            LFG_SLOTS_SELECT_CUSTOM_ID => {
                let Some(slots) = parse_requested_slots(selected, draft.mode) else {
                    return BridgeReply::ephemeral_text(LFG_ERR_PLAETZE_UNGUELTIG);
                };
                draft.slots = Some(slots);
            }
            LFG_WHEN_SELECT_CUSTOM_ID => {
                draft.play_window = LfgPlayWindow::from_str(selected);
            }
            _ => {
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            }
        }
        let updated = draft.clone();
        drop(drafts);
        lfg_draft_reply(&updated, true)
    }

    async fn handle_post_draft(
        &self,
        interaction: BridgeInteraction,
        mode: LfgMode,
    ) -> BridgeReply {
        let draft = {
            let drafts = self.pending_drafts.lock().await;
            let Some(draft) = drafts.get(&interaction.user_id).cloned() else {
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            };
            draft
        };
        if draft.mode != mode {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        }
        let Some(requested_slots) = draft.slots else {
            return BridgeReply::ephemeral_text(LFG_ERR_PLAETZE_UNGUELTIG);
        };
        let Some(rank_range) = lfg_rank_range_from_draft(&draft) else {
            return BridgeReply::ephemeral_text(LFG_ERR_RANG_UNBEKANNT);
        };
        let guild_id = if interaction.guild_id == 0 {
            LFG_GUILD_ID
        } else {
            interaction.guild_id
        };
        if !self.ranked_allowed(&interaction, guild_id, mode).await {
            return BridgeReply::ephemeral_text(LFG_ERR_KEIN_RANKED_RANG);
        }
        let owner_id = if let Some(lane_id) = draft.lane_id {
            let Ok(Some((owner_id, current_mode))) = self.lane_owner_and_mode(lane_id).await else {
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            };
            if current_mode != mode {
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            }
            if owner_id != interaction.user_id && !interaction.author_can_manage_channels {
                return BridgeReply::ephemeral_text(LFG_ERR_OPEN_NICHT_DEIN_POST);
            }
            owner_id
        } else {
            interaction.user_id
        };
        let post_id = match self
            .create_lfg_post_from_values(
                guild_id,
                owner_id,
                mode,
                rank_range,
                requested_slots,
                draft.lane_id,
                draft.play_window,
            )
            .await
        {
            Ok(post_id) => post_id,
            Err(LfgPostCreateError::AlreadyOpen) if draft.lane_id.is_some() => {
                return BridgeReply::ephemeral_text(LFG_ERR_PUBLISH_LANE_SCHON_VEROEFFENTLICHT);
            }
            Err(LfgPostCreateError::AlreadyOpen) => {
                return BridgeReply::ephemeral_text(LFG_ERR_SCHON_AKTIVE_SUCHE);
            }
            Err(LfgPostCreateError::Failed) => {
                return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
            }
        };
        if draft.lane_id.is_none() {
            self.save_user_preset(interaction.user_id, &draft).await;
        }
        self.pending_drafts
            .lock()
            .await
            .remove(&interaction.user_id);
        BridgeReply {
            content: Some(LFG_ERFOLG_POST_ERSTELLT.to_string()),
            components: draft
                .lane_id
                .is_none()
                .then(|| lfg_success_components(post_id)),
            update_message: true,
            ..BridgeReply::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn reserve_lfg_post(
        &self,
        guild_id: u64,
        forum_channel_id: u64,
        owner_id: u64,
        mode: LfgMode,
        rank_range: LfgRankRange,
        requested_slots: i32,
        lane_id: Option<u64>,
        play_window: Option<LfgPlayWindow>,
    ) -> Result<i64, LfgReservationError> {
        let post_id: i64 = sqlx::query_scalar(
            "INSERT INTO voice.lfg_posts (
                 guild_id,
                 forum_channel_id,
                 thread_id,
                 starter_message_id,
                 lane_id,
                 owner_id,
                 mode,
                 rank_min,
                 rank_max,
                 requested_slots,
                 status,
                 created_at,
                 updated_at,
                 expires_at,
                 closed_at,
                 last_render_hash,
                 last_post_edit_at,
                 play_window
             )
             VALUES (
                 $1, $2, NULL, NULL, $8, $3, $4, $5, $6, $7, 'creating',
                 now(), now(), now() + INTERVAL '24 hours', NULL, NULL, NULL, $9
             )
             RETURNING id",
        )
        .bind(discord_id_i64(guild_id, "guild_id").map_err(LfgReservationError::Db)?)
        .bind(
            discord_id_i64(forum_channel_id, "forum_channel_id")
                .map_err(LfgReservationError::Db)?,
        )
        .bind(discord_id_i64(owner_id, "owner_id").map_err(LfgReservationError::Db)?)
        .bind(mode.as_str())
        .bind(rank_range.min)
        .bind(rank_range.max)
        .bind(requested_slots)
        .bind(
            lane_id
                .map(|id| discord_id_i64(id, "lane_id"))
                .transpose()
                .map_err(LfgReservationError::Db)?,
        )
        .bind(play_window.map(LfgPlayWindow::as_str))
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                LfgReservationError::AlreadyOpen
            } else {
                LfgReservationError::Db(err.to_string())
            }
        })?;
        Ok(post_id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_lfg_post_from_values(
        &self,
        guild_id: u64,
        owner_id: u64,
        mode: LfgMode,
        rank_range: LfgRankRange,
        requested_slots: i32,
        lane_id: Option<u64>,
        play_window: Option<LfgPlayWindow>,
    ) -> Result<i64, LfgPostCreateError> {
        let Some(forum_channel_id) = self.forum_channel_id else {
            return Err(LfgPostCreateError::Failed);
        };
        let post_id = match self
            .reserve_lfg_post(
                guild_id,
                forum_channel_id,
                owner_id,
                mode,
                rank_range,
                requested_slots,
                lane_id,
                play_window,
            )
            .await
        {
            Ok(post_id) => post_id,
            Err(LfgReservationError::AlreadyOpen) => return Err(LfgPostCreateError::AlreadyOpen),
            Err(LfgReservationError::Db(err)) => {
                tracing::error!(
                    %err,
                    owner_id,
                    ?lane_id,
                    "LFG-Reservation konnte nicht erstellt werden"
                );
                return Err(LfgPostCreateError::Failed);
            }
        };
        let occupancy = match lane_id {
            Some(lane_id) => self.port.lane_occupancy(guild_id, lane_id).await,
            None => None,
        };
        let draft = lfg_post_draft(
            post_id,
            owner_id,
            mode,
            rank_range,
            requested_slots,
            occupancy,
            lane_id.is_some(),
            play_window,
        );
        let render_hash = render_hash(&draft.body);
        let created = match self.port.create_forum_post(forum_channel_id, draft).await {
            Ok(created) => created,
            Err(err) => {
                tracing::error!(
                    %err,
                    owner_id,
                    ?lane_id,
                    "LFG-Forum-Post konnte nach DB-Reservation nicht erstellt werden"
                );
                self.delete_lfg_reservation(post_id).await;
                return Err(LfgPostCreateError::Failed);
            }
        };
        let starter_message_id = match self.port.first_thread_message_id(created.thread_id).await {
            Ok(message_id) => message_id,
            Err(err) => {
                tracing::warn!(
                    %err,
                    thread_id = created.thread_id,
                    "LFG-Starter-Message konnte nicht kontrolliert gefetcht werden"
                );
                None
            }
        };
        if let Err(err) = self
            .open_lfg_post_reservation(post_id, created.thread_id, starter_message_id, &render_hash)
            .await
        {
            tracing::error!(
                %err,
                owner_id,
                ?lane_id,
                thread_id = created.thread_id,
                "LFG-Forum-Post konnte nach Discord-Erstellung nicht persistiert werden"
            );
            self.delete_lfg_reservation(post_id).await;
            if let Err(cleanup_err) = self.port.archive_and_lock_thread(created.thread_id).await {
                tracing::error!(
                    %cleanup_err,
                    owner_id,
                    ?lane_id,
                    thread_id = created.thread_id,
                    "LFG-Forum-Thread konnte nach Persistenzfehler nicht archiviert/gelockt werden"
                );
            }
            return Err(LfgPostCreateError::Failed);
        }
        {
            let pool = self.pool.clone();
            let port = self.port.clone();
            let thread_id = created.thread_id;
            let mode_str = mode.as_str().to_string();
            let mode_display = mode.display_name().to_string();
            let rank_label = rank_range_label(rank_range);
            let post_window = play_window.map(|window| window.as_str().to_string());
            tokio::spawn(async move {
                crate::lfg_watch::run_match_for_new_post(
                    &pool,
                    port.as_ref(),
                    guild_id,
                    post_id,
                    thread_id,
                    owner_id,
                    &mode_str,
                    &mode_display,
                    rank_range,
                    post_window,
                    rank_label,
                )
                .await;
            });
        }
        Ok(post_id)
    }

    async fn open_lfg_post_reservation(
        &self,
        post_id: i64,
        thread_id: u64,
        starter_message_id: Option<u64>,
        render_hash: &str,
    ) -> Result<(), String> {
        let result = sqlx::query(
            "UPDATE voice.lfg_posts
                SET thread_id = $1,
                    starter_message_id = $2,
                    status = 'open',
                    last_render_hash = $4,
                    updated_at = now()
              WHERE id = $3
                AND status = 'creating'
                AND thread_id IS NULL",
        )
        .bind(discord_id_i64(thread_id, "thread_id")?)
        .bind(
            starter_message_id
                .map(|id| discord_id_i64(id, "starter_message_id"))
                .transpose()?,
        )
        .bind(post_id)
        .bind(render_hash)
        .execute(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        if result.rows_affected() != 1 {
            return Err(format!(
                "LFG-Reservation nicht eindeutig aktualisiert: rows_affected={}",
                result.rows_affected()
            ));
        }
        Ok(())
    }

    async fn delete_lfg_reservation(&self, post_id: i64) {
        if let Err(err) = sqlx::query(
            "DELETE FROM voice.lfg_posts
              WHERE id = $1
                AND status = 'creating'
                AND thread_id IS NULL",
        )
        .bind(post_id)
        .execute(&self.pool)
        .await
        {
            tracing::error!(
                %err,
                post_id,
                "LFG-Reservation-Cleanup konnte Reservation-Row nicht loeschen"
            );
        }
    }

    async fn load_lfg_post(&self, post_id: i64) -> Result<Option<LfgPostRecord>, String> {
        let Some(row) = sqlx::query(
            "SELECT id,
                    guild_id,
                    forum_channel_id,
                    thread_id,
                    starter_message_id,
                    lane_id,
                    owner_id,
                    mode,
                    rank_min,
                    rank_max,
                    requested_slots,
                    play_window,
                    status,
                    last_render_hash,
                    last_post_edit_at
               FROM voice.lfg_posts
              WHERE id = $1",
        )
        .bind(post_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| err.to_string())?
        else {
            return Ok(None);
        };
        let mode_raw: String = row.try_get("mode").map_err(|err| err.to_string())?;
        let Some(mode) = LfgMode::from_str(&mode_raw) else {
            return Err(format!("unbekannter LFG-Modus `{mode_raw}`"));
        };
        let play_window_raw: Option<String> =
            row.try_get("play_window").map_err(|err| err.to_string())?;
        let play_window = match play_window_raw.as_deref() {
            Some(raw) => match LfgPlayWindow::from_str(raw) {
                Some(window) => Some(window),
                None => {
                    tracing::warn!(post_id, raw, "unbekanntes LFG-play_window ignoriert");
                    None
                }
            },
            None => None,
        };
        Ok(Some(LfgPostRecord {
            id: row.try_get("id").map_err(|err| err.to_string())?,
            guild_id: db_i64_to_u64(
                row.try_get("guild_id").map_err(|err| err.to_string())?,
                "lfg_posts.guild_id",
            )?,
            thread_id: row
                .try_get::<Option<i64>, _>("thread_id")
                .map_err(|err| err.to_string())?
                .map(|id| db_i64_to_u64(id, "lfg_posts.thread_id"))
                .transpose()?,
            starter_message_id: row
                .try_get::<Option<i64>, _>("starter_message_id")
                .map_err(|err| err.to_string())?
                .map(|id| db_i64_to_u64(id, "lfg_posts.starter_message_id"))
                .transpose()?,
            lane_id: row
                .try_get::<Option<i64>, _>("lane_id")
                .map_err(|err| err.to_string())?
                .map(|id| db_i64_to_u64(id, "lfg_posts.lane_id"))
                .transpose()?,
            owner_id: db_i64_to_u64(
                row.try_get("owner_id").map_err(|err| err.to_string())?,
                "lfg_posts.owner_id",
            )?,
            mode,
            rank_range: LfgRankRange {
                min: row.try_get("rank_min").map_err(|err| err.to_string())?,
                max: row.try_get("rank_max").map_err(|err| err.to_string())?,
            },
            requested_slots: row
                .try_get("requested_slots")
                .map_err(|err| err.to_string())?,
            play_window,
            status: row.try_get("status").map_err(|err| err.to_string())?,
            last_render_hash: row
                .try_get("last_render_hash")
                .map_err(|err| err.to_string())?,
            last_post_edit_at: row
                .try_get("last_post_edit_at")
                .map_err(|err| err.to_string())?,
        }))
    }

    async fn update_starter_message_id(
        &self,
        post_id: i64,
        starter_message_id: u64,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE voice.lfg_posts
                SET starter_message_id = COALESCE(starter_message_id, $2),
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(post_id)
        .bind(discord_id_i64(starter_message_id, "starter_message_id")?)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
    }

    async fn post_ids_for_lane(&self, lane_id: u64) -> Result<Vec<i64>, String> {
        let rows = sqlx::query_scalar::<_, i64>(
            "SELECT id
               FROM voice.lfg_posts
              WHERE status = 'open'
                AND lane_id = $1",
        )
        .bind(discord_id_i64(lane_id, "lane_id")?)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(rows)
    }

    async fn link_post_lane(&self, post_id: i64, lane_id: u64) -> Result<bool, String> {
        let result = sqlx::query(
            "UPDATE voice.lfg_posts
                SET lane_id = $2,
                    updated_at = now()
              WHERE id = $1
                AND status = 'open'
                AND lane_id IS NULL",
        )
        .bind(post_id)
        .bind(discord_id_i64(lane_id, "lane_id")?)
        .execute(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(result.rows_affected() == 1)
    }

    async fn lane_mode(&self, guild_id: u64, lane_id: u64) -> Option<LfgMode> {
        let row = sqlx::query(
            "SELECT category_id, base_name, source_staging_id
               FROM voice.tempvoice_lanes
              WHERE channel_id = $1",
        )
        .bind(discord_id_i64(lane_id, "lane_id").ok()?)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        if let Some(row) = row {
            let category_id = row
                .try_get("category_id")
                .ok()
                .and_then(|id| db_i64_to_u64(id, "tempvoice_lanes.category_id").ok())?;
            let base_name: String = row.try_get("base_name").ok()?;
            let source_staging_id: Option<i64> = row.try_get("source_staging_id").ok()?;
            let source_staging_id = source_staging_id
                .and_then(|id| db_i64_to_u64(id, "tempvoice_lanes.source_staging_id").ok());
            return mode_from_lane_record(category_id, source_staging_id, &base_name);
        }
        self.port
            .channel_category(guild_id, lane_id)
            .await
            .and_then(mode_from_category_id)
    }

    async fn lane_owner_and_mode(&self, lane_id: u64) -> Result<Option<(u64, LfgMode)>, String> {
        let Some(row) = sqlx::query(
            "SELECT guild_id, owner_id, category_id, base_name, source_staging_id
               FROM voice.tempvoice_lanes
              WHERE channel_id = $1",
        )
        .bind(discord_id_i64(lane_id, "lane_id")?)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| err.to_string())?
        else {
            return Ok(None);
        };
        let owner_id = db_i64_to_u64(
            row.try_get("owner_id").map_err(|err| err.to_string())?,
            "tempvoice_lanes.owner_id",
        )?;
        let guild_id = db_i64_to_u64(
            row.try_get("guild_id").map_err(|err| err.to_string())?,
            "tempvoice_lanes.guild_id",
        )?;
        let category_id = db_i64_to_u64(
            row.try_get("category_id").map_err(|err| err.to_string())?,
            "tempvoice_lanes.category_id",
        )?;
        let base_name: String = row.try_get("base_name").map_err(|err| err.to_string())?;
        let source_staging_id: Option<i64> = row
            .try_get("source_staging_id")
            .map_err(|err| err.to_string())?;
        let source_staging_id = source_staging_id
            .map(|id| db_i64_to_u64(id, "tempvoice_lanes.source_staging_id"))
            .transpose()?;
        let mode = match mode_from_lane_record(category_id, source_staging_id, &base_name) {
            Some(mode) => Some(mode),
            None => self
                .port
                .channel_category(guild_id, lane_id)
                .await
                .and_then(mode_from_category_id),
        };
        Ok(mode.map(|mode| (owner_id, mode)))
    }

    pub async fn handle_publish_lane_start(
        &self,
        interaction: BridgeInteraction,
        lane_id: u64,
    ) -> BridgeReply {
        if !self.cutover_active {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        }
        let guild_id = if interaction.guild_id == 0 {
            LFG_GUILD_ID
        } else {
            interaction.guild_id
        };
        let Some(mode) = self.lane_mode(guild_id, lane_id).await else {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        };
        if !self.ranked_allowed(&interaction, guild_id, mode).await {
            return BridgeReply::ephemeral_text(LFG_ERR_KEIN_RANKED_RANG);
        }
        let draft = LfgDraft::new(mode, Some(lane_id));
        self.pending_drafts
            .lock()
            .await
            .insert(interaction.user_id, draft.clone());
        lfg_draft_reply(&draft, false)
    }

    async fn handle_open_lane(&self, interaction: BridgeInteraction, post_id: i64) -> BridgeReply {
        let Ok(Some(post)) = self.load_lfg_post(post_id).await else {
            return BridgeReply::ephemeral_text(LFG_ERR_OPEN_NICHT_DEIN_POST);
        };
        if post.status != "open" || post.owner_id != interaction.user_id {
            return BridgeReply::ephemeral_text(LFG_ERR_OPEN_NICHT_DEIN_POST);
        }
        if post.lane_id.is_some() {
            return BridgeReply::ephemeral_text(LFG_ERR_OPEN_LANE_SCHON_VERKNUEPFT);
        }
        let Some(spawner) = self.lane_spawner.read().await.clone() else {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        };
        match spawner
            .spawn_lane_from_current_voice(
                post.guild_id,
                interaction.user_id,
                post.mode.as_str(),
                &interaction.role_ids,
            )
            .await
        {
            crate::router::RouterSpawnOutcome::Created { lane_id } => {
                match self.link_post_lane(post_id, lane_id).await {
                    Ok(true) => {
                        if let Some(thread_id) = post.thread_id {
                            if let Err(err) = self
                                .port
                                .edit_forum_post_tags(thread_id, post.forum_tag_ids(true))
                                .await
                            {
                                tracing::warn!(
                                    %err,
                                    post_id,
                                    thread_id,
                                    "LFG-Forum-Tags konnten nach Lane-Open nicht aktualisiert werden"
                                );
                            }
                        }
                        self.enqueue_render(post_id).await;
                        BridgeReply::ephemeral_text(LFG_ERFOLG_LANE_AUFGEMACHT)
                    }
                    Ok(false) => {
                        self.cleanup_created_lane(spawner.as_ref(), post.guild_id, lane_id)
                            .await;
                        BridgeReply::ephemeral_text(LFG_ERR_OPEN_LANE_SCHON_VERKNUEPFT)
                    }
                    Err(err) => {
                        tracing::error!(%err, post_id, lane_id, "LFG-Post konnte nicht mit Lane verknuepft werden");
                        self.cleanup_created_lane(spawner.as_ref(), post.guild_id, lane_id)
                            .await;
                        BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN)
                    }
                }
            }
            crate::router::RouterSpawnOutcome::NotInVoice => {
                BridgeReply::ephemeral_text(LFG_ERR_OPEN_NICHT_IN_VOICE)
            }
            crate::router::RouterSpawnOutcome::AlreadyOwnLane { .. } => {
                BridgeReply::ephemeral_text(LFG_ERR_OPEN_LANE_SCHON_VERKNUEPFT)
            }
            crate::router::RouterSpawnOutcome::FloodLimited
            | crate::router::RouterSpawnOutcome::NotCreated
            | crate::router::RouterSpawnOutcome::UnknownMode => {
                BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN)
            }
        }
    }

    async fn cleanup_created_lane(
        &self,
        spawner: &dyn LfgLaneSpawner,
        guild_id: u64,
        lane_id: u64,
    ) {
        if let Err(err) = spawner
            .cleanup_created_lane(guild_id, lane_id, "LFG: Post-Lane-Link fehlgeschlagen")
            .await
        {
            tracing::warn!(%err, lane_id, "LFG: frisch erstellte Lane konnte nach Link-Fehler nicht bereinigt werden");
        }
    }

    async fn handle_join(&self, interaction: BridgeInteraction, post_id: i64) -> BridgeReply {
        let Ok(Some(post)) = self.load_lfg_post(post_id).await else {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_LANE_TOT);
        };
        if post.status != "open" {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_LANE_TOT);
        }
        if post.owner_id == interaction.user_id {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_EIGENER_POST);
        }
        let Some(lane_id) = post.lane_id else {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_LANE_TOT);
        };
        let Some(current_voice_channel) = self
            .port
            .member_voice_channel(post.guild_id, interaction.user_id)
            .await
        else {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_NICHT_IN_VOICE);
        };
        if current_voice_channel == lane_id {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_SCHON_DRIN);
        };
        let Some(occupancy) = self.port.lane_occupancy(post.guild_id, lane_id).await else {
            let _ = self.close_post(post.id, "closed").await;
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_LANE_TOT);
        };
        let capacity = rendered_capacity(post.mode, post.requested_slots, Some(occupancy));
        if capacity.full {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_LANE_VOLL);
        }
        if !self
            .ranked_allowed(&interaction, post.guild_id, post.mode)
            .await
        {
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_KEIN_RANG);
        }
        if let Err(err) = self
            .port
            .move_member(post.guild_id, interaction.user_id, lane_id)
            .await
        {
            tracing::warn!(%err, post_id, lane_id, user_id = interaction.user_id, "LFG-Join-Move fehlgeschlagen");
            return BridgeReply::ephemeral_text(LFG_ERR_JOIN_MOVE_FEHLGESCHLAGEN);
        }
        self.enqueue_render(post_id).await;
        BridgeReply::ephemeral_text(LFG_ERFOLG_JOIN_MOVED)
    }

    pub async fn enqueue_render(&self, post_id: i64) {
        self.edit_queue.pending.lock().await.insert(post_id);
        self.edit_queue.notify.notify_one();
    }

    pub async fn enqueue_open_lane_posts_for_initial_render(&self) {
        if !self.cutover_active {
            return;
        }
        let post_ids = sqlx::query_scalar::<_, i64>(
            "SELECT id
               FROM voice.lfg_posts
              WHERE status = 'open'
                AND lane_id IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await;
        match post_ids {
            Ok(post_ids) => {
                for post_id in post_ids {
                    self.enqueue_render(post_id).await;
                }
            }
            Err(err) => tracing::warn!(%err, "LFG-Edit-Queue: Initial-Enqueue fehlgeschlagen"),
        }
    }

    async fn pop_pending_render(&self) -> Option<i64> {
        let mut pending = self.edit_queue.pending.lock().await;
        let post_id = *pending.iter().next()?;
        pending.remove(&post_id);
        Some(post_id)
    }

    async fn render_update_once_inner(&self, post_id: i64) -> Result<bool, LfgEditError> {
        let Some(post) = self
            .load_lfg_post(post_id)
            .await
            .map_err(LfgEditError::Other)?
        else {
            return Ok(false);
        };
        if post.status != "open" {
            return Ok(false);
        }
        let Some(thread_id) = post.thread_id else {
            return Ok(false);
        };
        let occupancy = match post.lane_id {
            Some(lane_id) => match self.port.lane_occupancy(post.guild_id, lane_id).await {
                Some(occupancy) => Some(occupancy),
                None => return Ok(false),
            },
            None => None,
        };
        let body = post.body_with(occupancy);
        let body_hash = render_hash(&body);
        if post.last_render_hash.as_deref() == Some(body_hash.as_str()) {
            return Ok(false);
        }
        let starter_message_id = match post.starter_message_id {
            Some(message_id) => message_id,
            None => {
                let message_id = self
                    .port
                    .first_thread_message_id(thread_id)
                    .await
                    .map_err(LfgEditError::Other)?
                    .ok_or_else(|| LfgEditError::Other("LFG-Starter-Message fehlt".to_string()))?;
                self.update_starter_message_id(post.id, message_id)
                    .await
                    .map_err(LfgEditError::Other)?;
                message_id
            }
        };
        self.port
            .edit_forum_starter_message(thread_id, starter_message_id, body)
            .await?;
        self.port
            .edit_forum_post_tags(thread_id, post.forum_tag_ids(post.lane_id.is_some()))
            .await?;
        sqlx::query(
            "UPDATE voice.lfg_posts
                SET last_render_hash = $2,
                    last_post_edit_at = now(),
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(post.id)
        .bind(body_hash)
        .execute(&self.pool)
        .await
        .map_err(|err| LfgEditError::Other(err.to_string()))?;
        Ok(true)
    }

    pub async fn render_update_once(&self, post_id: i64) -> Result<bool, String> {
        self.render_update_once_inner(post_id)
            .await
            .map_err(|err| err.to_string())
    }

    async fn edit_worker(self: Arc<Self>) {
        loop {
            let Some(post_id) = self.pop_pending_render().await else {
                self.edit_queue.notify.notified().await;
                continue;
            };
            let last_edit = self
                .load_lfg_post(post_id)
                .await
                .ok()
                .flatten()
                .and_then(|post| post.last_post_edit_at);
            if let Some(delay) = lfg_edit_delay_from_last(Utc::now(), last_edit) {
                tokio::time::sleep(delay).await;
            }
            match self.render_update_once_inner(post_id).await {
                Ok(_) => {}
                Err(err) => {
                    if let Some(retry_after) = err.retry_after_seconds() {
                        self.enqueue_render(post_id).await;
                        tokio::time::sleep(Duration::from_secs_f64(retry_after.max(0.0))).await;
                    } else {
                        tracing::warn!(%err, post_id, "LFG-Render-Update fehlgeschlagen");
                    }
                }
            }
        }
    }

    pub async fn handle_voice_event(&self, event: VoiceEvent) {
        if !self.cutover_active {
            return;
        }
        let mut lanes = HashSet::new();
        match event {
            VoiceEvent::Join { channel_id, .. } | VoiceEvent::Leave { channel_id, .. } => {
                lanes.insert(channel_id);
            }
            VoiceEvent::Move {
                from_channel_id,
                to_channel_id,
                ..
            } => {
                lanes.insert(from_channel_id);
                lanes.insert(to_channel_id);
            }
            VoiceEvent::Update { .. } => {}
        }
        for lane_id in lanes {
            match self.post_ids_for_lane(lane_id).await {
                Ok(post_ids) => {
                    for post_id in post_ids {
                        self.enqueue_render(post_id).await;
                    }
                }
                Err(err) => tracing::warn!(%err, lane_id, "LFG-VoiceEvent-Mapping fehlgeschlagen"),
            }
        }
    }

    pub async fn close_post(&self, post_id: i64, status: &str) -> Result<(), String> {
        let Some(post) = self.load_lfg_post(post_id).await? else {
            return Ok(());
        };
        if post.status != "open" {
            return Ok(());
        }
        if let Some(thread_id) = post.thread_id {
            if let Err(err) = self
                .port
                .edit_forum_post_tags(thread_id, post.forum_tag_ids(false))
                .await
            {
                tracing::warn!(
                    %err,
                    post_id,
                    thread_id,
                    "LFG-Forum-Tags konnten beim Schliessen nicht aktualisiert werden"
                );
            }
            match self.port.archive_and_lock_thread(thread_id).await {
                Ok(()) => {}
                Err(err) if is_not_found_error(&err) => {
                    tracing::debug!(%err, post_id, thread_id, "LFG-Thread bereits weg; DB-Close wird fortgesetzt");
                }
                Err(err) => {
                    tracing::warn!(%err, post_id, thread_id, "LFG-Thread konnte nicht archiviert/gelockt werden; DB-Close wird fortgesetzt");
                }
            }
        }
        sqlx::query(
            "UPDATE voice.lfg_posts
                SET status = $2,
                    closed_at = now(),
                    updated_at = now()
              WHERE id = $1
                AND status = 'open'",
        )
        .bind(post_id)
        .bind(status)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
    }

    pub async fn on_lane_deleted(&self, lane_id: u64) {
        if !self.cutover_active {
            return;
        }
        match self.post_ids_for_lane(lane_id).await {
            Ok(post_ids) => {
                for post_id in post_ids {
                    if let Err(err) = self.close_post(post_id, "closed").await {
                        tracing::warn!(%err, post_id, lane_id, "LFG-Post konnte nach Lane-Delete nicht geschlossen werden");
                    }
                }
            }
            Err(err) => tracing::warn!(%err, lane_id, "LFG-Lane-Delete-Mapping fehlgeschlagen"),
        }
    }

    pub async fn reconcile_once(&self) {
        if !self.cutover_active {
            return;
        }
        if let Err(err) = sqlx::query(
            "DELETE FROM voice.lfg_posts
              WHERE status = 'creating'
                AND created_at < now() - ($1::text || ' minutes')::interval",
        )
        .bind(LFG_CREATING_STALE_MINUTES.to_string())
        .execute(&self.pool)
        .await
        {
            tracing::warn!(%err, "LFG-Reconcile: stale creating cleanup fehlgeschlagen");
        }

        let dead_lane_posts = sqlx::query_scalar::<_, i64>(
            "SELECT p.id
               FROM voice.lfg_posts p
               LEFT JOIN voice.tempvoice_lanes l ON l.channel_id = p.lane_id
              WHERE p.status = 'open'
                AND p.lane_id IS NOT NULL
                AND l.channel_id IS NULL",
        )
        .fetch_all(&self.pool)
        .await;
        match dead_lane_posts {
            Ok(post_ids) => {
                for post_id in post_ids {
                    if let Err(err) = self.close_post(post_id, "closed").await {
                        tracing::warn!(%err, post_id, "LFG-Reconcile: Lane-toter Post konnte nicht geschlossen werden");
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, "LFG-Reconcile: Lane-tote Posts konnten nicht geladen werden")
            }
        }

        let expired_posts = sqlx::query_scalar::<_, i64>(
            "SELECT id
               FROM voice.lfg_posts
              WHERE status = 'open'
                AND lane_id IS NULL
                AND expires_at < now()",
        )
        .fetch_all(&self.pool)
        .await;
        match expired_posts {
            Ok(post_ids) => {
                for post_id in post_ids {
                    if let Err(err) = self.close_post(post_id, "expired").await {
                        tracing::warn!(%err, post_id, "LFG-Reconcile: expired Post konnte nicht geschlossen werden");
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, "LFG-Reconcile: expired Posts konnten nicht geladen werden")
            }
        }

        match crate::lfg_watch::delete_expired_watches(&self.pool).await {
            Ok(0) => {}
            Ok(geloescht) => tracing::info!(
                geloescht,
                gnadenfrist_tage = crate::lfg_watch::LFG_WATCH_CLEANUP_GRACE_DAYS,
                "LFG-Reconcile: abgelaufene Watches geloescht"
            ),
            Err(err) => {
                tracing::warn!(%err, "LFG-Reconcile: Cleanup abgelaufener Watches fehlgeschlagen")
            }
        }
    }
}

#[async_trait::async_trait]
impl InteractionHandler for LfgPanelInterface {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !self.cutover_active {
            return BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN);
        }
        if interaction.custom_id == LFG_CREATE_START_CUSTOM_ID {
            return self.handle_start().await;
        }
        if interaction.custom_id == LFG_WATCH_START_CUSTOM_ID {
            return self.handle_watch_start(interaction).await;
        }
        if let Some(mode) = interaction
            .custom_id
            .strip_prefix(LFG_CREATE_MODE_PREFIX)
            .and_then(LfgMode::from_str)
        {
            return self.handle_mode(interaction, mode).await;
        }
        if let Some(mode) = interaction
            .custom_id
            .strip_prefix(LFG_WATCH_MODE_PREFIX)
            .and_then(LfgMode::from_str)
        {
            return self.handle_watch_select(interaction, Some(mode)).await;
        }
        if matches!(
            interaction.custom_id.as_str(),
            LFG_RANK_FROM_SELECT_CUSTOM_ID
                | LFG_RANK_TO_SELECT_CUSTOM_ID
                | LFG_SLOTS_SELECT_CUSTOM_ID
                | LFG_WHEN_SELECT_CUSTOM_ID
        ) {
            return self.handle_draft_select(interaction).await;
        }
        if matches!(
            interaction.custom_id.as_str(),
            LFG_WATCH_RANK_FROM_SELECT_CUSTOM_ID
                | LFG_WATCH_RANK_TO_SELECT_CUSTOM_ID
                | LFG_WATCH_WINDOW_SELECT_CUSTOM_ID
        ) {
            return self.handle_watch_select(interaction, None).await;
        }
        if interaction.custom_id == LFG_WATCH_ACTIVATE_CUSTOM_ID {
            return self.handle_watch_activate(interaction).await;
        }
        if let Some(mode) = interaction
            .custom_id
            .strip_prefix(LFG_POST_PREFIX)
            .and_then(LfgMode::from_str)
        {
            return self.handle_post_draft(interaction, mode).await;
        }
        if let Some(post_id) = parse_i64_suffix(&interaction.custom_id, LFG_OPEN_LANE_PREFIX) {
            return self.handle_open_lane(interaction, post_id).await;
        }
        if let Some(post_id) = parse_i64_suffix(&interaction.custom_id, LFG_JOIN_PREFIX) {
            return self.handle_join(interaction, post_id).await;
        }
        BridgeReply::ephemeral_text(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN)
    }
}

pub fn register(router: &mut InteractionRouter, interface: Arc<LfgPanelInterface>) {
    router.on_prefix("lfg:create:", interface.clone());
    router.on_custom_id(LFG_WATCH_START_CUSTOM_ID, interface.clone());
    router.on_prefix(LFG_WATCH_MODE_PREFIX, interface.clone());
    router.on_custom_id(LFG_RANK_FROM_SELECT_CUSTOM_ID, interface.clone());
    router.on_custom_id(LFG_RANK_TO_SELECT_CUSTOM_ID, interface.clone());
    router.on_custom_id(LFG_SLOTS_SELECT_CUSTOM_ID, interface.clone());
    router.on_custom_id(LFG_WHEN_SELECT_CUSTOM_ID, interface.clone());
    router.on_custom_id(LFG_WATCH_RANK_FROM_SELECT_CUSTOM_ID, interface.clone());
    router.on_custom_id(LFG_WATCH_RANK_TO_SELECT_CUSTOM_ID, interface.clone());
    router.on_custom_id(LFG_WATCH_WINDOW_SELECT_CUSTOM_ID, interface.clone());
    router.on_custom_id(LFG_WATCH_ACTIVATE_CUSTOM_ID, interface.clone());
    router.on_prefix(LFG_POST_PREFIX, interface.clone());
    router.on_prefix(LFG_OPEN_LANE_PREFIX, interface.clone());
    router.on_prefix(LFG_JOIN_PREFIX, interface);
}

pub fn spawn(interface: Arc<LfgPanelInterface>, dispatcher: &Dispatcher) {
    if !interface.cutover_active() {
        return;
    }
    let mut events = dispatcher.subscribe_voice();
    let event_interface = interface.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => event_interface.handle_voice_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "LFG: VoiceEvents verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let reconcile_interface = interface.clone();
    tokio::spawn(async move {
        loop {
            reconcile_interface.reconcile_once().await;
            tokio::time::sleep(Duration::from_secs(LFG_RECONCILE_INTERVAL_SECONDS)).await;
        }
    });

    let initial_enqueue_interface = interface.clone();
    tokio::spawn(async move {
        initial_enqueue_interface
            .enqueue_open_lane_posts_for_initial_render()
            .await;
    });

    tokio::spawn(interface.edit_worker());
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::{BridgeInteraction, InteractionRouter};
    use serde_json::{json, Map, Value};
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    fn lfg_interaction(custom_id: impl Into<String>, user_id: u64) -> BridgeInteraction {
        BridgeInteraction {
            custom_id: custom_id.into(),
            guild_id: LFG_GUILD_ID,
            user_id,
            ..BridgeInteraction::default()
        }
    }

    fn lfg_select_interaction(custom_id: &str, user_id: u64, value: &str) -> BridgeInteraction {
        BridgeInteraction {
            custom_id: custom_id.to_string(),
            guild_id: LFG_GUILD_ID,
            user_id,
            values: vec![value.to_string()],
            ..BridgeInteraction::default()
        }
    }

    async fn select_lfg_value(
        interface: &Arc<LfgPanelInterface>,
        custom_id: &str,
        user_id: u64,
        value: &str,
    ) -> BridgeReply {
        interface
            .handle(lfg_select_interaction(custom_id, user_id, value))
            .await
    }

    async fn post_lfg_draft(
        interface: &Arc<LfgPanelInterface>,
        mode: LfgMode,
        user_id: u64,
    ) -> BridgeReply {
        interface
            .handle(lfg_interaction(lfg_post_custom_id(mode), user_id))
            .await
    }

    async fn fill_lfg_draft(
        interface: &Arc<LfgPanelInterface>,
        user_id: u64,
        rank_from: &str,
        rank_to: &str,
        slots: &str,
    ) {
        let from = select_lfg_value(
            interface,
            LFG_RANK_FROM_SELECT_CUSTOM_ID,
            user_id,
            rank_from,
        )
        .await;
        assert!(from.update_message);
        let to = select_lfg_value(interface, LFG_RANK_TO_SELECT_CUSTOM_ID, user_id, rank_to).await;
        assert!(to.update_message);
        let slots = select_lfg_value(interface, LFG_SLOTS_SELECT_CUSTOM_ID, user_id, slots).await;
        assert!(slots.update_message);
    }

    fn assert_select_has_no_default(row: &Value) {
        let options = row["components"][0]["options"]
            .as_array()
            .expect("select options");
        assert!(
            options.iter().all(|option| option["default"] != true),
            "fresh select must not preselect an option: {options:?}"
        );
    }

    fn assert_select_default_value(row: &Value, expected: &str) {
        let options = row["components"][0]["options"]
            .as_array()
            .expect("select options");
        let defaults = options
            .iter()
            .filter_map(|option| (option["default"] == true).then_some(option["value"].as_str()))
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(defaults, vec![expected]);
    }

    #[test]
    fn lfg_panel_body_ist_components_v2_mit_start_button_und_banner() {
        let attachments = lfg_panel_attachments();
        let body = lfg_panel_body_for_attachments(&attachments);

        assert_eq!(body.get("flags"), Some(&json!(LFG_COMPONENTS_V2_FLAG)));
        assert_eq!(body.get("allowed_mentions"), Some(&json!({"parse": []})));
        assert_eq!(
            body.get("attachments"),
            Some(&json!([{"id": 0, "filename": LFG_PANEL_BANNER_FILENAME}]))
        );
        let components = body["components"][0]["components"]
            .as_array()
            .expect("container components");
        assert!(components.iter().any(|component| {
            component["type"] == 12
                && component["items"][0]["media"]["url"]
                    == format!("attachment://{LFG_PANEL_BANNER_FILENAME}")
        }));
        let custom_ids: Vec<&str> = components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert_eq!(
            custom_ids,
            vec![LFG_CREATE_START_CUSTOM_ID, LFG_WATCH_START_CUSTOM_ID]
        );
    }

    #[test]
    fn preset_roundtrip_preserves_play_window() {
        let draft = LfgDraft {
            mode: LfgMode::Casual,
            rank_from: None,
            rank_to: None,
            slots: Some(2),
            play_window: Some(LfgPlayWindow::HeuteAbend),
            lane_id: None,
        };
        let preset = LfgUserPreset::from_draft(&draft);
        let back = preset.into_draft(LfgMode::Casual).expect("preset -> draft");
        assert_eq!(back.play_window, Some(LfgPlayWindow::HeuteAbend));
    }

    #[tokio::test]
    async fn lfg_panel_interface_postet_und_editiert_idempotent_ueber_kv() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let first = interface.apply_panel(true).await.expect("first apply");
        assert_eq!(first.action, "posted");
        assert_eq!(first.channel_id, Some(777));
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9101")
        );

        let second = interface.apply_panel(true).await.expect("second apply");
        assert_eq!(second.action, "edited");
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(port.edits.lock().expect("edits").len(), 1);
    }

    #[tokio::test]
    async fn lfg_panel_apply_postet_neu_wenn_kv_message_geloescht_ist() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        dl_central_db::kv::set(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY, "7777")
            .await
            .expect("kv set");
        let port = Arc::new(MockLfgPanelPort::default());
        port.not_found_edits.lock().expect("not found").push(7777);
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let output = interface.apply_panel(true).await.expect("apply");

        assert_eq!(output.action, "posted");
        assert_eq!(output.message_id, Some(9101));
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("stale (404)")));
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9101")
        );
    }

    #[tokio::test]
    async fn lfg_panel_apply_adoptiert_nur_lfg_create_v2_panel_aus_history() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.recent.lock().expect("recent") = vec![
            LfgPanelMessage {
                message_id: 7001,
                has_embeds: false,
                has_components: true,
                custom_ids: vec!["router_spawn_casual".to_string()],
            },
            LfgPanelMessage {
                message_id: 7002,
                has_embeds: true,
                has_components: true,
                custom_ids: vec![LFG_CREATE_START_CUSTOM_ID.to_string()],
            },
            LfgPanelMessage {
                message_id: 7003,
                has_embeds: false,
                has_components: true,
                custom_ids: vec![LFG_CREATE_START_CUSTOM_ID.to_string()],
            },
            LfgPanelMessage {
                message_id: 7004,
                has_embeds: false,
                has_components: true,
                custom_ids: vec![LFG_CREATE_START_CUSTOM_ID.to_string()],
            },
        ];
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let output = interface.apply_panel(true).await.expect("apply");

        assert_eq!(output.action, "adopted_edit");
        assert_eq!(output.message_id, Some(7004));
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        let edited_message_id = {
            let edits = port.edits.lock().expect("edits");
            assert_eq!(edits.len(), 1);
            edits[0].1
        };
        assert_eq!(edited_message_id, 7004);
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("7004")
        );
    }

    #[tokio::test]
    async fn lfg_panel_dry_run_braucht_keine_channel_config() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port.clone(), None);

        let output = interface.apply_panel(false).await.expect("dry run");

        assert!(output.dry_run);
        assert_eq!(output.action, "blocked_missing_channel");
        assert!(output.channel_id.is_none());
        assert_eq!(
            output.blocked_reason.as_deref(),
            Some("DL_LFG_PANEL_CHANNEL_ID fehlt")
        );
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
    }

    #[tokio::test]
    async fn lfg_panel_apply_blockt_confirm_bei_inaktivem_cutover() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new_with_channel_config(
            db.pool().clone(),
            port.clone(),
            Some(777),
            None,
            false,
        );

        let dry_run = interface.apply_panel(false).await.expect("dry run");
        assert!(dry_run.dry_run);
        assert_eq!(dry_run.action, "blocked_cutover_disabled");
        assert_eq!(dry_run.blocked_reason.as_deref(), Some("cutover_disabled"));
        let confirm = interface.apply_panel(true).await.expect("confirm");
        assert!(confirm.dry_run);
        assert_eq!(confirm.action, "blocked_cutover_disabled");
        assert_eq!(confirm.blocked_reason.as_deref(), Some("cutover_disabled"));
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);

        let no_channel = LfgPanelInterface::new_with_channel_config(
            db.pool().clone(),
            port.clone(),
            None,
            Some("DL_LFG_PANEL_CHANNEL_ID fehlt".to_string()),
            false,
        );
        let no_channel_dry_run = no_channel.apply_panel(false).await.expect("dry run");
        assert_eq!(
            no_channel_dry_run.blocked_reason.as_deref(),
            Some("cutover_disabled")
        );
        let no_channel_confirm = no_channel.apply_panel(true).await.expect("confirm");
        assert_eq!(
            no_channel_confirm.blocked_reason.as_deref(),
            Some("cutover_disabled")
        );
    }

    #[tokio::test]
    async fn lfg_env_split_panel_apply_nutzt_panel_und_forum_post_nutzt_forum_channel() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9901;
        let interface = LfgPanelInterface::new_with_split_channel_config(
            pool.clone(),
            port.clone(),
            Some(700),
            None,
            Some(800),
            None,
            true,
        );

        let apply = interface.apply_panel(true).await.expect("panel apply");
        assert_eq!(apply.action, "posted");
        assert_eq!(apply.channel_id, Some(700));
        assert_eq!(port.posts.lock().expect("posts")[0].0, 700);

        let draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), 42))
            .await;
        assert!(draft.update_message);
        fill_lfg_draft(&interface, 42, "ritualist", "phantom", "3").await;
        let reply = post_lfg_draft(&interface, LfgMode::Casual, 42).await;

        assert!(reply.update_message);
        assert_eq!(reply.content.as_deref(), Some(LFG_ERFOLG_POST_ERSTELLT));
        assert_eq!(port.forum_posts.lock().expect("forum posts")[0].0, 800);
        let forum_channel_id: i64 =
            sqlx::query_scalar("SELECT forum_channel_id FROM voice.lfg_posts WHERE thread_id = $1")
                .bind(9901_i64)
                .fetch_one(&pool)
                .await
                .expect("forum channel id");
        assert_eq!(forum_channel_id, 800);
    }

    #[tokio::test]
    async fn lfg_cutover_bleibt_ohne_panel_id_fuer_tempvoice_forum_publish_aktiv() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::query(
            "INSERT INTO voice.tempvoice_lanes (
                 channel_id, guild_id, owner_id, base_name, category_id, source_staging_id, initial_owner_id
             )
             VALUES ($1, $2, 42, 'Chill Lane 1', $3, NULL, 42)",
        )
        .bind(7777_i64)
        .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
        .bind(i64::try_from(crate::router::mode_to_category("casual")).expect("category"))
        .execute(&pool)
        .await
        .expect("lane");
        let port = Arc::new(MockLfgPanelPort::default());
        port.categories
            .lock()
            .expect("categories")
            .insert(7777, crate::router::mode_to_category("casual"));
        *port.next_thread_id.lock().expect("next thread") = 9910;
        let interface = LfgPanelInterface::new_with_split_channel_config(
            pool.clone(),
            port.clone(),
            None,
            Some("DL_LFG_PANEL_CHANNEL_ID ist nicht gesetzt".to_string()),
            Some(800),
            None,
            true,
        );

        assert!(interface.cutover_active());
        let panel_dry_run = interface.apply_panel(false).await.expect("panel dry run");
        assert_eq!(panel_dry_run.action, "blocked_missing_channel");
        assert_eq!(
            panel_dry_run.blocked_reason.as_deref(),
            Some("DL_LFG_PANEL_CHANNEL_ID ist nicht gesetzt")
        );
        let panel_confirm = interface.apply_panel(true).await.expect("panel confirm");
        assert_eq!(panel_confirm.action, "blocked_missing_channel");
        assert!(port.posts.lock().expect("posts").is_empty());

        let start = interface
            .handle_publish_lane_start(
                BridgeInteraction {
                    guild_id: LFG_GUILD_ID,
                    user_id: 42,
                    ..BridgeInteraction::default()
                },
                7777,
            )
            .await;
        assert!(start.ephemeral);
        assert!(start.components.is_some());
        fill_lfg_draft(&interface, 42, LFG_RANK_ANY_VALUE, LFG_RANK_ANY_VALUE, "2").await;
        let reply = post_lfg_draft(&interface, LfgMode::Casual, 42).await;

        assert!(reply.update_message);
        assert_eq!(reply.content.as_deref(), Some(LFG_ERFOLG_POST_ERSTELLT));
        assert_eq!(port.forum_posts.lock().expect("forum posts")[0].0, 800);
    }

    #[tokio::test]
    async fn lfg_panel_apply_blockt_forum_als_panel_ziel_aus_cache() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        *port.panel_channel_kind.lock().expect("panel channel kind") =
            Some(LfgPanelChannelKind::Forum);
        let interface = LfgPanelInterface::new_with_split_channel_config(
            db.pool().clone(),
            port.clone(),
            Some(700),
            None,
            Some(800),
            None,
            true,
        );

        let dry_run = interface.apply_panel(false).await.expect("dry run");
        assert_eq!(dry_run.action, "blocked_invalid_channel_type");
        assert!(dry_run
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("forum")));

        let confirm = interface.apply_panel(true).await.expect("confirm");
        assert_eq!(confirm.action, "blocked_invalid_channel_type");
        assert!(confirm.blocked_reason.is_some());
        assert!(port.posts.lock().expect("posts").is_empty());
        assert!(port.edits.lock().expect("edits").is_empty());
    }

    #[tokio::test]
    async fn lfg_create_start_button_zeigt_ephemere_moduswahl() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port, Some(777));
        let mut router = InteractionRouter::new();
        register(&mut router, interface);

        let handler = router
            .resolve_component(LFG_CREATE_START_CUSTOM_ID)
            .expect("lfg handler");
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: LFG_CREATE_START_CUSTOM_ID.to_string(),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_MODE_PROMPT));
        let custom_ids: Vec<String> = reply.components.as_ref().expect("components")[0]
            ["components"]
            .as_array()
            .expect("buttons")
            .iter()
            .map(|button| button["custom_id"].as_str().expect("custom_id").to_string())
            .collect();
        assert_eq!(
            custom_ids,
            vec![
                LfgMode::Casual.mode_custom_id(),
                LfgMode::Ranked.mode_custom_id(),
                LfgMode::StreetBrawl.mode_custom_id(),
            ]
        );
    }

    #[tokio::test]
    async fn lfg_create_interactions_blocken_bei_inaktivem_cutover() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new_with_channel_config(
            db.pool().clone(),
            port.clone(),
            Some(777),
            None,
            false,
        );

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LFG_CREATE_START_CUSTOM_ID.to_string(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            reply.content.as_deref(),
            Some(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN)
        );
        assert!(reply.components.is_none());
        assert!(reply.modal.is_none());
        assert!(port.forum_posts.lock().expect("forum posts").is_empty());
    }

    #[test]
    fn lfg_rank_parser_akzeptiert_nur_bekannte_grammatik() {
        assert_eq!(
            parse_rank_range("phantom"),
            Some(LfgRankRange {
                min: Some(9),
                max: Some(9)
            })
        );
        assert_eq!(
            parse_rank_range("oracle  bis  phantom"),
            Some(LfgRankRange {
                min: Some(8),
                max: Some(9)
            })
        );
        assert_eq!(
            parse_rank_range("Ritualist bis Phantom"),
            Some(LfgRankRange {
                min: Some(6),
                max: Some(9)
            })
        );
        assert_eq!(
            parse_rank_range(""),
            Some(LfgRankRange {
                min: None,
                max: None
            })
        );

        for raw in [
            "Phantom bis Kartoffel",
            "Kartoffel bis Ritualist",
            "PHANTOM  bis  oracle",
            "Phantom-Oracle",
            "Müll",
            "Phantom Ritualist",
        ] {
            assert_eq!(parse_rank_range(raw), None, "raw={raw}");
        }
    }

    #[test]
    fn lfg_forum_tags_werden_aus_modus_rang_und_lane_status_abgeleitet() {
        let range = |min, max| LfgRankRange {
            min: Some(min),
            max: Some(max),
        };

        assert_eq!(
            derive_lfg_forum_tag_ids(
                LfgMode::Casual,
                LfgRankRange {
                    min: None,
                    max: None
                },
                false,
            ),
            vec![
                LFG_FORUM_TAG_MODE_CASUAL,
                LFG_FORUM_TAG_RANK_ANY,
                LFG_FORUM_TAG_STATUS_LOOKING,
            ]
        );
        assert_eq!(
            derive_lfg_forum_tag_ids(LfgMode::Ranked, range(1, 2), false),
            vec![
                LFG_FORUM_TAG_MODE_RANKED,
                LFG_FORUM_TAG_RANK_BEGINNER,
                LFG_FORUM_TAG_STATUS_LOOKING,
            ]
        );
        assert_eq!(
            derive_lfg_forum_tag_ids(LfgMode::Ranked, range(2, 3), false),
            vec![
                LFG_FORUM_TAG_MODE_RANKED,
                LFG_FORUM_TAG_RANK_BEGINNER,
                LFG_FORUM_TAG_RANK_ADVANCED,
                LFG_FORUM_TAG_STATUS_LOOKING,
            ]
        );
        assert_eq!(
            derive_lfg_forum_tag_ids(LfgMode::StreetBrawl, range(9, 11), true),
            vec![
                LFG_FORUM_TAG_MODE_STREET_BRAWL,
                LFG_FORUM_TAG_RANK_ANY,
                LFG_FORUM_TAG_STATUS_ACTIVE,
            ]
        );
        assert_eq!(
            derive_lfg_forum_tag_ids(LfgMode::Ranked, range(9, 11), true),
            vec![
                LFG_FORUM_TAG_MODE_RANKED,
                LFG_FORUM_TAG_RANK_ELITE,
                LFG_FORUM_TAG_STATUS_ACTIVE,
            ]
        );
        assert_eq!(
            derive_lfg_forum_tag_ids(
                LfgMode::Ranked,
                LfgRankRange {
                    min: Some(7),
                    max: None,
                },
                false,
            ),
            vec![
                LFG_FORUM_TAG_MODE_RANKED,
                LFG_FORUM_TAG_RANK_EXPERIENCED,
                LFG_FORUM_TAG_RANK_ELITE,
                LFG_FORUM_TAG_STATUS_LOOKING,
            ]
        );

        let capped = derive_lfg_forum_tag_ids(LfgMode::Ranked, range(1, 11), true);
        assert_eq!(capped.len(), LFG_FORUM_MAX_APPLIED_TAGS);
        assert!(capped.contains(&LFG_FORUM_TAG_MODE_RANKED));
        assert!(capped.contains(&LFG_FORUM_TAG_STATUS_ACTIVE));
    }

    #[test]
    fn lfg_draft_rank_range_tauscht_vertauchte_werte_und_erlaubt_offene_bereiche() {
        let mut draft = LfgDraft::new(LfgMode::Ranked, None);
        draft.rank_from = Some("phantom".to_string());
        draft.rank_to = Some("archon".to_string());
        assert_eq!(
            lfg_rank_range_from_draft(&draft),
            Some(LfgRankRange {
                min: Some(7),
                max: Some(9),
            })
        );

        draft.rank_from = Some("archon".to_string());
        draft.rank_to = None;
        assert_eq!(
            lfg_rank_range_from_draft(&draft),
            Some(LfgRankRange {
                min: Some(7),
                max: None,
            })
        );
    }

    #[test]
    fn lfg_draft_content_zeigt_live_zusammenfassung_mit_rangrichtung() {
        let mut draft = LfgDraft::new(LfgMode::Ranked, None);
        draft.rank_from = Some("archon".to_string());
        draft.rank_to = Some("phantom".to_string());
        draft.slots = Some(3);

        assert_eq!(
            lfg_draft_content(&draft),
            "**Ranked** · Emissary → Phantom · 3 Plätze\nWähl Rang-Bereich und Plätze, dann **Suche veröffentlichen**."
        );

        draft.play_window = Some(LfgPlayWindow::HeuteAbend);
        assert_eq!(
            lfg_draft_content(&draft),
            "**Ranked** · Emissary → Phantom · 3 Plätze · 🌙 Heute Abend\nWähl Rang-Bereich und Plätze, dann **Suche veröffentlichen**."
        );
    }

    #[test]
    fn lfg_one_category_mode_nutzt_source_staging_vor_namen() {
        assert_eq!(
            mode_from_lane_record(
                crate::router::ROUTER_CATEGORY_CHILL,
                Some(crate::router::mode_to_staging("ranked")),
                "Custom Name"
            ),
            Some(LfgMode::Ranked)
        );
        assert_eq!(
            mode_from_lane_record(
                crate::router::ROUTER_CATEGORY_CHILL,
                Some(crate::router::mode_to_staging("street_brawl")),
                "Custom Name"
            ),
            Some(LfgMode::StreetBrawl)
        );
    }

    #[tokio::test]
    async fn lfg_ranked_mode_ohne_rankrolle_rendert_select_draft() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port, Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Ranked.mode_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.update_message);
        assert_eq!(
            reply.content.as_deref(),
            Some(
                "**Ranked** · Rang egal · Plätze offen\nWähl Rang-Bereich und Plätze, dann **Suche veröffentlichen**."
            )
        );
        assert!(reply.components.is_some());
        assert!(reply.modal.is_none());
    }

    #[tokio::test]
    async fn lfg_ranked_mode_mit_rankrolle_rendert_select_draft() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        *port.roles.lock().expect("roles") = vec![crate::router::VERIFIED_RANK_ROLE_IDS[0]];
        let interface = LfgPanelInterface::new(db.pool().clone(), port, Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Ranked.mode_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.update_message);
        assert!(reply.modal.is_none());
        assert_eq!(
            reply.content.as_deref(),
            Some(
                "**Ranked** · Rang egal · Plätze offen\nWähl Rang-Bereich und Plätze, dann **Suche veröffentlichen**."
            )
        );
        let rows = reply
            .components
            .as_ref()
            .expect("components")
            .as_array()
            .expect("component rows");
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[0]["components"][0]["placeholder"],
            LFG_RANK_FROM_PLACEHOLDER
        );
        assert_eq!(
            rows[1]["components"][0]["placeholder"],
            LFG_RANK_TO_PLACEHOLDER
        );
        assert_eq!(
            rows[2]["components"][0]["placeholder"],
            LFG_SLOTS_PLACEHOLDER
        );
        assert_eq!(
            rows[3]["components"][0]["placeholder"],
            "Wann willst du spielen? (optional)"
        );
        assert_select_has_no_default(&rows[0]);
        assert_select_has_no_default(&rows[1]);
        assert_select_has_no_default(&rows[2]);
        assert_select_has_no_default(&rows[3]);
        let rank_options = rows[0]["components"][0]["options"]
            .as_array()
            .expect("rank options");
        assert_eq!(rank_options[0]["label"], LFG_RANK_ANY_LABEL);
        assert_eq!(rank_options[0]["emoji"]["name"], "dl_rang_egal");
        assert_eq!(rank_options[0]["emoji"]["id"], "1522801043803472064");
        let emissary = rank_options
            .iter()
            .find(|option| option["value"] == "emissary")
            .expect("emissary option");
        assert_eq!(emissary["label"], "Emissary");
        assert_eq!(emissary["emoji"]["name"], "emissary");
        assert_eq!(emissary["emoji"]["id"], "1397687455313952918");
        assert_eq!(
            rows[4]["components"][0]["custom_id"],
            lfg_post_custom_id(LfgMode::Ranked)
        );
        assert_eq!(rows[4]["components"][0]["label"], LFG_BTN_POSTEN);
    }

    #[tokio::test]
    async fn lfg_ranked_mode_nutzt_interaction_roles_bei_leerem_cache() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port, Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Ranked.mode_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                role_ids: vec![crate::router::VERIFIED_RANK_ROLE_IDS[0]],
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.update_message);
        assert!(reply.components.is_some());
        assert!(reply.modal.is_none());
    }

    #[tokio::test]
    async fn lfg_select_validiert_slots_und_postet_nicht_bei_muell() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port.clone(), Some(777));

        let draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), 42))
            .await;
        assert!(draft.update_message);
        let reply = interface
            .handle(lfg_select_interaction(LFG_SLOTS_SELECT_CUSTOM_ID, 42, "x"))
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_ERR_PLAETZE_UNGUELTIG));
        assert!(port.forum_posts.lock().expect("forum posts").is_empty());
    }

    #[tokio::test]
    async fn lfg_select_flow_erstellt_forum_post_und_persistiert_row() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9901;
        *port.first_message_id.lock().expect("first message") = Ok(Some(9902));
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), 42))
            .await;
        assert!(draft.update_message);
        fill_lfg_draft(&interface, 42, "ritualist", "phantom", "3").await;
        let reply = post_lfg_draft(&interface, LfgMode::Casual, 42).await;

        assert!(reply.update_message);
        assert_eq!(reply.content.as_deref(), Some(LFG_ERFOLG_POST_ERSTELLT));
        let (posted_channel_id, posted_title) = {
            let posts = port.forum_posts.lock().expect("forum posts");
            assert_eq!(posts.len(), 1);
            assert_eq!(
                posts[0].1.applied_tags,
                vec![
                    LFG_FORUM_TAG_MODE_CASUAL,
                    LFG_FORUM_TAG_RANK_ANY,
                    LFG_FORUM_TAG_STATUS_LOOKING,
                ]
            );
            (posts[0].0, posts[0].1.title.clone())
        };
        assert_eq!(posted_channel_id, 777);
        assert!(posted_title.contains("Normale Lane"));
        assert!(posted_title.contains("Ritualist bis Phantom"));

        type LfgPostRow = (
            i64,
            i64,
            i64,
            Option<i64>,
            i64,
            String,
            Option<i32>,
            Option<i32>,
            i32,
            String,
        );
        let row: LfgPostRow = sqlx::query_as(
            "SELECT guild_id,
                        forum_channel_id,
                        thread_id,
                        starter_message_id,
                        owner_id,
                        mode,
                        rank_min,
                        rank_max,
                        requested_slots,
                        status
                   FROM voice.lfg_posts
                  WHERE thread_id = $1",
        )
        .bind(9901_i64)
        .fetch_one(&pool)
        .await
        .expect("lfg row");
        assert_eq!(row.0, i64::try_from(LFG_GUILD_ID).expect("guild id"));
        assert_eq!(row.1, 777);
        assert_eq!(row.2, 9901);
        assert_eq!(row.3, Some(9902));
        assert_eq!(row.4, 42);
        assert_eq!(row.5, "casual");
        assert_eq!(row.6, Some(5));
        assert_eq!(row.7, Some(9));
        assert_eq!(row.8, 3);
        assert_eq!(row.9, "open");

        let expires_in_hours: f64 = sqlx::query_scalar(
            "SELECT EXTRACT(EPOCH FROM (expires_at - created_at))::DOUBLE PRECISION / 3600.0
               FROM voice.lfg_posts
              WHERE thread_id = $1",
        )
        .bind(9901_i64)
        .fetch_one(&pool)
        .await
        .expect("expires diff");
        assert!((expires_in_hours - LFG_EXPIRY_HOURS as f64).abs() < 0.01);
    }

    #[tokio::test]
    async fn created_post_persists_play_window() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));
        let post_id = interface
            .create_lfg_post_from_values(
                LFG_GUILD_ID,
                424_242,
                LfgMode::Casual,
                LfgRankRange {
                    min: None,
                    max: None,
                },
                2,
                None,
                Some(LfgPlayWindow::HeuteAbend),
            )
            .await
            .expect("post");
        let stored: Option<String> =
            sqlx::query_scalar("SELECT play_window FROM voice.lfg_posts WHERE id = $1")
                .bind(post_id)
                .fetch_one(&pool)
                .await
                .expect("row");
        assert_eq!(stored.as_deref(), Some("heute_abend"));
    }

    #[tokio::test]
    async fn mock_records_sent_dm() {
        let port = MockLfgPanelPort::new();
        port.send_dm(42, "hallo".to_string()).await.expect("dm");
        assert_eq!(port.sent_dms(), vec![(42, "hallo".to_string())]);
    }

    #[tokio::test]
    async fn watch_activate_inserts_row() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::new());
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));
        interface.pending_watch_drafts.lock().await.insert(
            42,
            LfgWatchDraft {
                mode: Some(LfgMode::Casual),
                rank_from: None,
                rank_to: None,
                window: Some(LfgWatchWindow::Now3h),
            },
        );
        let reply = interface
            .handle_watch_activate(lfg_interaction(LFG_WATCH_ACTIVATE_CUSTOM_ID, 42))
            .await;
        assert!(reply.update_message);
        let armed = crate::lfg_watch::armed_watches_for_match(&pool, LFG_GUILD_ID, "casual", 0)
            .await
            .expect("armed");
        assert_eq!(armed.len(), 1);
    }

    #[tokio::test]
    async fn watch_weekend_expires_after_sunday_late_reference() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let reference = DateTime::parse_from_rfc3339("2026-07-05T21:59:30Z")
            .expect("reference")
            .with_timezone(&Utc);
        let sql = lfg_watch_expires_at_sql(
            "TIMESTAMPTZ '2026-07-05 21:59:30+00'",
            LfgWatchWindow::Weekend,
        );
        let expires_at: DateTime<Utc> = sqlx::query_scalar(&sql)
            .fetch_one(&pool)
            .await
            .expect("expires_at");

        assert!(expires_at > reference);
        assert!(expires_at >= reference + chrono::Duration::minutes(1));
    }

    #[tokio::test]
    async fn matching_gesuch_fires_one_dm_then_stops() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::new());
        let exp = Utc::now() + chrono::Duration::hours(3);
        crate::lfg_watch::insert_or_replace_watch(
            &pool,
            LFG_GUILD_ID,
            99,
            "casual",
            LfgRankRange {
                min: None,
                max: None,
            },
            LfgWatchWindow::Now3h,
            exp,
        )
        .await
        .expect("watch");

        crate::lfg_watch::run_match_for_new_post(
            &pool,
            port.as_ref(),
            LFG_GUILD_ID,
            500,
            910_100,
            42,
            "casual",
            "Normale Lane",
            LfgRankRange {
                min: None,
                max: None,
            },
            Some("jetzt".into()),
            "Alle Ränge".into(),
        )
        .await;
        assert_eq!(port.sent_dms().len(), 1);
        assert_eq!(port.sent_dms()[0].0, 99);
        assert!(port.sent_dms()[0]
            .1
            .contains("https://discord.com/channels/"));
        let claimed: (Option<DateTime<Utc>>, Option<i64>) = sqlx::query_as(
            "SELECT fired_at, matched_post_id
               FROM activity.lfg_watches
              WHERE guild_id = $1
                AND user_id = $2",
        )
        .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
        .bind(99_i64)
        .fetch_one(&pool)
        .await
        .expect("claimed");
        assert!(claimed.0.is_some());
        assert_eq!(claimed.1, Some(500));

        crate::lfg_watch::run_match_for_new_post(
            &pool,
            port.as_ref(),
            LFG_GUILD_ID,
            501,
            910_101,
            43,
            "casual",
            "Normale Lane",
            LfgRankRange {
                min: None,
                max: None,
            },
            Some("jetzt".into()),
            "Alle Ränge".into(),
        )
        .await;
        assert_eq!(port.sent_dms().len(), 1, "one-shot: keine zweite DM");
    }

    #[tokio::test]
    async fn matching_geclaimter_watch_sendet_bei_zweitem_passenden_post_keine_dm() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::new());
        let exp = Utc::now() + chrono::Duration::hours(3);
        crate::lfg_watch::insert_or_replace_watch(
            &pool,
            LFG_GUILD_ID,
            199,
            "casual",
            LfgRankRange {
                min: None,
                max: None,
            },
            LfgWatchWindow::Now3h,
            exp,
        )
        .await
        .expect("watch");

        crate::lfg_watch::run_match_for_new_post(
            &pool,
            port.as_ref(),
            LFG_GUILD_ID,
            700,
            910_300,
            42,
            "casual",
            "Normale Lane",
            LfgRankRange {
                min: None,
                max: None,
            },
            Some("jetzt".into()),
            "Alle Ränge".into(),
        )
        .await;
        let claimed_post_id: Option<i64> = sqlx::query_scalar(
            "SELECT matched_post_id
               FROM activity.lfg_watches
              WHERE guild_id = $1
                AND user_id = $2
                AND fired_at IS NOT NULL",
        )
        .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
        .bind(199_i64)
        .fetch_one(&pool)
        .await
        .expect("claimed");
        assert_eq!(claimed_post_id, Some(700));

        crate::lfg_watch::run_match_for_new_post(
            &pool,
            port.as_ref(),
            LFG_GUILD_ID,
            701,
            910_301,
            43,
            "casual",
            "Normale Lane",
            LfgRankRange {
                min: None,
                max: None,
            },
            Some("jetzt".into()),
            "Alle Ränge".into(),
        )
        .await;

        assert_eq!(port.sent_dms().len(), 1);
    }

    #[tokio::test]
    async fn matching_gesuch_ignoriert_falschen_modus_disjunkten_rang_und_abgelaufen() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::new());
        let future = Utc::now() + chrono::Duration::hours(3);
        crate::lfg_watch::insert_or_replace_watch(
            &pool,
            LFG_GUILD_ID,
            100,
            "casual",
            LfgRankRange {
                min: None,
                max: None,
            },
            LfgWatchWindow::Now3h,
            future,
        )
        .await
        .expect("watch wrong mode");
        crate::lfg_watch::insert_or_replace_watch(
            &pool,
            LFG_GUILD_ID,
            101,
            "ranked",
            LfgRankRange {
                min: Some(1),
                max: Some(3),
            },
            LfgWatchWindow::Now3h,
            future,
        )
        .await
        .expect("watch disjoint");
        crate::lfg_watch::insert_or_replace_watch(
            &pool,
            LFG_GUILD_ID,
            102,
            "ranked",
            LfgRankRange {
                min: None,
                max: None,
            },
            LfgWatchWindow::Now3h,
            Utc::now() - chrono::Duration::hours(1),
        )
        .await
        .expect("watch expired");

        crate::lfg_watch::run_match_for_new_post(
            &pool,
            port.as_ref(),
            LFG_GUILD_ID,
            600,
            910_200,
            42,
            "ranked",
            "Ranked",
            LfgRankRange {
                min: Some(7),
                max: Some(9),
            },
            Some("jetzt".into()),
            "Emissary bis Phantom".into(),
        )
        .await;
        assert!(port.sent_dms().is_empty());
    }

    #[tokio::test]
    async fn lfg_preset_roundtrip_speichert_nach_post_und_laedt_neuen_draft_geclamped() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9950;
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));
        let user_id = 47;

        let draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), user_id))
            .await;
        assert!(draft.update_message);
        fill_lfg_draft(&interface, user_id, "emissary", "phantom", "5").await;
        let reply = post_lfg_draft(&interface, LfgMode::Casual, user_id).await;
        assert!(reply.update_message);

        let raw = dl_central_db::kv::get(&pool, LFG_USER_PREF_KV_NS, &user_id.to_string())
            .await
            .expect("preset kv")
            .expect("stored preset");
        let saved: Value = serde_json::from_str(&raw).expect("preset json");
        assert_eq!(
            saved,
            json!({
                "rank_from": "emissary",
                "rank_to": "phantom",
                "slots": 5,
            })
        );

        let reopened = interface
            .handle(lfg_interaction(
                LfgMode::StreetBrawl.mode_custom_id(),
                user_id,
            ))
            .await;
        assert!(reopened.update_message);
        assert_eq!(
            reopened.content.as_deref(),
            Some(
                "**Street Brawl** · Emissary → Phantom · 4 Plätze\nWähl Rang-Bereich und Plätze, dann **Suche veröffentlichen**."
            )
        );
        let rows = reopened
            .components
            .as_ref()
            .expect("components")
            .as_array()
            .expect("component rows");
        assert_select_default_value(&rows[0], "emissary");
        assert_select_default_value(&rows[1], "phantom");
        assert_select_default_value(&rows[2], "4");

        let stored_draft = interface
            .pending_drafts
            .lock()
            .await
            .get(&user_id)
            .cloned()
            .expect("stored draft");
        assert_eq!(stored_draft.rank_from.as_deref(), Some("emissary"));
        assert_eq!(stored_draft.rank_to.as_deref(), Some("phantom"));
        assert_eq!(stored_draft.slots, Some(4));
        assert_eq!(stored_draft.lane_id, None);
    }

    #[tokio::test]
    async fn lfg_select_flow_archon_bis_phantom_slots_3_erstellt_ranked_post_mit_auto_tags() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.roles.lock().expect("roles") = vec![crate::router::VERIFIED_RANK_ROLE_IDS[0]];
        *port.next_thread_id.lock().expect("next thread") = 9904;
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let draft = interface
            .handle(lfg_interaction(LfgMode::Ranked.mode_custom_id(), 42))
            .await;
        assert!(draft.update_message);
        fill_lfg_draft(&interface, 42, "emissary", "phantom", "3").await;
        let reply = post_lfg_draft(&interface, LfgMode::Ranked, 42).await;

        assert!(reply.update_message);
        assert_eq!(reply.content.as_deref(), Some(LFG_ERFOLG_POST_ERSTELLT));
        {
            let posts = port.forum_posts.lock().expect("forum posts");
            assert_eq!(posts.len(), 1);
            assert_eq!(
                posts[0].1.applied_tags,
                vec![
                    LFG_FORUM_TAG_MODE_RANKED,
                    LFG_FORUM_TAG_RANK_EXPERIENCED,
                    LFG_FORUM_TAG_RANK_ELITE,
                    LFG_FORUM_TAG_STATUS_LOOKING,
                ]
            );
            assert!(posts[0].1.title.contains("Emissary bis Phantom"));
        }

        let row: (Option<i32>, Option<i32>, i32) = sqlx::query_as(
            "SELECT rank_min, rank_max, requested_slots
               FROM voice.lfg_posts
              WHERE thread_id = $1",
        )
        .bind(9904_i64)
        .fetch_one(&pool)
        .await
        .expect("lfg row");
        assert_eq!(row, (Some(7), Some(9), 3));
    }

    #[tokio::test]
    async fn lfg_select_flow_blockt_zweiten_offenen_post_des_owners() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9911;
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let first_draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), 44))
            .await;
        assert!(first_draft.update_message);
        fill_lfg_draft(&interface, 44, "ritualist", "phantom", "2").await;
        let first = post_lfg_draft(&interface, LfgMode::Casual, 44).await;
        assert!(first.update_message);
        *port.next_thread_id.lock().expect("next thread") = 9912;
        let second_draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), 44))
            .await;
        assert!(second_draft.update_message);
        fill_lfg_draft(&interface, 44, "ritualist", "phantom", "2").await;
        let second = post_lfg_draft(&interface, LfgMode::Casual, 44).await;

        assert!(second.ephemeral);
        assert_eq!(second.content.as_deref(), Some(LFG_ERR_SCHON_AKTIVE_SUCHE));
        assert_eq!(port.forum_posts.lock().expect("forum posts").len(), 1);
        let active_count: i64 = sqlx::query_scalar(
            "SELECT count(*)
               FROM voice.lfg_posts
              WHERE owner_id = $1
                AND status IN ('creating', 'open')",
        )
        .bind(44_i64)
        .fetch_one(&pool)
        .await
        .expect("active count");
        assert_eq!(active_count, 1);
    }

    #[tokio::test]
    async fn lfg_select_flow_loescht_reservation_wenn_discord_post_fehlschlaegt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.create_forum_post_error.lock().expect("create error") = Some("HTTP 500".to_string());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), 45))
            .await;
        assert!(draft.update_message);
        fill_lfg_draft(&interface, 45, LFG_RANK_ANY_VALUE, LFG_RANK_ANY_VALUE, "1").await;
        let reply = post_lfg_draft(&interface, LfgMode::Casual, 45).await;

        assert!(reply.ephemeral);
        assert!(port.forum_posts.lock().expect("forum posts").is_empty());
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voice.lfg_posts WHERE owner_id = $1")
                .bind(45_i64)
                .fetch_one(&pool)
                .await
                .expect("row count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn lfg_select_flow_archiviert_thread_wenn_open_update_fehlschlaegt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::query(
            "INSERT INTO voice.lfg_posts (
                 guild_id,
                 forum_channel_id,
                 thread_id,
                 starter_message_id,
                 lane_id,
                 owner_id,
                 mode,
                 rank_min,
                 rank_max,
                 requested_slots,
                 status,
                 created_at,
                 updated_at,
                 expires_at,
                 closed_at,
                 last_render_hash,
                 last_post_edit_at
             )
             VALUES (
                 $1, 777, 9913, NULL, NULL, 999, 'casual', NULL, NULL, 1, 'closed',
                 now(), now(), now() + INTERVAL '24 hours', now(), NULL, NULL
             )",
        )
        .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
        .execute(&pool)
        .await
        .expect("existing thread row");
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9913;
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let draft = interface
            .handle(lfg_interaction(LfgMode::Casual.mode_custom_id(), 46))
            .await;
        assert!(draft.update_message);
        fill_lfg_draft(&interface, 46, LFG_RANK_ANY_VALUE, LFG_RANK_ANY_VALUE, "1").await;
        let reply = post_lfg_draft(&interface, LfgMode::Casual, 46).await;

        assert!(reply.ephemeral);
        assert_eq!(
            port.archived_threads.lock().expect("archives").as_slice(),
            &[9913]
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voice.lfg_posts WHERE owner_id = $1")
                .bind(46_i64)
                .fetch_one(&pool)
                .await
                .expect("row count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn lfg_select_flow_persistiert_null_wenn_starter_fetch_fehlschlaegt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9903;
        *port.first_message_id.lock().expect("first message") = Err("HTTP 500".to_string());
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));

        let draft = interface
            .handle(lfg_interaction(LfgMode::StreetBrawl.mode_custom_id(), 43))
            .await;
        assert!(draft.update_message);
        fill_lfg_draft(&interface, 43, LFG_RANK_ANY_VALUE, LFG_RANK_ANY_VALUE, "1").await;
        let reply = post_lfg_draft(&interface, LfgMode::StreetBrawl, 43).await;

        assert!(reply.update_message);
        let starter_message_id: Option<i64> = sqlx::query_scalar(
            "SELECT starter_message_id FROM voice.lfg_posts WHERE thread_id = $1",
        )
        .bind(9903_i64)
        .fetch_one(&pool)
        .await
        .expect("starter_message_id");
        assert_eq!(starter_message_id, None);
    }

    async fn insert_open_lfg_post(
        pool: &PgPool,
        owner_id: i64,
        lane_id: Option<i64>,
        thread_id: i64,
        starter_message_id: Option<i64>,
        expires_sql: &str,
    ) -> i64 {
        let sql = format!(
            "INSERT INTO voice.lfg_posts (
                 guild_id,
                 forum_channel_id,
                 thread_id,
                 starter_message_id,
                 lane_id,
                 owner_id,
                 mode,
                 rank_min,
                 rank_max,
                 requested_slots,
                 status,
                 created_at,
                 updated_at,
                 expires_at,
                 closed_at,
                 last_render_hash,
                 last_post_edit_at
             )
             VALUES (
                 $1, 777, $2, $3, $4, $5, 'casual', NULL, NULL, 2, 'open',
                 now(), now(), {expires_sql}, NULL, 'old', NULL
             )
             RETURNING id"
        );
        sqlx::query_scalar::<_, i64>(&sql)
            .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
            .bind(thread_id)
            .bind(starter_message_id)
            .bind(lane_id)
            .bind(owner_id)
            .fetch_one(pool)
            .await
            .expect("insert open lfg post")
    }

    #[tokio::test]
    async fn lfg_open_lane_verknuepft_created_lane_race_sicher() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let spawner = Arc::new(MockLfgLaneSpawner {
            outcome: StdMutex::new(crate::router::RouterSpawnOutcome::Created { lane_id: 7777 }),
            pool: None,
            race_link: StdMutex::new(None),
            drop_posts_before_return: StdMutex::new(false),
            cleaned_lanes: StdMutex::default(),
        });
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));
        interface.set_lane_spawner(spawner).await;
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            None,
            9904,
            Some(9905),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: lfg_open_lane_custom_id(post_id),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        let lane_id: Option<i64> =
            sqlx::query_scalar("SELECT lane_id FROM voice.lfg_posts WHERE id = $1")
                .bind(post_id)
                .fetch_one(&pool)
                .await
                .expect("lane_id");
        assert_eq!(lane_id, Some(7777));
    }

    #[tokio::test]
    async fn lfg_open_lane_cleanup_bei_rows_affected_race() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            None,
            9920,
            Some(9921),
            "now() + INTERVAL '24 hours'",
        )
        .await;
        let spawner = Arc::new(MockLfgLaneSpawner {
            outcome: StdMutex::new(crate::router::RouterSpawnOutcome::Created { lane_id: 7777 }),
            pool: Some(pool.clone()),
            race_link: StdMutex::new(Some((post_id, 8888))),
            drop_posts_before_return: StdMutex::new(false),
            cleaned_lanes: StdMutex::default(),
        });
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));
        interface.set_lane_spawner(spawner.clone()).await;

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: lfg_open_lane_custom_id(post_id),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            reply.content.as_deref(),
            Some(LFG_ERR_OPEN_LANE_SCHON_VERKNUEPFT)
        );
        assert_eq!(
            spawner.cleaned_lanes.lock().expect("cleaned").as_slice(),
            &[7777]
        );
    }

    #[tokio::test]
    async fn lfg_open_lane_cleanup_bei_db_err_nach_spawn() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            None,
            9922,
            Some(9923),
            "now() + INTERVAL '24 hours'",
        )
        .await;
        let spawner = Arc::new(MockLfgLaneSpawner {
            outcome: StdMutex::new(crate::router::RouterSpawnOutcome::Created { lane_id: 7778 }),
            pool: Some(pool.clone()),
            race_link: StdMutex::new(None),
            drop_posts_before_return: StdMutex::new(true),
            cleaned_lanes: StdMutex::default(),
        });
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));
        interface.set_lane_spawner(spawner.clone()).await;

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: lfg_open_lane_custom_id(post_id),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            reply.content.as_deref(),
            Some(LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN)
        );
        assert_eq!(
            spawner.cleaned_lanes.lock().expect("cleaned").as_slice(),
            &[7778]
        );
    }

    #[tokio::test]
    async fn lfg_join_moved_targeted_in_gemappte_lane() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.voice_channel.lock().expect("voice channel") = Some(1234);
        port.occupancy.lock().expect("occupancy").insert(
            5555,
            LfgLaneOccupancy {
                member_count: 2,
                user_limit: 4,
            },
        );
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            Some(5555),
            9906,
            Some(9907),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: lfg_join_custom_id(post_id),
                guild_id: LFG_GUILD_ID,
                user_id: 99,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            port.moves.lock().expect("moves").as_slice(),
            &[(LFG_GUILD_ID, 99, 5555)]
        );
    }

    #[tokio::test]
    async fn lfg_join_blockt_volle_lane_ohne_move() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.voice_channel.lock().expect("voice channel") = Some(1234);
        port.occupancy.lock().expect("occupancy").insert(
            5556,
            LfgLaneOccupancy {
                member_count: 4,
                user_limit: 4,
            },
        );
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            Some(5556),
            9924,
            Some(9925),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: lfg_join_custom_id(post_id),
                guild_id: LFG_GUILD_ID,
                user_id: 99,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_ERR_JOIN_LANE_VOLL));
        assert!(port.moves.lock().expect("moves").is_empty());
    }

    #[tokio::test]
    async fn lfg_join_blockt_user_der_schon_in_ziel_lane_ist() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.voice_channel.lock().expect("voice channel") = Some(5557);
        port.occupancy.lock().expect("occupancy").insert(
            5557,
            LfgLaneOccupancy {
                member_count: 2,
                user_limit: 4,
            },
        );
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            Some(5557),
            9926,
            Some(9927),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: lfg_join_custom_id(post_id),
                guild_id: LFG_GUILD_ID,
                user_id: 99,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_ERR_JOIN_SCHON_DRIN));
        assert!(port.moves.lock().expect("moves").is_empty());
    }

    #[tokio::test]
    async fn lfg_join_move_403_bleibt_ephemeral_und_rendert_nicht() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.voice_channel.lock().expect("voice channel") = Some(1234);
        *port.move_error.lock().expect("move error") = Some("HTTP 403".to_string());
        port.occupancy.lock().expect("occupancy").insert(
            5558,
            LfgLaneOccupancy {
                member_count: 2,
                user_limit: 4,
            },
        );
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            Some(5558),
            9928,
            Some(9929),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: lfg_join_custom_id(post_id),
                guild_id: LFG_GUILD_ID,
                user_id: 99,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            reply.content.as_deref(),
            Some(LFG_ERR_JOIN_MOVE_FEHLGESCHLAGEN)
        );
        assert_eq!(
            port.moves.lock().expect("moves").as_slice(),
            &[(LFG_GUILD_ID, 99, 5558)]
        );
        assert!(!interface.edit_queue.pending.lock().await.contains(&post_id));
    }

    #[tokio::test]
    async fn lfg_render_update_editiert_nur_bei_hash_diff() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        port.occupancy.lock().expect("occupancy").insert(
            6666,
            LfgLaneOccupancy {
                member_count: 3,
                user_limit: 4,
            },
        );
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            Some(6666),
            9908,
            Some(9909),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        assert!(interface.render_update_once(post_id).await.expect("render"));
        {
            let edits = port.starter_edits.lock().expect("starter edits");
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].0, 9908);
            assert_eq!(edits[0].1, 9909);
            assert!(edits[0].2.contains("1/4"));
        }
        assert_eq!(
            port.tag_edits.lock().expect("tag edits").as_slice(),
            &[(
                9908,
                vec![
                    LFG_FORUM_TAG_MODE_CASUAL,
                    LFG_FORUM_TAG_RANK_ANY,
                    LFG_FORUM_TAG_STATUS_ACTIVE,
                ]
            )]
        );

        assert!(!interface
            .render_update_once(post_id)
            .await
            .expect("second render"));
        assert_eq!(port.starter_edits.lock().expect("starter edits").len(), 1);
    }

    #[tokio::test]
    async fn lfg_publish_lane_select_flow_erstellt_direkt_verknuepften_post() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::query(
            "INSERT INTO voice.tempvoice_lanes (
                 channel_id, guild_id, owner_id, base_name, category_id, source_staging_id, initial_owner_id
             )
             VALUES ($1, $2, 42, 'Chill Lane 1', $3, NULL, 42)",
        )
        .bind(7777_i64)
        .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
        .bind(i64::try_from(crate::router::mode_to_category("casual")).expect("category"))
        .execute(&pool)
        .await
        .expect("lane");
        let port = Arc::new(MockLfgPanelPort::default());
        port.categories
            .lock()
            .expect("categories")
            .insert(7777, crate::router::mode_to_category("casual"));
        port.occupancy.lock().expect("occupancy").insert(
            7777,
            LfgLaneOccupancy {
                member_count: 1,
                user_limit: 4,
            },
        );
        *port.next_thread_id.lock().expect("next thread") = 9910;
        *port.first_message_id.lock().expect("first message") = Ok(Some(9911));
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let start = interface
            .handle_publish_lane_start(
                BridgeInteraction {
                    guild_id: LFG_GUILD_ID,
                    user_id: 42,
                    ..BridgeInteraction::default()
                },
                7777,
            )
            .await;
        assert!(start.ephemeral);
        assert!(start.components.is_some());

        fill_lfg_draft(&interface, 42, LFG_RANK_ANY_VALUE, LFG_RANK_ANY_VALUE, "2").await;
        let reply = post_lfg_draft(&interface, LfgMode::Casual, 42).await;

        assert!(reply.update_message);
        {
            let posts = port.forum_posts.lock().expect("forum posts");
            assert_eq!(posts.len(), 1);
            assert_eq!(
                posts[0].1.applied_tags,
                vec![
                    LFG_FORUM_TAG_MODE_CASUAL,
                    LFG_FORUM_TAG_RANK_ANY,
                    LFG_FORUM_TAG_STATUS_ACTIVE,
                ]
            );
        }
        let lane_id: Option<i64> =
            sqlx::query_scalar("SELECT lane_id FROM voice.lfg_posts WHERE thread_id = 9910")
                .fetch_one(&pool)
                .await
                .expect("lane id");
        assert_eq!(lane_id, Some(7777));
    }

    #[tokio::test]
    async fn lfg_reconcile_schliesst_tote_und_expired_posts_und_loescht_stale_creating() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let dead = insert_open_lfg_post(
            &pool,
            42,
            Some(8888),
            9912,
            Some(9913),
            "now() + INTERVAL '24 hours'",
        )
        .await;
        let expired = insert_open_lfg_post(
            &pool,
            43,
            None,
            9914,
            Some(9915),
            "now() - INTERVAL '1 hour'",
        )
        .await;
        let stale: i64 = sqlx::query_scalar(
            "INSERT INTO voice.lfg_posts (
                 guild_id, forum_channel_id, thread_id, starter_message_id, lane_id, owner_id,
                 mode, rank_min, rank_max, requested_slots, status, created_at, updated_at,
                 expires_at, closed_at, last_render_hash, last_post_edit_at
             )
             VALUES (
                 $1, 777, NULL, NULL, NULL, 44, 'casual', NULL, NULL, 1, 'creating',
                 now() - INTERVAL '10 minutes', now() - INTERVAL '10 minutes',
                 now() + INTERVAL '24 hours', NULL, NULL, NULL
             )
             RETURNING id",
        )
        .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
        .fetch_one(&pool)
        .await
        .expect("stale creating");

        interface.reconcile_once().await;

        let dead_status: String =
            sqlx::query_scalar("SELECT status FROM voice.lfg_posts WHERE id = $1")
                .bind(dead)
                .fetch_one(&pool)
                .await
                .expect("dead status");
        let expired_status: String =
            sqlx::query_scalar("SELECT status FROM voice.lfg_posts WHERE id = $1")
                .bind(expired)
                .fetch_one(&pool)
                .await
                .expect("expired status");
        let stale_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voice.lfg_posts WHERE id = $1")
                .bind(stale)
                .fetch_one(&pool)
                .await
                .expect("stale count");
        assert_eq!(dead_status, "closed");
        assert_eq!(expired_status, "expired");
        assert_eq!(stale_count, 0);
        assert_eq!(
            port.archived_threads.lock().expect("archives").as_slice(),
            &[9912, 9914]
        );
    }

    #[tokio::test]
    async fn flag_aus_lane_delete_sink_macht_keine_lfg_arbeit() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new_with_channel_config(
            pool.clone(),
            port.clone(),
            Some(777),
            None,
            false,
        );
        let post_id = insert_open_lfg_post(
            &pool,
            42,
            Some(8890),
            9930,
            Some(9931),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        interface.on_lane_deleted(8890).await;

        let status: String = sqlx::query_scalar("SELECT status FROM voice.lfg_posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&pool)
            .await
            .expect("status");
        assert_eq!(status, "open");
        assert!(port
            .archive_attempts
            .lock()
            .expect("archive attempts")
            .is_empty());
    }

    #[tokio::test]
    async fn close_post_setzt_status_trotz_archive_404_und_500() {
        for (owner_id, thread_id, err) in [
            (50_i64, 9932_i64, "HTTP 404: Unknown Channel"),
            (51_i64, 9934_i64, "HTTP 500: Discord kaputt"),
        ] {
            let db = dl_central_db::testing::test_pool()
                .await
                .expect("test_pool");
            let pool = db.pool().clone();
            let port = Arc::new(MockLfgPanelPort::default());
            *port.archive_thread_error.lock().expect("archive error") = Some(err.to_string());
            let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
            let post_id = insert_open_lfg_post(
                &pool,
                owner_id,
                Some(8891),
                thread_id,
                Some(thread_id + 1),
                "now() + INTERVAL '24 hours'",
            )
            .await;

            interface
                .close_post(post_id, "closed")
                .await
                .expect("close best effort");

            let status: String =
                sqlx::query_scalar("SELECT status FROM voice.lfg_posts WHERE id = $1")
                    .bind(post_id)
                    .fetch_one(&pool)
                    .await
                    .expect("status");
            assert_eq!(status, "closed");
            let new_post_id = insert_open_lfg_post(
                &pool,
                owner_id,
                None,
                thread_id + 100,
                Some(thread_id + 101),
                "now() + INTERVAL '24 hours'",
            )
            .await;
            assert!(new_post_id > post_id);
            assert_eq!(
                port.archive_attempts
                    .lock()
                    .expect("archive attempts")
                    .as_slice(),
                &[thread_id as u64]
            );
        }
    }

    #[tokio::test]
    async fn reconcile_archive_fehler_schliesst_db_und_retryt_nicht_endlos() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.archive_thread_error.lock().expect("archive error") =
            Some("HTTP 500: Discord kaputt".to_string());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let post_id = insert_open_lfg_post(
            &pool,
            52,
            Some(8892),
            9936,
            Some(9937),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        interface.reconcile_once().await;
        interface.reconcile_once().await;

        let status: String = sqlx::query_scalar("SELECT status FROM voice.lfg_posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&pool)
            .await
            .expect("status");
        assert_eq!(status, "closed");
        assert_eq!(
            port.archive_attempts
                .lock()
                .expect("archive attempts")
                .as_slice(),
            &[9936]
        );
    }

    #[tokio::test]
    async fn initial_enqueue_packt_offene_lane_posts_in_render_queue() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));
        let first = insert_open_lfg_post(
            &pool,
            53,
            Some(8893),
            9938,
            Some(9939),
            "now() + INTERVAL '24 hours'",
        )
        .await;
        let second = insert_open_lfg_post(
            &pool,
            54,
            Some(8894),
            9940,
            Some(9941),
            "now() + INTERVAL '24 hours'",
        )
        .await;
        let lane_loser = insert_open_lfg_post(
            &pool,
            55,
            None,
            9942,
            Some(9943),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        interface.enqueue_open_lane_posts_for_initial_render().await;

        let pending = interface.edit_queue.pending.lock().await;
        assert!(pending.contains(&first));
        assert!(pending.contains(&second));
        assert!(!pending.contains(&lane_loser));
    }

    #[tokio::test]
    async fn reconcile_und_lane_delete_doppelclose_bleibt_idempotent() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let post_id = insert_open_lfg_post(
            &pool,
            56,
            Some(8895),
            9944,
            Some(9945),
            "now() + INTERVAL '24 hours'",
        )
        .await;

        interface.reconcile_once().await;
        interface.on_lane_deleted(8895).await;

        let status: String = sqlx::query_scalar("SELECT status FROM voice.lfg_posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&pool)
            .await
            .expect("status");
        assert_eq!(status, "closed");
        assert_eq!(
            port.archive_attempts
                .lock()
                .expect("archive attempts")
                .as_slice(),
            &[9944]
        );
    }

    type MockPost = (u64, Map<String, Value>, Vec<LfgPanelAttachment>);
    type MockEdit = (u64, u64, Map<String, Value>, Vec<LfgPanelAttachment>);
    type MockStarterEdit = (u64, u64, String);
    type MockTagEdit = (u64, Vec<u64>);

    struct MockLfgLaneSpawner {
        outcome: StdMutex<crate::router::RouterSpawnOutcome>,
        pool: Option<PgPool>,
        race_link: StdMutex<Option<(i64, u64)>>,
        drop_posts_before_return: StdMutex<bool>,
        cleaned_lanes: StdMutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl LfgLaneSpawner for MockLfgLaneSpawner {
        async fn spawn_lane_from_current_voice(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _mode: &str,
            _interaction_role_ids: &[u64],
        ) -> crate::router::RouterSpawnOutcome {
            let race_link = self.race_link.lock().expect("race link").take();
            if let (Some(pool), Some((post_id, lane_id))) = (&self.pool, race_link) {
                sqlx::query("UPDATE voice.lfg_posts SET lane_id = $2 WHERE id = $1")
                    .bind(post_id)
                    .bind(i64::try_from(lane_id).expect("lane id"))
                    .execute(pool)
                    .await
                    .expect("race link");
            }
            let drop_posts_before_return = *self
                .drop_posts_before_return
                .lock()
                .expect("drop posts before return");
            if drop_posts_before_return {
                if let Some(pool) = &self.pool {
                    sqlx::query("DROP TABLE voice.lfg_posts")
                        .execute(pool)
                        .await
                        .expect("drop lfg_posts");
                }
            }
            self.outcome.lock().expect("outcome").clone()
        }

        async fn cleanup_created_lane(
            &self,
            _guild_id: u64,
            lane_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            self.cleaned_lanes.lock().expect("cleaned").push(lane_id);
            Ok(())
        }
    }

    struct MockLfgPanelPort {
        posts: StdMutex<Vec<MockPost>>,
        edits: StdMutex<Vec<MockEdit>>,
        starter_edits: StdMutex<Vec<MockStarterEdit>>,
        tag_edits: StdMutex<Vec<MockTagEdit>>,
        not_found_edits: StdMutex<Vec<u64>>,
        recent: StdMutex<Vec<LfgPanelMessage>>,
        roles: StdMutex<Vec<u64>>,
        voice_channel: StdMutex<Option<u64>>,
        sent_dms: StdMutex<Vec<(u64, String)>>,
        moves: StdMutex<Vec<(u64, u64, u64)>>,
        move_error: StdMutex<Option<String>>,
        occupancy: StdMutex<HashMap<u64, LfgLaneOccupancy>>,
        categories: StdMutex<HashMap<u64, u64>>,
        panel_channel_kind: StdMutex<Option<LfgPanelChannelKind>>,
        forum_posts: StdMutex<Vec<(u64, LfgForumPostDraft)>>,
        next_thread_id: StdMutex<u64>,
        first_message_id: StdMutex<Result<Option<u64>, String>>,
        create_forum_post_error: StdMutex<Option<String>>,
        archive_attempts: StdMutex<Vec<u64>>,
        archived_threads: StdMutex<Vec<u64>>,
        archive_thread_error: StdMutex<Option<String>>,
    }

    impl Default for MockLfgPanelPort {
        fn default() -> Self {
            Self {
                posts: StdMutex::default(),
                edits: StdMutex::default(),
                starter_edits: StdMutex::default(),
                tag_edits: StdMutex::default(),
                not_found_edits: StdMutex::default(),
                recent: StdMutex::default(),
                roles: StdMutex::default(),
                voice_channel: StdMutex::default(),
                sent_dms: StdMutex::default(),
                moves: StdMutex::default(),
                move_error: StdMutex::default(),
                occupancy: StdMutex::default(),
                categories: StdMutex::default(),
                panel_channel_kind: StdMutex::default(),
                forum_posts: StdMutex::default(),
                next_thread_id: StdMutex::new(910_001),
                first_message_id: StdMutex::new(Ok(None)),
                create_forum_post_error: StdMutex::default(),
                archive_attempts: StdMutex::default(),
                archived_threads: StdMutex::default(),
                archive_thread_error: StdMutex::default(),
            }
        }
    }

    impl MockLfgPanelPort {
        fn new() -> Self {
            Self::default()
        }

        fn sent_dms(&self) -> Vec<(u64, String)> {
            self.sent_dms.lock().expect("sent_dms").clone()
        }
    }

    #[async_trait::async_trait]
    impl LfgPanelPort for MockLfgPanelPort {
        async fn post_rich(
            &self,
            channel_id: u64,
            body: Map<String, Value>,
            attachments: &[LfgPanelAttachment],
        ) -> Result<u64, String> {
            self.posts
                .lock()
                .expect("posts")
                .push((channel_id, body, attachments.to_vec()));
            Ok(9100 + self.posts.lock().expect("posts").len() as u64)
        }

        async fn edit_rich(
            &self,
            channel_id: u64,
            message_id: u64,
            body: Map<String, Value>,
            attachments: &[LfgPanelAttachment],
        ) -> Result<(), String> {
            if self
                .not_found_edits
                .lock()
                .expect("not found")
                .contains(&message_id)
            {
                return Err("Discord PATCH fehlgeschlagen: HTTP 404: Unknown Message".to_string());
            }
            self.edits.lock().expect("edits").push((
                channel_id,
                message_id,
                body,
                attachments.to_vec(),
            ));
            Ok(())
        }

        async fn recent_bot_messages(
            &self,
            _channel_id: u64,
            _limit: u8,
        ) -> Result<Vec<LfgPanelMessage>, String> {
            Ok(self.recent.lock().expect("recent").clone())
        }

        async fn panel_channel_kind(
            &self,
            _guild_id: u64,
            _channel_id: u64,
        ) -> Option<LfgPanelChannelKind> {
            self.panel_channel_kind
                .lock()
                .expect("panel channel kind")
                .clone()
        }

        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            self.roles.lock().expect("roles").clone()
        }

        async fn create_forum_post(
            &self,
            forum_channel_id: u64,
            draft: LfgForumPostDraft,
        ) -> Result<LfgCreatedForumPost, String> {
            if let Some(err) = self
                .create_forum_post_error
                .lock()
                .expect("create error")
                .clone()
            {
                return Err(err);
            }
            self.forum_posts
                .lock()
                .expect("forum posts")
                .push((forum_channel_id, draft));
            Ok(LfgCreatedForumPost {
                thread_id: *self.next_thread_id.lock().expect("next thread"),
            })
        }

        async fn first_thread_message_id(&self, _thread_id: u64) -> Result<Option<u64>, String> {
            self.first_message_id.lock().expect("first message").clone()
        }

        async fn archive_and_lock_thread(&self, thread_id: u64) -> Result<(), String> {
            self.archive_attempts
                .lock()
                .expect("archive attempts")
                .push(thread_id);
            if let Some(err) = self
                .archive_thread_error
                .lock()
                .expect("archive error")
                .clone()
            {
                return Err(err);
            }
            self.archived_threads
                .lock()
                .expect("archives")
                .push(thread_id);
            Ok(())
        }

        async fn edit_forum_starter_message(
            &self,
            thread_id: u64,
            starter_message_id: u64,
            body: String,
        ) -> Result<(), LfgEditError> {
            self.starter_edits.lock().expect("starter edits").push((
                thread_id,
                starter_message_id,
                body,
            ));
            Ok(())
        }

        async fn edit_forum_post_tags(
            &self,
            thread_id: u64,
            applied_tags: Vec<u64>,
        ) -> Result<(), LfgEditError> {
            self.tag_edits
                .lock()
                .expect("tag edits")
                .push((thread_id, applied_tags));
            Ok(())
        }

        async fn member_voice_channel(&self, _guild_id: u64, _user_id: u64) -> Option<u64> {
            *self.voice_channel.lock().expect("voice channel")
        }

        async fn send_dm(&self, user_id: u64, content: String) -> Result<(), String> {
            self.sent_dms
                .lock()
                .expect("sent_dms")
                .push((user_id, content));
            Ok(())
        }

        async fn move_member(
            &self,
            guild_id: u64,
            user_id: u64,
            lane_id: u64,
        ) -> Result<(), String> {
            self.moves
                .lock()
                .expect("moves")
                .push((guild_id, user_id, lane_id));
            if let Some(err) = self.move_error.lock().expect("move error").clone() {
                return Err(err);
            }
            Ok(())
        }

        async fn lane_occupancy(&self, _guild_id: u64, lane_id: u64) -> Option<LfgLaneOccupancy> {
            self.occupancy
                .lock()
                .expect("occupancy")
                .get(&lane_id)
                .copied()
        }

        async fn channel_category(&self, _guild_id: u64, channel_id: u64) -> Option<u64> {
            self.categories
                .lock()
                .expect("categories")
                .get(&channel_id)
                .copied()
        }
    }
}
