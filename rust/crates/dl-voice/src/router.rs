//! Lane-Router — Port von `cogs/tempvoice/router.py` +
//! `router_interface.py`.
//!
//! Wer den Router-VC betritt, wird nach seiner gespeicherten Präferenz
//! (ranked/casual/street_brawl + Auto-Join) in eine passende Lane gelotst:
//! bevorzugt eine Lane mit bekannten Mitspielern (Co-Spieler-Graph), sonst
//! die erste mit Platz (1–5 Mitglieder), sonst wird über die
//! TempVoice-Engine eine neue Lane im Modus erstellt. Ranked verlangt eine
//! verifizierte Rang-Rolle (sonst DM-Hinweis).
//!
//! Panel-Buttons mit Original-custom_ids: `router_mode_{mode}` +
//! `router_autojoin_toggle`. Bewusste Lücke (dokumentiert): der
//! New-Player-Routing-Hook (eigene Anfänger-Kategorie) — das Original
//! behandelt einen fehlenden Hook identisch (kein Reroute).

use std::sync::Arc;

use dl_db::Db;
use dl_discord::{
    BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler, InteractionRouter, VoiceEvent,
};
use rusqlite::OptionalExtension;
use serde_json::{json, Map, Value};

use crate::tempvoice::TempVoiceEngine;

pub const ROUTER_VC_ID: u64 = 1513468587195633674;
pub const ROUTER_TEXT_CHANNEL_ID: u64 = 1513468476365209670;
pub const RANKED_INFO_CHANNEL_ID: u64 = 1474827277610254570;
pub const MAX_LANE_MEMBERS: usize = 6;
pub const ROUTER_PANEL_KV_NS: &str = "tempvoice_router";
pub const ROUTER_GUIDE_MESSAGE_KEY: &str = "guide_message_id";
pub const ROUTER_INTERFACE_MESSAGE_KEY: &str = "interface_message_id";
pub const ROUTER_SELECT_MODE_BEFORE_AUTOJOIN: &str = "Wähle zuerst einen Spielmodus.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterPanelMessage {
    pub message_id: u64,
    pub has_embeds: bool,
    pub has_components: bool,
}

/// Die 11 Haupt-Rang-Rollen (wie VERIFIED_RANK_ROLE_IDS).
pub const VERIFIED_RANK_ROLE_IDS: [u64; 11] = [
    1331457571118387210,
    1331457652877955072,
    1331457699992436829,
    1331457724848017539,
    1331457879345070110,
    1331457898781474836,
    1331457949654319114,
    1316966867033653338,
    1331458016356208680,
    1331458049637875785,
    1331458087349129296,
];

pub fn mode_to_category(mode: &str) -> u64 {
    match mode {
        "ranked" => 1412804540994162789,
        "street_brawl" => 1357422957017698478,
        _ => 1289721245281292290, // casual
    }
}

