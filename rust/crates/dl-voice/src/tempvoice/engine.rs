//! TempVoice-Engine — Join-to-create, Owner-Lifecycle, Lane-Aufräumen.
//!
//! Port des Verhaltens-Kerns von cogs/tempvoice/core.py als Subscriber des
//! Voice-Dispatchers. Discord-Aktionen laufen über [`LanePort`] (testbar).
//!
//! Bewusste 4b-Lücken (dokumentiert in rust/docs/05): Tag-Filter/Ragebaiter,
//! Lurker-Flow, New-Player-Routing-Hook, RTC-Region-Apply und die
//! Rang-Permission-Kopplung (kommt mit dem rank_voice_manager-Port in 4c).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{NaiveDateTime, Utc};
use dl_discord::{Dispatcher, VoiceEvent};

use super::logic;
use super::store::{LaneRecord, TempVoiceStore};

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
    /// Rollen-Overwrite (Region: English-Only-Rolle deny); None löscht.
    async fn set_role_connect(
        &self,
        channel_id: u64,
        role_id: u64,
        connect: Option<bool>,
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

    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    async fn member_role_names(&self, guild_id: u64, user_id: u64) -> Vec<String>;
    async fn channel_members(&self, guild_id: u64, channel_id: u64) -> Vec<u64>;
    async fn channel_name(&self, guild_id: u64, channel_id: u64) -> Option<String>;
    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64>;
    async fn category_voice_channel_names(&self, guild_id: u64, category_id: u64) -> Vec<String>;
    /// Unix-Timestamp der Kanal-Erstellung (Snowflake).
    async fn channel_created_at(&self, channel_id: u64) -> Option<i64>;
}

/// Regeln pro Staging-Kanal (STAGING_RULES-Pendant).
#[derive(Debug, Clone, Default)]
pub struct StagingRules {
    pub prefix: Option<String>,
    pub user_limit: Option<i64>,
    pub prefix_from_rank: bool,
}

#[derive(Debug, Clone)]
pub struct TempVoiceConfig {
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
        let category_chill = 1289721245281292290;
        let category_comp = 1412804540994162789;
        let category_street_brawl = 1357422957017698478;

        let mut staging_rules = HashMap::new();
        staging_rules.insert(
            staging_street_brawl,
            StagingRules {
                prefix: Some("Street Brawl".to_string()),
                user_limit: Some(4),
                prefix_from_rank: false,
            },
        );
        staging_rules.insert(
            staging_casual,
            StagingRules {
                prefix: None,
                user_limit: None,
                prefix_from_rank: true,
            },
        );

        Self {
            staging_channels: HashSet::from([staging_casual, staging_street_brawl, staging_comp]),
            fixed_lane_ids: HashSet::from([
                1493690350580138114, // permanenter Chill-Voice
                1411391356278018245,
                1470126503252721845,
                1505618194017161267,
            ]),
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
}

pub const ENGLISH_ONLY_ROLE_ID: u64 = 1309741866098491479;

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
}

pub struct TempVoiceEngine {
    pub config: TempVoiceConfig,
    pub store: TempVoiceStore,
    pub port: Arc<dyn LanePort>,
    state: tokio::sync::Mutex<EngineState>,
}

