//! TempVoice-Engine — Join-to-create, Owner-Lifecycle, Lane-Aufräumen.
//!
//! Port des Verhaltens-Kerns von cogs/tempvoice/core.py als Subscriber des
//! Voice-Dispatchers. Discord-Aktionen laufen über [`LanePort`] (testbar).
//!
//! Bewusste 4b-Lücken (dokumentiert in rust/docs/05): Tag-Filter/Ragebaiter,
//! Lurker-Flow, New-Player-Routing-Hook, RTC-Region-Apply und die
//! Rang-Permission-Kopplung (kommt mit dem rank_voice_manager-Port in 4c).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use chrono::{NaiveDateTime, Utc};
use dl_discord::{ChannelEvent, Dispatcher, GatewayEvent, VoiceEvent};

use super::logic;
use super::store::{DefaultPresetRecord, LaneRecord, TempVoiceStore};
use crate::voice_pair_guard::VoicePairOperationLock;

pub const PURGE_INTERVAL_SECONDS: u64 = 180;
pub const VERIFIED_ROLE_ID: u64 = 1419608095533043774;
pub const MIN_RANK_DISABLED_REPLY: &str = "Mindest-Rang ist hier deaktiviert.";
pub const TEMPVOICE_ONE_CATEGORY_ID: u64 = 1289721245281292290;
pub const LEGACY_RANKED_CATEGORY_ID: u64 = 1412804540994162789;
pub const LEGACY_STREET_BRAWL_CATEGORY_ID: u64 = 1357422957017698478;

fn router_lane_limit(mode: &str, preset: Option<&DefaultPresetRecord>) -> i64 {
    match (mode, preset.map(|preset| preset.limit)) {
        ("street_brawl", Some(limit)) => limit.clamp(1, 4),
        ("street_brawl", None) => 4,
        (_, Some(limit)) => limit.clamp(0, 99),
        _ => 6,
    }
}

fn rank_label(rank: &str, subrank: i64) -> Option<String> {
    if rank == "unknown" || logic::rank_index(rank) == 0 {
        return None;
    }
    let label = logic::capitalize(rank);
    if (1..=6).contains(&subrank) {
        Some(format!("{label} {subrank}"))
    } else {
        Some(label)
    }
}

fn mode_for_staging_id(staging_id: u64) -> Option<&'static str> {
    match staging_id {
        1501089974093873232 => Some("casual"),
        1412804671432818890 => Some("ranked"),
        1357422958544420944 => Some("street_brawl"),
        _ => None,
    }
}

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

/// Discord-Seite der Engine (Cache-Reads + REST-Aktionen).
#[async_trait::async_trait]
pub trait LanePort: Send + Sync {
    async fn create_voice_channel(
        &self,
        guild_id: u64,
        category_id: Option<u64>,
        name: &str,
        user_limit: i64,
    ) -> Result<u64, String>;
    async fn create_restricted_voice_channel(
        &self,
        guild_id: u64,
        category_id: u64,
        name: &str,
        connect_user_ids: &[u64],
    ) -> Result<u64, String>;
    async fn delete_channel(&self, channel_id: u64, reason: &str) -> Result<(), String>;
    async fn move_member(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    async fn rename_channel(&self, channel_id: u64, name: &str, reason: &str)
        -> Result<(), String>;
    /// connect=None löscht das Member-Overwrite (Unban), Some(false) = Bann.
    async fn set_member_connect(
        &self,
        channel_id: u64,
        user_id: u64,
        connect: Option<bool>,
    ) -> Result<(), String>;
    /// Member-Connect gesammelt per Channel-Batch: denied => connect deny,
    /// cleared => Member-Overwrite entfernen.
    async fn apply_member_connect_batch(
        &self,
        channel_id: u64,
        denied_user_ids: &HashSet<u64>,
        clear_user_ids: &HashSet<u64>,
    ) -> Result<(), String>;
    /// Rollen-Overwrite (Region: English-Only-Rolle deny); None löscht.
    async fn set_role_connect(
        &self,
        channel_id: u64,
        role_id: u64,
        connect: Option<bool>,
    ) -> Result<(), String>;
    /// Rollen-Connect gesammelt per Channel-Batch: allowed => connect allow,
    /// cleared => Rollen-Overwrite entfernen. Keine Deny-Syncs für Rang-Rollen.
    async fn apply_role_connect_batch(
        &self,
        guild_id: u64,
        channel_id: u64,
        allowed_role_ids: &HashSet<u64>,
        clear_role_ids: &HashSet<u64>,
    ) -> Result<(), String>;
    async fn apply_role_connect_overwrites(
        &self,
        guild_id: u64,
        channel_id: u64,
        allowed_role_ids: &HashSet<u64>,
        denied_role_ids: &HashSet<u64>,
        clear_role_ids: &HashSet<u64>,
    ) -> Result<(), String>;
    async fn set_user_limit(&self, channel_id: u64, limit: i64, reason: &str)
        -> Result<(), String>;
    async fn disconnect_member(
        &self,
        guild_id: u64,
        user_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> Option<String>;
    async fn add_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    async fn remove_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String>;
    /// nick=None setzt den Nick zurück.
    async fn set_nick(
        &self,
        guild_id: u64,
        user_id: u64,
        nick: Option<&str>,
        reason: &str,
    ) -> Result<(), String>;
    async fn member_nick(&self, guild_id: u64, user_id: u64) -> Option<String>;
    async fn channel_user_limit(&self, guild_id: u64, channel_id: u64) -> Option<i64>;
    /// Alle Guild-Rollen (id, name) — für den Min-Rang-Rollen-Scan.
    async fn guild_role_names(&self, guild_id: u64) -> Vec<(u64, String)>;
    async fn set_channel_category(
        &self,
        channel_id: u64,
        category_id: u64,
        reason: &str,
    ) -> Result<(), String>;

    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    async fn member_role_names(&self, guild_id: u64, user_id: u64) -> Vec<String>;
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    async fn channel_members(&self, guild_id: u64, channel_id: u64) -> Vec<u64>;
    async fn channel_name(&self, guild_id: u64, channel_id: u64) -> Option<String>;
    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64>;
    async fn category_voice_channel_names(&self, guild_id: u64, category_id: u64) -> Vec<String>;
    async fn category_voice_channels(&self, guild_id: u64, category_id: u64) -> Vec<(u64, String)>;
    /// Unix-Timestamp der Kanal-Erstellung (Snowflake).
    async fn channel_created_at(&self, channel_id: u64) -> Option<i64>;
}

/// Regeln pro Staging-Kanal (STAGING_RULES-Pendant).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StagingRules {
    pub prefix: Option<String>,
    pub user_limit: Option<i64>,
    pub max_limit: Option<i64>,
    pub prefix_from_rank: bool,
    pub disable_min_rank: bool,
}

#[derive(Debug, Clone)]
pub struct TempVoiceConfig {
    /// Haupt-Guild (für Abläufe ohne Event-Kontext, z. B. Tag-Listener).
    pub guild_id_hint: u64,
    pub staging_channels: HashSet<u64>,
    pub fixed_lane_ids: HashSet<u64>,
    pub tempvoice_categories: HashSet<u64>,
    pub minrank_categories: HashSet<u64>,
    pub ranked_category_id: u64,
    pub staging_rules: HashMap<u64, StagingRules>,
}

impl TempVoiceConfig {
    /// Produktions-IDs aus service/guild_config.py (env-überschreibbar dort,
    /// hier bewusst als Konstanten — die IDs sind seit Jahren stabil).
    pub fn production() -> Self {
        let staging_casual = 1501089974093873232;
        let staging_street_brawl = 1357422958544420944;
        let staging_comp = 1412804671432818890;
        let category_chill = TEMPVOICE_ONE_CATEGORY_ID;
        let category_comp = LEGACY_RANKED_CATEGORY_ID;
        let category_street_brawl = LEGACY_STREET_BRAWL_CATEGORY_ID;

        let mut staging_rules = HashMap::new();
        staging_rules.insert(
            staging_street_brawl,
            StagingRules {
                prefix: Some("Street Brawl".to_string()),
                user_limit: Some(4),
                max_limit: Some(4),
                prefix_from_rank: false,
                disable_min_rank: true,
            },
        );
        staging_rules.insert(
            staging_casual,
            StagingRules {
                prefix: None,
                user_limit: None,
                max_limit: None,
                prefix_from_rank: true,
                disable_min_rank: false,
            },
        );

        Self {
            guild_id_hint: 1289721245281292288,
            staging_channels: HashSet::from([staging_casual, staging_street_brawl, staging_comp]),
            // Eine Quelle für „das ist keine Lane": der Router filtert dieselbe
            // Liste beim Routing, sonst landet jemand im Einstiegskanal.
            fixed_lane_ids: HashSet::from(crate::router::NON_LANE_CHANNEL_IDS),
            tempvoice_categories: HashSet::from([
                category_chill,
                category_comp,
                category_street_brawl,
            ]),
            minrank_categories: HashSet::from([category_comp]),
            ranked_category_id: category_comp,
            staging_rules,
        }
    }

    fn rules_for_staging(&self, staging_id: u64) -> StagingRules {
        self.staging_rules
            .get(&staging_id)
            .cloned()
            .unwrap_or_default()
    }

    fn default_cap(&self, category_id: Option<u64>) -> i64 {
        if category_id == Some(self.ranked_category_id) {
            logic::DEFAULT_RANKED_CAP
        } else {
            logic::DEFAULT_CASUAL_CAP
        }
    }

    fn target_category_for_staging(&self, staging_id: u64) -> Option<u64> {
        if self.staging_channels.contains(&staging_id) {
            Some(TEMPVOICE_ONE_CATEGORY_ID)
        } else {
            None
        }
    }
}

pub const ENGLISH_ONLY_ROLE_ID: u64 = 1309741866098491479;
pub const LURKER_ROLE_ID: u64 = 1447747896253485127;

#[derive(Debug, Clone)]
struct LaneState {
    owner_id: u64,
    /// Erstbesitzer — Anker-Priorität des Rank-Managers + Claim-Regeln.
    initial_owner_id: u64,
    base_name: String,
    min_rank: String,
    category_id: Option<u64>,
    prefix_from_rank: bool,
    source_staging_id: Option<u64>,
}

#[derive(Default)]
struct EngineState {
    lanes: HashMap<u64, LaneState>,
    /// channel → user → join-Zeit
    join_time: HashMap<u64, HashMap<u64, NaiveDateTime>>,
    creating: HashSet<u64>,
    /// channel → von Tag-Filtern geblockte User (Schutz vor Bann-Löschung)
    tag_blocked: HashMap<u64, HashSet<u64>>,
    minrank_blocked: HashSet<u64>,
}

pub struct TempVoiceEngine {
    pub config: TempVoiceConfig,
    pub store: TempVoiceStore,
    pub port: Arc<dyn LanePort>,
    /// Tag-Dienst (None → Filter inaktiv, wie Original ohne TagService-Cog).
    pub tags: tokio::sync::RwLock<Option<Arc<dl_community::tags::TagService>>>,
    /// Anfänger-Routing-Hook (None = kein Reroute, wie Original ohne Cog).
    pub adaptive: tokio::sync::RwLock<Option<Arc<crate::adaptive::AdaptiveLanes>>>,
    lfg: tokio::sync::RwLock<Option<Weak<crate::lfg_panel::LfgPanelInterface>>>,
    voice_pair_operations: Arc<VoicePairOperationLock>,
    state: tokio::sync::Mutex<EngineState>,
}

impl TempVoiceEngine {
    pub fn new(
        config: TempVoiceConfig,
        store: TempVoiceStore,
        port: Arc<dyn LanePort>,
    ) -> Arc<Self> {
        Self::new_with_voice_pair_operations(
            config,
            store,
            port,
            Arc::new(VoicePairOperationLock::new(())),
        )
    }

