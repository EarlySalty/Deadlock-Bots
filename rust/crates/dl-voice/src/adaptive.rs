//! Adaptive Spezial-Lanes — Port von `cogs/tempvoice/new_player_lanes.py`,
//! `duo_lanes.py` und `lane_sorting.py` (die letzten Voice-Lücken).
//!
//! Drei Subsysteme:
//! - **Neue-Spieler-Lanes**: Anfänger (Rang ≤ Arcanist, verifiziert oder
//!   Unverifiziert-Rolle) werden beim Staging-Join in die Anfänger-Kategorie
//!   umgeleitet; Lanes wachsen ab 6 Mitgliedern nach (Anker + „🆕Neue
//!   Spieler Lane N"), mit 4-Minuten-Rückkehr-Fenster zum normalen Flow.
//! - **Duo-Lanes**: „🗨️Off Topic Voice" bekommt ab 2 Personen im Anker
//!   genau eine zweite Lane; Leere werden abgebaut.
//! - **Lane-Sortierung**: Chill-Lanes werden nach ihrem Rang-Label
//!   (Haupt- + Unterrang) auf die vorhandenen Slots sortiert.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::tempvoice::logic;
use crate::tempvoice::TempVoiceEngine;
use dl_discord::GatewayEvent;

pub const NP_ANCHOR_CHANNEL_ID: u64 = 1470126503252721845;
pub const NP_LANE_BASE_NAME: &str = "🆕Neue Spieler Lane";
pub const NP_EXPAND_THRESHOLD: usize = 6;
pub const NP_MAX_RANK_VALUE: i64 = 4;
pub const RETURN_WINDOW_SECONDS: i64 = 4 * 60;

pub const DUO_ANCHOR_CHANNEL_ID: u64 = 1411391356278018245;
pub const DUO_LANE_BASE_NAME: &str = "🗨️Off Topic Voice";
pub const DUO_EXPAND_THRESHOLD: usize = 2;

/// Unverifizierte Anfänger-Rollen → Rang-Wert.
pub const UNVERIFIED_RANK_ROLES: [(u64, i64); 4] = [
    (1492960891619250408, 1), // Initiate (unverifiziert)
    (1492959966284218611, 2), // Seeker (unverifiziert)
    (1492960350755225730, 3), // Alchemist (unverifiziert)
    (1492960274096066831, 4), // Arcanist (unverifiziert)
];

/// Stagings, aus denen Anfänger umgeleitet werden (Casual + Comp).
pub const ELIGIBLE_STAGING_IDS: [u64; 2] = [1501089974093873232, 1412804671432818890];
pub const MAIN_GUILD_ID: u64 = 1289721245281292288;
pub const CHILL_CATEGORY_ID: u64 = 1289721245281292290;
pub const COMP_RANKED_CATEGORY_ID: u64 = 1412804540994162789;
pub const CASUAL_STAGING_ID: u64 = 1501089974093873232;
pub const PERMANENT_CHILL_ID: u64 = 1493690350580138114;
pub const PINNED_CHILL_END_ID: u64 = 1505618194017161267;