impl TempVoiceEngine {
    pub fn new(
        config: TempVoiceConfig,
        store: TempVoiceStore,
        port: Arc<dyn LanePort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            store,
            port,
            state: tokio::sync::Mutex::new(EngineState::default()),
        })
    }

    /// Lanes aus der DB rehydrieren (Bot-Neustart).
    pub async fn rehydrate(&self) {
        match self.store.all_lanes().await {
            Ok(lanes) => {
                let mut state = self.state.lock().await;
                for lane in lanes {
                    state.lanes.insert(
                        lane.channel_id,
                        LaneState {
                            owner_id: lane.owner_id,
                            initial_owner_id: lane.initial_owner_id.unwrap_or(lane.owner_id),
                            base_name: lane.base_name,
                            min_rank: "unknown".to_string(),
                            category_id: Some(lane.category_id).filter(|c| *c > 0),
                            prefix_from_rank: lane
                                .source_staging_id
                                .map(|s| self.config.rules_for_staging(s).prefix_from_rank)
                                .unwrap_or(false),
                            source_staging_id: lane.source_staging_id,
                        },
                    );
                }
                tracing::info!(lanes = state.lanes.len(), "TempVoice: Lanes rehydriert");
            }
            Err(err) => tracing::error!(%err, "TempVoice: Rehydrierung fehlgeschlagen"),
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
            {
                let mut state = self.state.lock().await;
                state.lanes.insert(
                    channel_id,
                    LaneState {
                        owner_id: user_id,
                        initial_owner_id: user_id,
                        base_name: base_name.clone(),
                        min_rank: "unknown".to_string(),
                        category_id,
                        prefix_from_rank: false,
                        source_staging_id: None,
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
                    source_staging_id: None,
                })
                .await
            {
                tracing::warn!(%err, channel_id, "TempVoice: Owner-Backfill-Persist fehlgeschlagen");
            }
            self.apply_owner_bans(guild_id, channel_id, user_id).await;
        }
        self.refresh_name(guild_id, channel_id).await;
    }

    async fn on_leave(self: &Arc<Self>, guild_id: u64, user_id: u64, channel_id: u64) {
        if !self.is_managed_lane(guild_id, channel_id).await {
            return;
        }
        let members = self.port.channel_members(guild_id, channel_id).await;

        let was_owner = {
            let mut state = self.state.lock().await;
            if let Some(times) = state.join_time.get_mut(&channel_id) {
                times.remove(&user_id);
            }
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
                // Bans des alten Owners von der Lane nehmen, neue anwenden
                self.clear_owner_bans(channel_id, user_id).await;
                self.apply_owner_bans(guild_id, channel_id, new_owner).await;
                tracing::info!(
                    channel_id,
                    old = user_id,
                    new = new_owner,
                    "TempVoice: Owner-Transfer"
                );
            }
        }
        self.refresh_name(guild_id, channel_id).await;
    }

    async fn is_managed_lane(&self, guild_id: u64, channel_id: u64) -> bool {
        if self.config.fixed_lane_ids.contains(&channel_id)
            || self.config.staging_channels.contains(&channel_id)
        {
            return false;
        }
        let Some(category) = self.port.channel_category(guild_id, channel_id).await else {
            return false;
        };
        if !self.config.tempvoice_categories.contains(&category) {
            return false;
        }
        let Some(name) = self.port.channel_name(guild_id, channel_id).await else {
            return false;
        };
        logic::is_managed_lane_name(&name)
    }

    /// Join-to-create (Kern von `_create_lane`).
    async fn create_lane(self: &Arc<Self>, guild_id: u64, user_id: u64, staging_id: u64) {
        // Doppel-Klick-Schutz pro User
        {
            let mut state = self.state.lock().await;
            if !state.creating.insert(user_id) {
                return;
            }
        }
        let result = self.create_lane_inner(guild_id, user_id, staging_id).await;
        self.state.lock().await.creating.remove(&user_id);
        if let Err(err) = result {
            tracing::warn!(%err, user_id, staging_id, "TempVoice: Lane-Erstellung fehlgeschlagen");
        }
    }

    async fn create_lane_inner(
        self: &Arc<Self>,
        guild_id: u64,
        user_id: u64,
        staging_id: u64,
    ) -> Result<(), String> {
        // User noch im Staging?
        if self.port.member_voice_channel(guild_id, user_id).await != Some(staging_id) {
            return Ok(());
        }
        let rules = self.config.rules_for_staging(staging_id);
        let category_id = self.port.channel_category(guild_id, staging_id).await;
        let in_minrank = category_id
            .map(|c| self.config.minrank_categories.contains(&c))
            .unwrap_or(false);
        let use_rank_name = rules.prefix_from_rank || in_minrank;

        // Basis-Name: Rang (Pref → Rollen) oder "Prefix N"
        let base = if use_rank_name {
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
        let base = match base {
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
        };

        let cap = rules
            .user_limit
            .unwrap_or_else(|| self.config.default_cap(category_id));
        let lane_id = self
            .port
            .create_voice_channel(guild_id, category_id, &base, cap)
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
                    category_id,
                    prefix_from_rank: rules.prefix_from_rank,
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

        // Owner muss noch im Staging stehen, sonst Lane wieder abbauen
        if self.port.member_voice_channel(guild_id, user_id).await != Some(staging_id) {
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
        self.apply_owner_bans(guild_id, lane_id, user_id).await;
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
    pub async fn claim_owner(&self, guild_id: u64, channel_id: u64, new_owner: u64) {
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
        self.apply_owner_bans(guild_id, channel_id, new_owner).await;
    }

    /// Limit setzen (Street-Brawl-Regel kappt auf max_limit).
    pub async fn set_limit(&self, channel_id: u64, requested: i64) -> Result<i64, String> {
        let max_limit = {
            let state = self.state.lock().await;
            state.lanes.get(&channel_id).and_then(|lane| {
                lane.source_staging_id
                    .and_then(|s| self.config.staging_rules.get(&s))
                    .and_then(|r| r.user_limit)
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

    /// Owner-Rename: Basisnamen mitführen, damit refresh_name nicht zurücksetzt.
    pub async fn set_base_name(&self, channel_id: u64, name: &str) {
        let mut state = self.state.lock().await;
        if let Some(lane) = state.lanes.get_mut(&channel_id) {
            lane.base_name = logic::strip_suffixes(name);
        }
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
        let desired = logic::compose_name(&base, &lane.min_rank, in_minrank);
        if desired != current {
            let _ = self
                .port
                .rename_channel(channel_id, &desired, "TempVoice: Name aktualisiert")
                .await;
        }
    }

    /// Owner-Bans als Connect-Overwrites auf die Lane legen.
    async fn apply_owner_bans(&self, _guild_id: u64, channel_id: u64, owner_id: u64) {
        let bans = self.store.list_bans(owner_id).await.unwrap_or_default();
        for banned in bans {
            let _ = self
                .port
                .set_member_connect(channel_id, banned, Some(false))
                .await;
        }
    }

    async fn clear_owner_bans(&self, channel_id: u64, owner_id: u64) {
        let bans = self.store.list_bans(owner_id).await.unwrap_or_default();
        for banned in bans {
            let _ = self.port.set_member_connect(channel_id, banned, None).await;
        }
    }
}

/// Engine als Dispatcher-Subscriber starten.
pub fn spawn(engine: Arc<TempVoiceEngine>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_voice();
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
        limits: StdMutex<Vec<(u64, i64)>>,
        next_channel_id: StdMutex<u64>,
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
            Ok(())
        }
        async fn set_role_connect(
            &self,
            channel_id: u64,
            role_id: u64,
            connect: Option<bool>,
        ) -> Result<(), String> {
            self.overwrites
                .lock()
                .expect("lock")
                .push((channel_id, role_id, connect));
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
            _user_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn member_display_name(&self, _guild_id: u64, user_id: u64) -> Option<String> {
            Some(format!("User {user_id}"))
        }
        async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
            self.voice
                .lock()
                .expect("lock")
                .get(&(guild_id, user_id))
                .copied()
        }
        async fn member_role_names(&self, _guild_id: u64, _user_id: u64) -> Vec<String> {
            vec!["Phantom 2".to_string()]
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
        async fn channel_created_at(&self, _channel_id: u64) -> Option<i64> {
            Some(Utc::now().timestamp())
        }
    }

    const LANE_DDL: &str = "CREATE TABLE tempvoice_lanes (channel_id INTEGER PRIMARY KEY, guild_id INTEGER NOT NULL, owner_id INTEGER NOT NULL, base_name TEXT NOT NULL, category_id INTEGER NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, source_staging_id INTEGER, initial_owner_id INTEGER)";
    const BAN_DDL: &str = "CREATE TABLE tempvoice_bans (owner_id BIGINT NOT NULL, banned_id BIGINT NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, PRIMARY KEY (owner_id, banned_id))";
    const PREF_DDL: &str = "CREATE TABLE tempvoice_rank_pref (user_id INTEGER PRIMARY KEY, rank TEXT NOT NULL, subrank INTEGER NOT NULL DEFAULT 0)";

    async fn setup() -> (
        tempfile::TempDir,
        Arc<TempVoiceEngine>,
        Arc<MockPort>,
        u64, // casual staging id
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dl_db::Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in [LANE_DDL, BAN_DDL, PREF_DDL] {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        let config = TempVoiceConfig::production();
        let staging = 1501089974093873232; // casual (prefix_from_rank)
        let port = Arc::new(MockPort::default());
        // Staging liegt in der Chill-Kategorie
        port.categories
            .lock()
            .expect("lock")
            .insert(staging, 1289721245281292290);
        let engine = TempVoiceEngine::new(config, TempVoiceStore::new(db), port.clone());
        (dir, engine, port, staging)
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

        // prefix_from_rank: Rollen liefern "Phantom 2" → Lane heißt "Phantom"
        let created = port.created.lock().expect("lock").clone();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].0, "Phantom");
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
        assert_eq!(created[0].0, "Ascendant 3");
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
        let overwrites = port.overwrites.lock().expect("lock").clone();
        assert!(overwrites
            .iter()
            .any(|(_, user, deny)| *user == 666 && *deny == Some(false)));
    }
}