    pub fn new_with_voice_pair_operations(
        config: TempVoiceConfig,
        store: TempVoiceStore,
        port: Arc<dyn LanePort>,
        voice_pair_operations: Arc<VoicePairOperationLock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            store,
            port,
            tags: tokio::sync::RwLock::new(None),
            adaptive: tokio::sync::RwLock::new(None),
            lfg: tokio::sync::RwLock::new(None),
            voice_pair_operations,
            state: tokio::sync::Mutex::new(EngineState::default()),
        })
    }

    pub async fn add_owner_ban(
        &self,
        owner_id: u64,
        target_id: u64,
        channel_id: Option<u64>,
    ) -> Result<(), String> {
        let _operation = self.voice_pair_operations.lock().await;
        self.store
            .add_ban(owner_id, target_id)
            .await
            .map_err(|err| err.to_string())?;
        if let Some(channel_id) = channel_id {
            self.port
                .set_member_connect(channel_id, target_id, Some(false))
                .await?;
        }
        Ok(())
    }

    pub async fn remove_owner_ban(
        &self,
        owner_id: u64,
        target_id: u64,
        channel_id: Option<u64>,
    ) -> Result<(), String> {
        let _operation = self.voice_pair_operations.lock().await;
        self.store
            .remove_ban(owner_id, target_id)
            .await
            .map_err(|err| err.to_string())?;
        if let Some(channel_id) = channel_id {
            self.port
                .set_member_connect(channel_id, target_id, None)
                .await?;
        }
        Ok(())
    }

    pub async fn set_lfg_panel(&self, lfg: Arc<crate::lfg_panel::LfgPanelInterface>) {
        *self.lfg.write().await = Some(Arc::downgrade(&lfg));
    }

    pub async fn create_restricted_voice_channel(
        &self,
        category_id: u64,
        name: &str,
        connect_user_ids: &[u64],
    ) -> Result<u64, String> {
        self.port
            .create_restricted_voice_channel(
                self.config.guild_id_hint,
                category_id,
                name,
                connect_user_ids,
            )
            .await
    }

    pub async fn delete_managed_voice_channel(
        &self,
        channel_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.port.delete_channel(channel_id, reason).await
    }

    fn rules_for_category(&self, category_id: Option<u64>) -> (StagingRules, Option<u64>) {
        let Some(category_id) = category_id else {
            return (StagingRules::default(), None);
        };
        for staging_id in &self.config.staging_channels {
            let rules = self.config.rules_for_staging(*staging_id);
            if rules == StagingRules::default() {
                continue;
            }
            // Produktionsvertrag: Staging-Kanal und Zielkategorie teilen sich die
            // Staging-Regel. Die konkreten Kategorien sind stabil und bereits in
            // `production()` abgebildet.
            let mapped_category = match *staging_id {
                1501089974093873232 => 1289721245281292290,
                1357422958544420944 => 1357422957017698478,
                _ => 0,
            };
            if mapped_category == category_id {
                return (rules, Some(*staging_id));
            }
        }
        (StagingRules::default(), None)
    }

    fn rules_from_base(
        &self,
        base_name: &str,
        category_id: Option<u64>,
    ) -> (StagingRules, Option<u64>) {
        let base_lower = base_name.to_lowercase();
        for (staging_id, rule) in &self.config.staging_rules {
            if rule.prefix_from_rank {
                let first = base_lower.split_whitespace().next().unwrap_or_default();
                if logic::RANK_ORDER.contains(&first) || first == "lane" {
                    if category_id
                        .map(|id| self.config.minrank_categories.contains(&id))
                        .unwrap_or(false)
                    {
                        return (StagingRules::default(), None);
                    }
                    return (rule.clone(), Some(*staging_id));
                }
                continue;
            }
            let prefix = rule.prefix.as_deref().unwrap_or("Lane").to_lowercase();
            if base_lower.starts_with(&prefix) {
                return (rule.clone(), Some(*staging_id));
            }
        }
        (StagingRules::default(), None)
    }

    async fn apply_category_source(&self, channel_id: u64, category_id: u64) -> StagingRules {
        let (rules, source_staging_id) = self.rules_for_category(Some(category_id));
        {
            let mut state = self.state.lock().await;
            if let Some(lane) = state.lanes.get_mut(&channel_id) {
                lane.category_id = Some(category_id);
                lane.prefix_from_rank = rules.prefix_from_rank;
                lane.source_staging_id = source_staging_id;
                if rules.disable_min_rank {
                    lane.min_rank = "unknown".to_string();
                }
            }
            if rules.disable_min_rank {
                state.minrank_blocked.insert(channel_id);
            } else {
                state.minrank_blocked.remove(&channel_id);
            }
        }
        let _ = self
            .store
            .set_lane_category_source(channel_id, category_id, source_staging_id)
            .await;
        rules
    }

    pub async fn apply_lane_rules(&self, channel_id: u64, rules: &StagingRules) {
        let mut state = self.state.lock().await;
        if rules.disable_min_rank {
            if let Some(lane) = state.lanes.get_mut(&channel_id) {
                lane.min_rank = "unknown".to_string();
            }
            state.minrank_blocked.insert(channel_id);
        } else {
            state.minrank_blocked.remove(&channel_id);
        }
    }

    pub async fn is_min_rank_blocked(&self, channel_id: u64) -> bool {
        self.state
            .lock()
            .await
            .minrank_blocked
            .contains(&channel_id)
    }

    pub async fn owner_rank_anchor(&self, guild_id: u64, user_id: u64) -> Option<String> {
        if let Ok((rank, subrank)) = self.store.rank_pref(user_id).await {
            if let Some(label) = rank_label(&rank, subrank) {
                return Some(label);
            }
        }
        let roles = self.port.member_role_names(guild_id, user_id).await;
        logic::rank_prefix_for(&roles)
    }

    async fn tempvoice_mode_name(&self, guild_id: u64, user_id: u64, mode: &str) -> String {
        let existing = self
            .port
            .category_voice_channel_names(guild_id, TEMPVOICE_ONE_CATEGORY_ID)
            .await;
        match mode {
            "ranked" => {
                let prefix = self
                    .owner_rank_anchor(guild_id, user_id)
                    .await
                    .map(|rank| format!("Ranked {rank}"))
                    .unwrap_or_else(|| "Ranked".to_string());
                logic::next_name(&existing, &prefix)
            }
            "street_brawl" => logic::next_name(&existing, "Street Brawl"),
            _ => {
                let name = logic::next_name(&existing, "Chill Lane");
                match self.owner_rank_anchor(guild_id, user_id).await {
                    Some(rank) => format!("{name} · {rank}"),
                    None => name,
                }
            }
        }
    }

    /// Lanes aus der DB rehydrieren (Bot-Neustart).
    pub async fn rehydrate(&self) {
        match self.store.all_lanes().await {
            Ok(lanes) => {
                let mut state = self.state.lock().await;
                let mut source_updates = Vec::new();
                for lane in lanes {
                    let category_id = Some(lane.category_id).filter(|c| *c > 0);
                    let (rules, inferred_source) = match lane.source_staging_id {
                        Some(source_id) => {
                            (self.config.rules_for_staging(source_id), Some(source_id))
                        }
                        None => self.rules_from_base(&lane.base_name, category_id),
                    };
                    let source_staging_id = lane.source_staging_id.or(inferred_source);
                    let should_persist_source =
                        lane.source_staging_id.is_none() && source_staging_id.is_some();
                    let channel_id = lane.channel_id;
                    let persisted_category_id = lane.category_id;
                    if rules.disable_min_rank {
                        state.minrank_blocked.insert(channel_id);
                    }
                    state.lanes.insert(
                        channel_id,
                        LaneState {
                            owner_id: lane.owner_id,
                            initial_owner_id: lane.initial_owner_id.unwrap_or(lane.owner_id),
                            base_name: lane.base_name,
                            min_rank: "unknown".to_string(),
                            category_id,
                            prefix_from_rank: rules.prefix_from_rank,
                            source_staging_id,
                        },
                    );
                    if should_persist_source {
                        source_updates.push((channel_id, persisted_category_id, source_staging_id));
                    }
                }
                let lane_count = state.lanes.len();
                drop(state);
                for (channel_id, category_id, source_staging_id) in source_updates {
                    let _ = self
                        .store
                        .set_lane_category_source(channel_id, category_id, source_staging_id)
                        .await;
                }
                tracing::info!(lanes = lane_count, "TempVoice: Lanes rehydriert");
            }
            Err(err) => tracing::error!(%err, "TempVoice: Rehydrierung fehlgeschlagen"),
        }
    }

    /// Startup-Purge: bekannte Lanes ohne Mitglieder abbauen
    /// (wie `_purge_empty_lanes_once`).
    pub async fn purge_empty_lanes(&self) {
        let mut lanes: HashSet<u64> = {
            let state = self.state.lock().await;
            state
                .lanes
                .keys()
                .copied()
                .filter(|channel_id| {
                    !self.config.fixed_lane_ids.contains(channel_id)
                        && !self.config.staging_channels.contains(channel_id)
                })
                .collect()
        };
        let guild_id = self.config.guild_id_hint;
        for category_id in &self.config.tempvoice_categories {
            for (channel_id, _) in self
                .port
                .category_voice_channels(guild_id, *category_id)
                .await
            {
                if self.config.fixed_lane_ids.contains(&channel_id)
                    || self.config.staging_channels.contains(&channel_id)
                {
                    continue;
                }
                lanes.insert(channel_id);
            }
        }
        let mut purged = 0usize;
        for channel_id in lanes {
            let Some(_) = self.port.channel_name(guild_id, channel_id).await else {
                tracing::debug!(
                    channel_id,
                    "TempVoice: Startup-Purge ueberspringt Cache-Miss"
                );
                continue;
            };
            if self
                .port
                .channel_members(guild_id, channel_id)
                .await
                .is_empty()
            {
                self.cleanup_lane(channel_id, "TempVoice: Lane leer (Startup-Purge)")
                    .await;
                purged += 1;
            }
        }
        if purged > 0 {
            tracing::info!(purged, "TempVoice: leere Lanes beim Start geräumt");
        }
    }

    /// Erstbesitzer der Lane (None wenn unbekannt) — blockierungsfrei für
    /// den Rank-Manager (try_lock: bei Contention lieber None als Deadlock).
    pub fn initial_owner_blocking(&self, channel_id: u64) -> Option<u64> {
        self.state
            .try_lock()
            .ok()
            .and_then(|state| state.lanes.get(&channel_id).map(|l| l.initial_owner_id))
    }

    pub async fn lane_owner(&self, channel_id: u64) -> Option<u64> {
        self.state
            .lock()
            .await
            .lanes
            .get(&channel_id)
            .map(|l| l.owner_id)
    }

    pub async fn handle_event(self: &Arc<Self>, event: VoiceEvent) {
        match event {
            VoiceEvent::Join {
                guild_id,
                user_id,
                channel_id,
            } => {
                self.on_join(guild_id, user_id, channel_id).await;
            }
            VoiceEvent::Leave {
                guild_id,
                user_id,
                channel_id,
            } => {
                self.on_leave(guild_id, user_id, channel_id).await;
            }
            VoiceEvent::Move {
                guild_id,
                user_id,
                from_channel_id,
                to_channel_id,
            } => {
                self.on_leave(guild_id, user_id, from_channel_id).await;
                self.on_join(guild_id, user_id, to_channel_id).await;
            }
            VoiceEvent::Update { .. } => {}
        }
    }

    async fn on_join(self: &Arc<Self>, guild_id: u64, user_id: u64, channel_id: u64) {
        if self.config.staging_channels.contains(&channel_id) {
            if let Some(adaptive) = self.adaptive.read().await.clone() {
                if adaptive
                    .maybe_route_new_player(guild_id, user_id, channel_id)
                    .await
                {
                    return; // Anfänger umgeleitet — kein Join-to-create
                }
            }
            self.create_lane(guild_id, user_id, channel_id).await;
            return;
        }
        if !self.is_managed_lane(guild_id, channel_id).await {
            return;
        }

        let now = Utc::now().naive_utc();
        let needs_backfill = {
            let mut state = self.state.lock().await;
            state
                .join_time
                .entry(channel_id)
                .or_default()
                .insert(user_id, now);
            !state.lanes.contains_key(&channel_id)
        };

        // Owner-Backfill: verwaltete Lane ohne bekannten Owner → Joiner wird Owner
        if needs_backfill {
            let name = self
                .port
                .channel_name(guild_id, channel_id)
                .await
                .unwrap_or_default();
            let base_name = logic::strip_suffixes(&name);
            let category_id = self.port.channel_category(guild_id, channel_id).await;
            let (rules, source_staging_id) = self.rules_from_base(&base_name, category_id);
            {
                let mut state = self.state.lock().await;
                if rules.disable_min_rank {
                    state.minrank_blocked.insert(channel_id);
                }
                state.lanes.insert(
                    channel_id,
                    LaneState {
                        owner_id: user_id,
                        initial_owner_id: user_id,
                        base_name: base_name.clone(),
                        min_rank: "unknown".to_string(),
                        category_id,
                        prefix_from_rank: rules.prefix_from_rank,
                        source_staging_id,
                    },
                );
            }
            if let Err(err) = self
                .store
                .upsert_lane(LaneRecord {
                    channel_id,
                    guild_id,
                    owner_id: user_id,
                    initial_owner_id: Some(user_id),
                    base_name,
                    category_id: category_id.unwrap_or(0),
                    source_staging_id,
                })
                .await
            {
                tracing::warn!(%err, channel_id, "TempVoice: Owner-Backfill-Persist fehlgeschlagen");
            }
            self.apply_owner_settings(guild_id, channel_id, user_id)
                .await;
        }
        self.apply_tag_filter(guild_id, channel_id, Some(vec![user_id]), true)
            .await;
    }

    async fn on_leave(self: &Arc<Self>, guild_id: u64, user_id: u64, channel_id: u64) {
        self.cleanup_lurker_on_leave(guild_id, channel_id, user_id)
            .await;
        self.cleanup_tag_block_on_leave(channel_id, user_id).await;
        {
            let mut state = self.state.lock().await;
            if let Some(times) = state.join_time.get_mut(&channel_id) {
                times.remove(&user_id);
            }
        }
        if !self.is_managed_lane(guild_id, channel_id).await {
            return;
        }
        let members = self.port.channel_members(guild_id, channel_id).await;

        let was_owner = {
            let state = self.state.lock().await;
            state
                .lanes
                .get(&channel_id)
                .map(|l| l.owner_id == user_id)
                .unwrap_or(false)
        };

        if members.is_empty() {
            self.cleanup_lane(channel_id, "TempVoice: Lane leer").await;
            return;
        }

        if was_owner {
            // Auto-Transfer an das am längsten verbundene Mitglied
            let new_owner = {
                let state = self.state.lock().await;
                let times = state.join_time.get(&channel_id);
                let mut candidates: Vec<u64> = members.clone();
                candidates.sort_by_key(|m| {
                    times
                        .and_then(|t| t.get(m).copied())
                        .unwrap_or(NaiveDateTime::MAX)
                });
                candidates.first().copied()
            };
            if let Some(new_owner) = new_owner {
                {
                    let mut state = self.state.lock().await;
                    if let Some(lane) = state.lanes.get_mut(&channel_id) {
                        lane.owner_id = new_owner;
                    }
                }
                if let Err(err) = self.store.set_owner(channel_id, new_owner).await {
                    tracing::warn!(%err, channel_id, "TempVoice: Owner-Transfer-Persist fehlgeschlagen");
                }
                // Bans des alten Owners von der Lane nehmen, neuen Owner-Stand anwenden
                self.clear_owner_bans(channel_id, user_id).await;
                self.apply_owner_settings(guild_id, channel_id, new_owner)
                    .await;
                tracing::info!(
                    channel_id,
                    old = user_id,
                    new = new_owner,
                    "TempVoice: Owner-Transfer"
                );
            }
        }
    }

    async fn is_managed_lane(&self, guild_id: u64, channel_id: u64) -> bool {
        if self.config.fixed_lane_ids.contains(&channel_id)
            || self.config.staging_channels.contains(&channel_id)
        {
            return false;
        }
        if self.state.lock().await.lanes.contains_key(&channel_id) {
            return true;
        }
        self.port
            .channel_category(guild_id, channel_id)
            .await
            .is_some_and(|category| self.config.tempvoice_categories.contains(&category))
    }

    /// Join-to-create (Kern von `_create_lane`).
    async fn create_lane(self: &Arc<Self>, guild_id: u64, user_id: u64, staging_id: u64) {
        self.create_lane_from(guild_id, user_id, staging_id, staging_id)
            .await;
    }

    /// Lane nach Staging-Regeln erstellen, während der User in
    /// `expected_channel_id` steht (Router-VC ≠ Staging).
    pub async fn create_lane_from(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        staging_id: u64,
        expected_channel_id: u64,
    ) {
        // Doppel-Klick-Schutz pro User
        {
            let mut state = self.state.lock().await;
            if !state.creating.insert(user_id) {
                return;
            }
        }
        let result = self
            .create_lane_inner(guild_id, user_id, staging_id, expected_channel_id)
            .await;
        self.state.lock().await.creating.remove(&user_id);
        if let Err(err) = result {
            tracing::warn!(%err, user_id, staging_id, "TempVoice: Lane-Erstellung fehlgeschlagen");
        }
    }

    pub async fn create_router_lane(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        mode: &str,
        expected_channel_id: u64,
    ) -> Option<u64> {
        {
            let mut state = self.state.lock().await;
            if !state.creating.insert(user_id) {
                return None;
            }
        }
        let result = self
            .create_router_lane_inner(guild_id, user_id, mode, expected_channel_id, false)
            .await;
        self.state.lock().await.creating.remove(&user_id);
        match result {
            Ok(lane_id) => lane_id,
            Err(err) => {
                tracing::warn!(%err, user_id, mode, "TempVoice: Router-Lane-Erstellung fehlgeschlagen");
                None
            }
        }
    }

    pub async fn create_router_lane_if_alone(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        mode: &str,
        expected_channel_id: u64,
    ) -> Result<Option<u64>, String> {
        {
            let mut state = self.state.lock().await;
            if !state.creating.insert(user_id) {
                return Ok(None);
            }
        }
        let result = self
            .create_router_lane_inner(guild_id, user_id, mode, expected_channel_id, true)
            .await;
        self.state.lock().await.creating.remove(&user_id);
        result
    }

    async fn create_router_lane_inner(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        mode: &str,
        expected_channel_id: u64,
        require_sole_member: bool,
    ) -> Result<Option<u64>, String> {
        if require_sole_member
            && (self.port.member_voice_channel(guild_id, user_id).await
                != Some(expected_channel_id)
                || self
                    .port
                    .channel_members(guild_id, expected_channel_id)
                    .await
                    != [user_id])
        {
            return Ok(None);
        }
        if matches!(
            self.port.member_voice_channel(guild_id, user_id).await,
            Some(c) if c != expected_channel_id
        ) {
            return Ok(None);
        }
        let category_id = TEMPVOICE_ONE_CATEGORY_ID;
        let default_preset = self.store.get_default_preset(user_id).await.ok().flatten();
        let matching_default = default_preset
            .as_ref()
            .filter(|preset| preset.mode == mode)
            .cloned();
        let preset_base = matching_default
            .as_ref()
            .map(|preset| logic::strip_suffixes(&preset.base_name))
            .map(|base| base.trim().to_string())
            .filter(|base| !base.is_empty());
        let base = match preset_base {
            Some(base) => base,
            None => self.tempvoice_mode_name(guild_id, user_id, mode).await,
        };
        let cap = router_lane_limit(mode, matching_default.as_ref());
        let source_staging_id = crate::router::router_mode(mode).map(|mode| mode.staging_id);
        let lane_id = self
            .port
            .create_voice_channel(guild_id, Some(category_id), &base, cap)
            .await?;
        {
            let mut state = self.state.lock().await;
            state.lanes.insert(
                lane_id,
                LaneState {
                    owner_id: user_id,
                    initial_owner_id: user_id,
                    base_name: base.clone(),
                    min_rank: "unknown".to_string(),
                    category_id: Some(category_id),
                    prefix_from_rank: false,
                    source_staging_id,
                },
            );
            state.join_time.entry(lane_id).or_default();
        }
        if let Err(err) = self
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id,
                owner_id: user_id,
                initial_owner_id: Some(user_id),
                base_name: base.clone(),
                category_id,
                source_staging_id,
            })
            .await
        {
            tracing::warn!(%err, lane_id, "TempVoice: Router-Lane-Persist fehlgeschlagen");
        }
        let current_channel = self.port.member_voice_channel(guild_id, user_id).await;
        if matches!(current_channel, Some(c) if c != expected_channel_id)
            || (require_sole_member
                && (current_channel != Some(expected_channel_id)
                    || self
                        .port
                        .channel_members(guild_id, expected_channel_id)
                        .await
                        != [user_id]))
        {
            self.cleanup_lane(lane_id, "TempVoice: Router-Owner nicht mehr im VC")
                .await;
            return Ok(None);
        }
        if let Err(err) = self
            .port
            .move_member(guild_id, user_id, lane_id, "Router: neue Lane erstellt")
            .await
        {
            self.cleanup_lane(lane_id, "Router: Move fehlgeschlagen")
                .await;
            return Err(err);
        }
        self.apply_owner_settings(guild_id, lane_id, user_id).await;
        if let Some(preset) = matching_default.as_ref() {
            if preset.min_rank != "unknown" {
                if let Err(err) = self.set_min_rank(guild_id, lane_id, &preset.min_rank).await {
                    tracing::debug!(%err, lane_id, "TempVoice: Default-Mindest-Rang konnte nicht angewendet werden");
                }
            }
        }
        tracing::debug!(user_id, mode, category_id, lane_id, "Router: Lane erstellt");
        Ok(Some(lane_id))
    }

    async fn create_lane_inner(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        staging_id: u64,
        expected_channel_id: u64,
    ) -> Result<(), String> {
        // User noch im erwarteten Kanal (Staging bzw. Router-VC)? Der Gateway-
        // Cache kann direkt beim Join-Event den neuen Voice-State noch nicht
        // committed haben (Race) und liefert dann None — in dem Fall dem Event
        // vertrauen und fortfahren. Nur abbrechen, wenn der User nachweislich in
        // einem ANDEREN Kanal steht.
        if matches!(
            self.port.member_voice_channel(guild_id, user_id).await,
            Some(c) if c != expected_channel_id
        ) {
            return Ok(());
        }
        let rules = self.config.rules_for_staging(staging_id);
        let category_id = match self.config.target_category_for_staging(staging_id) {
            Some(category_id) => Some(category_id),
            None => self.port.channel_category(guild_id, staging_id).await,
        };
        let mode = mode_for_staging_id(staging_id);
        let in_minrank = category_id
            .map(|c| self.config.minrank_categories.contains(&c))
            .unwrap_or(false);
        let use_rank_name = mode.is_none() && (rules.prefix_from_rank || in_minrank);

        let base = if let Some(mode) = mode {
            self.tempvoice_mode_name(guild_id, user_id, mode).await
        } else {
            // Basis-Name: Rang (Pref → Rollen) oder "Prefix N"
            let rank_base = if use_rank_name {
                let (pref_rank, pref_sub) = self
                    .store
                    .rank_pref(user_id)
                    .await
                    .unwrap_or(("unknown".to_string(), 0));
                if pref_rank != "unknown" && logic::rank_index(&pref_rank) > 0 {
                    if pref_sub > 0 {
                        Some(format!("{} {}", logic::capitalize(&pref_rank), pref_sub))
                    } else {
                        Some(logic::capitalize(&pref_rank))
                    }
                } else {
                    let roles = self.port.member_role_names(guild_id, user_id).await;
                    logic::rank_prefix_for(&roles)
                }
            } else {
                None
            };
            match rank_base {
                Some(base) => base,
                None => {
                    let prefix = rules.prefix.clone().unwrap_or_else(|| "Lane".to_string());
                    let existing = match category_id {
                        Some(category) => {
                            self.port
                                .category_voice_channel_names(guild_id, category)
                                .await
                        }
                        None => Vec::new(),
                    };
                    logic::next_name(&existing, &prefix)
                }
            }
        };

        let cap = rules.user_limit.unwrap_or_else(|| match mode {
            Some("ranked") => logic::DEFAULT_RANKED_CAP,
            Some("street_brawl") => 4,
            _ => self.config.default_cap(category_id),
        });
        let lane_id = self
            .port
            .create_voice_channel(guild_id, category_id, &base, cap)
            .await?;

        {
            let mut state = self.state.lock().await;
            let disable_min_rank = rules.disable_min_rank || mode == Some("street_brawl");
            if disable_min_rank {
                state.minrank_blocked.insert(lane_id);
            }
            state.lanes.insert(
                lane_id,
                LaneState {
                    owner_id: user_id,
                    initial_owner_id: user_id,
                    base_name: base.clone(),
                    min_rank: "unknown".to_string(),
                    category_id,
                    prefix_from_rank: mode.is_none() && rules.prefix_from_rank,
                    source_staging_id: Some(staging_id),
                },
            );
            state.join_time.entry(lane_id).or_default();
        }
        if let Err(err) = self
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id,
                owner_id: user_id,
                initial_owner_id: Some(user_id),
                base_name: base.clone(),
                category_id: category_id.unwrap_or(0),
                source_staging_id: Some(staging_id),
            })
            .await
        {
            tracing::warn!(%err, lane_id, "TempVoice: Lane-Persist fehlgeschlagen");
        }

        // Owner noch im erwarteten Kanal? (None = Cache-Lag → Move trotzdem
        // versuchen; der Move-API-Call scheitert sauber, falls er doch weg ist.
        // Nur abbauen, wenn er nachweislich in einem ANDEREN Kanal steht.)
        if matches!(
            self.port.member_voice_channel(guild_id, user_id).await,
            Some(c) if c != expected_channel_id
        ) {
            self.cleanup_lane(lane_id, "TempVoice: Owner nicht mehr im Staging")
                .await;
            return Ok(());
        }
        if let Err(err) = self
            .port
            .move_member(guild_id, user_id, lane_id, "TempVoice: Auto-Lane erstellt")
            .await
        {
            self.cleanup_lane(lane_id, "TempVoice: Move fehlgeschlagen")
                .await;
            return Err(err);
        }
        self.apply_owner_settings(guild_id, lane_id, user_id).await;
        tracing::info!(lane_id, user_id, staging_id, base = %base, "TempVoice: Lane erstellt");
        Ok(())
    }

    /// Join-Reihenfolge der aktuellen Kanal-Member (ältester zuerst).
    pub async fn join_order(&self, guild_id: u64, channel_id: u64) -> Vec<(u64, i64)> {
        let members = self.port.channel_members(guild_id, channel_id).await;
        let state = self.state.lock().await;
        let times = state.join_time.get(&channel_id);
        let mut ranked: Vec<(u64, i64)> = members
            .into_iter()
            .map(|user_id| {
                let ts = times
                    .and_then(|t| t.get(&user_id))
                    .map(|t| t.and_utc().timestamp())
                    .unwrap_or(i64::MAX);
                (user_id, ts)
            })
            .collect();
        ranked.sort_by_key(|(_, ts)| *ts);
        ranked
    }

    /// Claim-Regeln wie evaluate_owner_claim: Owner weg UND Top-3 UND 20 min.
    pub async fn evaluate_claim(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
    ) -> Result<(), String> {
        let owner = self.lane_owner(channel_id).await;
        if owner == Some(user_id) {
            return Err("Du bist bereits Owner dieser Lane.".to_string());
        }
        let ranked = self.join_order(guild_id, channel_id).await;
        let now = Utc::now().timestamp();
        let Some(position) = ranked.iter().position(|(id, _)| *id == user_id) else {
            return Err(
                "Owner-Claim derzeit nicht möglich: du bist nicht mehr sauber in dieser Lane erfasst."
                    .to_string(),
            );
        };
        let owner_present = owner
            .map(|owner_id| ranked.iter().any(|(id, _)| *id == owner_id))
            .unwrap_or(false);
        let mut details: Vec<String> = Vec::new();
        if owner_present {
            details.push("der aktuelle Owner ist noch im Channel".to_string());
        }
        if position >= logic::OWNER_CLAIM_TOP_N {
            details.push(format!(
                "du bist aktuell Platz {} nach Verbindungszeit; claimen dürfen nur die ersten {}",
                position + 1,
                logic::OWNER_CLAIM_TOP_N
            ));
        }
        let connected = ranked
            .iter()
            .find(|(id, _)| *id == user_id)
            .map(|(_, ts)| {
                if *ts == i64::MAX {
                    0
                } else {
                    (now - ts).max(0)
                }
            })
            .unwrap_or(0);
        if connected < logic::OWNER_CLAIM_MIN_SECONDS {
            details.push(format!(
                "du bist erst seit {}m {}s im Channel; mindestens 20 Minuten sind nötig",
                connected / 60,
                connected % 60
            ));
        }
        if details.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Owner-Claim derzeit nicht möglich: {}.",
                details.join("; ")
            ))
        }
    }

    /// Owner-Wechsel (Claim/Transfer): State + DB + Bann-Swap.
    pub async fn claim_owner(self: &Arc<Self>, guild_id: u64, channel_id: u64, new_owner: u64) {
        let previous = {
            let mut state = self.state.lock().await;
            let Some(lane) = state.lanes.get_mut(&channel_id) else {
                return;
            };
            let previous = lane.owner_id;
            if previous == new_owner {
                return;
            }
            lane.owner_id = new_owner;
            previous
        };
        if let Err(err) = self.store.set_owner(channel_id, new_owner).await {
            tracing::warn!(%err, channel_id, "TempVoice: Claim-Persist fehlgeschlagen");
        }
        self.clear_owner_bans(channel_id, previous).await;
        self.apply_owner_default_preset(guild_id, channel_id, new_owner)
            .await;
        self.apply_owner_settings(guild_id, channel_id, new_owner)
            .await;
    }

    /// Limit setzen (Street-Brawl-Regel kappt auf max_limit).
    pub async fn set_limit(&self, channel_id: u64, requested: i64) -> Result<i64, String> {
        let max_limit = {
            let state = self.state.lock().await;
            state.lanes.get(&channel_id).and_then(|lane| {
                lane.source_staging_id
                    .and_then(|s| self.config.staging_rules.get(&s))
                    .and_then(|r| r.max_limit.or(r.user_limit))
            })
        };
        let effective = match max_limit {
            Some(max) => requested.clamp(1, max),
            None => requested.clamp(0, 99),
        };
        self.port
            .set_user_limit(channel_id, effective, "TempVoice: Limit gesetzt")
            .await?;
        Ok(effective)
    }

    /// Region DE = English-Only-Rolle deny; EU = Overwrite weg. Persistiert Pref.
    pub async fn set_region(&self, channel_id: u64, owner_id: u64, region: &str) {
        let _ = self.store.set_region_pref(owner_id, region).await;
        let connect = if region == "DE" { Some(false) } else { None };
        let _ = self
            .port
            .set_role_connect(channel_id, ENGLISH_ONLY_ROLE_ID, connect)
            .await;
    }

    /// (base_name, category_id) der Lane — fürs Interface.
    pub async fn lane_snapshot(&self, channel_id: u64) -> Option<(String, u64)> {
        let state = self.state.lock().await;
        state
            .lanes
            .get(&channel_id)
            .map(|lane| (lane.base_name.clone(), lane.category_id.unwrap_or(0)))
    }

    /// Router-Modus der Lane, sofern sie eine ist (`casual`/`ranked`/
    /// `street_brawl`). Basis ist die Staging-Quelle, aus der sie entstand.
    pub async fn lane_mode(&self, channel_id: u64) -> Option<&'static str> {
        let state = self.state.lock().await;
        state
            .lanes
            .get(&channel_id)
            .and_then(|lane| lane.source_staging_id)
            .and_then(mode_for_staging_id)
    }

    pub async fn lane_is_ranked(&self, channel_id: u64) -> bool {
        let state = self.state.lock().await;
        let Some(lane) = state.lanes.get(&channel_id) else {
            return false;
        };
        if let Some(source_staging_id) = lane.source_staging_id {
            return mode_for_staging_id(source_staging_id) == Some("ranked");
        }
        let lower = lane.base_name.trim().to_ascii_lowercase();
        lower == "ranked" || lower.starts_with("ranked ")
    }

    pub async fn lane_preset_snapshot(&self, channel_id: u64) -> Option<(String, u64, String)> {
        let state = self.state.lock().await;
        state.lanes.get(&channel_id).map(|lane| {
            (
                lane.base_name.clone(),
                lane.category_id.unwrap_or(0),
                lane.min_rank.clone(),
            )
        })
    }

    pub async fn lane_owner_or_actor(&self, channel_id: u64, actor_id: u64) -> u64 {
        self.lane_owner(channel_id).await.unwrap_or(actor_id)
    }

    /// Owner-Rename: Basisnamen mitführen, damit refresh_name nicht zurücksetzt.
    pub async fn set_base_name(&self, channel_id: u64, name: &str) {
        let base_name = logic::strip_suffixes(name);
        let mut state = self.state.lock().await;
        if let Some(lane) = state.lanes.get_mut(&channel_id) {
            lane.base_name = base_name.clone();
        }
        drop(state);
        let _ = self.store.set_lane_base(channel_id, base_name).await;
    }

    pub async fn set_lane_template(
        self: &Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        base_name: &str,
        limit: i64,
    ) -> Result<i64, String> {
        let base = logic::strip_suffixes(base_name);
        if base.trim().is_empty() {
            return Err("empty base name".to_string());
        }
        self.set_base_name(channel_id, &base).await;
        let effective = self.set_limit(channel_id, limit).await?;
        let current = self.port.channel_name(guild_id, channel_id).await;
        if current.as_deref().is_some_and(logic::has_live_suffix) {
            return Ok(effective);
        }
        let lane = {
            let state = self.state.lock().await;
            state.lanes.get(&channel_id).cloned()
        };
        let desired = match lane {
            Some(lane) => self.desired_lane_name(guild_id, &lane).await,
            None => base,
        };
        self.port
            .rename_channel(channel_id, &desired, "TempVoice: Template")
            .await?;
        Ok(effective)
    }

    pub async fn reset_lane_template_base(&self, guild_id: u64, channel_id: u64) -> String {
        let (base_name, category_id, source_staging_id) = {
            let state = self.state.lock().await;
            let lane = state.lanes.get(&channel_id);
            (
                lane.map(|lane| lane.base_name.clone()).unwrap_or_default(),
                lane.and_then(|lane| lane.category_id),
                lane.and_then(|lane| lane.source_staging_id),
            )
        };
        let category_id = match category_id {
            Some(category_id) if category_id > 0 => Some(category_id),
            _ => self.port.channel_category(guild_id, channel_id).await,
        };
        let rules = source_staging_id
            .map(|staging_id| self.config.rules_for_staging(staging_id))
            .unwrap_or_else(|| self.rules_for_category(category_id).0);
        let prefix = rules.prefix.as_deref().unwrap_or("Lane");
        let base_lower = base_name.to_lowercase();
        let prefix_lower = prefix.to_lowercase();
        let already_numbered = base_lower
            .strip_prefix(&(prefix_lower + " "))
            .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()));
        if already_numbered {
            return base_name;
        }
        let Some(category_id) = category_id else {
            return format!("{prefix} 1");
        };
        let names = self
            .port
            .category_voice_channel_names(guild_id, category_id)
            .await;
        logic::next_name(&names, prefix)
    }

    pub async fn default_limit_for_lane(&self, channel_id: u64) -> i64 {
        let category_id = self
            .state
            .lock()
            .await
            .lanes
            .get(&channel_id)
            .and_then(|lane| lane.category_id);
        self.config.default_cap(category_id)
    }

    pub async fn status_base_name(&self, guild_id: u64, channel_id: u64) -> Option<String> {
        let (base_name, prefix_from_rank, owner_id) = {
            let state = self.state.lock().await;
            let lane = state.lanes.get(&channel_id)?;
            (lane.base_name.clone(), lane.prefix_from_rank, lane.owner_id)
        };
        if !prefix_from_rank {
            return Some(base_name);
        }
        let rank_base = match self.store.rank_pref(owner_id).await {
            Ok((rank, sub)) if rank != "unknown" => {
                let mut label = logic::capitalize(&rank);
                if sub > 0 {
                    label.push_str(&format!(" {sub}"));
                }
                Some(label)
            }
            _ => {
                let roles = self.port.member_role_names(guild_id, owner_id).await;
                logic::rank_prefix_for(&roles)
            }
        };
        Some(rank_base.unwrap_or(base_name))
    }

    pub async fn handle_category_changed(
        self: &Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        after_category_id: Option<u64>,
    ) {
        if self.config.fixed_lane_ids.contains(&channel_id)
            || self.config.staging_channels.contains(&channel_id)
        {
            return;
        }
        let Some(category_id) = after_category_id else {
            return;
        };
        if !self.config.tempvoice_categories.contains(&category_id) {
            return;
        }
        let known = self.state.lock().await.lanes.contains_key(&channel_id);
        if !known {
            return;
        }
        let rules = self.apply_category_source(channel_id, category_id).await;
        let base = {
            let state = self.state.lock().await;
            state
                .lanes
                .get(&channel_id)
                .map(|lane| lane.base_name.clone())
                .unwrap_or_else(|| "Lane".to_string())
        };
        let desired_limit = rules
            .user_limit
            .unwrap_or_else(|| self.config.default_cap(Some(category_id)));
        let _ = self
            .set_lane_template(guild_id, channel_id, &base, desired_limit)
            .await;
    }

    pub async fn set_tag_service(&self, tags: Arc<dl_community::tags::TagService>) {
        *self.tags.write().await = Some(tags);
    }

    pub async fn set_adaptive(&self, adaptive: Arc<crate::adaptive::AdaptiveLanes>) {
        *self.adaptive.write().await = Some(adaptive);
    }

    /// Blockier-Grund wie `_member_block_reason`: nur min_age + ragebaiter
    /// (required_tone_tag ist im Original toter Zweig — s. Store-Doku).
    async fn member_block_reason(
        &self,
        tags: &Arc<dl_community::tags::TagService>,
        user_id: u64,
        filter: &super::store::LaneTagFilter,
    ) -> Option<&'static str> {
        if let Some(min_age) = &filter.min_age_tag {
            let user_tags = tags.get_user_tags(user_id).await;
            if user_tags.get("age").map(String::as_str) != Some(min_age.as_str()) {
                return Some("min_age");
            }
        }
        if filter.deny_ragebaiter && tags.has_active_mod_tag(user_id, "ragebaiter").await {
            return Some("ragebaiter");
        }
        None
    }

    /// Filter auf Mitglieder anwenden (wie `_apply_tag_filter`).
    pub async fn apply_tag_filter(
        &self,
        guild_id: u64,
        channel_id: u64,
        target_members: Option<Vec<u64>>,
        disconnect_blocked: bool,
    ) {
        let filter = self.store.lane_tag_filter(channel_id).await;
        let members = match target_members {
            Some(members) => members,
            None => self.port.channel_members(guild_id, channel_id).await,
        };
        if !filter.is_enabled() {
            for user_id in members {
                self.set_tag_block(channel_id, user_id, false).await;
            }
            return;
        }
        let Some(tags) = self.tags.read().await.clone() else {
            return; // wie Original ohne TagService-Cog
        };
        for user_id in members {
            let blocked = self
                .member_block_reason(&tags, user_id, &filter)
                .await
                .is_some();
            self.set_tag_block(channel_id, user_id, blocked).await;
            if blocked && disconnect_blocked {
                let _ = self
                    .port
                    .disconnect_member(guild_id, user_id, "TempVoice: Tag-Filter enforced")
                    .await;
            }
        }
    }

    /// Connect-Overwrite setzen/räumen mit Bann-Schutz beim Räumen
    /// (wie `_set_tag_filter_permission`).
    async fn set_tag_block(&self, channel_id: u64, user_id: u64, deny: bool) {
        if deny {
            let fresh = {
                let mut state = self.state.lock().await;
                state
                    .tag_blocked
                    .entry(channel_id)
                    .or_default()
                    .insert(user_id)
            };
            if fresh {
                let _operation = self.voice_pair_operations.lock().await;
                let _ = self
                    .port
                    .set_member_connect(channel_id, user_id, Some(false))
                    .await;
            }
            return;
        }
        let was_blocked = {
            let mut state = self.state.lock().await;
            state
                .tag_blocked
                .get_mut(&channel_id)
                .map(|set| set.remove(&user_id))
                .unwrap_or(false)
        };
        if !was_blocked {
            return;
        }
        let _operation = self.voice_pair_operations.lock().await;
        // Owner-Bann hat Vorrang: Overwrite dann NICHT löschen
        if let Some(owner_id) = self.lane_owner(channel_id).await {
            if self
                .store
                .is_banned_by_owner(owner_id, user_id)
                .await
                .unwrap_or(false)
            {
                return;
            }
        }
        let _ = self
            .port
            .set_member_connect(channel_id, user_id, None)
            .await;
    }

    /// Filter speichern + sofort durchsetzen (Panel-Save).
    pub async fn save_tag_filter(
        &self,
        guild_id: u64,
        channel_id: u64,
        filter: super::store::LaneTagFilter,
    ) -> Result<(), String> {
        self.store
            .set_lane_tag_filter(channel_id, filter)
            .await
            .map_err(|e| e.to_string())?;
        self.apply_tag_filter(guild_id, channel_id, None, true)
            .await;
        Ok(())
    }

    /// Lurker-Status umschalten (wie util.toggle_lurker) → Antworttext.
    pub async fn toggle_lurker(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
    ) -> (bool, String) {
        let display = self
            .port
            .member_display_name(guild_id, user_id)
            .await
            .unwrap_or_else(|| format!("User {user_id}"));
        let limit = self
            .port
            .channel_user_limit(guild_id, channel_id)
            .await
            .unwrap_or(0);

        if let Some(original_nick) = self.store.get_lurker(channel_id, user_id).await {
            // Lurker entfernen: Rolle weg, Nick zurück, Limit-1
            if let Err(err) = self
                .port
                .remove_role(
                    guild_id,
                    user_id,
                    LURKER_ROLE_ID,
                    "TempVoice: Remove Lurker",
                )
                .await
            {
                return (false, format!("Konnte Rolle nicht entfernen: {err}"));
            }
            let _ = self
                .port
                .set_nick(
                    guild_id,
                    user_id,
                    original_nick.as_deref(),
                    "TempVoice: Restore Nick",
                )
                .await;
            if limit > 0 {
                let _ = self
                    .port
                    .set_user_limit(channel_id, (limit - 1).max(0), "TempVoice: Lurker removed")
                    .await;
            }
            let _ = self.store.remove_lurker(channel_id, user_id).await;
            return (true, format!("{display} ist kein Lurker mehr."));
        }

        // Lurker hinzufügen: DB zuerst (Rollback bei Rollen-Fehler), Nick, Limit+1
        let original_nick = self.port.member_nick(guild_id, user_id).await;
        if self
            .store
            .add_lurker(guild_id, channel_id, user_id, original_nick)
            .await
            .is_err()
        {
            return (
                false,
                "Datenbankfehler beim Hinzufügen des Lurker-Status.".to_string(),
            );
        }
        if let Err(err) = self
            .port
            .add_role(guild_id, user_id, LURKER_ROLE_ID, "TempVoice: Make Lurker")
            .await
        {
            let _ = self.store.remove_lurker(channel_id, user_id).await;
            return (false, format!("Konnte Rolle nicht vergeben: {err}"));
        }
        let _ = self
            .port
            .set_nick(guild_id, user_id, Some("Lurker"), "TempVoice: Make Lurker")
            .await;
        if limit > 0 {
            let _ = self
                .port
                .set_user_limit(channel_id, (limit + 1).min(99), "TempVoice: Lurker added")
                .await;
        }
        (true, format!("{display} ist jetzt Lurker."))
    }

    /// Lurker-Aufräumen beim Verlassen (wie der on_voice_state_update-Block).
    async fn cleanup_lurker_on_leave(&self, guild_id: u64, channel_id: u64, user_id: u64) {
        let Some(original_nick) = self.store.get_lurker(channel_id, user_id).await else {
            return;
        };
        let _ = self.store.remove_lurker(channel_id, user_id).await;
        let _ = self
            .port
            .remove_role(guild_id, user_id, LURKER_ROLE_ID, "TempVoice: Lurker left")
            .await;
        let _ = self
            .port
            .set_nick(
                guild_id,
                user_id,
                original_nick.as_deref(),
                "TempVoice: Lurker left",
            )
            .await;
        if let Some(limit) = self.port.channel_user_limit(guild_id, channel_id).await {
            if limit > 0 {
                let _ = self
                    .port
                    .set_user_limit(channel_id, (limit - 1).max(0), "TempVoice: Lurker left")
                    .await;
            }
        }
    }

    async fn cleanup_tag_block_on_leave(&self, channel_id: u64, user_id: u64) {
        self.set_tag_block(channel_id, user_id, false).await;
    }

    /// Min-Rang setzen: Rang-Rollen unter der Schwelle bekommen connect=deny,
    /// ab Schwelle wird das Overwrite geräumt; "unknown" räumt alles
    /// (wie `_apply_min_rank`). Aktualisiert auch das " • ab X"-Suffix.
    pub async fn set_min_rank(
        self: &Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        min_rank: &str,
    ) -> Result<(), String> {
        if self.is_min_rank_blocked(channel_id).await {
            return Err(MIN_RANK_DISABLED_REPLY.to_string());
        }
        let in_minrank = {
            let state = self.state.lock().await;
            state
                .lanes
                .get(&channel_id)
                .and_then(|lane| lane.category_id)
                .map(|category| self.config.minrank_categories.contains(&category))
                .unwrap_or(false)
        };
        if !in_minrank {
            return Err("Min-Rang gilt nur für Comp/Ranked-Lanes.".to_string());
        }
        let min_rank = min_rank.trim().to_lowercase();
        // Gültigkeit über rank_score prüfen, nicht rank_index: so werden auch
        // Sub-Rang-Labels wie "phantom 3" akzeptiert (rank_index kennt nur die
        // 11 Haupt-Ränge und würde sie fälschlich als "unbekannt" abweisen).
        if min_rank != "unknown" && logic::rank_score(&min_rank) == 0 {
            return Err(format!("Unbekannter Rang: {min_rank}"));
        }

        let min_score = logic::rank_score(&min_rank);
        let mut allowed_role_ids = HashSet::new();
        let mut clear_role_ids = HashSet::new();
        let roles = self.port.guild_role_names(guild_id).await;
        for (role_id, name) in roles {
            let score = logic::rank_score(&name);
            if score == 0 {
                continue; // keine Rang-Rolle
            }
            if min_rank != "unknown" && score >= min_score {
                allowed_role_ids.insert(role_id);
            } else {
                clear_role_ids.insert(role_id);
            }
        }
        self.port
            .apply_role_connect_batch(guild_id, channel_id, &allowed_role_ids, &clear_role_ids)
            .await
            .map_err(|err| format!("Rang-Rechte konnten nicht aktualisiert werden: {err}"))?;

        {
            let mut state = self.state.lock().await;
            if let Some(lane) = state.lanes.get_mut(&channel_id) {
                lane.min_rank = min_rank.clone();
            }
        }
        self.refresh_name(guild_id, channel_id).await;
        Ok(())
    }

    pub async fn rank_gate_active(&self, channel_id: u64) -> bool {
        crate::rank::RankStore {
            pool: self.store.pool.clone(),
        }
        .load_anchors()
        .await
        .contains_key(&channel_id)
    }

    pub async fn apply_rank_gate(
        &self,
        guild_id: u64,
        channel_id: u64,
        owner_id: u64,
        rank: &str,
        subrank: i64,
        tolerance: i64,
    ) -> Result<(), String> {
        let rank = rank.trim().to_lowercase();
        let rank_value = logic::rank_index(&rank) as i64;
        if rank_value == 0 {
            return Err(format!("Unbekannter Rang: {rank}"));
        }
        let subrank = subrank.clamp(1, 6);
        let tolerance = tolerance.clamp(0, 24);
        let (score_min, score_max, allowed_min, allowed_max) =
            crate::rank::anchor_range_with_tolerance(rank_value, subrank, tolerance);
        let guild_roles = self.port.guild_role_names(guild_id).await;
        let allowed = crate::rank::allowed_subrank_roles(&guild_roles, score_min, score_max);
        let mut rank_roles: HashSet<u64> = guild_roles
            .iter()
            .filter_map(|(role_id, name)| (logic::rank_score(name) > 0).then_some(*role_id))
            .collect();
        rank_roles.extend(crate::rank::major_rank_roles().keys().copied());
        let clear: HashSet<u64> = rank_roles.difference(&allowed).copied().collect();
        let denied = HashSet::from([guild_id]);
        self.port
            .apply_role_connect_overwrites(guild_id, channel_id, &allowed, &denied, &clear)
            .await?;
        crate::rank::RankStore {
            pool: self.store.pool.clone(),
        }
        .upsert_anchor(
            channel_id,
            guild_id,
            crate::rank::Anchor {
                user_id: owner_id,
                rank_name: logic::capitalize(&rank),
                rank_value,
                allowed_min,
                allowed_max,
                subrank,
                score_min,
                score_max,
            },
        )
        .await;
        Ok(())
    }

    pub async fn clear_rank_gate(&self, guild_id: u64, channel_id: u64) -> Result<(), String> {
        let guild_roles = self.port.guild_role_names(guild_id).await;
        let mut clear: HashSet<u64> = guild_roles
            .iter()
            .filter_map(|(role_id, name)| (logic::rank_score(name) > 0).then_some(*role_id))
            .collect();
        clear.extend(crate::rank::major_rank_roles().keys().copied());
        clear.insert(guild_id);
        self.port
            .apply_role_connect_overwrites(
                guild_id,
                channel_id,
                &HashSet::new(),
                &HashSet::new(),
                &clear,
            )
            .await?;
        crate::rank::RankStore {
            pool: self.store.pool.clone(),
        }
        .delete_anchor(channel_id)
        .await;
        Ok(())
    }

    /// Lane in einen anderen Modus umziehen (wie `switch_lane_mode`).
    /// Rückgabe: Fehlertext oder None bei Erfolg.
    pub async fn switch_lane_mode(
        self: &Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        owner_id: u64,
        new_mode: &str,
    ) -> Option<String> {
        let category_id = match new_mode {
            "ranked" | "casual" | "street_brawl" => TEMPVOICE_ONE_CATEGORY_ID,
            "off_topic" => 1513468298728308757,
            _ => return Some("Unbekannter Modus.".to_string()),
        };
        if let Err(err) = self
            .port
            .set_channel_category(
                channel_id,
                category_id,
                &format!("Modus-Wechsel → {new_mode}"),
            )
            .await
        {
            return Some(format!("Fehler beim Verschieben: {err}"));
        }
        if matches!(new_mode, "ranked" | "casual" | "street_brawl") {
            let source_staging_id =
                crate::router::router_mode(new_mode).map(|mode| mode.staging_id);
            let mut state = self.state.lock().await;
            if let Some(lane) = state.lanes.get_mut(&channel_id) {
                lane.category_id = Some(category_id);
                lane.prefix_from_rank = false;
                lane.source_staging_id = source_staging_id;
                lane.min_rank = "unknown".to_string();
            }
            if new_mode == "street_brawl" {
                state.minrank_blocked.insert(channel_id);
            } else {
                state.minrank_blocked.remove(&channel_id);
            }
            drop(state);
            let _ = self
                .store
                .set_lane_category_source(channel_id, category_id, source_staging_id)
                .await;
        } else {
            self.apply_category_source(channel_id, category_id).await;
        }
        let desired_limit = match new_mode {
            "street_brawl" => 4,
            "ranked" => logic::DEFAULT_RANKED_CAP,
            _ => self.config.default_cap(Some(category_id)),
        };
        let _ = self.set_limit(channel_id, desired_limit).await;
        let new_name = if matches!(new_mode, "ranked" | "casual" | "street_brawl") {
            self.tempvoice_mode_name(guild_id, owner_id, new_mode).await
        } else {
            let snapshot = self.lane_snapshot(channel_id).await;
            snapshot
                .map(|(base, _)| base)
                .unwrap_or_else(|| "Lane".to_string())
        };
        let _ = self
            .port
            .rename_channel(channel_id, &new_name, &format!("Modus → {new_mode}"))
            .await;
        self.set_base_name(channel_id, &new_name).await;
        None
    }

    pub async fn cleanup_lane(&self, channel_id: u64, reason: &str) {
        {
            let mut state = self.state.lock().await;
            state.lanes.remove(&channel_id);
            state.join_time.remove(&channel_id);
        }
        if let Err(err) = self.store.delete_lane(channel_id).await {
            tracing::warn!(%err, channel_id, "TempVoice: Lane-DB-Delete fehlgeschlagen");
        }
        if let Err(err) = self.port.delete_channel(channel_id, reason).await {
            tracing::debug!(%err, channel_id, "TempVoice: Channel-Delete fehlgeschlagen");
        }
        let lfg = self.lfg.read().await.as_ref().and_then(Weak::upgrade);
        if let Some(lfg) = lfg {
            lfg.on_lane_deleted(channel_id).await;
        }
    }

    async fn desired_lane_name(&self, guild_id: u64, lane: &LaneState) -> String {
        let base = if lane.prefix_from_rank {
            // Dynamischer Rang-Prefix aus Owner-Pref/Rollen + Lane-Nummer
            let (pref_rank, _) = self
                .store
                .rank_pref(lane.owner_id)
                .await
                .unwrap_or(("unknown".to_string(), 0));
            let prefix = if pref_rank != "unknown" && logic::rank_index(&pref_rank) > 0 {
                logic::capitalize(&pref_rank)
            } else {
                let roles = self.port.member_role_names(guild_id, lane.owner_id).await;
                logic::rank_prefix_for(&roles).unwrap_or_else(|| "Lane".to_string())
            };
            match logic::extract_lane_number(&lane.base_name) {
                Some(number) => format!("{prefix} {number}"),
                None => prefix,
            }
        } else {
            lane.base_name.clone()
        };

        let in_minrank = lane
            .category_id
            .map(|c| self.config.minrank_categories.contains(&c))
            .unwrap_or(false);
        logic::compose_name(&base, &lane.min_rank, in_minrank)
    }

    /// Name aktualisieren — nur im Create-Fenster (45s) außer prefix_from_rank;
    /// nie bei LiveMatch-Suffix.
    async fn refresh_name(self: &Arc<Self>, guild_id: u64, channel_id: u64) {
        let Some(current) = self.port.channel_name(guild_id, channel_id).await else {
            return;
        };
        if logic::has_live_suffix(&current) {
            return;
        }
        let lane = {
            let state = self.state.lock().await;
            state.lanes.get(&channel_id).cloned()
        };
        let Some(lane) = lane else { return };

        if !lane.prefix_from_rank {
            let age = self
                .port
                .channel_created_at(channel_id)
                .await
                .map(|created| Utc::now().timestamp() - created)
                .unwrap_or(i64::MAX);
            if age > logic::CREATE_RENAME_WINDOW_SEC {
                return;
            }
        }

        let desired = self.desired_lane_name(guild_id, &lane).await;
        if desired != current {
            let _ = self
                .port
                .rename_channel(channel_id, &desired, "TempVoice: Name aktualisiert")
                .await;
        }
    }

    /// Owner-Bans als Connect-Overwrites auf die Lane legen.
    async fn apply_owner_bans(&self, _guild_id: u64, channel_id: u64, owner_id: u64) {
        let _operation = self.voice_pair_operations.lock().await;
        let bans = self.store.list_bans(owner_id).await.unwrap_or_default();
        if bans.is_empty() {
            return;
        }
        let denied: HashSet<u64> = bans.into_iter().collect();
        let _ = self
            .port
            .apply_member_connect_batch(channel_id, &denied, &HashSet::new())
            .await;
    }

    async fn apply_owner_settings(&self, guild_id: u64, channel_id: u64, owner_id: u64) {
        let region = self
            .store
            .region_pref(owner_id)
            .await
            .unwrap_or_else(|_| "EU".to_string());
        let connect = if region == "DE" { Some(false) } else { None };
        let _ = self
            .port
            .set_role_connect(channel_id, ENGLISH_ONLY_ROLE_ID, connect)
            .await;
        self.apply_owner_bans(guild_id, channel_id, owner_id).await;
        self.apply_tag_filter(guild_id, channel_id, None, true)
            .await;
    }

    async fn apply_owner_default_preset(
        self: &Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        owner_id: u64,
    ) {
        let Some(default) = self.store.get_default_preset(owner_id).await.ok().flatten() else {
            // Kein gespeichertes Preset: Modus, Limit und Rang-Gate bleiben
            // unangetastet — nur der Name folgt dem neuen Owner und seinem
            // Rang, auch wenn die Lane schon aelter als das 45s-Fenster ist.
            // Fixes #409, ohne fremden Lane-Zustand zu ueberschreiben (vorher
            // wurde hier ein synthetisches Preset erfunden und der volle
            // Preset-Apply-Pfad gefahren, was Limit und Rang-Gate der Lane
            // fuer jeden Owner ohne gespeichertes Preset zurueckgesetzt hat).
            // Baut wie set_lane_template ausschliesslich auf lane.base_name
            // auf, nie auf dem live angezeigten Channel-Namen: der kann einen
            // LiveMatch-Suffix tragen, den strip_suffixes nicht kennt, sonst
            // landet "· 3/6 Im Match" mitten im neuen Namen und wird als
            // Basis persistiert. Das Rang-Gate-Suffix haengt compose_name
            // wieder an, statt es beim Claim stillschweigend zu verlieren
            // (lane.min_rank bleibt unveraendert aktiv, nur die Anzeige hatte
            // es vorher verloren).
            let mode = self.lane_mode(channel_id).await.unwrap_or("casual");
            let lane = {
                let state = self.state.lock().await;
                state.lanes.get(&channel_id).cloned()
            };
            let Some(lane) = lane else { return };
            let base = self
                .claimed_owner_lane_base(guild_id, owner_id, &lane.base_name, mode)
                .await;
            self.set_base_name(channel_id, &base).await;
            let current = self.port.channel_name(guild_id, channel_id).await;
            if current.as_deref().is_some_and(logic::has_live_suffix) {
                // Nie ueber ein laufendes Match renamen (wie refresh_name /
                // set_lane_template); base_name ist schon persistiert und
                // greift, sobald der naechste Refresh nach Matchende laeuft.
                return;
            }
            let in_minrank = lane
                .category_id
                .map(|c| self.config.minrank_categories.contains(&c))
                .unwrap_or(false);
            let desired = logic::compose_name(&base, &lane.min_rank, in_minrank);
            if current.as_deref() != Some(desired.as_str()) {
                let _ = self
                    .port
                    .rename_channel(channel_id, &desired, "TempVoice: Claim-Name")
                    .await;
            }
            return;
        };
        if let Some(err) = self
            .switch_lane_mode(guild_id, channel_id, owner_id, &default.mode)
            .await
        {
            tracing::debug!(%err, channel_id, owner_id, "TempVoice: Owner-Preset-Modus nicht angewendet");
            return;
        }
        if let Err(err) = self
            .set_lane_template(guild_id, channel_id, &default.base_name, default.limit)
            .await
        {
            tracing::debug!(%err, channel_id, owner_id, "TempVoice: Owner-Preset-Template nicht angewendet");
        }
        if default.min_rank != "unknown" {
            if let Err(err) = self
                .set_min_rank(guild_id, channel_id, &default.min_rank)
                .await
            {
                tracing::debug!(%err, channel_id, owner_id, "TempVoice: Owner-Preset-Rang nicht angewendet");
            }
        }
    }

    /// Basis-Name wie beim normalen Lane-Apply: Modus plus Rang des neuen
    /// Owners, bestehende Nummer bleibt. Baut auf lane.base_name auf, nie
    /// auf dem live angezeigten Channel-Namen; das Rang-Gate-Suffix haengt
    /// der Aufrufer ueber compose_name an.
    async fn claimed_owner_lane_base(
        &self,
        guild_id: u64,
        owner_id: u64,
        lane_base: &str,
        mode: &str,
    ) -> String {
        let number = logic::extract_lane_number(lane_base);
        match mode {
            "ranked" => {
                let prefix = self
                    .owner_rank_anchor(guild_id, owner_id)
                    .await
                    .map(|rank| format!("Ranked {rank}"))
                    .unwrap_or_else(|| "Ranked".to_string());
                match number {
                    Some(number) => format!("{prefix} {number}"),
                    None => prefix,
                }
            }
            "street_brawl" => match number {
                Some(number) => format!("Street Brawl {number}"),
                None => "Street Brawl".to_string(),
            },
            _ => {
                let name = match lane_base.find(" · ") {
                    Some(idx) => lane_base[..idx].trim().to_string(),
                    None => {
                        if lane_base.trim().is_empty() {
                            "Chill Lane".to_string()
                        } else {
                            lane_base.to_string()
                        }
                    }
                };
                match self.owner_rank_anchor(guild_id, owner_id).await {
                    Some(rank) => format!("{name} · {rank}"),
                    None => name,
                }
            }
        }
    }

    async fn clear_owner_bans(&self, channel_id: u64, owner_id: u64) {
        let _operation = self.voice_pair_operations.lock().await;
        let bans = self.store.list_bans(owner_id).await.unwrap_or_default();
        if bans.is_empty() {
            return;
        }
        let cleared: HashSet<u64> = bans.into_iter().collect();
        let _ = self
            .port
            .apply_member_connect_batch(channel_id, &HashSet::new(), &cleared)
            .await;
    }
}