pub fn mode_to_staging(mode: &str) -> u64 {
    match mode {
        "ranked" => 1412804671432818890,
        "street_brawl" => 1357422958544420944,
        _ => 1501089974093873232,
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

fn router_guide_embed() -> Value {
    json!({
        "title": "📖 Router — Wie funktioniert das?",
        "color": 0x2B2D31,
        "fields": [
            {
                "name": "1️⃣  Modus wählen",
                "value": concat!(
                    "Klick unten auf **Casual**, **Ranked** oder **Street Brawl**. ",
                    "Deine Wahl wird gespeichert und gilt für alle zukünftigen Joins."
                ),
                "inline": false,
            },
            {
                "name": "2️⃣  Auto-Join",
                "value": concat!(
                    "**Aus (grau)** → du bekommst immer eine eigene, leere Lane.\n",
                    "**An (grün)** → der Bot sucht eine passende Lane mit freien Plätzen (<6 Personen). ",
                    "Bekannte Mitspieler werden dabei bevorzugt. ",
                    "Gibt es keine freie Lane, wird eine neue für dich erstellt."
                ),
                "inline": false,
            },
            {
                "name": "3️⃣  Router-VC betreten",
                "value": concat!(
                    "Sobald du den **Deadlock Router**-Sprachkanal betrittst, passiert alles automatisch:\n",
                    "• Neuer Spieler → New-Player-Lane\n",
                    "• Kein Modus gesetzt → du bleibst im Router-VC bis du unten einen wählst\n",
                    "• Modus gesetzt, Auto-Join aus → sofort eigene Lane\n",
                    "• Modus gesetzt, Auto-Join an → Smart Routing"
                ),
                "inline": false,
            },
            {
                "name": "🏆  Ranked",
                "value": concat!(
                    "Ranked-Lanes erfordern einen **verifizierten Rang** (Steam-Verknüpfung). ",
                    "Ohne Rang bekommst du eine DM mit dem Link zur Verifizierung — ",
                    "du bleibst dann im Router-VC und kannst danach Casual oder Street Brawl wählen."
                ),
                "inline": false,
            },
            {
                "name": "🔄  Lane-Modus wechseln",
                "value": concat!(
                    "Als Lane-Owner kannst du deinen aktiven Kanal nachträglich umstellen: ",
                    "im Lane-Control-Interface gibt es **Modus wechseln** (Casual / Ranked / Street Brawl / Off Topic) ",
                    "und **Umbenennen**. ",
                    "Der Kanal zieht dabei physisch in die passende Kategorie um."
                ),
                "inline": false,
            },
        ],
        "footer": {
            "text": "Die alten Staging-Kanäle (Casual / Comp / Street Brawl) laufen weiterhin parallel."
        },
    })
}

fn router_interface_embed() -> Value {
    json!({
        "title": "🎮 Spielmodus wählen",
        "description": concat!(
            "Wähle deinen **Standard-Spielmodus** und stelle den Auto-Join-Toggle ein.\n\n",
            "**Auto-Join aus** → eigene Lane wird für dich erstellt\n",
            "**Auto-Join an** → Smart Routing in eine passende Lane"
        ),
        "color": 0x5865F2,
    })
}

fn router_guide_body() -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("embeds".to_string(), json!([router_guide_embed()]));
    body
}

fn router_interface_body() -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("embeds".to_string(), json!([router_interface_embed()]));
    body.insert(
        "components".to_string(),
        json!([
            router_action_row(vec![
                router_button("🎮 Casual", 2, "router_mode_casual"),
                router_button("🏆 Ranked", 2, "router_mode_ranked"),
                router_button("⚡ Street Brawl", 2, "router_mode_street_brawl"),
            ]),
            router_action_row(vec![json!({
                "type": 2,
                "style": 2,
                "label": "Auto-Join",
                "emoji": {"name": "⬜"},
                "custom_id": "router_autojoin_toggle",
            })]),
        ]),
    );
    body
}

fn router_action_row(components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "components": components,
    })
}

fn router_button(label: &str, style: u8, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
    })
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
    async fn post_rich(&self, channel_id: u64, body: Map<String, Value>) -> Result<u64, String>;
    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), String>;
    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<RouterPanelMessage>, String>;
}

pub struct RouterInterface {
    db: Db,
    port: Arc<dyn RouterInterfacePort>,
}

impl RouterInterface {
    pub fn new(db: Db, port: Arc<dyn RouterInterfacePort>) -> Arc<Self> {
        Arc::new(Self { db, port })
    }

    pub async fn ensure_panel(&self) {
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
        let guide_message_id = recent.as_deref().and_then(find_existing_router_guide);
        let interface_message_id = recent.as_deref().and_then(find_existing_router_interface);
        self.upsert_panel_message(
            ROUTER_GUIDE_MESSAGE_KEY,
            router_guide_body(),
            guide_message_id,
            history_checked,
        )
        .await;
        self.upsert_panel_message(
            ROUTER_INTERFACE_MESSAGE_KEY,
            router_interface_body(),
            interface_message_id,
            history_checked,
        )
        .await;
    }

