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
use dl_discord::{Dispatcher, VoiceEvent};
use rusqlite::OptionalExtension;

use crate::status::{select_best_presence, select_channel_cohort, CohortEntry, PresenceRow};

pub const RANKED_SUBRANK_TOLERANCE: i64 = 9;
pub const SCORE_MIN_ABSOLUTE: i64 = 7;
pub const SCORE_MAX_ABSOLUTE: i64 = 72;
pub const PRESENCE_STALE_SECONDS: i64 = 180;

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

    async fn is_monitored(&self, guild_id: u64, channel_id: u64) -> bool {
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

        // Anker bestimmen: Erstbesitzer > bestehender Anker (wenn anwesend) > erstes rangiertes Mitglied
        let anchor_member = match (self.initial_owner)(channel_id) {
            Some(owner_id) => members.iter().find(|(id, _)| *id == owner_id).cloned(),
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
    tokio::spawn(async move {
        manager.rehydrate().await;
        loop {
            match events.recv().await {
                Ok(event) => manager.handle_event(event).await,
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
}