/// Ragebaiter-Tag gesetzt → Lane des Users sofort nachziehen
/// (wie `on_mod_tag_added`: nur wenn der Lane-Filter deny_ragebaiter hat).
pub fn spawn_tag_listener(
    engine: Arc<TempVoiceEngine>,
    tags: Arc<dl_community::tags::TagService>,
) -> tokio::task::JoinHandle<()> {
    let mut events = tags.subscribe();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_community::tags::TagEvent::ModTagAdded { user_id, tag, .. })
                    if tag == "ragebaiter" =>
                {
                    // Lane des Users finden (über alle bekannten Lanes)
                    let lanes: Vec<(u64, u64)> = {
                        let state = engine.state.lock().await;
                        state.lanes.keys().map(|id| (*id, user_id)).collect()
                    };
                    for (channel_id, user_id) in lanes {
                        let filter = engine.store.lane_tag_filter(channel_id).await;
                        if !filter.deny_ragebaiter {
                            continue;
                        }
                        // Nur anwenden, wenn der User wirklich in dieser Lane sitzt
                        let guild_id = engine.config.guild_id_hint;
                        let members = engine.port.channel_members(guild_id, channel_id).await;
                        if members.contains(&user_id) {
                            engine
                                .apply_tag_filter(guild_id, channel_id, Some(vec![user_id]), true)
                                .await;
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Engine als Dispatcher-Subscriber starten.
pub fn spawn(engine: Arc<TempVoiceEngine>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_voice();
    let mut channel_events = dispatcher.subscribe_channels();
    let mut gateway_events = dispatcher.subscribe_gateway();
    let purge_engine = engine.clone();
    tokio::spawn(async move {
        wait_for_cache_ready(&mut gateway_events, purge_engine.config.guild_id_hint).await;
        loop {
            purge_engine.purge_empty_lanes().await;
            tokio::time::sleep(std::time::Duration::from_secs(PURGE_INTERVAL_SECONDS)).await;
        }
    });
    let category_engine = engine.clone();
    tokio::spawn(async move {
        loop {
            match channel_events.recv().await {
                Ok(ChannelEvent::VoiceCategoryChanged {
                    guild_id,
                    channel_id,
                    after_category_id,
                    ..
                }) => {
                    category_engine
                        .handle_category_changed(guild_id, channel_id, after_category_id)
                        .await;
                }
                Ok(ChannelEvent::VoiceChannelUpdated { .. }) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    tokio::spawn(async move {
        engine.rehydrate().await;
        loop {
            match events.recv().await {
                Ok(event) => engine.handle_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "TempVoice: Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct MockPort {
        /// (guild, user) → channel
        voice: StdMutex<HashMap<(u64, u64), u64>>,
        names: StdMutex<HashMap<u64, String>>,
        categories: StdMutex<HashMap<u64, u64>>,
        members: StdMutex<HashMap<u64, Vec<u64>>>,
        created: StdMutex<Vec<(String, i64)>>,
        deleted: StdMutex<Vec<u64>>,
        moved: StdMutex<Vec<(u64, u64)>>,
        renamed: StdMutex<Vec<(u64, String)>>,
        overwrites: StdMutex<Vec<(u64, u64, Option<bool>)>>,
        role_batches: StdMutex<Vec<RoleBatch>>,
        role_overwrite_batches: StdMutex<Vec<RoleOverwriteBatch>>,
        member_batches: StdMutex<Vec<MemberBatch>>,
        member_connect: StdMutex<HashMap<(u64, u64), Option<bool>>>,
        pause_member_batch: AtomicBool,
        member_batch_started: tokio::sync::Notify,
        resume_member_batch: tokio::sync::Notify,
        limits: StdMutex<Vec<(u64, i64)>>,
        role_names: StdMutex<Vec<String>>,
        guild_roles: StdMutex<Vec<(u64, String)>>,
        disconnects: StdMutex<Vec<u64>>,
        next_channel_id: StdMutex<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RoleBatch {
        channel_id: u64,
        allowed: HashSet<u64>,
        denied: HashSet<u64>,
        cleared: HashSet<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MemberBatch {
        channel_id: u64,
        denied: HashSet<u64>,
        cleared: HashSet<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RoleOverwriteBatch {
        channel_id: u64,
        allowed: HashSet<u64>,
        denied: HashSet<u64>,
        cleared: HashSet<u64>,
    }

    #[async_trait::async_trait]
    impl LanePort for MockPort {
        async fn create_voice_channel(
            &self,
            _guild_id: u64,
            category_id: Option<u64>,
            name: &str,
            user_limit: i64,
        ) -> Result<u64, String> {
            let mut next = self.next_channel_id.lock().expect("lock");
            *next += 1;
            let id = 5000 + *next;
            self.created
                .lock()
                .expect("lock")
                .push((name.to_string(), user_limit));
            self.names
                .lock()
                .expect("lock")
                .insert(id, name.to_string());
            if let Some(category) = category_id {
                self.categories.lock().expect("lock").insert(id, category);
            }
            Ok(id)
        }
        async fn create_restricted_voice_channel(
            &self,
            guild_id: u64,
            category_id: u64,
            name: &str,
            _connect_user_ids: &[u64],
        ) -> Result<u64, String> {
            self.create_voice_channel(guild_id, Some(category_id), name, 0)
                .await
        }
        async fn delete_channel(&self, channel_id: u64, _reason: &str) -> Result<(), String> {
            self.deleted.lock().expect("lock").push(channel_id);
            Ok(())
        }
        async fn move_member(
            &self,
            guild_id: u64,
            user_id: u64,
            channel_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            self.moved.lock().expect("lock").push((user_id, channel_id));
            self.voice
                .lock()
                .expect("lock")
                .insert((guild_id, user_id), channel_id);
            Ok(())
        }
        async fn rename_channel(
            &self,
            channel_id: u64,
            name: &str,
            _reason: &str,
        ) -> Result<(), String> {
            self.renamed
                .lock()
                .expect("lock")
                .push((channel_id, name.to_string()));
            self.names
                .lock()
                .expect("lock")
                .insert(channel_id, name.to_string());
            Ok(())
        }
        async fn set_member_connect(
            &self,
            channel_id: u64,
            user_id: u64,
            connect: Option<bool>,
        ) -> Result<(), String> {
            self.overwrites
                .lock()
                .expect("lock")
                .push((channel_id, user_id, connect));
            self.member_connect
                .lock()
                .expect("lock")
                .insert((channel_id, user_id), connect);
            Ok(())
        }
        async fn apply_member_connect_batch(
            &self,
            channel_id: u64,
            denied_user_ids: &HashSet<u64>,
            clear_user_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            if self.pause_member_batch.swap(false, Ordering::SeqCst) {
                self.member_batch_started.notify_one();
                self.resume_member_batch.notified().await;
            }
            self.member_batches.lock().expect("lock").push(MemberBatch {
                channel_id,
                denied: denied_user_ids.clone(),
                cleared: clear_user_ids.clone(),
            });
            let mut member_connect = self.member_connect.lock().expect("lock");
            for user_id in clear_user_ids {
                member_connect.insert((channel_id, *user_id), None);
            }
            for user_id in denied_user_ids {
                member_connect.insert((channel_id, *user_id), Some(false));
            }
            Ok(())
        }
        async fn set_role_connect(
            &self,
            channel_id: u64,
            role_id: u64,
            connect: Option<bool>,
        ) -> Result<(), String> {
            let mut allowed = HashSet::new();
            let mut denied = HashSet::new();
            let mut cleared = HashSet::new();
            match connect {
                Some(true) => {
                    allowed.insert(role_id);
                }
                Some(false) => {
                    denied.insert(role_id);
                }
                None => {
                    cleared.insert(role_id);
                }
            }
            self.role_batches.lock().expect("lock").push(RoleBatch {
                channel_id,
                allowed,
                denied,
                cleared,
            });
            Ok(())
        }
        async fn apply_role_connect_batch(
            &self,
            _guild_id: u64,
            channel_id: u64,
            allowed_role_ids: &HashSet<u64>,
            clear_role_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            self.role_batches.lock().expect("lock").push(RoleBatch {
                channel_id,
                allowed: allowed_role_ids.clone(),
                denied: HashSet::new(),
                cleared: clear_role_ids.clone(),
            });
            Ok(())
        }
        async fn apply_role_connect_overwrites(
            &self,
            _guild_id: u64,
            channel_id: u64,
            allowed_role_ids: &HashSet<u64>,
            denied_role_ids: &HashSet<u64>,
            clear_role_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            self.role_overwrite_batches
                .lock()
                .expect("lock")
                .push(RoleOverwriteBatch {
                    channel_id,
                    allowed: allowed_role_ids.clone(),
                    denied: denied_role_ids.clone(),
                    cleared: clear_role_ids.clone(),
                });
            Ok(())
        }
        async fn set_user_limit(
            &self,
            channel_id: u64,
            limit: i64,
            _reason: &str,
        ) -> Result<(), String> {
            self.limits.lock().expect("lock").push((channel_id, limit));
            Ok(())
        }
        async fn disconnect_member(
            &self,
            _guild_id: u64,
            user_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            self.disconnects.lock().expect("lock").push(user_id);
            Ok(())
        }
        async fn member_display_name(&self, _guild_id: u64, user_id: u64) -> Option<String> {
            Some(format!("User {user_id}"))
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
            Some(6)
        }
        async fn guild_role_names(&self, _guild_id: u64) -> Vec<(u64, String)> {
            self.guild_roles.lock().expect("lock").clone()
        }
        async fn set_channel_category(
            &self,
            channel_id: u64,
            category_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            self.categories
                .lock()
                .expect("lock")
                .insert(channel_id, category_id);
            Ok(())
        }
        async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
            self.voice
                .lock()
                .expect("lock")
                .get(&(guild_id, user_id))
                .copied()
        }
        async fn member_role_names(&self, _guild_id: u64, _user_id: u64) -> Vec<String> {
            let roles = self.role_names.lock().expect("lock").clone();
            if roles.is_empty() {
                Vec::new()
            } else {
                roles
            }
        }
        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            vec![VERIFIED_ROLE_ID]
        }
        async fn channel_members(&self, _guild_id: u64, channel_id: u64) -> Vec<u64> {
            self.members
                .lock()
                .expect("lock")
                .get(&channel_id)
                .cloned()
                .unwrap_or_default()
        }
        async fn channel_name(&self, _guild_id: u64, channel_id: u64) -> Option<String> {
            self.names.lock().expect("lock").get(&channel_id).cloned()
        }
        async fn channel_category(&self, _guild_id: u64, channel_id: u64) -> Option<u64> {
            self.categories
                .lock()
                .expect("lock")
                .get(&channel_id)
                .copied()
        }
        async fn category_voice_channel_names(
            &self,
            _guild_id: u64,
            category_id: u64,
        ) -> Vec<String> {
            let categories = self.categories.lock().expect("lock");
            let names = self.names.lock().expect("lock");
            categories
                .iter()
                .filter(|(_, cat)| **cat == category_id)
                .filter_map(|(id, _)| names.get(id).cloned())
                .collect()
        }
        async fn category_voice_channels(
            &self,
            _guild_id: u64,
            category_id: u64,
        ) -> Vec<(u64, String)> {
            let categories = self.categories.lock().expect("lock");
            let names = self.names.lock().expect("lock");
            categories
                .iter()
                .filter(|(_, cat)| **cat == category_id)
                .filter_map(|(id, _)| names.get(id).cloned().map(|name| (*id, name)))
                .collect()
        }
        async fn channel_created_at(&self, _channel_id: u64) -> Option<i64> {
            Some(Utc::now().timestamp())
        }
    }

    const CASUAL_STAGING: u64 = 1501089974093873232;
    const STREET_STAGING: u64 = 1357422958544420944;
    const CASUAL_CATEGORY: u64 = 1289721245281292290;
    const RANKED_CATEGORY: u64 = 1412804540994162789;
    const STREET_CATEGORY: u64 = 1357422957017698478;

    #[test]
    fn production_schuetzt_router_vc_in_chill_als_fixed_lane() {
        let config = TempVoiceConfig::production();

        assert!(config.tempvoice_categories.contains(&CASUAL_CATEGORY));
        assert!(config.fixed_lane_ids.contains(&crate::router::ROUTER_VC_ID));
    }

    async fn setup() -> (
        dl_central_db::TestDb,
        Arc<TempVoiceEngine>,
        Arc<MockPort>,
        u64, // casual staging id
    ) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let config = TempVoiceConfig::production();
        let staging = CASUAL_STAGING; // casual (prefix_from_rank)
        let port = Arc::new(MockPort::default());
        *port.role_names.lock().expect("lock") = vec!["Phantom 2".to_string()];
        *port.guild_roles.lock().expect("lock") = vec![
            (1, "Phantom".to_string()),
            (2, "Seeker".to_string()),
            (3, "Ascendant".to_string()),
            (4, "Eternus".to_string()),
        ];
        // Staging liegt in der Chill-Kategorie
        port.categories
            .lock()
            .expect("lock")
            .insert(staging, CASUAL_CATEGORY);
        let engine =
            TempVoiceEngine::new(config, TempVoiceStore::new(db.pool().clone()), port.clone());
        (db, engine, port, staging)
    }

    #[tokio::test]
    async fn staging_join_erstellt_rang_lane_und_moved() {
        let (_dir, engine, port, staging) = setup().await;
        port.voice.lock().expect("lock").insert((1, 100), staging);

        engine
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: staging,
            })
            .await;

        // Rollen liefern "Phantom 2" → Casual zeigt den Rang informativ am Ende.
        let created = port.created.lock().expect("lock").clone();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].0, "Chill Lane 1 · Phantom");
        assert_eq!(created[0].1, logic::DEFAULT_CASUAL_CAP);
        let moved = port.moved.lock().expect("lock").clone();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].0, 100);

        // DB-Persist
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].owner_id, 100);
        assert_eq!(lanes[0].source_staging_id, Some(staging));
    }

    #[tokio::test]
    async fn guarded_router_create_skippt_wenn_user_nicht_allein_ist() {
        let (_db, engine, port, _staging) = setup().await;
        let guild_id = 1;
        let user_id = 100;
        let router_id = crate::router::ROUTER_VC_ID;
        port.voice
            .lock()
            .expect("lock")
            .insert((guild_id, user_id), router_id);
        port.members
            .lock()
            .expect("lock")
            .insert(router_id, vec![user_id, 101]);

        let outcome = engine
            .create_router_lane_if_alone(guild_id, user_id, "casual", router_id)
            .await
            .expect("guarded create");

        assert_eq!(outcome, None);
        assert!(port.created.lock().expect("lock").is_empty());
        assert!(port.moved.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn switch_mode_to_ranked_clears_street_source_and_limit() {
        let (_dir, engine, port, _staging) = setup().await;
        let guild_id = 1;
        let owner_id = 100;
        port.categories
            .lock()
            .expect("lock")
            .insert(STREET_STAGING, STREET_CATEGORY);
        port.voice
            .lock()
            .expect("lock")
            .insert((guild_id, owner_id), STREET_STAGING);

        engine
            .handle_event(VoiceEvent::Join {
                guild_id,
                user_id: owner_id,
                channel_id: STREET_STAGING,
            })
            .await;
        let lane_id = engine.store.all_lanes().await.expect("lanes")[0].channel_id;
        assert!(engine.is_min_rank_blocked(lane_id).await);

        assert_eq!(
            engine
                .switch_lane_mode(guild_id, lane_id, owner_id, "ranked")
                .await,
            None
        );

        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes[0].category_id, CASUAL_CATEGORY);
        assert_eq!(
            lanes[0].source_staging_id,
            Some(crate::router::mode_to_staging("ranked"))
        );
        assert!(!engine.is_min_rank_blocked(lane_id).await);
        assert_eq!(
            port.limits.lock().expect("lock").last().copied(),
            Some((lane_id, logic::DEFAULT_RANKED_CAP))
        );
    }

    #[tokio::test]
    async fn rehydrate_preserves_known_source_and_persists_inferred_source() {
        let (_dir, engine, _port, _staging) = setup().await;
        let known_id = 4242;
        let inferred_id = 4243;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: known_id,
                guild_id: engine.config.guild_id_hint,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Lane 1".to_string(),
                category_id: STREET_CATEGORY,
                source_staging_id: Some(STREET_STAGING),
            })
            .await
            .expect("known lane");
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: inferred_id,
                guild_id: engine.config.guild_id_hint,
                owner_id: 200,
                initial_owner_id: Some(200),
                base_name: "Street Brawl 7".to_string(),
                category_id: STREET_CATEGORY,
                source_staging_id: None,
            })
            .await
            .expect("inferred lane");

        engine.rehydrate().await;

        {
            let state = engine.state.lock().await;
            assert_eq!(
                state
                    .lanes
                    .get(&known_id)
                    .and_then(|lane| lane.source_staging_id),
                Some(STREET_STAGING)
            );
            assert_eq!(
                state
                    .lanes
                    .get(&inferred_id)
                    .and_then(|lane| lane.source_staging_id),
                Some(STREET_STAGING)
            );
            assert!(state.minrank_blocked.contains(&known_id));
            assert!(state.minrank_blocked.contains(&inferred_id));
        }
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(
            lanes
                .iter()
                .find(|lane| lane.channel_id == known_id)
                .and_then(|lane| lane.source_staging_id),
            Some(STREET_STAGING)
        );
        assert_eq!(
            lanes
                .iter()
                .find(|lane| lane.channel_id == inferred_id)
                .and_then(|lane| lane.source_staging_id),
            Some(STREET_STAGING)
        );
    }

    #[tokio::test]
    async fn join_in_bestehender_chill_lane_renamed_nicht_automatisch() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        port.role_names.lock().expect("lock").clear();
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Chill Lane 1".to_string());
        port.categories
            .lock()
            .expect("lock")
            .insert(lane_id, CASUAL_CATEGORY);
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: engine.config.guild_id_hint,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Chill Lane 1".to_string(),
                category_id: CASUAL_CATEGORY,
                source_staging_id: Some(CASUAL_STAGING),
            })
            .await
            .expect("lane");
        engine.rehydrate().await;

        engine
            .handle_event(VoiceEvent::Join {
                guild_id: engine.config.guild_id_hint,
                user_id: 200,
                channel_id: lane_id,
            })
            .await;

        assert!(port.renamed.lock().expect("lock").is_empty());
        assert_eq!(
            port.names.lock().expect("lock").get(&lane_id).cloned(),
            Some("Chill Lane 1".to_string())
        );
    }

    #[tokio::test]
    async fn auto_owner_transfer_wendet_keinen_preset_rename_an() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Alte Lane".to_string());
        port.categories
            .lock()
            .expect("lock")
            .insert(lane_id, CASUAL_CATEGORY);
        port.members
            .lock()
            .expect("lock")
            .insert(lane_id, vec![200]);
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: engine.config.guild_id_hint,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Alte Lane".to_string(),
                category_id: CASUAL_CATEGORY,
                source_staging_id: Some(CASUAL_STAGING),
            })
            .await
            .expect("lane");
        engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 200,
                mode: "casual".to_string(),
                base_name: "Neue Owner Lane".to_string(),
                limit: 3,
                min_rank: "unknown".to_string(),
            })
            .await
            .expect("default preset");
        engine.rehydrate().await;

        engine
            .handle_event(VoiceEvent::Leave {
                guild_id: engine.config.guild_id_hint,
                user_id: 100,
                channel_id: lane_id,
            })
            .await;

        assert_eq!(engine.lane_owner(lane_id).await, Some(200));
        assert!(port.renamed.lock().expect("lock").is_empty());
        let lane = engine
            .store
            .all_lanes()
            .await
            .expect("lanes")
            .into_iter()
            .find(|lane| lane.channel_id == lane_id)
            .expect("lane");
        assert_eq!(lane.base_name, "Alte Lane");
    }

    #[tokio::test]
    async fn min_rank_setzt_nur_allow_overwrites_fuer_erlaubte_rangrollen() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: engine.config.guild_id_hint,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Lane 1".to_string(),
                category_id: RANKED_CATEGORY,
                source_staging_id: None,
            })
            .await
            .expect("lane");
        engine.rehydrate().await;

        engine
            .set_min_rank(1, lane_id, "phantom")
            .await
            .expect("min rank");

        assert!(port.overwrites.lock().expect("lock").is_empty());
        let batches = port.role_batches.lock().expect("lock").clone();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0],
            RoleBatch {
                channel_id: lane_id,
                allowed: HashSet::from([1, 3, 4]),
                denied: HashSet::new(),
                cleared: HashSet::from([2]),
            }
        );
    }

    #[tokio::test]
    async fn rank_gate_toggle_setzt_und_entfernt_overwrites_ohne_disconnect() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        *port.guild_roles.lock().expect("lock") = vec![
            (10, "Ritualist 6".to_string()),
            (11, "Emissary 1".to_string()),
            (12, "Oracle 6".to_string()),
            (13, "Phantom 1".to_string()),
        ];

        engine
            .apply_rank_gate(1, lane_id, 100, "archon", 3, 9)
            .await
            .expect("gate on");
        assert!(engine.rank_gate_active(lane_id).await);

        let batches = port.role_overwrite_batches.lock().expect("lock").clone();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].channel_id, lane_id);
        assert_eq!(batches[0].denied, HashSet::from([1]));
        assert_eq!(batches[0].allowed, HashSet::from([10, 11, 12]));
        assert!(batches[0].cleared.contains(&13));
        assert!(port.disconnects.lock().expect("lock").is_empty());

        engine.clear_rank_gate(1, lane_id).await.expect("gate off");
        assert!(!engine.rank_gate_active(lane_id).await);

        let batches = port.role_overwrite_batches.lock().expect("lock").clone();
        assert_eq!(batches.len(), 2);
        assert!(batches[1].allowed.is_empty());
        assert!(batches[1].denied.is_empty());
        assert!(batches[1].cleared.contains(&1));
        assert!(batches[1].cleared.contains(&10));
        assert!(batches[1].cleared.contains(&13));
        assert!(port.disconnects.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn rank_pref_schlaegt_rollen() {
        let (_dir, engine, port, staging) = setup().await;
        engine
            .store
            .set_rank_pref(100, "ascendant", 3)
            .await
            .expect("pref");
        port.voice.lock().expect("lock").insert((1, 100), staging);
        engine
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: staging,
            })
            .await;
        let created = port.created.lock().expect("lock").clone();
        assert_eq!(created[0].0, "Chill Lane 1 · Ascendant 3");
    }

    #[tokio::test]
    async fn leave_letzter_user_loescht_lane() {
        let (_dir, engine, port, staging) = setup().await;
        port.voice.lock().expect("lock").insert((1, 100), staging);
        engine
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: staging,
            })
            .await;
        let lane_id = engine.store.all_lanes().await.expect("lanes")[0].channel_id;
        // niemand mehr drin
        port.members.lock().expect("lock").insert(lane_id, vec![]);

        engine
            .handle_event(VoiceEvent::Leave {
                guild_id: 1,
                user_id: 100,
                channel_id: lane_id,
            })
            .await;
        assert!(engine.store.all_lanes().await.expect("leer").is_empty());
        assert_eq!(port.deleted.lock().expect("lock").clone(), vec![lane_id]);
    }

    #[tokio::test]
    async fn startup_purge_loescht_cache_miss_nicht() {
        let (_dir, engine, port, _staging) = setup().await;
        let channel_id = 4242;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id,
                guild_id: engine.config.guild_id_hint,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Lane 1".to_string(),
                category_id: 1289721245281292290,
                source_staging_id: None,
            })
            .await
            .expect("lane");
        engine.rehydrate().await;

        engine.purge_empty_lanes().await;

        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].channel_id, channel_id);
        assert!(port.deleted.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn startup_purge_loescht_custom_lane_aber_nie_staging_oder_fixed() {
        let (_dir, engine, port, staging) = setup().await;
        let custom_channel = 4242;
        let fixed_channel = 1493690350580138114;
        {
            let mut categories = port.categories.lock().expect("lock");
            categories.insert(custom_channel, CASUAL_CATEGORY);
            categories.insert(fixed_channel, CASUAL_CATEGORY);
            categories.insert(staging, CASUAL_CATEGORY);
        }
        {
            let mut names = port.names.lock().expect("lock");
            names.insert(custom_channel, "Team Kekse".to_string());
            names.insert(fixed_channel, "Fester Treffpunkt".to_string());
            names.insert(staging, "(+) Casual".to_string());
        }

        engine.purge_empty_lanes().await;

        assert_eq!(
            port.deleted.lock().expect("lock").clone(),
            vec![custom_channel]
        );
    }

    #[tokio::test]
    async fn startup_purge_schuetzt_leeren_router_vc_aber_loescht_andere_leere_lane() {
        let (_dir, engine, port, _staging) = setup().await;
        let custom_channel = 4242;
        {
            let mut categories = port.categories.lock().expect("lock");
            categories.insert(crate::router::ROUTER_VC_ID, CASUAL_CATEGORY);
            categories.insert(custom_channel, CASUAL_CATEGORY);
        }
        {
            let mut names = port.names.lock().expect("lock");
            names.insert(
                crate::router::ROUTER_VC_ID,
                "➕Sprachkanal erstellen".to_string(),
            );
            names.insert(custom_channel, "Team Kekse".to_string());
        }

        engine.purge_empty_lanes().await;

        assert_eq!(
            port.deleted.lock().expect("lock").clone(),
            vec![custom_channel]
        );
    }

    #[tokio::test]
    async fn router_vc_in_tempvoice_category_ist_keine_managed_lane() {
        let (_dir, engine, port, _staging) = setup().await;
        port.categories
            .lock()
            .expect("lock")
            .insert(crate::router::ROUTER_VC_ID, CASUAL_CATEGORY);

        assert!(
            !engine
                .is_managed_lane(engine.config.guild_id_hint, crate::router::ROUTER_VC_ID)
                .await
        );
    }

    #[tokio::test]
    async fn leave_loescht_getrackte_custom_lane_auch_bei_category_cache_miss() {
        let (_dir, engine, port, _staging) = setup().await;
        let channel_id = 4242;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id,
                guild_id: engine.config.guild_id_hint,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Team Kekse".to_string(),
                category_id: CASUAL_CATEGORY,
                source_staging_id: None,
            })
            .await
            .expect("lane");
        engine.rehydrate().await;

        engine
            .handle_event(VoiceEvent::Leave {
                guild_id: engine.config.guild_id_hint,
                user_id: 100,
                channel_id,
            })
            .await;

        assert_eq!(port.deleted.lock().expect("lock").clone(), vec![channel_id]);
    }

    #[tokio::test]
    async fn owner_leave_transferiert_an_laengsten_verbundenen() {
        let (_dir, engine, port, staging) = setup().await;
        port.voice.lock().expect("lock").insert((1, 100), staging);
        engine
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: staging,
            })
            .await;
        let lane_id = engine.store.all_lanes().await.expect("lanes")[0].channel_id;

        // Zwei weitere joinen nacheinander (200 zuerst → ältester)
        for user in [200, 300] {
            engine
                .handle_event(VoiceEvent::Join {
                    guild_id: 1,
                    user_id: user,
                    channel_id: lane_id,
                })
                .await;
        }
        port.members
            .lock()
            .expect("lock")
            .insert(lane_id, vec![200, 300]);

        engine
            .handle_event(VoiceEvent::Leave {
                guild_id: 1,
                user_id: 100,
                channel_id: lane_id,
            })
            .await;
        assert_eq!(engine.lane_owner(lane_id).await, Some(200));
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(lanes[0].owner_id, 200);
        assert_eq!(lanes[0].initial_owner_id, Some(100)); // bleibt erhalten
    }

    #[tokio::test]
    async fn owner_claim_wendet_default_preset_des_neuen_owners_an() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: 1,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Alte Lane".to_string(),
                category_id: CASUAL_CATEGORY,
                source_staging_id: Some(CASUAL_STAGING),
            })
            .await
            .expect("lane");
        engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 200,
                mode: "casual".to_string(),
                base_name: "Neue Owner Lane".to_string(),
                limit: 3,
                min_rank: "unknown".to_string(),
            })
            .await
            .expect("default preset");
        engine.rehydrate().await;

        engine.claim_owner(1, lane_id, 200).await;

        assert_eq!(engine.lane_owner(lane_id).await, Some(200));
        let lane = engine
            .store
            .all_lanes()
            .await
            .expect("lanes")
            .into_iter()
            .find(|lane| lane.channel_id == lane_id)
            .expect("lane");
        assert_eq!(lane.owner_id, 200);
        assert_eq!(lane.base_name, "Neue Owner Lane");
        assert_eq!(
            port.names.lock().expect("lock").get(&lane_id).cloned(),
            Some("Neue Owner Lane".to_string())
        );
        assert_eq!(
            port.limits.lock().expect("lock").last().copied(),
            Some((lane_id, 3))
        );
    }

    #[tokio::test]
    async fn owner_claim_setzt_name_und_rang_wie_beim_apply() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4243;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: 1,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Alte Lane".to_string(),
                category_id: CASUAL_CATEGORY,
                source_staging_id: Some(CASUAL_STAGING),
            })
            .await
            .expect("lane");
        engine
            .store
            .set_rank_pref(200, "phantom", 3)
            .await
            .expect("rank pref");
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Alte Lane".to_string());
        port.categories
            .lock()
            .expect("lock")
            .insert(lane_id, CASUAL_CATEGORY);
        engine.rehydrate().await;

        engine.claim_owner(1, lane_id, 200).await;

        assert_eq!(engine.lane_owner(lane_id).await, Some(200));
        assert_eq!(
            port.names.lock().expect("lock").get(&lane_id).cloned(),
            Some("Alte Lane · Phantom 3".to_string())
        );
    }

    #[tokio::test]
    async fn owner_claim_setzt_ranked_name_auf_rang_des_neuen_owners() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4244;
        let ranked_staging = 1412804671432818890;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: 1,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Ranked Initiate".to_string(),
                category_id: TEMPVOICE_ONE_CATEGORY_ID,
                source_staging_id: Some(ranked_staging),
            })
            .await
            .expect("lane");
        engine
            .store
            .set_rank_pref(200, "phantom", 3)
            .await
            .expect("rank pref");
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Ranked Initiate".to_string());
        port.categories
            .lock()
            .expect("lock")
            .insert(lane_id, TEMPVOICE_ONE_CATEGORY_ID);
        engine.rehydrate().await;

        engine.claim_owner(1, lane_id, 200).await;

        assert_eq!(engine.lane_owner(lane_id).await, Some(200));
        assert_eq!(
            port.names.lock().expect("lock").get(&lane_id).cloned(),
            Some("Ranked Phantom 3".to_string())
        );
    }

    #[tokio::test]
    async fn owner_claim_laesst_live_match_suffix_unangetastet_und_persistiert_base() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4245;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: 1,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Alte Lane".to_string(),
                category_id: CASUAL_CATEGORY,
                source_staging_id: Some(CASUAL_STAGING),
            })
            .await
            .expect("lane");
        engine
            .store
            .set_rank_pref(200, "phantom", 3)
            .await
            .expect("rank pref");
        let live_name = "Alte Lane • 3/6 Im Match".to_string();
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, live_name.clone());
        port.categories
            .lock()
            .expect("lock")
            .insert(lane_id, CASUAL_CATEGORY);
        engine.rehydrate().await;

        engine.claim_owner(1, lane_id, 200).await;

        assert_eq!(engine.lane_owner(lane_id).await, Some(200));
        // Waehrend des laufenden Matches bleibt der live angezeigte Name
        // unangetastet (wie refresh_name / set_lane_template): kein Rename
        // baut auf dem Live-Suffix auf und persistiert Muell als base_name.
        assert_eq!(
            port.names.lock().expect("lock").get(&lane_id).cloned(),
            Some(live_name)
        );
        // Die neue Basis ist trotzdem gespeichert, greift beim naechsten
        // Refresh nach Matchende.
        let snapshot = engine.lane_preset_snapshot(lane_id).await;
        assert_eq!(
            snapshot.map(|(base, _, _)| base),
            Some("Alte Lane · Phantom 3".to_string())
        );
    }

    #[tokio::test]
    async fn owner_claim_behaelt_rang_gate_suffix_der_lane() {
        let (_dir, engine, port, _staging) = setup().await;
        let lane_id = 4246;
        let ranked_staging = 1412804671432818890;
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: lane_id,
                guild_id: 1,
                owner_id: 100,
                initial_owner_id: Some(100),
                base_name: "Ranked Initiate".to_string(),
                category_id: RANKED_CATEGORY,
                source_staging_id: Some(ranked_staging),
            })
            .await
            .expect("lane");
        engine
            .store
            .set_rank_pref(200, "phantom", 3)
            .await
            .expect("rank pref");
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Ranked Initiate".to_string());
        port.categories
            .lock()
            .expect("lock")
            .insert(lane_id, RANKED_CATEGORY);
        engine.rehydrate().await;
        engine
            .set_min_rank(1, lane_id, "emissary")
            .await
            .expect("min rank");

        engine.claim_owner(1, lane_id, 200).await;

        assert_eq!(engine.lane_owner(lane_id).await, Some(200));
        // Das Rang-Gate (lane.min_rank) bleibt aktiv und muss auch nach dem
        // Claim ohne gespeichertes Preset im Namen sichtbar bleiben, sonst
        // greift ein unsichtbares Gate, das der User nicht mehr erkennt.
        assert_eq!(
            port.names.lock().expect("lock").get(&lane_id).cloned(),
            Some("Ranked Phantom 3 • ab Emissary".to_string())
        );
    }

    #[tokio::test]
    async fn owner_bans_landen_als_overwrites() {
        let (_dir, engine, port, staging) = setup().await;
        engine.store.add_ban(100, 666).await.expect("ban");
        port.voice.lock().expect("lock").insert((1, 100), staging);
        engine
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: staging,
            })
            .await;
        let lane_id = engine.store.all_lanes().await.expect("lanes")[0].channel_id;
        let overwrites = port.overwrites.lock().expect("lock").clone();
        assert!(overwrites.is_empty());
        let batches = port.member_batches.lock().expect("lock").clone();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0],
            MemberBatch {
                channel_id: lane_id,
                denied: HashSet::from([666]),
                cleared: HashSet::new(),
            }
        );
    }

    #[tokio::test]
    async fn owner_ban_batch_kann_paralleles_unban_nicht_ueberschreiben() {
        let (_dir, engine, port, _staging) = setup().await;
        let channel_id = 5_001;
        let owner_id = 100;
        let target = 666;
        engine.store.add_ban(owner_id, target).await.expect("ban");
        port.pause_member_batch.store(true, Ordering::SeqCst);

        let apply = {
            let engine = engine.clone();
            tokio::spawn(async move {
                engine.apply_owner_bans(1, channel_id, owner_id).await;
            })
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            port.member_batch_started.notified(),
        )
        .await
        .expect("batch start");

        let mut remove = {
            let engine = engine.clone();
            tokio::spawn(async move {
                engine
                    .remove_owner_ban(owner_id, target, Some(channel_id))
                    .await
            })
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut remove)
                .await
                .is_err(),
            "unban darf den laufenden Batch nicht ueberholen"
        );
        port.resume_member_batch.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), apply)
            .await
            .expect("batch timeout")
            .expect("batch task");
        tokio::time::timeout(std::time::Duration::from_secs(1), remove)
            .await
            .expect("unban timeout")
            .expect("unban task")
            .expect("unban");

        assert_eq!(
            port.member_connect
                .lock()
                .expect("lock")
                .get(&(channel_id, target))
                .copied(),
            Some(None)
        );
    }

    #[tokio::test]
    async fn engine_nutzt_uebergebenen_pair_lock_ohne_deadlock() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let operations = Arc::new(VoicePairOperationLock::new(()));
        let port = Arc::new(MockPort::default());
        let engine = TempVoiceEngine::new_with_voice_pair_operations(
            TempVoiceConfig::production(),
            TempVoiceStore::new(db.pool().clone()),
            port,
            operations.clone(),
        );

        assert!(Arc::ptr_eq(&engine.voice_pair_operations, &operations));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            engine.add_owner_ban(100, 666, Some(5_001)).await?;
            engine.remove_owner_ban(100, 666, Some(5_001)).await
        })
        .await
        .expect("pair lock deadlock")
        .expect("owner ban roundtrip");
    }

    #[tokio::test]
    async fn router_lane_nutzt_passendes_default_preset_beim_create() {
        let (_db, engine, port, _staging) = setup().await;
        let router_vc = 777;
        engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 100,
                mode: "casual".to_string(),
                base_name: "Meine Lane".to_string(),
                limit: 3,
                min_rank: "unknown".to_string(),
            })
            .await
            .expect("default preset");
        port.voice.lock().expect("lock").insert((1, 100), router_vc);

        let lane_id = engine
            .create_router_lane(1, 100, "casual", router_vc)
            .await
            .expect("casual lane");

        assert_eq!(
            port.created.lock().expect("lock").as_slice(),
            &[("Meine Lane".to_string(), 3)]
        );
        assert!(port.renamed.lock().expect("lock").is_empty());
        assert_eq!(
            engine
                .lane_preset_snapshot(lane_id)
                .await
                .map(|snapshot| snapshot.0),
            Some("Meine Lane".to_string())
        );
        let lane = engine
            .store
            .all_lanes()
            .await
            .expect("lanes")
            .into_iter()
            .find(|lane| lane.channel_id == lane_id)
            .expect("created lane");
        assert_eq!(lane.base_name, "Meine Lane");
    }

    #[tokio::test]
    async fn router_lane_nutzt_fallback_bei_leerem_preset_namen() {
        let (_db, engine, port, _staging) = setup().await;
        let router_vc = 777;
        engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 100,
                mode: "casual".to_string(),
                base_name: " \t • ab Emissary".to_string(),
                limit: 3,
                min_rank: "unknown".to_string(),
            })
            .await
            .expect("default preset");
        port.voice.lock().expect("lock").insert((1, 100), router_vc);

        engine
            .create_router_lane(1, 100, "casual", router_vc)
            .await
            .expect("casual lane");

        assert_eq!(
            port.created.lock().expect("lock").as_slice(),
            &[("Chill Lane 1 · Phantom".to_string(), 3)]
        );
        assert!(port.renamed.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn router_lane_ohne_preset_nutzt_bisherigen_fallback() {
        let (_db, engine, port, _staging) = setup().await;
        let router_vc = 777;
        port.voice.lock().expect("lock").insert((1, 100), router_vc);

        engine
            .create_router_lane(1, 100, "casual", router_vc)
            .await
            .expect("casual lane");

        assert_eq!(
            port.created.lock().expect("lock").as_slice(),
            &[("Chill Lane 1 · Phantom".to_string(), 6)]
        );
        assert!(port.renamed.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn lane_template_sendet_genau_einen_finalen_namen() {
        let (_db, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Alte Lane".to_string());
        engine.state.lock().await.lanes.insert(
            lane_id,
            LaneState {
                owner_id: 100,
                initial_owner_id: 100,
                base_name: "Alte Lane".to_string(),
                min_rank: "emissary".to_string(),
                category_id: Some(RANKED_CATEGORY),
                prefix_from_rank: false,
                source_staging_id: None,
            },
        );

        engine
            .set_lane_template(1, lane_id, "Meine Lane", 3)
            .await
            .expect("template");

        assert_eq!(
            port.renamed.lock().expect("lock").as_slice(),
            &[(lane_id, "Meine Lane • ab Emissary".to_string())]
        );
    }

    #[tokio::test]
    async fn lane_template_erhaelt_live_suffix_ohne_rename() {
        let (_db, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Alte Lane • 2/6 Im Match".to_string());
        engine.state.lock().await.lanes.insert(
            lane_id,
            LaneState {
                owner_id: 100,
                initial_owner_id: 100,
                base_name: "Alte Lane".to_string(),
                min_rank: "unknown".to_string(),
                category_id: Some(CASUAL_CATEGORY),
                prefix_from_rank: false,
                source_staging_id: Some(CASUAL_STAGING),
            },
        );

        engine
            .set_lane_template(1, lane_id, "Meine Lane", 3)
            .await
            .expect("template");

        assert!(port.renamed.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn lane_template_nutzt_dynamischen_rang_prefix_fuer_finalen_namen() {
        let (_db, engine, port, _staging) = setup().await;
        let lane_id = 4242;
        port.names
            .lock()
            .expect("lock")
            .insert(lane_id, "Lane 7".to_string());
        engine.state.lock().await.lanes.insert(
            lane_id,
            LaneState {
                owner_id: 100,
                initial_owner_id: 100,
                base_name: "Lane 7".to_string(),
                min_rank: "unknown".to_string(),
                category_id: Some(CASUAL_CATEGORY),
                prefix_from_rank: true,
                source_staging_id: Some(CASUAL_STAGING),
            },
        );

        engine
            .set_lane_template(1, lane_id, "Meine Lane", 3)
            .await
            .expect("template");

        assert_eq!(
            port.renamed.lock().expect("lock").as_slice(),
            &[(lane_id, "Phantom".to_string())]
        );
    }

    #[tokio::test]
    async fn router_lane_namen_nutzen_pref_vor_rolle_vor_leer() {
        let router_vc = 777;

        let (_db, engine, port, _staging) = setup().await;
        engine
            .store
            .set_rank_pref(100, "ascendant", 3)
            .await
            .expect("pref");
        port.voice.lock().expect("lock").insert((1, 100), router_vc);
        let ranked = engine
            .create_router_lane(1, 100, "ranked", router_vc)
            .await
            .expect("ranked lane");
        assert_eq!(
            port.created.lock().expect("lock")[0].0,
            "Ranked Ascendant 3 1"
        );
        assert_eq!(
            port.categories.lock().expect("lock").get(&ranked).copied(),
            Some(CASUAL_CATEGORY)
        );
        let lanes = engine.store.all_lanes().await.expect("lanes");
        assert_eq!(
            lanes[0].source_staging_id,
            Some(crate::router::mode_to_staging("ranked"))
        );
        engine.set_base_name(ranked, "Custom Name").await;
        assert!(engine.lane_is_ranked(ranked).await);

        let (_db, engine, port, _staging) = setup().await;
        engine
            .store
            .set_rank_pref(100, "ascendant", 3)
            .await
            .expect("pref");
        port.voice.lock().expect("lock").insert((1, 100), router_vc);
        engine
            .create_router_lane(1, 100, "casual", router_vc)
            .await
            .expect("casual lane");
        assert_eq!(
            port.created.lock().expect("lock")[0].0,
            "Chill Lane 1 · Ascendant 3"
        );

        let (_db, engine, port, _staging) = setup().await;
        engine
            .store
            .set_rank_pref(100, "ascendant", 3)
            .await
            .expect("pref");
        port.voice.lock().expect("lock").insert((1, 100), router_vc);
        engine
            .create_router_lane(1, 100, "street_brawl", router_vc)
            .await
            .expect("street lane");
        assert_eq!(port.created.lock().expect("lock")[0].0, "Street Brawl 1");

        let (_db, engine, port, _staging) = setup().await;
        *port.role_names.lock().expect("lock") = vec!["Phantom 2".to_string()];
        port.voice.lock().expect("lock").insert((1, 100), router_vc);
        engine
            .create_router_lane(1, 100, "ranked", router_vc)
            .await
            .expect("ranked lane");
        assert_eq!(port.created.lock().expect("lock")[0].0, "Ranked Phantom 1");

        let (_db, engine, port, _staging) = setup().await;
        *port.role_names.lock().expect("lock") = Vec::new();
        port.voice.lock().expect("lock").insert((1, 100), router_vc);
        engine
            .create_router_lane(1, 100, "casual", router_vc)
            .await
            .expect("casual lane");
        assert_eq!(port.created.lock().expect("lock")[0].0, "Chill Lane 1");
    }
}