    async fn upsert_panel_message(
        &self,
        key: &str,
        body: Map<String, Value>,
        history_message_id: Option<u64>,
        history_checked: bool,
    ) {
        let kv_message_id = self.panel_message_id(key).await;
        if let Some(message_id) = kv_message_id {
            if self
                .port
                .edit_rich(ROUTER_TEXT_CHANNEL_ID, message_id, body.clone())
                .await
                .is_ok()
            {
                return;
            }
            if history_message_id == Some(message_id) {
                tracing::warn!(
                    key,
                    message_id,
                    "RouterInterface: vorhandenes Panel konnte nicht editiert werden"
                );
                return;
            }
        }

        if !history_checked {
            tracing::warn!(
                key,
                "RouterInterface: ohne erfolgreichen History-Scan wird kein neues Panel gepostet"
            );
            return;
        }

        if let Some(message_id) = history_message_id {
            match self
                .port
                .edit_rich(ROUTER_TEXT_CHANNEL_ID, message_id, body.clone())
                .await
            {
                Ok(()) => {
                    self.store_panel_message_id(key, message_id).await;
                    return;
                }
                Err(err) => {
                    tracing::warn!(%err, key, message_id, "RouterInterface: adoptiertes Panel konnte nicht editiert werden");
                    return;
                }
            }
        }

        match self.port.post_rich(ROUTER_TEXT_CHANNEL_ID, body).await {
            Ok(message_id) => {
                self.store_panel_message_id(key, message_id).await;
            }
            Err(err) => {
                tracing::warn!(%err, key, "RouterInterface: Panel konnte nicht gepostet werden")
            }
        }
    }

    async fn panel_message_id(&self, key: &str) -> Option<u64> {
        self.db
            .kv_get(ROUTER_PANEL_KV_NS, key)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
    }

    async fn store_panel_message_id(&self, key: &str, message_id: u64) {
        if let Err(err) = self
            .db
            .kv_set(ROUTER_PANEL_KV_NS, key, message_id.to_string())
            .await
        {
            tracing::warn!(%err, key, "RouterInterface: Message-ID konnte nicht gespeichert werden");
        }
    }
}

fn find_existing_router_guide(messages: &[RouterPanelMessage]) -> Option<u64> {
    messages
        .iter()
        .find(|message| message.has_embeds && !message.has_components)
        .map(|message| message.message_id)
}

fn find_existing_router_interface(messages: &[RouterPanelMessage]) -> Option<u64> {
    messages
        .iter()
        .find(|message| message.has_embeds && message.has_components)
        .map(|message| message.message_id)
}

pub struct LaneRouter {
    pub db: Db,
    pub port: Arc<dyn RouterPort>,
    pub engine: Arc<TempVoiceEngine>,
    pub analyzer: Option<Arc<dl_activity::analyzer::ActivityAnalyzer>>,
}

