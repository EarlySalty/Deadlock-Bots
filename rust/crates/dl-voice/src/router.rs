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

use crate::tempvoice::TempVoiceEngine;

pub const ROUTER_VC_ID: u64 = 1513468587195633674;
pub const RANKED_INFO_CHANNEL_ID: u64 = 1474827277610254570;
pub const MAX_LANE_MEMBERS: usize = 6;

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
            .create_lane_from(guild_id, user_id, mode_to_staging(mode), ROUTER_VC_ID)
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
            let (mode, auto_join) = self
                .router
                .user_pref(interaction.user_id)
                .await
                .unwrap_or(("casual".to_string(), false));
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
}
