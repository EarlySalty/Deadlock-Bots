//! Rank-Voice-Manager — Port von `cogs/rank_voice_manager.py`
//! (RolePermissionVoiceManager).
//!
//! Comp/Ranked-Lanes bekommen einen Rang-Anker (Erstbesitzer bzw. erstes
//! rangiertes Mitglied): Score-Fenster ±9 Sub-Rang-Punkte, Sub-Rang-Rollen
//! im Fenster dürfen connecten (Batch-Overwrites, @everyone deny), der
//! Kanal heißt nach dem Anker („Phantom 3"). Niemals kicken — nur Rechte.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dl_db::Db;
use dl_discord::{ChannelSender, Dispatcher, GatewayEvent, VoiceEvent};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::status::{select_best_presence, select_channel_cohort, CohortEntry, PresenceRow};

pub const RANKED_SUBRANK_TOLERANCE: i64 = 9;
pub const SCORE_MIN_ABSOLUTE: i64 = 7;
pub const SCORE_MAX_ABSOLUTE: i64 = 72;
pub const PRESENCE_STALE_SECONDS: i64 = 180;
pub const MAIN_GUILD_ID: u64 = 1289721245281292288;
pub const RRANG_INFO_BALANCING_RULE: &str = "Platzhalter";

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

/// Comp/Ranked-Kategorie → Modus "lane".
pub const MONITORED_CATEGORY_ID: u64 = 1412804540994162789;
pub const EXCLUDED_CHANNEL_IDS: [u64; 6] = [
    1375933460841234514,
    1375934283931451512,
    1357422958544420944,
    1412804671432818890,
    1411391356278018245,
    1470126503252721845,
];

/// Haupt-Rang-Rollen der Guild (ID → (Name, Tier)).
pub fn major_rank_roles() -> HashMap<u64, (&'static str, i64)> {
    HashMap::from([
        (1331457571118387210, ("Initiate", 1)),
        (1331457652877955072, ("Seeker", 2)),
        (1331457699992436829, ("Alchemist", 3)),
        (1331457724848017539, ("Arcanist", 4)),
        (1331457879345070110, ("Ritualist", 5)),
        (1331457898781474836, ("Emissary", 6)),
        (1331457949654319114, ("Archon", 7)),
        (1316966867033653338, ("Oracle", 8)),
        (1331458016356208680, ("Phantom", 9)),
        (1331458049637875785, ("Ascendant", 10)),
        (1331458087349129296, ("Eternus", 11)),
    ])
}

const RANK_VALUES: [(&str, i64); 12] = [
    ("obscurus", 0),
    ("initiate", 1),
    ("seeker", 2),
    ("alchemist", 3),
    ("arcanist", 4),
    ("ritualist", 5),
    ("emissary", 6),
    ("archon", 7),
    ("oracle", 8),
    ("phantom", 9),
    ("ascendant", 10),
    ("eternus", 11),
];

const SHORT_TO_RANK: [(&str, &str); 11] = [
    ("ini", "Initiate"),
    ("see", "Seeker"),
    ("alc", "Alchemist"),
    ("arc", "Arcanist"),
    ("rit", "Ritualist"),
    ("emi", "Emissary"),
    ("arch", "Archon"),
    ("ora", "Oracle"),
    ("pha", "Phantom"),
    ("asc", "Ascendant"),
    ("ete", "Eternus"),
];

fn rank_value(name: &str) -> Option<i64> {
    let n = name.to_lowercase();
    RANK_VALUES.iter().find(|(r, _)| *r == n).map(|(_, v)| *v)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// "Asc 3" / "Ascendant 3" → (Name, Tier, Sub) — SUBRANK_ROLE_RE-Pendant.
pub fn parse_subrank_role(role_name: &str) -> Option<(String, i64, i64)> {
    let trimmed = role_name.trim();
    let (name_part, sub_part) = trimmed.rsplit_once(char::is_whitespace)?;
    let sub: i64 = sub_part.parse().ok()?;
    if !(1..=6).contains(&sub) {
        return None;
    }
    let lower = name_part.trim().to_lowercase();
    if let Some(value) = rank_value(&lower) {
        return Some((capitalize(&lower), value, sub));
    }
    SHORT_TO_RANK
        .iter()
        .find(|(short, _)| *short == lower)
        .and_then(|(_, full)| rank_value(&full.to_lowercase()).map(|v| (full.to_string(), v, sub)))
}

/// Höchster Rang aus Rollen: Sub-Rang-Rollen schlagen Haupt-Rollen gleichen
/// Tiers (Score tier·6+sub vs. tier·6).
pub fn user_rank_from_roles(roles: &[(u64, String)]) -> (String, i64, Option<i64>) {
    let majors = major_rank_roles();
    let mut best = ("Obscurus".to_string(), 0i64, None);
    let mut best_score = -1i64;
    for (role_id, role_name) in roles {
        if let Some((name, value, sub)) = parse_subrank_role(role_name) {
            let score = value * 6 + sub;
            if score > best_score {
                best_score = score;
                best = (name, value, Some(sub));
            }
            continue;
        }
        if let Some((name, value)) = majors.get(role_id) {
            let score = value * 6;
            if score > best_score {
                best_score = score;
                best = (name.to_string(), *value, None);
            }
        }
    }
    best
}

/// Score-Fenster + grobe Tier-Grenzen (Referenzwerte aus CPython im Test).
pub fn anchor_range(rank_value: i64, subrank: i64) -> (i64, i64, i64, i64) {
    let anchor_score = rank_value * 6 + subrank;
    let score_min = (anchor_score - RANKED_SUBRANK_TOLERANCE).max(SCORE_MIN_ABSOLUTE);
    let score_max = (anchor_score + RANKED_SUBRANK_TOLERANCE).min(SCORE_MAX_ABSOLUTE);
    let allowed_min = ((score_min - 1) / 6).max(1);
    let allowed_max = ((score_max - 1) / 6).min(11);
    (score_min, score_max, allowed_min, allowed_max)
}

pub fn balancing_rule_value() -> String {
    let full_rank_span = (RANKED_SUBRANK_TOLERANCE / 6).max(1);
    format!("±{full_rank_span} Ränge")
}

/// Welche Sub-Rang-Rollen der Guild fallen ins Score-Fenster?
pub fn allowed_subrank_roles(
    guild_roles: &[(u64, String)],
    score_min: i64,
    score_max: i64,
) -> HashSet<u64> {
    guild_roles
        .iter()
        .filter_map(|(role_id, name)| {
            let (_, value, sub) = parse_subrank_role(name)?;
            let score = value * 6 + sub;
            (score_min <= score && score <= score_max).then_some(*role_id)
        })
        .collect()
}

/// Kanal-Name aus Anker/Membern (wie update_channel_name).
pub fn desired_channel_name(
    anchor: Option<(&str, i64)>,
    first_member_rank: Option<(&str, i64)>,
) -> String {
    if let Some((name, sub)) = anchor {
        return format!("{name} {sub}");
    }
    if let Some((name, sub)) = first_member_rank {
        return format!("{name} {sub}");
    }
    "Rang-Sprachkanal".to_string()
}

/// Rang-Name nach Tier-Wert (Port von `get_rank_name_from_value`):
/// unbekannt → "Obscurus".
pub fn rank_name_from_value(value: i64) -> String {
    RANK_VALUES
        .iter()
        .find(|(_, v)| *v == value)
        .map(|(name, _)| capitalize(name))
        .unwrap_or_else(|| "Obscurus".to_string())
}

// ── Persistenz (voice_channel_anchors / voice_channel_settings) ────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub user_id: u64,
    pub rank_name: String,
    pub rank_value: i64,
    pub allowed_min: i64,
    pub allowed_max: i64,
    pub subrank: i64,
    pub score_min: i64,
    pub score_max: i64,
}

pub struct RankStore {
    pub db: Db,
}