async fn wait_for_cache_ready(
    events: &mut tokio::sync::broadcast::Receiver<GatewayEvent>,
    guild_id: u64,
) {
    loop {
        match events.recv().await {
            Ok(GatewayEvent::CacheReady { guild_ids }) => {
                if guild_ids.contains(&guild_id) {
                    return;
                }
            }
            Ok(GatewayEvent::Ready { .. }) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

// ── Pure Logik (Referenzwerte aus CPython in den Tests) ────────────────────

/// Anfänger-Rang aus Rollen: verifizierte Rang-Rollen zuerst (höchster
/// Score), sonst die höchste Unverifiziert-Rolle; None = kein Anfänger-Rang.
pub fn resolve_new_player_rank(roles: &[(u64, String)]) -> Option<i64> {
    let names: Vec<String> = roles.iter().map(|(_, name)| name.clone()).collect();
    let verified = logic::member_rank_index(&names) as i64;
    if verified > 0 {
        return Some(verified);
    }
    let role_ids: HashSet<u64> = roles.iter().map(|(id, _)| *id).collect();
    UNVERIFIED_RANK_ROLES
        .iter()
        .filter(|(role_id, _)| role_ids.contains(role_id))
        .map(|(_, rank)| *rank)
        .max()
}

pub fn lane_name_for_index(base: &str, index: usize) -> String {
    if index <= 1 {
        base.to_string()
    } else {
        format!("{base} {index}")
    }
}

/// Index einer verwalteten Lane: Anker = 1, sonst „Basis N" (N ≥ 2).
pub fn parse_lane_index(channel_id: u64, anchor_id: u64, base: &str, name: &str) -> Option<usize> {
    if channel_id == anchor_id {
        return Some(1);
    }
    let rest = name.trim().strip_prefix(base)?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || !rest[digits.len()..].is_empty() {
        return None;
    }
    let index: usize = digits.parse().ok()?;
    (index >= 2).then_some(index)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSnapshot {
    pub channel_id: u64,
    pub current_index: usize,
    pub member_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanePlan {
    /// (channel_id, neuer Index) — Umbenennungen bestehender Lanes.
    pub reassignments: Vec<(u64, usize)>,
    pub delete_ids: Vec<u64>,
    pub create_indices: Vec<usize>,
}

/// Wie `plan_managed_lanes` (Neue-Spieler): volle Lanes erzwingen eine
/// weitere, besetzte rutschen vor leere, Überschuss wird gelöscht.
pub fn plan_lanes(
    anchor_member_count: usize,
    snapshots: &[LaneSnapshot],
    expand_threshold: usize,
    max_total: Option<usize>,
) -> LanePlan {
    let mut occupied: Vec<&LaneSnapshot> =
        snapshots.iter().filter(|s| s.member_count > 0).collect();
    occupied.sort_by_key(|s| (s.current_index, s.channel_id));
    let mut empty: Vec<&LaneSnapshot> = snapshots.iter().filter(|s| s.member_count == 0).collect();
    empty.sort_by_key(|s| (s.current_index, s.channel_id));

    let desired_total = match max_total {
        // Duo-Variante: ab Schwelle im Anker genau 2, sonst 1
        Some(_) => {
            if anchor_member_count >= expand_threshold {
                2
            } else {
                1
            }
        }
        None => {
            let mut highest_occupied = usize::from(anchor_member_count > 0);
            if !occupied.is_empty() {
                highest_occupied = 1 + occupied.len();
            }
            let mut highest_full = usize::from(anchor_member_count >= expand_threshold);
            for (offset, snapshot) in occupied.iter().enumerate() {
                if snapshot.member_count >= expand_threshold {
                    highest_full = highest_full.max(offset + 2);
                }
            }
            [
                1,
                highest_occupied,
                if highest_full > 0 {
                    highest_full + 1
                } else {
                    1
                },
            ]
            .into_iter()
            .max()
            .unwrap_or(1)
        }
    };
    let extras_to_keep = desired_total.saturating_sub(1);

    let fill_from_empty = extras_to_keep.saturating_sub(occupied.len());
    let kept: Vec<&LaneSnapshot> = occupied
        .iter()
        .chain(empty.iter().take(fill_from_empty))
        .copied()
        .collect();
    LanePlan {
        reassignments: kept
            .iter()
            .enumerate()
            .map(|(offset, snapshot)| (snapshot.channel_id, offset + 2))
            .collect(),
        delete_ids: empty
            .iter()
            .skip(fill_from_empty)
            .map(|snapshot| snapshot.channel_id)
            .collect(),
        create_indices: ((kept.len() + 2)..=desired_total).collect(),
    }
}

/// Rang-Label einer Lane → (Hauptrang-Index, Unterrang) wie parse_rank_label.
pub fn parse_rank_label(label: &str) -> (usize, i64) {
    let trimmed = label.trim();
    let lower = trimmed.to_lowercase();
    for rank in logic::RANK_ORDER.iter().skip(1) {
        if let Some(rest) = lower.strip_prefix(rank) {
            // Wortgrenze: Ende, Leerzeichen oder Nicht-Alphanumerisches
            if rest.chars().next().map(|c| c.is_ascii_alphanumeric()) == Some(true) {
                continue;
            }
            let rank_index = logic::rank_index(rank);
            let subrank = rest
                .trim_start()
                .chars()
                .next()
                .filter(char::is_ascii_digit)
                .and_then(|c| c.to_digit(10))
                .map(|d| d as i64)
                .filter(|d| (1..=6).contains(d))
                .unwrap_or(0);
            return (rank_index, subrank);
        }
    }
    (0, 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSnapshot {
    pub lane_id: u64,
    pub current_position: i64,
    pub rank_index: usize,
    pub subrank: i64,
    pub stable_order: usize,
}

/// Zielpositionen wie `plan_lane_reorder`: nach (Rang, Unterrang,
/// Ursprungs-Reihenfolge) auf die vorhandenen Slots verteilt.
pub fn plan_lane_reorder(entries: &[SortSnapshot]) -> Vec<(u64, i64)> {
    if entries.len() <= 1 {
        return Vec::new();
    }
    let mut slot_positions: Vec<i64> = entries.iter().map(|e| e.current_position).collect();
    slot_positions.sort_unstable();
    let mut ordered: Vec<&SortSnapshot> = entries.iter().collect();
    ordered.sort_by_key(|e| (e.rank_index, e.subrank, e.stable_order, e.lane_id));
    ordered
        .iter()
        .zip(slot_positions)
        .filter(|(entry, target)| entry.current_position != *target)
        .map(|(entry, target)| (entry.lane_id, target))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneSortBlock {
    Ranked,
    Casual,
    StreetBrawl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedSortSnapshot {
    pub lane_id: u64,
    pub current_position: i64,
    pub block: LaneSortBlock,
    pub rank_index: usize,
    pub subrank: i64,
    pub stable_order: usize,
}

fn block_order(block: LaneSortBlock) -> usize {
    match block {
        LaneSortBlock::Ranked => 0,
        LaneSortBlock::Casual => 1,
        LaneSortBlock::StreetBrawl => 2,
    }
}

pub fn plan_unified_category_reorder(entries: &[UnifiedSortSnapshot]) -> Vec<(u64, i64)> {
    if entries.len() <= 1 {
        return Vec::new();
    }
    let mut slot_positions: Vec<i64> = entries.iter().map(|entry| entry.current_position).collect();
    slot_positions.sort_unstable();
    let mut ordered: Vec<&UnifiedSortSnapshot> = entries.iter().collect();
    ordered.sort_by_key(|entry| {
        let rank_index = if entry.block == LaneSortBlock::Ranked && entry.rank_index == 0 {
            usize::MAX
        } else {
            entry.rank_index
        };
        (
            block_order(entry.block),
            rank_index,
            entry.subrank,
            entry.stable_order,
            entry.lane_id,
        )
    });
    ordered
        .iter()
        .zip(slot_positions)
        .filter(|(entry, target)| entry.current_position != *target)
        .map(|(entry, target)| (entry.lane_id, target))
        .collect()
}

fn sort_snapshot_from_name(
    lane_id: u64,
    name: &str,
    current_position: i64,
    stable_order: usize,
) -> Option<UnifiedSortSnapshot> {
    let lower = name.trim().to_lowercase();
    let (block, rank_label) = if lower == "ranked" || lower.starts_with("ranked ") {
        (
            LaneSortBlock::Ranked,
            lower.strip_prefix("ranked").unwrap_or("").trim(),
        )
    } else if lower == "street brawl" || lower.starts_with("street brawl ") {
        (LaneSortBlock::StreetBrawl, "")
    } else if lower == "chill lane" || lower.starts_with("chill lane ") {
        (LaneSortBlock::Casual, "")
    } else {
        return None;
    };
    let (rank_index, subrank) = if block == LaneSortBlock::Ranked {
        parse_rank_label(rank_label)
    } else {
        (0, 0)
    };
    Some(UnifiedSortSnapshot {
        lane_id,
        current_position,
        block,
        rank_index,
        subrank,
        stable_order,
    })
}

// ── Engine ─────────────────────────────────────────────────────────────────

/// Discord-Seite (Tests mocken sie).
#[async_trait::async_trait]
pub trait AdaptivePort: Send + Sync {
    /// Voice-Kanäle einer Kategorie: (id, name, member_count, position).
    async fn category_channels(
        &self,
        guild_id: u64,
        category_id: u64,
    ) -> Vec<(u64, String, usize, i64)>;
    async fn member_role_pairs(&self, guild_id: u64, user_id: u64) -> Vec<(u64, String)>;
    async fn move_member(&self, guild_id: u64, user_id: u64, channel_id: u64)
        -> Result<(), String>;
    async fn rename_channel(&self, channel_id: u64, name: &str) -> Result<(), String>;
    async fn delete_channel(&self, channel_id: u64) -> Result<(), String>;
    async fn create_voice_channel(
        &self,
        guild_id: u64,
        category_id: u64,
        anchor_id: u64,
        name: &str,
    ) -> Result<u64, String>;
    async fn set_channel_position(&self, channel_id: u64, position: i64) -> Result<(), String>;
    async fn set_channel_positions(
        &self,
        guild_id: u64,
        positions: &[(u64, i64)],
    ) -> Result<(), String>;
}

pub struct AdaptiveLanes {
    pub port: Arc<dyn AdaptivePort>,
    routed_at: tokio::sync::Mutex<HashMap<u64, i64>>,
}

impl AdaptiveLanes {
    pub fn new(port: Arc<dyn AdaptivePort>) -> Arc<Self> {
        Arc::new(Self {
            port,
            routed_at: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub async fn set_tempvoice(&self, _engine: Arc<TempVoiceEngine>) {
        // Sortierung liest den Rang seit dem Ein-Kategorie-Umbau aus dem Namen.
    }

    /// Anfänger beim Staging-Join umleiten (wie `maybe_route_new_player`):
    /// true = umgeleitet, der normale Join-to-create-Flow entfällt.
    pub async fn maybe_route_new_player(
        &self,
        guild_id: u64,
        user_id: u64,
        staging_id: u64,
    ) -> bool {
        if !ELIGIBLE_STAGING_IDS.contains(&staging_id) {
            return false;
        }
        let now = chrono::Utc::now().timestamp();
        {
            let routed = self.routed_at.lock().await;
            if routed.contains_key(&user_id) {
                return false;
            }
        }
        let roles = self.port.member_role_pairs(guild_id, user_id).await;
        let Some(rank) = resolve_new_player_rank(&roles) else {
            return false;
        };
        if rank > NP_MAX_RANK_VALUE {
            return false;
        }
        // Ziel: vollste verwaltete Lane unter der Schwelle (Anker zählt mit)
        let channels = self
            .port
            .category_channels(guild_id, CHILL_CATEGORY_ID)
            .await;
        let mut candidates: Vec<&(u64, String, usize, i64)> = channels
            .iter()
            .filter(|(id, name, members, _)| {
                *members < NP_EXPAND_THRESHOLD
                    && parse_lane_index(*id, NP_ANCHOR_CHANNEL_ID, NP_LANE_BASE_NAME, name)
                        .is_some()
            })
            .collect();
        if candidates.is_empty() {
            return false; // kein Platz → normaler Flow (wie Original-Fallback)
        }
        candidates.sort_by_key(|(id, _, members, position)| {
            (std::cmp::Reverse(*members), *position, *id)
        });
        let target = candidates[0].0;
        if self
            .port
            .move_member(guild_id, user_id, target)
            .await
            .is_err()
        {
            return false;
        }
        self.routed_at.lock().await.insert(user_id, now);
        self.sync_new_player(guild_id).await;
        true
    }

    /// Neue-Spieler-Kategorie nachziehen (wie `_sync_guild`).
    pub async fn sync_new_player(&self, guild_id: u64) {
        self.sync_managed(
            guild_id,
            CHILL_CATEGORY_ID,
            NP_ANCHOR_CHANNEL_ID,
            NP_LANE_BASE_NAME,
            NP_EXPAND_THRESHOLD,
            None,
        )
        .await;
    }

    /// Duo-Lanes nachziehen.
    pub async fn sync_duo(&self, guild_id: u64) {
        let Some(category) = self.anchor_category(guild_id, DUO_ANCHOR_CHANNEL_ID).await else {
            return;
        };
        self.sync_managed(
            guild_id,
            category,
            DUO_ANCHOR_CHANNEL_ID,
            DUO_LANE_BASE_NAME,
            DUO_EXPAND_THRESHOLD,
            Some(2),
        )
        .await;
    }

    async fn anchor_category(&self, _guild_id: u64, _anchor_id: u64) -> Option<u64> {
        // Duo-Anker liegt in der Chill-Kategorie (Fixed-Lane)
        Some(1289721245281292290)
    }

    async fn sync_managed(
        &self,
        guild_id: u64,
        category_id: u64,
        anchor_id: u64,
        base_name: &str,
        expand_threshold: usize,
        max_total: Option<usize>,
    ) {
        let channels = self.port.category_channels(guild_id, category_id).await;
        let Some(anchor) = channels.iter().find(|(id, _, _, _)| *id == anchor_id) else {
            tracing::warn!(anchor_id, "Adaptive: Anker nicht gefunden");
            return;
        };
        if anchor.1 != base_name {
            let _ = self.port.rename_channel(anchor_id, base_name).await;
        }
        let snapshots: Vec<LaneSnapshot> = channels
            .iter()
            .filter(|(id, _, _, _)| *id != anchor_id)
            .filter_map(|(id, name, members, _)| {
                parse_lane_index(*id, anchor_id, base_name, name).map(|index| LaneSnapshot {
                    channel_id: *id,
                    current_index: index,
                    member_count: *members,
                })
            })
            .collect();
        let plan = plan_lanes(anchor.2, &snapshots, expand_threshold, max_total);
        for channel_id in &plan.delete_ids {
            let _ = self.port.delete_channel(*channel_id).await;
        }
        for (channel_id, index) in &plan.reassignments {
            let desired = lane_name_for_index(base_name, *index);
            let current = snapshots
                .iter()
                .find(|s| s.channel_id == *channel_id)
                .map(|s| lane_name_for_index(base_name, s.current_index));
            if current.as_deref() != Some(desired.as_str()) {
                let _ = self.port.rename_channel(*channel_id, &desired).await;
            }
            let desired_position = anchor.3 + *index as i64 - 1;
            let current_position = channels
                .iter()
                .find(|(id, _, _, _)| id == channel_id)
                .map(|(_, _, _, position)| *position);
            if current_position != Some(desired_position) {
                let _ = self
                    .port
                    .set_channel_position(*channel_id, desired_position)
                    .await;
            }
        }
        for index in &plan.create_indices {
            let name = lane_name_for_index(base_name, *index);
            let Ok(channel_id) = self
                .port
                .create_voice_channel(guild_id, category_id, anchor_id, &name)
                .await
            else {
                continue;
            };
            let desired_position = anchor.3 + *index as i64 - 1;
            let _ = self
                .port
                .set_channel_position(channel_id, desired_position)
                .await;
        }
    }

    /// Chill-Lanes nach Rang-Label sortieren (wie lane_sorting).
    pub async fn sort_chill_lanes(&self, guild_id: u64) {
        self.sort_tempvoice_category(guild_id).await;
    }

    pub async fn sort_tempvoice_category(&self, guild_id: u64) {
        const SKIP_IDS: [u64; 4] = [
            CASUAL_STAGING_ID,
            PERMANENT_CHILL_ID,
            PINNED_CHILL_END_ID,
            NP_ANCHOR_CHANNEL_ID,
        ];
        let channels = self
            .port
            .category_channels(guild_id, CHILL_CATEGORY_ID)
            .await;
        let entries: Vec<_> = channels
            .iter()
            .enumerate()
            .filter(|(_, (id, name, _, _))| {
                !SKIP_IDS.contains(id)
                    && *id != DUO_ANCHOR_CHANNEL_ID
                    && !name.starts_with(NP_LANE_BASE_NAME)
            })
            .filter_map(|(stable_order, (id, name, _, position))| {
                sort_snapshot_from_name(*id, name, *position, stable_order)
            })
            .collect();
        let plan = plan_unified_category_reorder(&entries);
        if !plan.is_empty() {
            tracing::info!(
                tempvoice_lanes = entries.len(),
                reorders = plan.len(),
                category_id = CHILL_CATEGORY_ID,
                "TempVoiceSort: ordne gemischte Kategorie neu"
            );
            if let Err(err) = self.port.set_channel_positions(guild_id, &plan).await {
                tracing::warn!(%err, guild_id, "TempVoiceSort: Bulk-Positionen fehlgeschlagen");
            }
        }
    }

    pub async fn sort_comp_ranked_lanes(&self, guild_id: u64) {
        let _ = guild_id;
    }
}

/// Voice-Subscriber: Events in den verwalteten Bereichen ziehen die
/// Lane-Layouts und die Chill-Sortierung nach (Syncs sind idempotent —
/// Discord-Calls passieren nur bei echter Abweichung).
pub fn spawn(
    adaptive: Arc<AdaptiveLanes>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_voice();
    let mut channel_events = dispatcher.subscribe_channels();
    let mut channel_gateway_events = dispatcher.subscribe_gateway();
    let channel_adaptive = adaptive.clone();
    tokio::spawn(async move {
        wait_for_cache_ready(&mut channel_gateway_events, MAIN_GUILD_ID).await;
        loop {
            match channel_events.recv().await {
                Ok(dl_discord::ChannelEvent::VoiceCategoryChanged { guild_id, .. }) => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    channel_adaptive.sync_new_player(guild_id).await;
                    channel_adaptive.sync_duo(guild_id).await;
                    channel_adaptive.sort_chill_lanes(guild_id).await;
                    channel_adaptive.sort_comp_ranked_lanes(guild_id).await;
                }
                Ok(dl_discord::ChannelEvent::VoiceChannelUpdated { guild_id, .. }) => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    channel_adaptive.sort_chill_lanes(guild_id).await;
                    channel_adaptive.sort_comp_ranked_lanes(guild_id).await;
                }
                Ok(dl_discord::ChannelEvent::VoiceChannelStatusUpdated { .. }) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    let mut gateway_events = dispatcher.subscribe_gateway();
    tokio::spawn(async move {
        wait_for_cache_ready(&mut gateway_events, MAIN_GUILD_ID).await;
        adaptive.sync_new_player(MAIN_GUILD_ID).await;
        adaptive.sync_duo(MAIN_GUILD_ID).await;
        adaptive.sort_chill_lanes(MAIN_GUILD_ID).await;
        adaptive.sort_comp_ranked_lanes(MAIN_GUILD_ID).await;
        loop {
            match events.recv().await {
                Ok(event) => {
                    let (guild_id, channels) = match event {
                        dl_discord::VoiceEvent::Join {
                            guild_id,
                            channel_id,
                            ..
                        }
                        | dl_discord::VoiceEvent::Leave {
                            guild_id,
                            channel_id,
                            ..
                        } => (guild_id, vec![channel_id]),
                        dl_discord::VoiceEvent::Move {
                            guild_id,
                            from_channel_id,
                            to_channel_id,
                            ..
                        } => (guild_id, vec![from_channel_id, to_channel_id]),
                        dl_discord::VoiceEvent::Update { .. } => continue,
                    };
                    // kleiner Debounce wie das Original (1 s)
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if channels.contains(&DUO_ANCHOR_CHANNEL_ID) {
                        adaptive.sync_duo(guild_id).await;
                    }
                    adaptive.sync_new_player(guild_id).await;
                    adaptive.sort_chill_lanes(guild_id).await;
                    adaptive.sort_comp_ranked_lanes(guild_id).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct AnchorAttrs {
        overwrites: Vec<String>,
        user_limit: i64,
        bitrate: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct CreateCall {
        guild_id: u64,
        category_id: u64,
        anchor_id: u64,
        name: String,
        copied_anchor: AnchorAttrs,
    }

    struct MockAdaptivePort {
        channels: StdMutex<Vec<(u64, String, usize, i64)>>,
        anchor_attrs: HashMap<u64, AnchorAttrs>,
        create_results: StdMutex<Vec<Result<u64, String>>>,
        create_calls: StdMutex<Vec<CreateCall>>,
        position_calls: StdMutex<Vec<(u64, i64)>>,
        bulk_position_calls: StdMutex<Vec<Vec<(u64, i64)>>>,
        rename_calls: StdMutex<Vec<(u64, String)>>,
        delete_calls: StdMutex<Vec<u64>>,
    }

    impl MockAdaptivePort {
        fn new(channels: Vec<(u64, String, usize, i64)>) -> Self {
            Self {
                channels: StdMutex::new(channels),
                anchor_attrs: HashMap::new(),
                create_results: StdMutex::new(Vec::new()),
                create_calls: StdMutex::new(Vec::new()),
                position_calls: StdMutex::new(Vec::new()),
                bulk_position_calls: StdMutex::new(Vec::new()),
                rename_calls: StdMutex::new(Vec::new()),
                delete_calls: StdMutex::new(Vec::new()),
            }
        }

        fn with_anchor_attrs(mut self, anchor_id: u64, attrs: AnchorAttrs) -> Self {
            self.anchor_attrs.insert(anchor_id, attrs);
            self
        }

        fn with_create_results(self, results: Vec<Result<u64, String>>) -> Self {
            *self.create_results.lock().expect("lock") = results;
            self
        }
    }

    #[async_trait::async_trait]
    impl AdaptivePort for MockAdaptivePort {
        async fn category_channels(
            &self,
            _guild_id: u64,
            _category_id: u64,
        ) -> Vec<(u64, String, usize, i64)> {
            self.channels.lock().expect("lock").clone()
        }

        async fn member_role_pairs(&self, _guild_id: u64, _user_id: u64) -> Vec<(u64, String)> {
            Vec::new()
        }

        async fn move_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _channel_id: u64,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn rename_channel(&self, channel_id: u64, name: &str) -> Result<(), String> {
            self.rename_calls
                .lock()
                .expect("lock")
                .push((channel_id, name.to_string()));
            Ok(())
        }

        async fn delete_channel(&self, channel_id: u64) -> Result<(), String> {
            self.delete_calls.lock().expect("lock").push(channel_id);
            Ok(())
        }

        async fn create_voice_channel(
            &self,
            guild_id: u64,
            category_id: u64,
            anchor_id: u64,
            name: &str,
        ) -> Result<u64, String> {
            let copied_anchor = self
                .anchor_attrs
                .get(&anchor_id)
                .cloned()
                .unwrap_or_default();
            self.create_calls.lock().expect("lock").push(CreateCall {
                guild_id,
                category_id,
                anchor_id,
                name: name.to_string(),
                copied_anchor,
            });
            self.create_results.lock().expect("lock").remove(0)
        }

        async fn set_channel_position(&self, channel_id: u64, position: i64) -> Result<(), String> {
            self.position_calls
                .lock()
                .expect("lock")
                .push((channel_id, position));
            Ok(())
        }

        async fn set_channel_positions(
            &self,
            _guild_id: u64,
            positions: &[(u64, i64)],
        ) -> Result<(), String> {
            self.bulk_position_calls
                .lock()
                .expect("lock")
                .push(positions.to_vec());
            Ok(())
        }
    }

    fn snap(channel_id: u64, index: usize, members: usize) -> LaneSnapshot {
        LaneSnapshot {
            channel_id,
            current_index: index,
            member_count: members,
        }
    }

    // Referenzwerte aus CPython (plan_duo_lanes / plan_managed_lanes)
    #[test]
    fn duo_plan_wie_python() {
        let plan = plan_lanes(1, &[], DUO_EXPAND_THRESHOLD, Some(2));
        assert_eq!(plan, LanePlan::default());
        let plan = plan_lanes(2, &[], DUO_EXPAND_THRESHOLD, Some(2));
        assert_eq!(plan.create_indices, vec![2]);
        let plan = plan_lanes(2, &[snap(50, 2, 0)], DUO_EXPAND_THRESHOLD, Some(2));
        assert_eq!(plan.reassignments, vec![(50, 2)]);
        assert!(plan.delete_ids.is_empty() && plan.create_indices.is_empty());
        let plan = plan_lanes(0, &[snap(50, 2, 0)], DUO_EXPAND_THRESHOLD, Some(2));
        assert_eq!(plan.delete_ids, vec![50]);
        let plan = plan_lanes(3, &[snap(50, 2, 1)], DUO_EXPAND_THRESHOLD, Some(2));
        assert_eq!(plan.reassignments, vec![(50, 2)]);
        assert!(plan.delete_ids.is_empty());
    }

    #[test]
    fn new_player_plan_wie_python() {
        let plan = plan_lanes(0, &[], NP_EXPAND_THRESHOLD, None);
        assert_eq!(plan, LanePlan::default());
        let plan = plan_lanes(6, &[], NP_EXPAND_THRESHOLD, None);
        assert_eq!(plan.create_indices, vec![2]);
        // Lane 2 voll → Lane 3 entsteht
        let plan = plan_lanes(6, &[snap(50, 2, 6)], NP_EXPAND_THRESHOLD, None);
        assert_eq!(plan.reassignments, vec![(50, 2)]);
        assert_eq!(plan.create_indices, vec![3]);
        // besetzte Lane rutscht vor, leere wird gelöscht
        let plan = plan_lanes(
            2,
            &[snap(50, 2, 0), snap(51, 3, 2)],
            NP_EXPAND_THRESHOLD,
            None,
        );
        assert_eq!(plan.reassignments, vec![(51, 2)]);
        assert_eq!(plan.delete_ids, vec![50]);
        assert!(plan.create_indices.is_empty());
    }

    #[test]
    fn lane_namen_und_index() {
        assert_eq!(lane_name_for_index(NP_LANE_BASE_NAME, 1), NP_LANE_BASE_NAME);
        assert_eq!(
            lane_name_for_index(NP_LANE_BASE_NAME, 3),
            "🆕Neue Spieler Lane 3"
        );
        assert_eq!(
            parse_lane_index(
                NP_ANCHOR_CHANNEL_ID,
                NP_ANCHOR_CHANNEL_ID,
                NP_LANE_BASE_NAME,
                "egal"
            ),
            Some(1)
        );
        assert_eq!(
            parse_lane_index(
                9,
                NP_ANCHOR_CHANNEL_ID,
                NP_LANE_BASE_NAME,
                "🆕Neue Spieler Lane 2"
            ),
            Some(2)
        );
        assert_eq!(
            parse_lane_index(
                9,
                NP_ANCHOR_CHANNEL_ID,
                NP_LANE_BASE_NAME,
                "🆕Neue Spieler Lane"
            ),
            None
        );
        assert_eq!(
            parse_lane_index(9, NP_ANCHOR_CHANNEL_ID, NP_LANE_BASE_NAME, "Andere Lane 2"),
            None
        );
    }

    #[test]
    fn anfaenger_rang() {
        // Verifiziert: Seeker-Rolle → 2
        let roles = vec![(1u64, "Seeker".to_string())];
        assert_eq!(resolve_new_player_rank(&roles), Some(2));
        // Unverifiziert-Rolle → max der Treffer
        let roles = vec![
            (1492960891619250408u64, "Unverifiziert Initiate".to_string()),
            (1492960274096066831u64, "Unverifiziert Arcanist".to_string()),
        ];
        assert_eq!(resolve_new_player_rank(&roles), Some(4));
        // Phantom (9) ist KEIN Anfänger — Wert kommt zurück, Aufrufer filtert >4
        let roles = vec![(1u64, "Phantom".to_string())];
        assert_eq!(resolve_new_player_rank(&roles), Some(9));
        assert_eq!(resolve_new_player_rank(&[]), None);
    }

    #[test]
    fn sortierung_wie_python() {
        assert_eq!(parse_rank_label("Phantom 3 irgendwas"), (9, 3));
        assert_eq!(parse_rank_label("seeker"), (2, 0));
        assert_eq!(parse_rank_label("Lane 4"), (0, 0));
        let entries = vec![
            SortSnapshot {
                lane_id: 1,
                current_position: 10,
                rank_index: 9,
                subrank: 3,
                stable_order: 0,
            },
            SortSnapshot {
                lane_id: 2,
                current_position: 11,
                rank_index: 2,
                subrank: 0,
                stable_order: 1,
            },
            SortSnapshot {
                lane_id: 3,
                current_position: 12,
                rank_index: 9,
                subrank: 1,
                stable_order: 2,
            },
        ];
        // Sortiert: seeker(2) < phantom1 < phantom3 → Lane 2 auf 10, 3 auf 11, 1 auf 12
        assert_eq!(plan_lane_reorder(&entries), vec![(2, 10), (3, 11), (1, 12)]);
        assert!(plan_lane_reorder(&entries[..1]).is_empty());
    }

    #[tokio::test]
    async fn sync_managed_positioniert_neue_lane_und_reicht_anchor_durch() {
        let copied_anchor = AnchorAttrs {
            overwrites: vec!["role:7 allow:1048576 deny:0".to_string()],
            user_limit: 6,
            bitrate: Some(96_000),
        };
        let port = Arc::new(
            MockAdaptivePort::new(vec![(100, "Lane".to_string(), 6, 10)])
                .with_anchor_attrs(100, copied_anchor.clone())
                .with_create_results(vec![Ok(200)]),
        );
        let adaptive = AdaptiveLanes::new(port.clone());

        adaptive.sync_managed(1, 2, 100, "Lane", 6, None).await;

        let creates = port.create_calls.lock().expect("lock").clone();
        assert_eq!(
            creates,
            vec![CreateCall {
                guild_id: 1,
                category_id: 2,
                anchor_id: 100,
                name: "Lane 2".to_string(),
                copied_anchor,
            }]
        );
        assert_eq!(*port.position_calls.lock().expect("lock"), vec![(200, 11)]);
    }

    #[tokio::test]
    async fn sync_managed_setzt_reassignment_position_nur_bei_abweichung() {
        let port = Arc::new(MockAdaptivePort::new(vec![
            (100, "Lane".to_string(), 1, 10),
            (201, "Lane 2".to_string(), 1, 11),
        ]));
        let adaptive = AdaptiveLanes::new(port.clone());

        adaptive.sync_managed(1, 2, 100, "Lane", 6, None).await;

        assert!(port.position_calls.lock().expect("lock").is_empty());

        let port = Arc::new(MockAdaptivePort::new(vec![
            (100, "Lane".to_string(), 1, 10),
            (201, "Lane 2".to_string(), 1, 99),
        ]));
        let adaptive = AdaptiveLanes::new(port.clone());

        adaptive.sync_managed(1, 2, 100, "Lane", 6, None).await;

        assert_eq!(*port.position_calls.lock().expect("lock"), vec![(201, 11)]);
    }

    #[tokio::test]
    async fn sync_managed_ueberspringt_create_fehler_ohne_panic() {
        let port = Arc::new(
            MockAdaptivePort::new(vec![(100, "Lane".to_string(), 6, 10)])
                .with_create_results(vec![Err("create failed".to_string())]),
        );
        let adaptive = AdaptiveLanes::new(port.clone());

        adaptive.sync_managed(1, 2, 100, "Lane", 6, None).await;

        assert_eq!(port.create_calls.lock().expect("lock").len(), 1);
        assert!(port.position_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn sync_new_player_erzeugt_lanes_in_chill_kategorie() {
        let port = Arc::new(
            MockAdaptivePort::new(vec![(
                NP_ANCHOR_CHANNEL_ID,
                NP_LANE_BASE_NAME.to_string(),
                NP_EXPAND_THRESHOLD,
                10,
            )])
            .with_create_results(vec![Ok(200)]),
        );
        let adaptive = AdaptiveLanes::new(port.clone());

        adaptive.sync_new_player(MAIN_GUILD_ID).await;

        let creates = port.create_calls.lock().expect("lock").clone();
        assert_eq!(creates.len(), 1);
        assert_eq!(creates[0].category_id, CHILL_CATEGORY_ID);
        assert_eq!(creates[0].anchor_id, NP_ANCHOR_CHANNEL_ID);
        assert_eq!(creates[0].name, "🆕Neue Spieler Lane 2");
    }

    #[test]
    fn tempvoice_one_category_sortiert_gemischte_bloecke_stabil() {
        let entries = vec![
            UnifiedSortSnapshot {
                lane_id: 10,
                current_position: 10,
                block: LaneSortBlock::Casual,
                rank_index: 0,
                subrank: 0,
                stable_order: 0,
            },
            UnifiedSortSnapshot {
                lane_id: 11,
                current_position: 11,
                block: LaneSortBlock::StreetBrawl,
                rank_index: 0,
                subrank: 0,
                stable_order: 1,
            },
            UnifiedSortSnapshot {
                lane_id: 12,
                current_position: 12,
                block: LaneSortBlock::Ranked,
                rank_index: 9,
                subrank: 3,
                stable_order: 2,
            },
            UnifiedSortSnapshot {
                lane_id: 13,
                current_position: 13,
                block: LaneSortBlock::Ranked,
                rank_index: 2,
                subrank: 1,
                stable_order: 3,
            },
            UnifiedSortSnapshot {
                lane_id: 14,
                current_position: 14,
                block: LaneSortBlock::Ranked,
                rank_index: 0,
                subrank: 0,
                stable_order: 4,
            },
            UnifiedSortSnapshot {
                lane_id: 15,
                current_position: 15,
                block: LaneSortBlock::Casual,
                rank_index: 0,
                subrank: 0,
                stable_order: 5,
            },
        ];

        assert_eq!(
            plan_unified_category_reorder(&entries),
            vec![(13, 10), (12, 11), (14, 12), (10, 13), (15, 14), (11, 15)]
        );
    }

    #[tokio::test]
    async fn tempvoice_one_category_sortierung_nutzt_genau_einen_bulk_call() {
        let port = Arc::new(MockAdaptivePort::new(vec![
            (10, "Chill Lane 1".to_string(), 1, 10),
            (11, "Street Brawl 1".to_string(), 1, 11),
            (12, "Ranked Phantom 3 1".to_string(), 1, 12),
            (13, "Ranked Seeker 1 1".to_string(), 1, 13),
        ]));
        let adaptive = AdaptiveLanes::new(port.clone());

        adaptive.sort_tempvoice_category(MAIN_GUILD_ID).await;

        assert!(port.position_calls.lock().expect("lock").is_empty());
        assert_eq!(
            *port.bulk_position_calls.lock().expect("lock"),
            vec![vec![(13, 10), (12, 11), (10, 12), (11, 13)]]
        );
    }
}
