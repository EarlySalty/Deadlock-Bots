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

pub const NP_TARGET_CATEGORY_ID: u64 = 1465839366634209361;
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
        name: &str,
    ) -> Result<u64, String>;
    async fn set_channel_position(&self, channel_id: u64, position: i64) -> Result<(), String>;
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
            // Rückkehr-Fenster: einmal umgeleitete User dürfen 4 min lang
            // zurück in den normalen Flow
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
            .category_channels(guild_id, NP_TARGET_CATEGORY_ID)
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

    /// Rückkehr-Fenster aufräumen (vom Sync mitgenutzt).
    async fn prune_routed(&self) {
        let now = chrono::Utc::now().timestamp();
        self.routed_at
            .lock()
            .await
            .retain(|_, at| now - *at <= RETURN_WINDOW_SECONDS);
    }

    /// Neue-Spieler-Kategorie nachziehen (wie `_sync_guild`).
    pub async fn sync_new_player(&self, guild_id: u64) {
        self.prune_routed().await;
        self.sync_managed(
            guild_id,
            NP_TARGET_CATEGORY_ID,
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
        }
        for index in &plan.create_indices {
            let name = lane_name_for_index(base_name, *index);
            let _ = self
                .port
                .create_voice_channel(guild_id, category_id, &name)
                .await;
        }
    }

    /// Chill-Lanes nach Rang-Label sortieren (wie lane_sorting).
    pub async fn sort_chill_lanes(&self, guild_id: u64) {
        const CHILL_CATEGORY_ID: u64 = 1289721245281292290;
        const SKIP_IDS: [u64; 2] = [1501089974093873232, 1493690350580138114];
        let channels = self
            .port
            .category_channels(guild_id, CHILL_CATEGORY_ID)
            .await;
        let entries: Vec<SortSnapshot> = channels
            .iter()
            .filter(|(id, _, _, _)| !SKIP_IDS.contains(id) && *id != DUO_ANCHOR_CHANNEL_ID)
            .enumerate()
            .filter_map(|(stable_order, (id, name, _, position))| {
                let (rank_index, subrank) = parse_rank_label(name);
                (rank_index > 0).then_some(SortSnapshot {
                    lane_id: *id,
                    current_position: *position,
                    rank_index,
                    subrank,
                    stable_order,
                })
            })
            .collect();
        let plan = plan_lane_reorder(&entries);
        if !plan.is_empty() {
            tracing::info!(
                rang_lanes = entries.len(),
                reorders = plan.len(),
                "RankSort: ordne Chill-Lanes nach Rang neu"
            );
        }
        for (lane_id, target) in plan {
            if let Err(err) = self.port.set_channel_position(lane_id, target).await {
                tracing::warn!(%err, lane_id, target, "RankSort: Position setzen fehlgeschlagen");
            }
        }
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
    tokio::spawn(async move {
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
}