impl RankStore {
    pub async fn upsert_anchor(&self, channel_id: u64, guild_id: u64, anchor: Anchor) {
        let result = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO voice_channel_anchors(
                       channel_id, guild_id, user_id, rank_name, rank_value,
                       allowed_min, allowed_max, anchor_subrank, score_min, score_max, updated_at
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,CURRENT_TIMESTAMP)
                     ON CONFLICT(channel_id) DO UPDATE SET
                       user_id=excluded.user_id, rank_name=excluded.rank_name,
                       rank_value=excluded.rank_value, allowed_min=excluded.allowed_min,
                       allowed_max=excluded.allowed_max, anchor_subrank=excluded.anchor_subrank,
                       score_min=excluded.score_min, score_max=excluded.score_max,
                       updated_at=CURRENT_TIMESTAMP",
                    rusqlite::params![
                        channel_id,
                        guild_id,
                        anchor.user_id,
                        anchor.rank_name,
                        anchor.rank_value,
                        anchor.allowed_min,
                        anchor.allowed_max,
                        anchor.subrank,
                        anchor.score_min,
                        anchor.score_max,
                    ],
                )
                .map(|_| ())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, channel_id, "RankVoice: Anchor-Persist fehlgeschlagen");
        }
    }

    pub async fn delete_anchor(&self, channel_id: u64) {
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM voice_channel_anchors WHERE channel_id = ?1",
                    [channel_id],
                )
                .map(|_| ())
            })
            .await;
    }

    pub async fn load_anchors(&self) -> HashMap<u64, Anchor> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT channel_id, user_id, rank_name, rank_value, allowed_min,
                            allowed_max, COALESCE(anchor_subrank, 3),
                            COALESCE(score_min, 7), COALESCE(score_max, 72)
                       FROM voice_channel_anchors",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        Anchor {
                            user_id: row.get(1)?,
                            rank_name: row.get(2)?,
                            rank_value: row.get(3)?,
                            allowed_min: row.get(4)?,
                            allowed_max: row.get(5)?,
                            subrank: row.get(6)?,
                            score_min: row.get(7)?,
                            score_max: row.get(8)?,
                        },
                    ))
                })?;
                rows.collect::<Result<HashMap<_, _>, _>>()
            })
            .await
            .unwrap_or_default()
    }

    pub async fn is_enabled(&self, channel_id: u64) -> bool {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT enabled FROM voice_channel_settings WHERE channel_id = ?1",
                    [channel_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(true)
    }

    /// Persistiert den Ein/Aus-Zustand des Rang-Systems je VC
    /// (`voice_channel_settings`, Port von `_db_upsert_setting`).
    pub async fn set_enabled(&self, channel_id: u64, guild_id: u64, enabled: bool) {
        let result = self
            .db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO voice_channel_settings(channel_id, guild_id, enabled, updated_at)
                     VALUES(?1, ?2, ?3, CURRENT_TIMESTAMP)
                     ON CONFLICT(channel_id) DO UPDATE SET
                       enabled = excluded.enabled,
                       updated_at = CURRENT_TIMESTAMP",
                    rusqlite::params![channel_id, guild_id, i64::from(enabled)],
                )
                .map(|_| ())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, channel_id, "RankVoice: Setting-Persist fehlgeschlagen");
        }
    }

    /// Sub-Rang des primären Steam-Accounts (Fallback 3 = Mitte).
    pub async fn subrank_from_db(&self, user_id: u64) -> i64 {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT deadlock_subrank FROM steam_links
                      WHERE user_id=?1 AND deadlock_rank IS NOT NULL AND deadlock_rank > 0
                      ORDER BY primary_account DESC, updated_at DESC LIMIT 1",
                    [user_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .flatten()
            .map(|s| s.clamp(1, 6))
            .unwrap_or(3)
    }

    pub async fn steam_ids_for(&self, user_ids: Vec<u64>) -> HashMap<u64, Vec<String>> {
        crate::status::StatusStore {
            db: self.db.clone(),
        }
        .steam_ids_for(user_ids)
        .await
    }

    pub async fn presence_rows(&self, steam_ids: Vec<String>) -> HashMap<String, PresenceRow> {
        crate::status::StatusStore {
            db: self.db.clone(),
        }
        .presence_rows(steam_ids)
        .await
    }
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait RankPort: Send + Sync {
    /// Non-Bot-Member im Kanal mit ihren Rollen (id, name).
    async fn channel_members_with_roles(
        &self,
        guild_id: u64,
        channel_id: u64,
    ) -> Vec<(u64, Vec<(u64, String)>)>;
    async fn guild_roles(&self, guild_id: u64) -> Vec<(u64, String)>;
    /// Rollen-IDs, die aktuell ein Overwrite auf dem Kanal haben.
    async fn current_role_overwrites(&self, guild_id: u64, channel_id: u64) -> Vec<u64>;
    /// Batch: @everyone deny connect/view true, erlaubte Rollen connect+speak+view,
    /// `removes` werden entfernt.
    async fn apply_overwrites(
        &self,
        guild_id: u64,
        channel_id: u64,
        allowed_role_ids: &HashSet<u64>,
        removes: &HashSet<u64>,
    ) -> Result<(), String>;
    async fn rename(&self, channel_id: u64, name: &str) -> Result<(), String>;
    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64>;

    // ── Read-only-Helfer für die `!rrang`-Admin-Oberfläche ──────────────────

    /// Voice-Kanal des Aufrufers (für `toggle`/`vcstatus`/`aktualisieren`);
    /// `None`, wenn er in keinem ist.
    async fn caller_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    /// Kanal-Anzeigename; `None`, wenn der Kanal nicht (mehr) existiert.
    async fn channel_name(&self, guild_id: u64, channel_id: u64) -> Option<String>;
    /// Kategorie-Anzeigename eines Kanals (für `vcstatus`).
    async fn category_name(&self, guild_id: u64, channel_id: u64) -> Option<String>;
    /// Anzahl Nicht-Bot-Mitglieder im Voice-Kanal.
    async fn channel_member_count(&self, guild_id: u64, channel_id: u64) -> usize;
    /// Anzeigename eines Gilden-Mitglieds; `None`, wenn nicht (mehr) auf dem
    /// Server.
    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> Option<String>;
    /// Rollen (id, name) eines Mitglieds (für `debug`); leer, wenn unbekannt.
    async fn member_roles(&self, guild_id: u64, user_id: u64) -> Vec<(u64, String)>;
    /// Rollen eines Gilden-Mitglieds für Ankerlogik; None bei unbekanntem Member
    /// oder Bot.
    async fn guild_member_roles(&self, guild_id: u64, user_id: u64) -> Option<Vec<(u64, String)>>;
    /// Mitgliederzahl je Rolle (für `rollen`); 0, wenn Rolle unbekannt.
    async fn role_member_count(&self, guild_id: u64, role_id: u64) -> usize;
    /// Voice-Kanäle einer Kategorie als (id, name) (für `kanäle`).
    async fn category_voice_channels(&self, guild_id: u64, category_id: u64) -> Vec<(u64, String)>;
}

pub struct RankVoiceManager {
    pub store: RankStore,
    pub port: Arc<dyn RankPort>,
    /// Erstbesitzer-Auskunft der TempVoice-Engine (Anker-Priorität).
    pub initial_owner: Arc<dyn Fn(u64) -> Option<u64> + Send + Sync>,
    anchors: tokio::sync::Mutex<HashMap<u64, Anchor>>,
    last_rename: tokio::sync::Mutex<HashMap<u64, std::time::Instant>>,
}

impl RankVoiceManager {
    pub fn new(
        db: Db,
        port: Arc<dyn RankPort>,
        initial_owner: Arc<dyn Fn(u64) -> Option<u64> + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: RankStore { db },
            port,
            initial_owner,
            anchors: tokio::sync::Mutex::new(HashMap::new()),
            last_rename: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub async fn rehydrate(&self) {
        let anchors = self.store.load_anchors().await;
        tracing::info!(anchors = anchors.len(), "RankVoice: Anker rehydriert");
        *self.anchors.lock().await = anchors;
    }

    pub async fn startup_reconcile(self: &Arc<Self>, guild_id: u64) {
        let channels = self
            .port
            .category_voice_channels(guild_id, MONITORED_CATEGORY_ID)
            .await;
        for (channel_id, _) in channels {
            if self.is_monitored(guild_id, channel_id).await {
                self.reconcile_channel(guild_id, channel_id).await;
            }
        }
    }

    pub async fn is_monitored(&self, guild_id: u64, channel_id: u64) -> bool {
        if EXCLUDED_CHANNEL_IDS.contains(&channel_id) {
            return false;
        }
        self.port.channel_category(guild_id, channel_id).await == Some(MONITORED_CATEGORY_ID)
    }

    pub async fn handle_event(self: &Arc<Self>, event: VoiceEvent) {
        let (guild_id, joined, left) = match event {
            VoiceEvent::Join {
                guild_id,
                channel_id,
                ..
            } => (guild_id, Some(channel_id), None),
            VoiceEvent::Leave {
                guild_id,
                channel_id,
                ..
            } => (guild_id, None, Some(channel_id)),
            VoiceEvent::Move {
                guild_id,
                from_channel_id,
                to_channel_id,
                ..
            } => (guild_id, Some(to_channel_id), Some(from_channel_id)),
            VoiceEvent::Update { .. } => return,
        };
        for channel_id in [joined, left].into_iter().flatten() {
            if self.is_monitored(guild_id, channel_id).await {
                self.reconcile_channel(guild_id, channel_id).await;
            }
        }
    }

    /// Kern: Anker prüfen/setzen, Rechte + Name nachziehen.
    pub async fn reconcile_channel(self: &Arc<Self>, guild_id: u64, channel_id: u64) {
        if !self.store.is_enabled(channel_id).await {
            return;
        }
        let members = self
            .port
            .channel_members_with_roles(guild_id, channel_id)
            .await;

        // Rang-relevante Member: größte zusammen spielende Gruppe (Presence-Kohorte)
        let relevant = self.rank_relevant_members(&members).await;

        // Anker bestimmen: Erstbesitzer (gildenweit) > bestehender Anker
        // (wenn anwesend) > erstes rangiertes Mitglied.
        let anchor_member = match (self.initial_owner)(channel_id) {
            Some(owner_id) => {
                if let Some((_, roles)) = members.iter().find(|(id, _)| *id == owner_id) {
                    Some((owner_id, roles.clone()))
                } else {
                    self.port
                        .guild_member_roles(guild_id, owner_id)
                        .await
                        .map(|roles| (owner_id, roles))
                }
            }
            None => None,
        };
        let existing = self.anchors.lock().await.get(&channel_id).cloned();

        let chosen = if let Some((user_id, roles)) = anchor_member {
            let (name, value, sub) = self.rank_tuple(user_id, &roles).await;
            if value > 0 {
                Some((user_id, name, value, sub))
            } else {
                None
            }
        } else {
            None
        };
        let chosen = match chosen {
            Some(chosen) => Some(chosen),
            None => {
                if members.is_empty() {
                    None
                } else if let Some(existing) = &existing {
                    if members.iter().any(|(id, _)| *id == existing.user_id) {
                        // Anker bleibt gültig
                        Some((
                            existing.user_id,
                            existing.rank_name.clone(),
                            existing.rank_value,
                            existing.subrank,
                        ))
                    } else {
                        self.first_ranked(&relevant, &members).await
                    }
                } else {
                    self.first_ranked(&relevant, &members).await
                }
            }
        };

        let Some((user_id, rank_name, rank_value, subrank)) = chosen else {
            // Kanal leer → Anker weg, Rechte zurücksetzen
            if existing.is_some() {
                self.anchors.lock().await.remove(&channel_id);
                self.store.delete_anchor(channel_id).await;
                let removes: HashSet<u64> = self
                    .port
                    .current_role_overwrites(guild_id, channel_id)
                    .await
                    .into_iter()
                    .collect();
                let _ = self
                    .port
                    .apply_overwrites(guild_id, channel_id, &HashSet::new(), &removes)
                    .await;
            }
            return;
        };

        let (score_min, score_max, allowed_min, allowed_max) = anchor_range(rank_value, subrank);
        let anchor = Anchor {
            user_id,
            rank_name: rank_name.clone(),
            rank_value,
            allowed_min,
            allowed_max,
            subrank,
            score_min,
            score_max,
        };
        let changed = existing.as_ref() != Some(&anchor);
        if changed {
            self.anchors.lock().await.insert(channel_id, anchor.clone());
            self.store.upsert_anchor(channel_id, guild_id, anchor).await;
        }

        // Rechte: erlaubte Sub-Rang-Rollen im Fenster, alle anderen Rang-Rollen raus
        let guild_roles = self.port.guild_roles(guild_id).await;
        let allowed = allowed_subrank_roles(&guild_roles, score_min, score_max);
        let majors: HashSet<u64> = major_rank_roles().keys().copied().collect();
        let current: HashSet<u64> = self
            .port
            .current_role_overwrites(guild_id, channel_id)
            .await
            .into_iter()
            .collect();
        let removes: HashSet<u64> = current
            .iter()
            .copied()
            .filter(|role_id| {
                let is_rank_role = majors.contains(role_id)
                    || guild_roles
                        .iter()
                        .any(|(id, name)| id == role_id && parse_subrank_role(name).is_some());
                is_rank_role && !allowed.contains(role_id)
            })
            .collect();
        if let Err(err) = self
            .port
            .apply_overwrites(guild_id, channel_id, &allowed, &removes)
            .await
        {
            tracing::warn!(%err, channel_id, "RankVoice: Overwrite-Batch fehlgeschlagen");
        }

        // Name: "Rank Sub", Rename-Cooldown 60s lokal
        let name = desired_channel_name(Some((&rank_name, subrank)), None);
        let should_rename = {
            let mut renames = self.last_rename.lock().await;
            let fresh = renames
                .get(&channel_id)
                .map(|t| t.elapsed().as_secs() >= 60)
                .unwrap_or(true);
            if fresh && changed {
                renames.insert(channel_id, std::time::Instant::now());
            }
            fresh && changed
        };
        if should_rename {
            let _ = self.port.rename(channel_id, &name).await;
        }
    }

    // ── Admin-Operationen (`!rrang`) ────────────────────────────────────────

    /// Momentaufnahme aller in-memory-Anker (für `!rrang anker`/`status`).
    pub async fn anchor_snapshot(&self) -> Vec<(u64, Anchor)> {
        self.anchors
            .lock()
            .await
            .iter()
            .map(|(channel_id, anchor)| (*channel_id, anchor.clone()))
            .collect()
    }

    /// Aktueller Anker eines Kanals (für `!rrang vcstatus`).
    pub async fn anchor_for(&self, channel_id: u64) -> Option<Anchor> {
        self.anchors.lock().await.get(&channel_id).cloned()
    }

    /// `!rrang toggle ein`: Rang-System für den VC aktivieren (persistiert)
    /// und sofort reconcilen (Port von `toggle_channel_system` ein-Zweig).
    pub async fn enable_channel(self: &Arc<Self>, guild_id: u64, channel_id: u64) {
        self.store.set_enabled(channel_id, guild_id, true).await;
        // Erneutes Initialisieren erzwingen: lokalen Rename-Cooldown lösen,
        // damit reconcile Rechte + Namen frisch setzt.
        self.last_rename.lock().await.remove(&channel_id);
        self.reconcile_channel(guild_id, channel_id).await;
    }

    /// `!rrang toggle aus`: Rang-System deaktivieren (persistiert), Anker
    /// entfernen und Rang-Rollen-Overwrites räumen (Port von
    /// `toggle_channel_system` aus-Zweig: `remove_channel_anchor` +
    /// `clear_role_permissions`).
    pub async fn disable_channel(self: &Arc<Self>, guild_id: u64, channel_id: u64) {
        self.store.set_enabled(channel_id, guild_id, false).await;
        self.anchors.lock().await.remove(&channel_id);
        self.store.delete_anchor(channel_id).await;
        self.clear_rank_overwrites(guild_id, channel_id).await;
    }

    /// `!rrang aktualisieren`: alten Anker verwerfen und frisch reconcilen
    /// (Port von `force_update`: `remove_channel_anchor` + erneuter Aufbau).
    pub async fn force_reconcile(self: &Arc<Self>, guild_id: u64, channel_id: u64) {
        self.anchors.lock().await.remove(&channel_id);
        self.store.delete_anchor(channel_id).await;
        self.last_rename.lock().await.remove(&channel_id);
        self.reconcile_channel(guild_id, channel_id).await;
    }

    /// Entfernt alle Rang-Rollen-Overwrites (Haupt- + Sub-Rang) vom Kanal.
    async fn clear_rank_overwrites(&self, guild_id: u64, channel_id: u64) {
        let guild_roles = self.port.guild_roles(guild_id).await;
        let majors: HashSet<u64> = major_rank_roles().keys().copied().collect();
        let current: HashSet<u64> = self
            .port
            .current_role_overwrites(guild_id, channel_id)
            .await
            .into_iter()
            .collect();
        let removes: HashSet<u64> = current
            .into_iter()
            .filter(|role_id| {
                majors.contains(role_id)
                    || guild_roles
                        .iter()
                        .any(|(id, name)| id == role_id && parse_subrank_role(name).is_some())
            })
            .collect();
        if removes.is_empty() {
            return;
        }
        if let Err(err) = self
            .port
            .apply_overwrites(guild_id, channel_id, &HashSet::new(), &removes)
            .await
        {
            tracing::warn!(%err, channel_id, "RankVoice: Overwrite-Räumung fehlgeschlagen");
        }
    }

    async fn rank_tuple(&self, user_id: u64, roles: &[(u64, String)]) -> (String, i64, i64) {
        let (name, value, sub) = user_rank_from_roles(roles);
        let sub = match sub {
            Some(sub) => sub,
            None => self.store.subrank_from_db(user_id).await,
        };
        (name, value, sub)
    }

    async fn first_ranked(
        &self,
        relevant: &[u64],
        members: &[(u64, Vec<(u64, String)>)],
    ) -> Option<(u64, String, i64, i64)> {
        // Bevorzugt Mitglieder der Spiel-Kohorte, sonst alle
        let order: Vec<u64> = if relevant.is_empty() {
            members.iter().map(|(id, _)| *id).collect()
        } else {
            relevant.to_vec()
        };
        for user_id in &order {
            if let Some((_, roles)) = members.iter().find(|(id, _)| id == user_id) {
                let (name, value, sub) = self.rank_tuple(*user_id, roles).await;
                if value > 0 {
                    return Some((*user_id, name, value, sub));
                }
            }
        }
        // Fallback: erster überhaupt (wie Python next(iter(...)))
        let (user_id, roles) = members.first()?;
        let (name, value, sub) = self.rank_tuple(*user_id, roles).await;
        Some((*user_id, name, value, sub))
    }

    /// Presence-Kohorte (wer spielt zusammen) — wie get_rank_relevant_members.
    async fn rank_relevant_members(&self, members: &[(u64, Vec<(u64, String)>)]) -> Vec<u64> {
        let user_ids: Vec<u64> = members.iter().map(|(id, _)| *id).collect();
        if user_ids.is_empty() {
            return Vec::new();
        }
        let steam_map = self.store.steam_ids_for(user_ids.clone()).await;
        let all_steam: Vec<String> = steam_map
            .values()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if all_steam.is_empty() {
            return Vec::new();
        }
        let presence = self.store.presence_rows(all_steam).await;
        let now = chrono::Utc::now().timestamp();
        let mut entries = Vec::new();
        for user_id in &user_ids {
            let steam_ids = steam_map.get(user_id).cloned().unwrap_or_default();
            if let Some((stage, minutes, server_id, _)) =
                select_best_presence(&steam_ids, &presence, now, PRESENCE_STALE_SECONDS)
            {
                entries.push(CohortEntry {
                    member_id: *user_id,
                    stage,
                    minutes: minutes.unwrap_or(0),
                    server_id,
                });
            }
        }
        select_channel_cohort(&entries, 1)
            .map(|c| c.member_ids)
            .unwrap_or_default()
    }
}

pub fn spawn(
    manager: Arc<RankVoiceManager>,
    dispatcher: &Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_voice();
    let mut gateway_events = dispatcher.subscribe_gateway();
    tokio::spawn(async move {
        manager.rehydrate().await;
        wait_for_cache_ready(&mut gateway_events, MAIN_GUILD_ID).await;
        manager.startup_reconcile(MAIN_GUILD_ID).await;
        loop {
            match events.recv().await {
                Ok(event) => manager.handle_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

// ── `!rrang`-Admin-Befehlsgruppe ────────────────────────────────────────────
//
// Port der `@commands.group("rrang", manage_guild)` aus
// `cogs/rank_voice_manager.py`. Folgt dem Muster eines eigenständigen
// Spawn-/Listener-Tasks: eigener `MessageEvent`-Subscriber, admin-gated,
// Antwort über den `ChannelSender`. Discord-Farben analog zum Original.

/// Antwort eines `!rrang`-Subcommands (Text und/oder Embed).
pub struct RankReply {
    pub content: Option<String>,
    pub embeds: Vec<Value>,
}

impl RankReply {
    fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            embeds: vec![],
        }
    }
    fn embed(embed: Value) -> Self {
        Self {
            content: None,
            embeds: vec![embed],
        }
    }
}

/// `!rrang`-Befehlsschicht über dem [`RankVoiceManager`].
pub struct RankCommands {
    pub manager: Arc<RankVoiceManager>,
}

impl RankCommands {
    pub fn new(manager: Arc<RankVoiceManager>) -> Arc<Self> {
        Arc::new(Self { manager })
    }

    /// Verarbeitet eine Nachricht; `None`, wenn es kein `!rrang`-Befehl ist.
    pub async fn reply_for(&self, content: &str, guild_id: u64, user_id: u64) -> Option<RankReply> {
        let mut parts = content.split_whitespace();
        let root = parts.next()?.to_lowercase();
        if root != "!rrang" {
            return None;
        }
        let sub = parts.next().unwrap_or_default().to_lowercase();
        let args: Vec<&str> = parts.collect();
        let reply = match sub.as_str() {
            "" => RankReply::embed(help_embed()),
            "toggle" => self.toggle(guild_id, user_id, args.first().copied()).await,
            "anker" => self.anchors_overview(guild_id).await,
            "vcstatus" => self.vcstatus(guild_id, user_id).await,
            "debug" => {
                self.debug(
                    guild_id,
                    user_id,
                    args.first().and_then(|a| parse_mention(a)),
                )
                .await
            }
            "aktualisieren" => {
                self.force_update(
                    guild_id,
                    user_id,
                    args.first().and_then(|a| parse_channel(a)),
                )
                .await
            }
            "rollen" => self.tracked_roles(guild_id).await,
            "kanäle" | "kanaele" | "channels" => self.channel_config(guild_id).await,
            "status" => self.system_status(guild_id, user_id).await,
            "info" => {
                self.rank_info(
                    guild_id,
                    user_id,
                    args.first().and_then(|a| parse_mention(a)),
                )
                .await
            }
            other => RankReply::text(format!("❌ Unbekannter Subcommand `{other}`.")),
        };
        Some(reply)
    }

    /// `toggle [ein/aus]` — schaltet das Rang-System für den aktuellen VC.
    async fn toggle(&self, guild_id: u64, user_id: u64, action: Option<&str>) -> RankReply {
        let Some(channel_id) = self
            .manager
            .port
            .caller_voice_channel(guild_id, user_id)
            .await
        else {
            return RankReply::text("❌ Du musst in einem Voice Channel sein.");
        };
        let name = self
            .manager
            .port
            .channel_name(guild_id, channel_id)
            .await
            .unwrap_or_else(|| channel_id.to_string());

        if !self.manager.is_monitored(guild_id, channel_id).await {
            return RankReply::text(format!("❌ **{name}** wird nicht überwacht."));
        }

        let current = self.manager.store.is_enabled(channel_id).await;
        let Some(action) = action else {
            return RankReply::text(format!(
                "🔧 Rang-System für **{name}**: {}",
                if current {
                    "✅ Aktiviert"
                } else {
                    "❌ Deaktiviert"
                }
            ));
        };

        match action.to_lowercase().as_str() {
            "ein" | "on" | "aktivieren" | "enable" => {
                if current {
                    return RankReply::text(format!("ℹ️ Bereits aktiviert für **{name}**."));
                }
                self.manager.enable_channel(guild_id, channel_id).await;
                RankReply::text(format!("✅ Aktiviert: **{name}**"))
            }
            "aus" | "off" | "deaktivieren" | "disable" => {
                if !current {
                    return RankReply::text(format!("ℹ️ Bereits deaktiviert für **{name}**."));
                }
                self.manager.disable_channel(guild_id, channel_id).await;
                RankReply::text(format!("❌ Deaktiviert: **{name}**"))
            }
            _ => RankReply::text("❌ Verwende: `ein`/`on` oder `aus`/`off`"),
        }
    }

    /// `anker` — Übersicht aktiver Kanal-Anker (DB-persistiert).
    async fn anchors_overview(&self, guild_id: u64) -> RankReply {
        let anchors = self.manager.anchor_snapshot().await;
        if anchors.is_empty() {
            return RankReply::embed(json!({
                "title": "🔗 Kanal-Anker Übersicht",
                "description": "❌ Keine aktiven Kanal-Anker",
                "color": 0x9B59B6,
            }));
        }
        let mut lines: Vec<String> = Vec::new();
        for (channel_id, anchor) in &anchors {
            let ch_name = self.manager.port.channel_name(guild_id, *channel_id).await;
            let user_name = self
                .manager
                .port
                .member_display_name(guild_id, anchor.user_id)
                .await;
            let (Some(ch_name), Some(user_name)) = (ch_name, user_name) else {
                lines.push(format!(
                    "❓ Veralteter Eintrag (Kanal {channel_id}, User {})",
                    anchor.user_id
                ));
                continue;
            };
            let min_rank = rank_name_from_value(anchor.allowed_min);
            let max_rank = rank_name_from_value(anchor.allowed_max);
            let count = self
                .manager
                .port
                .channel_member_count(guild_id, *channel_id)
                .await;
            lines.push(format!(
                "**{ch_name}**\n🔗 Anker: {user_name} ({} {})\n📊 Tiers: {min_rank}–{max_rank} | Score: {}–{}\n👥 Aktuelle User: {count}\n",
                anchor.rank_name, anchor.subrank, anchor.score_min, anchor.score_max
            ));
        }
        let shown: Vec<String> = lines.iter().take(10).cloned().collect();
        let mut embed = json!({
            "title": "🔗 Kanal-Anker Übersicht",
            "description": shown.join("\n"),
            "color": 0x9B59B6,
        });
        if lines.len() > 10 {
            embed["footer"] = json!({ "text": format!("{} weitere …", lines.len() - 10) });
        }
        RankReply::embed(embed)
    }

    /// `vcstatus` — Status des aktuellen VC des Aufrufers.
    async fn vcstatus(&self, guild_id: u64, user_id: u64) -> RankReply {
        let Some(channel_id) = self
            .manager
            .port
            .caller_voice_channel(guild_id, user_id)
            .await
        else {
            return RankReply::text("❌ Du musst in einem Voice Channel sein.");
        };
        let name = self
            .manager
            .port
            .channel_name(guild_id, channel_id)
            .await
            .unwrap_or_else(|| channel_id.to_string());
        let category = self
            .manager
            .port
            .category_name(guild_id, channel_id)
            .await
            .unwrap_or_else(|| "–".to_string());
        let count = self
            .manager
            .port
            .channel_member_count(guild_id, channel_id)
            .await;
        let is_mon = self.manager.is_monitored(guild_id, channel_id).await;

        let mut fields = vec![
            json!({
                "name": "📊 Kanal-Info",
                "value": format!("ID: {channel_id}\nKategorie: {category}\nMitglieder: {count}"),
                "inline": true,
            }),
            json!({
                "name": "👁️ Überwachung",
                "value": if is_mon { "✅ Überwacht" } else { "❌ Nicht überwacht" },
                "inline": true,
            }),
        ];

        if is_mon {
            let sys_en = self.manager.store.is_enabled(channel_id).await;
            fields.push(json!({
                "name": "🔧 Rang-System",
                "value": if sys_en { "✅ Aktiviert" } else { "❌ Deaktiviert" },
                "inline": true,
            }));
            let anchor = self.manager.anchor_for(channel_id).await;
            let anchor_value = match (anchor, sys_en) {
                (Some(anchor), true) => {
                    let user = self
                        .manager
                        .port
                        .member_display_name(guild_id, anchor.user_id)
                        .await
                        .unwrap_or_else(|| anchor.user_id.to_string());
                    let min_rank = rank_name_from_value(anchor.allowed_min);
                    let max_rank = rank_name_from_value(anchor.allowed_max);
                    format!(
                        "{user} ({} {})\nTiers: {min_rank}–{max_rank}\nScore: {}–{}",
                        anchor.rank_name, anchor.subrank, anchor.score_min, anchor.score_max
                    )
                }
                (_, true) => "Kein Anker gesetzt".to_string(),
                (_, false) => "System deaktiviert".to_string(),
            };
            fields.push(json!({ "name": "🔗 Anker", "value": anchor_value, "inline": false }));
        }

        RankReply::embed(json!({
            "title": format!("🔊 Status: {name}"),
            "color": 0x3498DB,
            "fields": fields,
        }))
    }

    /// `debug [@user]` — Rollen-Erkennung eines Users.
    async fn debug(&self, guild_id: u64, caller_id: u64, target: Option<u64>) -> RankReply {
        let user_id = target.unwrap_or(caller_id);
        let Some(display_name) = self
            .manager
            .port
            .member_display_name(guild_id, user_id)
            .await
        else {
            return RankReply::text("❌ Benutzer nicht gefunden.");
        };
        let roles = self.manager.port.member_roles(guild_id, user_id).await;
        let majors = major_rank_roles();
        let found: Vec<String> = roles
            .iter()
            .filter_map(|(rid, rname)| {
                majors
                    .get(rid)
                    .map(|(name, value)| format!("**{rname}** (ID {rid}) -> {name} ({value})"))
            })
            .collect();
        let (rn, rv, rs) = user_rank_from_roles(&roles);
        let sub_txt = rs.map(|s| format!(" {s}")).unwrap_or_default();

        let all_roles: Vec<String> = roles
            .iter()
            .take(10)
            .map(|(rid, rname)| format!("{rid}: {rname}"))
            .collect();
        let mut all_roles_text = all_roles.join("\n");
        if roles.len() > 10 {
            all_roles_text.push_str(&format!("\n… und {} weitere", roles.len() - 10));
        }

        RankReply::embed(json!({
            "title": format!("🔍 Debug: {display_name}"),
            "color": 0xE67E22,
            "fields": [
                { "name": "👤 User-Info", "value": format!("ID: {user_id}\nRollen: {}", roles.len()), "inline": true },
                { "name": "🎯 Erkannter Rang", "value": format!("**{rn}{sub_txt}** ({rv})"), "inline": true },
                {
                    "name": "🎭 Gefundene Rang-Rollen",
                    "value": if found.is_empty() { "❌ Keine".to_string() } else { found.join("\n") },
                    "inline": false,
                },
                {
                    "name": "📋 Alle Rollen (erste 10)",
                    "value": if all_roles_text.is_empty() { "—".to_string() } else { all_roles_text },
                    "inline": false,
                },
            ],
        }))
    }

    /// `aktualisieren [#vc]` — Forced Reconcile.
    async fn force_update(
        &self,
        guild_id: u64,
        caller_id: u64,
        channel_arg: Option<u64>,
    ) -> RankReply {
        let channel_id = match channel_arg {
            Some(id) => id,
            None => match self
                .manager
                .port
                .caller_voice_channel(guild_id, caller_id)
                .await
            {
                Some(id) => id,
                None => return RankReply::text("❌ In einem Sprachkanal sein oder Kanal angeben."),
            },
        };
        let name = self
            .manager
            .port
            .channel_name(guild_id, channel_id)
            .await
            .unwrap_or_else(|| channel_id.to_string());
        if !self.manager.is_monitored(guild_id, channel_id).await {
            return RankReply::text("❌ Dieser Kanal wird nicht überwacht.");
        }
        self.manager.force_reconcile(guild_id, channel_id).await;
        RankReply::text(format!("✅ Kanal **{name}** aktualisiert."))
    }

    /// `rollen` — konfigurierte Rang-Rollen + Mitgliederzahl.
    async fn tracked_roles(&self, guild_id: u64) -> RankReply {
        // Stabile Reihenfolge nach Tier-Wert (HashMap ist ungeordnet).
        let mut majors: Vec<(u64, &'static str, i64)> = major_rank_roles()
            .into_iter()
            .map(|(id, (name, value))| (id, name, value))
            .collect();
        majors.sort_by_key(|(_, _, value)| *value);
        let mut lines: Vec<String> = Vec::new();
        for (role_id, name, value) in majors {
            let count = self.manager.port.role_member_count(guild_id, role_id).await;
            if count > 0 || self.role_exists(guild_id, role_id).await {
                lines.push(format!(
                    "**{name}** ({value}): <@&{role_id}> – {count} Mitglieder"
                ));
            } else {
                lines.push(format!(
                    "**{name}** ({value}): ❌ Rolle nicht gefunden (ID {role_id})"
                ));
            }
        }
        RankReply::embed(json!({
            "title": "🎭 Überwachte Rang-Rollen",
            "description": lines.join("\n"),
            "color": 0xF1C40F,
        }))
    }

    async fn role_exists(&self, guild_id: u64, role_id: u64) -> bool {
        self.manager
            .port
            .guild_roles(guild_id)
            .await
            .iter()
            .any(|(id, _)| *id == role_id)
    }

    /// `kanäle` — überwachte/ausgeschlossene Kanäle.
    async fn channel_config(&self, guild_id: u64) -> RankReply {
        let vcs = self
            .manager
            .port
            .category_voice_channels(guild_id, MONITORED_CATEGORY_ID)
            .await;
        let mut fields: Vec<Value> = Vec::new();
        if vcs.is_empty() {
            fields.push(json!({
                "name": format!("📁 Kategorie (ID {MONITORED_CATEGORY_ID})"),
                "value": "❌ Kategorie nicht gefunden oder leer",
                "inline": false,
            }));
        } else {
            let monitored = vcs
                .iter()
                .filter(|(id, _)| !EXCLUDED_CHANNEL_IDS.contains(id))
                .count();
            fields.push(json!({
                "name": "📁 Comp/Ranked (lane)",
                "value": format!("Gesamt: {}\nÜberwacht: {monitored}", vcs.len()),
                "inline": false,
            }));
        }
        let mut ex_lines: Vec<String> = Vec::new();
        for cid in EXCLUDED_CHANNEL_IDS {
            match self.manager.port.channel_name(guild_id, cid).await {
                Some(name) => ex_lines.push(format!("🔇 {name}")),
                None => ex_lines.push(format!("❓ Unbekannt (ID {cid})")),
            }
        }
        if !ex_lines.is_empty() {
            fields.push(json!({
                "name": "🚫 Ausgeschlossene Kanäle",
                "value": ex_lines.join("\n"),
                "inline": false,
            }));
        }
        RankReply::embed(json!({
            "title": "🔊 Kanal-Konfiguration",
            "description": "Sprachkanal-Überwachung",
            "color": 0x3498DB,
            "fields": fields,
        }))
    }

    /// `status` — Systemstatus (Anker-/Settings-Zähler + eigener Rang).
    async fn system_status(&self, guild_id: u64, caller_id: u64) -> RankReply {
        let anchors = self.manager.anchor_snapshot().await;
        let roles = self.manager.port.member_roles(guild_id, caller_id).await;
        let (rn, rv, rs) = user_rank_from_roles(&roles);
        let sub_txt = rs.map(|s| format!(" {s}")).unwrap_or_default();
        RankReply::embed(json!({
            "title": "📊 System-Status",
            "description": "Rollen-Berechtigungen Voice Manager",
            "color": 0x2ECC71,
            "fields": [
                { "name": "🔧 Version", "value": "Sanftes Anker-System v4.0 (DB-persistiert)", "inline": false },
                {
                    "name": "📁 Überwachung",
                    "value": format!("Kategorie: {MONITORED_CATEGORY_ID}\nAusgeschlossen: {}\nRollen: {}", EXCLUDED_CHANNEL_IDS.len(), major_rank_roles().len()),
                    "inline": true,
                },
                { "name": "💾 State", "value": format!("Anker: {}", anchors.len()), "inline": true },
                { "name": "🎯 Dein Rang", "value": format!("{rn}{sub_txt} ({rv})"), "inline": true },
            ],
        }))
    }

    /// `info [@user]` — höchster Rang aus den Rollen.
    async fn rank_info(&self, guild_id: u64, caller_id: u64, target: Option<u64>) -> RankReply {
        let user_id = target.unwrap_or(caller_id);
        let Some(display_name) = self
            .manager
            .port
            .member_display_name(guild_id, user_id)
            .await
        else {
            return RankReply::text("❌ Benutzer nicht gefunden.");
        };
        let roles = self.manager.port.member_roles(guild_id, user_id).await;
        let (rn, rv, rs) = user_rank_from_roles(&roles);
        let sub_txt = rs.map(|s| format!(" {s}")).unwrap_or_default();
        let balancing_value = balancing_rule_value();
        RankReply::embed(json!({
            "title": format!("🎭 Rang-Information: {display_name}"),
            "color": 0x3498DB,
            "fields": [
                { "name": "Höchster Rang", "value": format!("{rn}{sub_txt}"), "inline": true },
                { "name": "Rang-Wert", "value": rv.to_string(), "inline": true },
                { "name": RRANG_INFO_BALANCING_RULE, "value": balancing_value, "inline": false },
            ],
        }))
    }
}

/// Parst eine einzelne User-Mention (`<@123>` / `<@!123>`).
fn parse_mention(token: &str) -> Option<u64> {
    let inner = token.strip_prefix("<@")?.strip_suffix('>')?;
    inner.strip_prefix('!').unwrap_or(inner).parse::<u64>().ok()
}

/// Parst eine Kanal-Mention (`<#123>`) oder eine reine ID.
fn parse_channel(token: &str) -> Option<u64> {
    if let Some(inner) = token.strip_prefix("<#").and_then(|s| s.strip_suffix('>')) {
        return inner.parse::<u64>().ok();
    }
    token.parse::<u64>().ok()
}

/// Root-Hilfe-Embed der `!rrang`-Gruppe (Port des Gruppen-Embeds).
fn help_embed() -> Value {
    json!({
        "title": "🎭 Rollen-Berechtigungen Rang-System",
        "description": "Verwaltet Sprachkanäle über Discord-Rollen-Berechtigungen (mit DB-Persistenz)",
        "color": 0x0099FF,
        "fields": [{
            "name": "📋 Befehle",
            "value": "`info` • Rang-Info eines Users\n\
                      `debug` • Debug zu User-Rollen\n\
                      `anker` • Zeigt Kanal-Anker\n\
                      `toggle [ein/aus]` • System für aktuellen VC\n\
                      `vcstatus` • Status des aktuellen VC\n\
                      `status` • Systemstatus\n\
                      `rollen` • Liste der Rang-Rollen\n\
                      `kanäle` • Überwachte/ausgeschlossene Kanäle\n\
                      `aktualisieren [#vc]` • Forced Update",
            "inline": false,
        }],
    })
}

/// Message-Listener für `!rrang` (Python: `@commands.group("rrang")`,
/// `manage_guild`). Folgt dem Muster eines eigenständigen Spawn-/Listener-Tasks:
/// eigener `MessageEvent`-Subscriber, admin-gated (Administrator, konsistent mit
/// den anderen Admin-Command-Listenern), Antwort über den `ChannelSender`.
pub fn spawn_command(
    commands: Arc<RankCommands>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let Some(guild_id) = event.guild_id else {
                        continue;
                    };
                    if !event.author_can_manage_guild {
                        continue;
                    }
                    let content = event.content.trim();
                    let Some(reply) = commands.reply_for(content, guild_id, event.author_id).await
                    else {
                        continue;
                    };
                    let _ = sender
                        .send_to_channel(event.channel_id, reply.content.as_deref(), &reply.embeds)
                        .await;
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

    #[test]
    fn anchor_range_wie_python() {
        assert_eq!(anchor_range(10, 3), (54, 72, 8, 11));
        assert_eq!(anchor_range(1, 1), (7, 16, 1, 2));
        assert_eq!(anchor_range(6, 3), (30, 48, 4, 7));
        assert_eq!(anchor_range(11, 6), (63, 72, 10, 11));
        assert_eq!(anchor_range(2, 5), (8, 26, 1, 4));
    }

    #[test]
    fn neue_rrang_texte_bleiben_platzhalter() {
        assert_eq!(RRANG_INFO_BALANCING_RULE, "Platzhalter");
    }

    #[test]
    fn subrank_rollen_parsing() {
        assert_eq!(
            parse_subrank_role("Ascendant 3"),
            Some(("Ascendant".to_string(), 10, 3))
        );
        assert_eq!(
            parse_subrank_role("Asc 3"),
            Some(("Ascendant".to_string(), 10, 3))
        );
        assert_eq!(
            parse_subrank_role("arch 2"),
            Some(("Archon".to_string(), 7, 2))
        );
        assert_eq!(parse_subrank_role("Phantom 7"), None);
        assert_eq!(parse_subrank_role("Moderator"), None);
    }

    #[test]
    fn rang_aus_rollen() {
        // Sub-Rang-Rolle (Score 63) schlägt Haupt-Rolle Phantom (Score 54)
        let roles = vec![
            (1331458016356208680u64, "Phantom".to_string()),
            (99, "Asc 3".to_string()),
        ];
        assert_eq!(
            user_rank_from_roles(&roles),
            ("Ascendant".to_string(), 10, Some(3))
        );
        // Nur Haupt-Rolle → Subrank unbekannt (kommt aus DB)
        let roles = vec![(1331458016356208680u64, "Phantom".to_string())];
        assert_eq!(
            user_rank_from_roles(&roles),
            ("Phantom".to_string(), 9, None)
        );
        assert_eq!(
            user_rank_from_roles(&[(5, "Mod".to_string())]),
            ("Obscurus".to_string(), 0, None)
        );
    }

    #[test]
    fn erlaubte_rollen_im_fenster() {
        let guild_roles = vec![
            (1, "Asc 1".to_string()),      // 61
            (2, "Phantom 6".to_string()),  // 60
            (3, "Emissary 3".to_string()), // 39
            (4, "Eternus 6".to_string()),  // 72
            (5, "Moderator".to_string()),
        ];
        let allowed = allowed_subrank_roles(&guild_roles, 54, 72);
        assert_eq!(allowed, HashSet::from([1, 2, 4]));
    }

    #[test]
    fn kanal_namen() {
        assert_eq!(
            desired_channel_name(Some(("Phantom", 3)), None),
            "Phantom 3"
        );
        assert_eq!(desired_channel_name(None, Some(("Seeker", 2))), "Seeker 2");
        assert_eq!(desired_channel_name(None, None), "Rang-Sprachkanal");
    }

    #[test]
    fn rang_name_aus_wert() {
        assert_eq!(rank_name_from_value(0), "Obscurus");
        assert_eq!(rank_name_from_value(9), "Phantom");
        assert_eq!(rank_name_from_value(11), "Eternus");
        assert_eq!(rank_name_from_value(99), "Obscurus");
    }

    #[test]
    fn mention_und_kanal_parsen() {
        assert_eq!(parse_mention("<@123>"), Some(123));
        assert_eq!(parse_mention("<@!456>"), Some(456));
        assert_eq!(parse_mention("abc"), None);
        assert_eq!(parse_channel("<#789>"), Some(789));
        assert_eq!(parse_channel("789"), Some(789));
        assert_eq!(parse_channel("foo"), None);
    }

    // ── `!rrang`-Command-Tests (Mock-Port + Test-DB) ────────────────────────

    use std::{collections::HashMap as StdHashMap, sync::Mutex as StdMutex};

    /// Mock der RankPort-Discord-Seite. Nur die für die Admin-Commands
    /// relevanten Felder; die reconcile-Pfade werden hier nicht getrieben.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ApplyCall {
        guild_id: u64,
        channel_id: u64,
        allowed: HashSet<u64>,
        removes: HashSet<u64>,
    }

    struct MockRankPort {
        caller_channel: Option<u64>,
        category: Option<u64>,
        channel_name: String,
        members: usize,
        target_roles: Vec<(u64, String)>,
        target_name: Option<String>,
        channel_members: Vec<(u64, Vec<(u64, String)>)>,
        guild_roles: Vec<(u64, String)>,
        current_overwrites: Vec<u64>,
        guild_member_roles: StdHashMap<u64, Option<Vec<(u64, String)>>>,
        guild_member_role_calls: StdMutex<Vec<u64>>,
        apply_calls: StdMutex<Vec<ApplyCall>>,
        rename_calls: StdMutex<Vec<(u64, String)>>,
        overwrites_cleared: StdMutex<bool>,
    }

    #[async_trait::async_trait]
    impl RankPort for MockRankPort {
        async fn channel_members_with_roles(
            &self,
            _g: u64,
            _c: u64,
        ) -> Vec<(u64, Vec<(u64, String)>)> {
            self.channel_members.clone()
        }
        async fn guild_roles(&self, _g: u64) -> Vec<(u64, String)> {
            self.guild_roles.clone()
        }
        async fn current_role_overwrites(&self, _g: u64, _c: u64) -> Vec<u64> {
            // eine Haupt-Rang-Rolle, damit clear_rank_overwrites etwas räumt
            self.current_overwrites.clone()
        }
        async fn apply_overwrites(
            &self,
            guild_id: u64,
            channel_id: u64,
            allowed: &HashSet<u64>,
            removes: &HashSet<u64>,
        ) -> Result<(), String> {
            *self.overwrites_cleared.lock().expect("lock") = true;
            self.apply_calls.lock().expect("lock").push(ApplyCall {
                guild_id,
                channel_id,
                allowed: allowed.clone(),
                removes: removes.clone(),
            });
            Ok(())
        }
        async fn rename(&self, channel_id: u64, name: &str) -> Result<(), String> {
            self.rename_calls
                .lock()
                .expect("lock")
                .push((channel_id, name.to_string()));
            Ok(())
        }
        async fn channel_category(&self, _g: u64, _c: u64) -> Option<u64> {
            self.category
        }
        async fn caller_voice_channel(&self, _g: u64, _u: u64) -> Option<u64> {
            self.caller_channel
        }
        async fn channel_name(&self, _g: u64, _c: u64) -> Option<String> {
            Some(self.channel_name.clone())
        }
        async fn category_name(&self, _g: u64, _c: u64) -> Option<String> {
            Some("Comp/Ranked".to_string())
        }
        async fn channel_member_count(&self, _g: u64, _c: u64) -> usize {
            self.members
        }
        async fn member_display_name(&self, _g: u64, _u: u64) -> Option<String> {
            self.target_name.clone()
        }
        async fn member_roles(&self, _g: u64, _u: u64) -> Vec<(u64, String)> {
            self.target_roles.clone()
        }
        async fn guild_member_roles(&self, _g: u64, user_id: u64) -> Option<Vec<(u64, String)>> {
            self.guild_member_role_calls
                .lock()
                .expect("lock")
                .push(user_id);
            self.guild_member_roles
                .get(&user_id)
                .cloned()
                .unwrap_or(None)
        }
        async fn role_member_count(&self, _g: u64, _r: u64) -> usize {
            0
        }
        async fn category_voice_channels(&self, _g: u64, _cat: u64) -> Vec<(u64, String)> {
            Vec::new()
        }
    }

    const RANK_DDLS: [&str; 2] = [
        "CREATE TABLE voice_channel_settings(channel_id INTEGER PRIMARY KEY, guild_id INTEGER NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, updated_at TEXT)",
        "CREATE TABLE voice_channel_anchors(channel_id INTEGER PRIMARY KEY, guild_id INTEGER NOT NULL, user_id INTEGER NOT NULL, rank_name TEXT NOT NULL, rank_value INTEGER NOT NULL, allowed_min INTEGER NOT NULL, allowed_max INTEGER NOT NULL, anchor_subrank INTEGER DEFAULT 3, score_min INTEGER, score_max INTEGER, updated_at TEXT)",
    ];

    async fn rank_setup_with_owner(
        port: MockRankPort,
        initial_owner: Arc<dyn Fn(u64) -> Option<u64> + Send + Sync>,
    ) -> (
        tempfile::TempDir,
        Arc<RankCommands>,
        Arc<RankVoiceManager>,
        Arc<MockRankPort>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in RANK_DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        let port = Arc::new(port);
        let manager = RankVoiceManager::new(db, port.clone(), initial_owner);
        let commands = RankCommands::new(manager.clone());
        (dir, commands, manager, port)
    }

    async fn rank_setup(
        port: MockRankPort,
    ) -> (
        tempfile::TempDir,
        Arc<RankCommands>,
        Arc<RankVoiceManager>,
        Arc<MockRankPort>,
    ) {
        rank_setup_with_owner(port, Arc::new(|_channel_id| None)).await
    }

    fn monitored_port() -> MockRankPort {
        MockRankPort {
            caller_channel: Some(500),
            category: Some(MONITORED_CATEGORY_ID),
            channel_name: "Phantom 3".to_string(),
            members: 2,
            target_roles: Vec::new(),
            target_name: Some("Tester".to_string()),
            channel_members: Vec::new(),
            guild_roles: Vec::new(),
            current_overwrites: vec![1331458016356208680],
            guild_member_roles: StdHashMap::new(),
            guild_member_role_calls: StdMutex::new(Vec::new()),
            apply_calls: StdMutex::new(Vec::new()),
            rename_calls: StdMutex::new(Vec::new()),
            overwrites_cleared: StdMutex::new(false),
        }
    }

    fn role(role_id: u64, name: &str) -> (u64, String) {
        (role_id, name.to_string())
    }

    fn owner_for(channel_id: u64, owner_id: u64) -> Arc<dyn Fn(u64) -> Option<u64> + Send + Sync> {
        Arc::new(move |candidate| (candidate == channel_id).then_some(owner_id))
    }

    #[tokio::test]
    async fn kein_rrang_befehl_ist_none() {
        let (_dir, cmds, _m, _p) = rank_setup(monitored_port()).await;
        assert!(cmds.reply_for("hallo", 1, 9).await.is_none());
    }

    #[tokio::test]
    async fn rrang_root_zeigt_hilfe() {
        let (_dir, cmds, _m, _p) = rank_setup(monitored_port()).await;
        let reply = cmds.reply_for("!rrang", 1, 9).await.unwrap();
        assert!(reply.embeds[0]["title"]
            .as_str()
            .unwrap()
            .contains("Rang-System"));
    }

    #[tokio::test]
    async fn toggle_ohne_voice_meldet_fehler() {
        let mut port = monitored_port();
        port.caller_channel = None;
        let (_dir, cmds, _m, _p) = rank_setup(port).await;
        let reply = cmds.reply_for("!rrang toggle ein", 1, 9).await.unwrap();
        assert!(reply.content.unwrap().contains("Voice Channel"));
    }

    #[tokio::test]
    async fn toggle_nicht_ueberwacht() {
        let mut port = monitored_port();
        port.category = Some(999); // andere Kategorie → nicht überwacht
        let (_dir, cmds, _m, _p) = rank_setup(port).await;
        let reply = cmds.reply_for("!rrang toggle ein", 1, 9).await.unwrap();
        assert!(reply.content.unwrap().contains("nicht überwacht"));
    }

    #[tokio::test]
    async fn toggle_aus_persistiert_und_raeumt() {
        let (_dir, cmds, manager, port) = rank_setup(monitored_port()).await;
        // Default ist enabled → aus schaltet ab + räumt Overwrites.
        let reply = cmds.reply_for("!rrang toggle aus", 1, 9).await.unwrap();
        assert!(reply.content.unwrap().contains("Deaktiviert"));
        assert!(!manager.store.is_enabled(500).await);
        assert!(*port.overwrites_cleared.lock().expect("lock"));
        // Erneut aus → "Bereits deaktiviert".
        let reply = cmds.reply_for("!rrang toggle aus", 1, 9).await.unwrap();
        assert!(reply.content.unwrap().contains("Bereits deaktiviert"));
    }

    #[tokio::test]
    async fn toggle_ein_persistiert() {
        let (_dir, cmds, manager, _p) = rank_setup(monitored_port()).await;
        // Erst aus, dann wieder ein.
        manager.store.set_enabled(500, 1, false).await;
        let reply = cmds.reply_for("!rrang toggle ein", 1, 9).await.unwrap();
        assert!(reply.content.unwrap().contains("Aktiviert"));
        assert!(manager.store.is_enabled(500).await);
    }

    #[tokio::test]
    async fn toggle_ohne_arg_zeigt_status() {
        let (_dir, cmds, _m, _p) = rank_setup(monitored_port()).await;
        let reply = cmds.reply_for("!rrang toggle", 1, 9).await.unwrap();
        let text = reply.content.unwrap();
        assert!(text.contains("Rang-System für"), "text: {text}");
        assert!(text.contains("Aktiviert"), "text: {text}");
    }

    #[tokio::test]
    async fn debug_erkennt_rang_aus_rollen() {
        let mut port = monitored_port();
        port.target_roles = vec![(1331458016356208680, "Phantom".to_string())];
        let (_dir, cmds, _m, _p) = rank_setup(port).await;
        let reply = cmds.reply_for("!rrang debug", 1, 9).await.unwrap();
        let fields = reply.embeds[0]["fields"].as_array().unwrap();
        // "Erkannter Rang" zeigt Phantom (9).
        assert!(fields[1]["value"].as_str().unwrap().contains("Phantom"));
    }

    #[tokio::test]
    async fn aktualisieren_nicht_ueberwacht() {
        let mut port = monitored_port();
        port.category = Some(999);
        let (_dir, cmds, _m, _p) = rank_setup(port).await;
        let reply = cmds.reply_for("!rrang aktualisieren", 1, 9).await.unwrap();
        assert!(reply.content.unwrap().contains("nicht überwacht"));
    }

    #[tokio::test]
    async fn anker_leer_meldet_keine() {
        let (_dir, cmds, _m, _p) = rank_setup(monitored_port()).await;
        let reply = cmds.reply_for("!rrang anker", 1, 9).await.unwrap();
        assert!(reply.embeds[0]["description"]
            .as_str()
            .unwrap()
            .contains("Keine aktiven"));
    }

    #[tokio::test]
    async fn reconcile_channel_loest_erstbesitzer_gildenweit_auf() {
        let mut port = monitored_port();
        port.channel_members = vec![(7, vec![role(1331458016356208680, "Phantom")])];
        port.guild_member_roles
            .insert(42, Some(vec![role(1331457949654319114, "Archon")]));
        let (_dir, _cmds, manager, port) = rank_setup_with_owner(port, owner_for(500, 42)).await;

        manager.reconcile_channel(1, 500).await;

        let anchor = manager.anchor_for(500).await.expect("anchor");
        assert_eq!(anchor.user_id, 42);
        assert_eq!(anchor.rank_name, "Archon");
        assert_eq!(anchor.rank_value, 7);
        assert_eq!(
            *port.guild_member_role_calls.lock().expect("lock"),
            vec![42]
        );
    }

    #[tokio::test]
    async fn reconcile_channel_priorisiert_owner_vor_fallback() {
        let mut port = monitored_port();
        port.channel_members = vec![
            (42, vec![role(1331457652877955072, "Seeker")]),
            (7, vec![role(1331458087349129296, "Eternus")]),
        ];
        port.guild_member_roles
            .insert(42, Some(vec![role(1331458087349129296, "Eternus")]));
        let (_dir, _cmds, manager, port) = rank_setup_with_owner(port, owner_for(500, 42)).await;

        manager.reconcile_channel(1, 500).await;

        let anchor = manager.anchor_for(500).await.expect("anchor");
        assert_eq!(anchor.user_id, 42);
        assert_eq!(anchor.rank_name, "Seeker");
        assert!(port
            .guild_member_role_calls
            .lock()
            .expect("lock")
            .is_empty());

        let mut port = monitored_port();
        port.channel_members = vec![
            (8, vec![role(999, "Moderator")]),
            (9, vec![role(1331458016356208680, "Phantom")]),
        ];
        let (_dir, _cmds, manager, _port) = rank_setup(port).await;

        manager.reconcile_channel(1, 500).await;

        let anchor = manager.anchor_for(500).await.expect("anchor");
        assert_eq!(anchor.user_id, 9);
        assert_eq!(anchor.rank_name, "Phantom");
    }

    #[tokio::test]
    async fn reconcile_channel_ignoriert_bot_und_ungerankten_owner() {
        let mut port = monitored_port();
        port.channel_members = vec![(7, vec![role(1331458016356208680, "Phantom")])];
        port.guild_member_roles.insert(42, None);
        let (_dir, _cmds, manager, port) = rank_setup_with_owner(port, owner_for(500, 42)).await;

        manager.reconcile_channel(1, 500).await;

        let anchor = manager.anchor_for(500).await.expect("anchor");
        assert_eq!(anchor.user_id, 7);
        assert_eq!(anchor.rank_name, "Phantom");
        assert_eq!(
            *port.guild_member_role_calls.lock().expect("lock"),
            vec![42]
        );

        let mut port = monitored_port();
        port.channel_members = vec![(8, vec![role(1331457949654319114, "Archon")])];
        port.guild_member_roles
            .insert(43, Some(vec![role(999, "Moderator")]));
        let (_dir, _cmds, manager, _port) = rank_setup_with_owner(port, owner_for(500, 43)).await;

        manager.reconcile_channel(1, 500).await;

        let anchor = manager.anchor_for(500).await.expect("anchor");
        assert_eq!(anchor.user_id, 8);
        assert_eq!(anchor.rank_name, "Archon");
    }
}