impl LaneRouter {
    pub fn new(
        db: Db,
        port: Arc<dyn RouterPort>,
        engine: Arc<TempVoiceEngine>,
        analyzer: Option<Arc<dl_activity::analyzer::ActivityAnalyzer>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            port,
            engine,
            analyzer,
        })
    }

    pub async fn ensure_schema(&self) -> Result<(), dl_db::DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS router_user_prefs (
                        user_id    INTEGER PRIMARY KEY,
                        mode       TEXT NOT NULL CHECK(mode IN ('ranked', 'casual', 'street_brawl')),
                        auto_join  INTEGER NOT NULL DEFAULT 0,
                        updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    );",
                )
            })
            .await
    }

    pub async fn user_pref(&self, user_id: u64) -> Option<(String, bool)> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT mode, auto_join FROM router_user_prefs WHERE user_id = ?1",
                    [user_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    pub async fn set_user_pref(&self, user_id: u64, mode: &str, auto_join: bool) {
        let mode = mode.to_string();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO router_user_prefs (user_id, mode, auto_join, updated_at)
                     VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
                     ON CONFLICT(user_id) DO UPDATE SET
                       mode = excluded.mode, auto_join = excluded.auto_join,
                       updated_at = CURRENT_TIMESTAMP",
                    rusqlite::params![user_id, mode, i64::from(auto_join)],
                )
                .map(|_| ())
            })
            .await;
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
        let Some((mode, auto_join)) = self.user_pref(user_id).await else {
            return; // ohne Präferenz: User bleibt im Router-VC (Panel hilft)
        };
        if !auto_join {
            self.engine
                .create_router_lane(guild_id, user_id, &mode, ROUTER_VC_ID)
                .await;
            return;
        }
        self.smart_route(guild_id, user_id, &mode).await;
    }

    /// Wie `_smart_route`: Ranked-Gate → Co-Spieler-Lane → erste passende
    /// → neue Lane über die TempVoice-Engine.
    pub async fn smart_route(self: &Arc<Self>, guild_id: u64, user_id: u64, mode: &str) {
        if mode == "ranked" {
            let roles = self.port.member_role_ids(guild_id, user_id).await;
            if !roles.iter().any(|r| VERIFIED_RANK_ROLE_IDS.contains(r)) {
                self.port
                    .send_dm(
                        user_id,
                        format!(
                            "Für Ranked-Lanes musst du deinen Rang verifizieren. Mehr Infos: <#{RANKED_INFO_CHANNEL_ID}>"
                        ),
                    )
                    .await;
                return;
            }
        }
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

/// Panel-Buttons: router_mode_{mode} + router_autojoin_toggle.
struct RouterPanelHandler {
    router: Arc<LaneRouter>,
}

#[async_trait::async_trait]
impl InteractionHandler for RouterPanelHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
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
        let mode = match mode {
            "ranked" | "casual" | "street_brawl" => mode.to_string(),
            _ => return BridgeReply::ephemeral_text("Unbekannter Modus."),
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
    router_panel.on_prefix("router_mode_", handler.clone());
    router_panel.on_custom_id("router_autojoin_toggle", handler);
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
    use super::*;
    use serde_json::Map;
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
        assert_eq!(mode_to_category("ranked"), 1412804540994162789);
        assert_eq!(mode_to_category("street_brawl"), 1357422957017698478);
        assert_eq!(mode_to_category("casual"), 1289721245281292290);
        assert_eq!(mode_to_category("quatsch"), 1289721245281292290);
        assert_eq!(mode_to_staging("ranked"), 1412804671432818890);
    }

    #[test]
    fn router_texte_sind_final() {
        assert_ne!(ROUTER_SELECT_MODE_BEFORE_AUTOJOIN, "Platzhalter");
        assert!(!ROUTER_SELECT_MODE_BEFORE_AUTOJOIN.is_empty());
    }

    #[test]
    fn router_panel_body_enthaelt_python_texte_und_buttons() {
        let guide = router_guide_embed();
        assert_eq!(guide["title"], "📖 Router — Wie funktioniert das?");
        assert_eq!(guide["fields"][0]["name"], "1️⃣  Modus wählen");
        assert!(guide["fields"][1]["value"]
            .as_str()
            .expect("value")
            .contains("**Aus (grau)** → du bekommst immer eine eigene, leere Lane."));
        assert!(guide["fields"][2]["value"]
            .as_str()
            .expect("value")
            .contains("• Modus gesetzt, Auto-Join an → Smart Routing"));

        let body = router_interface_body();
        let embed = &body
            .get("embeds")
            .and_then(serde_json::Value::as_array)
            .expect("embeds")[0];
        assert_eq!(embed["title"], "🎮 Spielmodus wählen");
        assert!(embed["description"]
            .as_str()
            .expect("description")
            .contains("**Auto-Join aus** → eigene Lane wird für dich erstellt"));
        let custom_ids: Vec<&str> = body
            .get("components")
            .and_then(serde_json::Value::as_array)
            .expect("components")
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert_eq!(
            custom_ids,
            vec![
                "router_mode_casual",
                "router_mode_ranked",
                "router_mode_street_brawl",
                "router_autojoin_toggle"
            ]
        );
    }

    #[derive(Default)]
    struct MockRouterInterfacePort {
        posts: StdMutex<Vec<(u64, Map<String, serde_json::Value>)>>,
        edits: StdMutex<Vec<(u64, u64, Map<String, serde_json::Value>)>>,
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
        ) -> Result<u64, String> {
            self.posts.lock().expect("posts").push((channel_id, body));
            Ok(9000 + self.posts.lock().expect("posts").len() as u64)
        }

        async fn edit_rich(
            &self,
            channel_id: u64,
            message_id: u64,
            body: Map<String, serde_json::Value>,
        ) -> Result<(), String> {
            if *self.fail_edits.lock().expect("fail") {
                return Err("missing".to_string());
            }
            self.edits
                .lock()
                .expect("edits")
                .push((channel_id, message_id, body));
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

    #[tokio::test]
    async fn router_interface_panel_wird_idempotent_ueber_kv_gepflegt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("router.sqlite3")).expect("db");
        db.write(|conn| {
            conn.execute(
                "CREATE TABLE kv_store(
                  ns TEXT NOT NULL,
                  k  TEXT NOT NULL,
                  v  TEXT NOT NULL,
                  PRIMARY KEY(ns, k)
                )",
                [],
            )
            .map(|_| ())
        })
        .await
        .expect("kv");
        let port = Arc::new(MockRouterInterfacePort::default());
        let interface = RouterInterface::new(db.clone(), port.clone());

        interface.ensure_panel().await;
        assert_eq!(port.posts.lock().expect("posts").len(), 2);
        assert_eq!(
            db.kv_get(ROUTER_PANEL_KV_NS, ROUTER_GUIDE_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9001")
        );
        assert_eq!(
            db.kv_get(ROUTER_PANEL_KV_NS, ROUTER_INTERFACE_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9002")
        );

        interface.ensure_panel().await;
        assert_eq!(port.posts.lock().expect("posts").len(), 2);
        assert_eq!(port.edits.lock().expect("edits").len(), 2);

        *port.fail_edits.lock().expect("fail") = true;
        interface.ensure_panel().await;
        assert_eq!(port.posts.lock().expect("posts").len(), 4);
    }

    #[tokio::test]
    async fn router_interface_adoptiert_python_panels_aus_history_bei_leerer_kv() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("router.sqlite3")).expect("db");
        db.write(|conn| {
            conn.execute(
                "CREATE TABLE kv_store(
                  ns TEXT NOT NULL,
                  k  TEXT NOT NULL,
                  v  TEXT NOT NULL,
                  PRIMARY KEY(ns, k)
                )",
                [],
            )
            .map(|_| ())
        })
        .await
        .expect("kv");
        let port = Arc::new(MockRouterInterfacePort::default());
        *port.recent.lock().expect("recent") = vec![
            RouterPanelMessage {
                message_id: 7001,
                has_embeds: true,
                has_components: false,
            },
            RouterPanelMessage {
                message_id: 7002,
                has_embeds: true,
                has_components: true,
            },
        ];
        let interface = RouterInterface::new(db.clone(), port.clone());

        interface.ensure_panel().await;

        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        let edits = port.edits.lock().expect("edits");
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].1, 7001);
        assert_eq!(edits[1].1, 7002);
        assert_eq!(
            db.kv_get(ROUTER_PANEL_KV_NS, ROUTER_GUIDE_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("7001")
        );
        assert_eq!(
            db.kv_get(ROUTER_PANEL_KV_NS, ROUTER_INTERFACE_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("7002")
        );
    }

    #[tokio::test]
    async fn router_interface_postet_nicht_blind_wenn_history_scan_fehlgeschlagen_ist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("router.sqlite3")).expect("db");
        db.write(|conn| {
            conn.execute(
                "CREATE TABLE kv_store(
                  ns TEXT NOT NULL,
                  k  TEXT NOT NULL,
                  v  TEXT NOT NULL,
                  PRIMARY KEY(ns, k)
                )",
                [],
            )
            .map(|_| ())
        })
        .await
        .expect("kv");
        let port = Arc::new(MockRouterInterfacePort::default());
        *port.fail_recent.lock().expect("fail recent") = true;
        let interface = RouterInterface::new(db, port.clone());

        interface.ensure_panel().await;

        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
    }
}
