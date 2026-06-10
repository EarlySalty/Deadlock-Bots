//! Deadlock-Voice-Status — Port von `cogs/deadlock_voice_status.py` +
//! `service/deadlock_voice_cohort.py`.
//!
//! Hängt den Lanes in den überwachten Kategorien einen Live-Status an
//! (`"Lane 1 - im Match Min 17 (4/6)"` / `"… - in der Lobby"`), gespeist aus
//! `live_player_state` (Steam-Presence-Worker) und `deadlock_party_members`.
//! Rename-Disziplin wie das Original: 6-min-Cooldown (10 min ab Match-Minute
//! 25), Match-Ende/Status-Löschung umgehen den Cooldown, reine
//! Member-Zahl-Änderungen ohne Spielstatus lösen NIE ein Rename aus.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use dl_db::Db;

pub const POLL_INTERVAL: Duration = Duration::from_secs(60);
pub const PRESENCE_STALE_SECONDS: i64 = 180;
pub const PARTY_MEMBER_STALE_SECONDS: i64 = 600;
pub const RENAME_COOLDOWN_SECONDS: f64 = 360.0;
pub const LATE_MATCH_COOLDOWN_SECONDS: f64 = 600.0;
pub const MIN_ACTIVE_PLAYERS: usize = 1;

/// Überwachte Kategorien (VOICE_STATUS_CATEGORY_* = TempVoice-Kategorien).
pub const TARGET_CATEGORY_IDS: [u64; 3] = [
    1289721245281292290,
    1412804540994162789,
    1357422957017698478,
];
/// Permanenter Chill-Voice — Status wird hier aktiv entfernt.
pub const EXCLUDED_CHANNEL_IDS: [u64; 1] = [1493690350580138114];

// ── Pure Logik (Referenzwerte aus CPython in den Tests) ────────────────────

/// Eine Zeile aus `live_player_state`.
#[derive(Debug, Clone, Default)]
pub struct PresenceRow {
    pub deadlock_stage: Option<String>,
    pub deadlock_minutes: Option<i64>,
    pub deadlock_localized: Option<String>,
    pub deadlock_updated_at: Option<i64>,
    pub last_seen_ts: Option<i64>,
    pub in_match_now_strict: bool,
    pub last_server_id: Option<String>,
    pub deadlock_party_hint: Option<String>,
}

/// `"Spiel (23. min.)"` → 23 (MATCH_STATUS_REGEX-Pendant).
fn parse_localized_minutes(localized: &str) -> Option<i64> {
    let bytes = localized.as_bytes();
    let mut i = 0;
    while let Some(offset) = localized[i..].find('(') {
        let start = i + offset + 1;
        let digits: String = localized[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(3)
            .collect();
        if !digits.is_empty() {
            let mut rest = start + digits.len();
            if rest < bytes.len() && bytes[rest] == b'.' {
                rest += 1;
            }
            let tail = localized[rest..].trim_start();
            let tail_lower = tail.to_lowercase();
            if let Some(after) = tail_lower.strip_prefix("min") {
                let after = after.strip_prefix('.').unwrap_or(after);
                if after.starts_with(')') {
                    return digits.parse().ok();
                }
            }
        }
        i = start;
    }
    None
}

/// (stage, minutes, server_id) — None wenn stale/leer.
pub fn evaluate_presence_row(
    row: &PresenceRow,
    now: i64,
    stale_seconds: i64,
) -> Option<(String, Option<i64>, Option<String>)> {
    let updated_at = row.deadlock_updated_at.or(row.last_seen_ts)?;
    if now - updated_at > stale_seconds {
        return None;
    }
    let localized = row.deadlock_localized.clone().unwrap_or_default();
    let match_minutes = parse_localized_minutes(localized.trim());
    let stage = row
        .deadlock_stage
        .clone()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let server_id = row
        .last_server_id
        .clone()
        .or_else(|| row.deadlock_party_hint.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let normalized_minutes = row.deadlock_minutes.map(|m| m.max(0));

    if row.in_match_now_strict || stage == "match" {
        if let Some(minutes) = normalized_minutes {
            return Some(("match".into(), Some(minutes), server_id));
        }
        if let Some(minutes) = match_minutes {
            return Some(("match".into(), Some(minutes.max(0)), server_id));
        }
        return Some(("match".into(), Some(0), server_id));
    }
    if let Some(minutes) = match_minutes {
        return Some(("match".into(), Some(minutes.max(0)), server_id));
    }
    if server_id.is_some() {
        return Some(("lobby".into(), None, server_id));
    }
    None
}

/// Beste Presence über mehrere Steam-Accounts (match > lobby, mehr Minuten).
pub fn select_best_presence(
    steam_ids: &[String],
    presence_map: &HashMap<String, PresenceRow>,
    now: i64,
    stale_seconds: i64,
) -> Option<(String, Option<i64>, Option<String>, String)> {
    let mut best: Option<(String, Option<i64>, Option<String>, String)> = None;
    let mut best_score = -1i64;
    for sid in steam_ids {
        let Some(row) = presence_map.get(sid) else {
            continue;
        };
        let Some((stage, minutes, server_id)) = evaluate_presence_row(row, now, stale_seconds)
        else {
            continue;
        };
        let stage_score = match stage.as_str() {
            "match" => 2,
            "lobby" => 1,
            _ => 0,
        };
        let score = stage_score * 100_000 + minutes.unwrap_or(-1);
        if score > best_score {
            best_score = score;
            best = Some((stage, minutes, server_id, sid.clone()));
        }
    }
    best
}

#[derive(Debug, Clone)]
pub struct CohortEntry {
    pub member_id: u64,
    pub stage: String,
    pub minutes: i64,
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cohort {
    pub stage: String,
    pub server_id: Option<String>,
    pub member_ids: Vec<u64>,
    pub minute_values: Vec<i64>,
}

/// Größte zusammengehörige Gruppe im Kanal (match schlägt lobby).
pub fn select_channel_cohort(entries: &[CohortEntry], min_active_players: usize) -> Option<Cohort> {
    // Insertion-Order wie Pythons dict — wichtig für Tie-Breaks
    let mut keys: Vec<(String, Option<String>)> = Vec::new();
    let mut members: HashMap<(String, Option<String>), Vec<u64>> = HashMap::new();
    let mut minutes: HashMap<(String, Option<String>), Vec<i64>> = HashMap::new();
    for entry in entries {
        if entry.stage != "lobby" && entry.stage != "match" {
            continue;
        }
        let key = (entry.stage.clone(), entry.server_id.clone());
        if !members.contains_key(&key) {
            keys.push(key.clone());
        }
        members
            .entry(key.clone())
            .or_default()
            .push(entry.member_id);
        minutes.entry(key).or_default().push(entry.minutes);
    }
    // Bekannte Server zuerst, dann unbekannte — wie die zwei Schleifen im Original
    keys.sort_by_key(|(_, server)| server.is_none());

    let mut candidate: Option<Cohort> = None;
    for key in keys {
        let member_ids = members.get(&key).cloned().unwrap_or_default();
        if member_ids.len() < min_active_players {
            continue;
        }
        let next = Cohort {
            stage: key.0.clone(),
            server_id: key.1.clone(),
            minute_values: minutes.get(&key).cloned().unwrap_or_default(),
            member_ids,
        };
        match &candidate {
            None => candidate = Some(next),
            Some(current) => {
                let upgrade_to_match = current.stage != "match" && next.stage == "match";
                let bigger_same_stage =
                    current.stage == next.stage && next.member_ids.len() > current.member_ids.len();
                if upgrade_to_match || bigger_same_stage {
                    candidate = Some(next);
                }
            }
        }
    }
    candidate
}

/// `"Lane 1 - im Match Min 17 (4/6)"` → ("Lane 1", Some(suffix)).
pub fn split_suffix(name: &str) -> (String, Option<String>) {
    let lower = name.to_lowercase();
    // Suffix beginnt am letzten " - " dessen Rest einem Status-Muster entspricht
    let mut search_from = 0;
    while let Some(offset) = lower[search_from..].find('-') {
        let dash = search_from + offset;
        let tail = lower[dash + 1..].trim();
        if is_status_suffix(tail) {
            let base = name[..dash].trim_end().trim_end_matches('-').trim_end();
            let suffix = name[dash..].trim();
            let base = if base.is_empty() { name.trim() } else { base };
            return (base.to_string(), Some(suffix.to_string()));
        }
        search_from = dash + 1;
    }
    (name.trim().to_string(), None)
}

fn is_status_suffix(tail: &str) -> bool {
    if let Some(rest) = tail.strip_prefix("in der lobby") {
        let rest = rest.trim();
        return rest.is_empty() || parse_counts(rest);
    }
    if let Some(rest) = tail.strip_prefix("im match min ") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return false;
        }
        let mut rest = &rest[digits.len()..];
        rest = rest.strip_prefix('+').unwrap_or(rest);
        return parse_counts(rest.trim());
    }
    false
}

/// `"(4/6)"` am Ende?
fn parse_counts(s: &str) -> bool {
    let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) else {
        return false;
    };
    let Some((a, b)) = inner.split_once('/') else {
        return false;
    };
    !a.is_empty()
        && !b.is_empty()
        && a.chars().all(|c| c.is_ascii_digit())
        && b.chars().all(|c| c.is_ascii_digit())
}

// ── Rename-Entscheidung ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ChannelState {
    pub stage: Option<String>,
    pub suffix: Option<String>,
    pub last_rename: f64,
    pub previous_member_count: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenameDecision {
    NoopTargetMatches,
    NoopNoMeaningfulChange,
    Cooldown,
    Rename,
}

#[allow(clippy::too_many_arguments)]
pub fn decide_rename(
    state: &ChannelState,
    current_name: &str,
    target_name: &str,
    base_clean: &str,
    desired_suffix: Option<&str>,
    current_suffix: Option<&str>,
    stage_label: Option<&str>,
    player_count: Option<i64>,
    minutes_value: Option<i64>,
    now: f64,
) -> RenameDecision {
    if current_name == target_name {
        return RenameDecision::NoopTargetMatches;
    }
    let previous_stage = state.stage.as_deref();
    let match_exit_override = previous_stage == Some("match") && stage_label == Some("lobby");
    let clear_status_override =
        matches!(previous_stage, Some("match") | Some("lobby")) && stage_label.is_none();
    let elapsed = now - state.last_rename;
    let mut effective_cooldown = RENAME_COOLDOWN_SECONDS;
    if stage_label == Some("match") && minutes_value.is_some_and(|m| m >= 25) {
        effective_cooldown = effective_cooldown.max(LATE_MATCH_COOLDOWN_SECONDS);
    }

    let meaningful_change = stage_label != previous_stage || desired_suffix != current_suffix;
    let should_rename = if meaningful_change {
        true
    } else if stage_label.is_none() && player_count != state.previous_member_count {
        // Nur Member-Zahl ohne Spielstatus → nie umbenennen
        false
    } else {
        desired_suffix != current_suffix || base_clean != current_name.trim_end()
    };
    if !should_rename {
        return RenameDecision::NoopNoMeaningfulChange;
    }
    if elapsed < effective_cooldown && !match_exit_override && !clear_status_override {
        return RenameDecision::Cooldown;
    }
    RenameDecision::Rename
}

// ── Party-Aufstockung ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PartyRow {
    pub party_id: String,
    pub steam_id: String,
    pub party_size: Option<i64>,
    pub seen_at: i64,
}

fn normalize_party_size(value: Option<i64>) -> Option<i64> {
    value.filter(|v| *v >= 1).map(|v| v.min(6))
}

/// Beste Party (Überlapp mit Kohorte > gemeldete Größe > frischer > längere ID).
pub fn select_best_party(cohort_steam_ids: &HashSet<String>, rows: &[PartyRow]) -> Option<String> {
    struct Bucket {
        overlap: HashSet<String>,
        sizes: Vec<i64>,
        latest: i64,
    }
    let mut grouped: HashMap<String, Bucket> = HashMap::new();
    for row in rows {
        if row.party_id.is_empty() || row.steam_id.is_empty() {
            continue;
        }
        let bucket = grouped.entry(row.party_id.clone()).or_insert(Bucket {
            overlap: HashSet::new(),
            sizes: Vec::new(),
            latest: 0,
        });
        if cohort_steam_ids.contains(&row.steam_id) {
            bucket.overlap.insert(row.steam_id.clone());
        }
        if let Some(size) = normalize_party_size(row.party_size) {
            bucket.sizes.push(size);
        }
        bucket.latest = bucket.latest.max(row.seen_at);
    }
    grouped
        .into_iter()
        .filter(|(_, b)| !b.overlap.is_empty())
        .max_by_key(|(party_id, b)| {
            (
                b.overlap.len(),
                b.sizes.iter().copied().max().unwrap_or(0),
                b.latest,
                party_id.len(),
            )
        })
        .map(|(party_id, _)| party_id)
}

// ── DB-Zugriffe ────────────────────────────────────────────────────────────

pub struct StatusStore {
    pub db: Db,
}

impl StatusStore {
    /// user_id → Steam-IDs, sortiert primary > verified > frisch.
    pub async fn steam_ids_for(&self, user_ids: Vec<u64>) -> HashMap<u64, Vec<String>> {
        if user_ids.is_empty() {
            return HashMap::new();
        }
        let ids_json = serde_json::to_string(&user_ids).unwrap_or_default();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT user_id, steam_id FROM steam_links
                      WHERE user_id IN (SELECT value FROM json_each(?1))
                        AND steam_id IS NOT NULL AND steam_id != ''
                      ORDER BY primary_account DESC, verified DESC, updated_at DESC",
                )?;
                let rows = stmt.query_map([ids_json], |row| {
                    Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut map: HashMap<u64, Vec<String>> = HashMap::new();
                for row in rows {
                    let (uid, sid) = row?;
                    let bucket = map.entry(uid).or_default();
                    if !bucket.contains(&sid) {
                        bucket.push(sid);
                    }
                }
                Ok(map)
            })
            .await
            .unwrap_or_default()
    }

    pub async fn presence_rows(&self, steam_ids: Vec<String>) -> HashMap<String, PresenceRow> {
        if steam_ids.is_empty() {
            return HashMap::new();
        }
        let ids_json = serde_json::to_string(&steam_ids).unwrap_or_default();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT steam_id, deadlock_stage, deadlock_minutes, deadlock_localized,
                            deadlock_updated_at, last_seen_ts, in_match_now_strict,
                            last_server_id, deadlock_party_hint
                       FROM live_player_state
                      WHERE steam_id IN (SELECT value FROM json_each(?1))",
                )?;
                let rows = stmt.query_map([ids_json], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        PresenceRow {
                            deadlock_stage: row.get(1)?,
                            deadlock_minutes: row.get(2)?,
                            deadlock_localized: row.get(3)?,
                            deadlock_updated_at: row.get(4)?,
                            last_seen_ts: row.get(5)?,
                            in_match_now_strict: row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
                            last_server_id: row.get(7)?,
                            deadlock_party_hint: row.get(8)?,
                        },
                    ))
                })?;
                rows.collect::<Result<HashMap<_, _>, _>>()
            })
            .await
            .unwrap_or_default()
    }

    pub async fn party_rows_for_steam_ids(
        &self,
        steam_ids: Vec<String>,
        now: i64,
    ) -> Vec<PartyRow> {
        if steam_ids.is_empty() {
            return Vec::new();
        }
        let ids_json = serde_json::to_string(&steam_ids).unwrap_or_default();
        let cutoff = now - PARTY_MEMBER_STALE_SECONDS;
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT party_id, steam_id, party_size, seen_at
                       FROM deadlock_party_members
                      WHERE steam_id IN (SELECT value FROM json_each(?1)) AND seen_at >= ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![ids_json, cutoff], |row| {
                    Ok(PartyRow {
                        party_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        steam_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        party_size: row.get(2)?,
                        seen_at: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    })
                })?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }

    pub async fn party_rows_for_party_ids(
        &self,
        party_ids: Vec<String>,
        now: i64,
    ) -> Vec<PartyRow> {
        if party_ids.is_empty() {
            return Vec::new();
        }
        let ids_json = serde_json::to_string(&party_ids).unwrap_or_default();
        let cutoff = now - PARTY_MEMBER_STALE_SECONDS;
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT party_id, steam_id, party_size, seen_at
                       FROM deadlock_party_members
                      WHERE party_id IN (SELECT value FROM json_each(?1)) AND seen_at >= ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![ids_json, cutoff], |row| {
                    Ok(PartyRow {
                        party_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        steam_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        party_size: row.get(2)?,
                        seen_at: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    })
                })?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }

    /// `deadlock_voice_watch` synchronisieren (Voice-Standorte je Steam-ID).
    pub async fn persist_voice_watch(&self, entries: Vec<(String, u64, u64)>, now: i64) {
        let result = self
            .db
            .write(move |conn| {
                if entries.is_empty() {
                    conn.execute("DELETE FROM deadlock_voice_watch", [])?;
                    return Ok(());
                }
                for (steam_id, guild_id, channel_id) in &entries {
                    conn.execute(
                        "INSERT INTO deadlock_voice_watch(steam_id, guild_id, channel_id, updated_at)
                         VALUES(?1, ?2, ?3, ?4)
                         ON CONFLICT(steam_id) DO UPDATE SET
                           guild_id=excluded.guild_id,
                           channel_id=excluded.channel_id,
                           updated_at=excluded.updated_at",
                        rusqlite::params![steam_id, guild_id, channel_id, now],
                    )?;
                }
                let keep: Vec<String> = entries.iter().map(|(s, _, _)| s.clone()).collect();
                let keep_json = serde_json::to_string(&keep).unwrap_or_default();
                conn.execute(
                    "DELETE FROM deadlock_voice_watch
                      WHERE steam_id NOT IN (SELECT value FROM json_each(?1))",
                    [keep_json],
                )?;
                Ok(())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, "VoiceStatus: voice_watch-Persist fehlgeschlagen");
        }
    }
}

// ── Worker ─────────────────────────────────────────────────────────────────

/// Kanal-Sicht für den Worker (Cache-Reads, Tests mocken sie).
#[async_trait::async_trait]
pub trait StatusPort: Send + Sync {
    /// Alle Voice-Kanäle in den Ziel-Kategorien: (guild, channel, name, non-bot-member-ids).
    async fn monitored_channels(&self) -> Vec<(u64, u64, String, Vec<u64>)>;
    async fn channel_info(&self, channel_id: u64) -> Option<(u64, String, Vec<u64>)>;
    async fn rename(&self, channel_id: u64, name: &str) -> Result<(), String>;
}

pub struct VoiceStatusWorker {
    pub store: StatusStore,
    pub port: Arc<dyn StatusPort>,
    states: tokio::sync::Mutex<HashMap<u64, ChannelState>>,
}

impl VoiceStatusWorker {
    pub fn new(db: Db, port: Arc<dyn StatusPort>) -> Arc<Self> {
        Arc::new(Self {
            store: StatusStore { db },
            port,
            states: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub async fn tick(self: &Arc<Self>) {
        // Status von ausgenommenen Kanälen aktiv räumen
        for channel_id in EXCLUDED_CHANNEL_IDS {
            if let Some((_, name, _)) = self.port.channel_info(channel_id).await {
                let (base, suffix) = split_suffix(&name);
                if suffix.is_some() {
                    self.apply(channel_id, &name, &base, None, None, None, None, None)
                        .await;
                }
            }
        }

        let channels = self.port.monitored_channels().await;
        if channels.is_empty() {
            return;
        }
        let all_user_ids: Vec<u64> = channels
            .iter()
            .flat_map(|(_, _, _, members)| members.iter().copied())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let steam_map = self.store.steam_ids_for(all_user_ids).await;
        let all_steam_ids: Vec<String> = steam_map
            .values()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let presence_map = self.store.presence_rows(all_steam_ids).await;
        let now = chrono::Utc::now().timestamp();

        let mut watch: HashMap<String, (String, u64, u64)> = HashMap::new();
        for (guild_id, channel_id, name, members) in &channels {
            for member in members {
                for sid in steam_map.get(member).cloned().unwrap_or_default() {
                    watch.insert(sid.clone(), (sid, *guild_id, *channel_id));
                }
            }
            self.process_channel(*channel_id, name, members, &steam_map, &presence_map, now)
                .await;
        }
        self.store
            .persist_voice_watch(watch.into_values().collect(), now)
            .await;
    }

    async fn process_channel(
        self: &Arc<Self>,
        channel_id: u64,
        name: &str,
        members: &[u64],
        steam_map: &HashMap<u64, Vec<String>>,
        presence_map: &HashMap<String, PresenceRow>,
        now: i64,
    ) {
        let (base_name, _) = split_suffix(name);
        if members.is_empty() {
            self.apply(channel_id, name, &base_name, None, None, None, None, None)
                .await;
            return;
        }

        let mut entries = Vec::new();
        let mut chosen_steam: HashMap<u64, String> = HashMap::new();
        for member in members {
            let steam_ids = steam_map.get(member).cloned().unwrap_or_default();
            let Some((stage, minutes, server_id, sid)) =
                select_best_presence(&steam_ids, presence_map, now, PRESENCE_STALE_SECONDS)
            else {
                continue;
            };
            if stage != "lobby" && stage != "match" {
                continue;
            }
            chosen_steam.insert(*member, sid);
            entries.push(CohortEntry {
                member_id: *member,
                stage,
                minutes: minutes.unwrap_or(0),
                server_id,
            });
        }
        let Some(cohort) = select_channel_cohort(&entries, MIN_ACTIVE_PLAYERS) else {
            self.apply(channel_id, name, &base_name, None, None, None, None, None)
                .await;
            return;
        };

        let raw_count = (cohort.minute_values.len() as i64).min(6);
        let cohort_steam_ids: HashSet<String> = cohort
            .member_ids
            .iter()
            .filter_map(|m| chosen_steam.get(m).cloned())
            .collect();
        let player_count = self
            .effective_player_count(members, steam_map, &cohort_steam_ids, raw_count, now)
            .await;
        let voice_slots = (members.len() as i64).max(player_count);

        if cohort.stage == "lobby" {
            let suffix = "in der Lobby".to_string();
            self.apply(
                channel_id,
                name,
                &base_name,
                Some(suffix),
                Some("lobby".into()),
                Some(player_count),
                None,
                cohort.server_id.clone(),
            )
            .await;
        } else {
            let max_minutes = cohort.minute_values.iter().copied().max().unwrap_or(0);
            let suffix = format!("im Match Min {max_minutes} ({player_count}/{voice_slots})");
            self.apply(
                channel_id,
                name,
                &base_name,
                Some(suffix),
                Some("match".into()),
                Some(player_count),
                Some(max_minutes),
                cohort.server_id.clone(),
            )
            .await;
        }
    }

    async fn effective_player_count(
        &self,
        members: &[u64],
        steam_map: &HashMap<u64, Vec<String>>,
        cohort_steam_ids: &HashSet<String>,
        raw_count: i64,
        now: i64,
    ) -> i64 {
        if raw_count <= 0 || cohort_steam_ids.is_empty() {
            return raw_count;
        }
        let rows = self
            .store
            .party_rows_for_steam_ids(cohort_steam_ids.iter().cloned().collect(), now)
            .await;
        let Some(party_id) = select_best_party(cohort_steam_ids, &rows) else {
            return raw_count;
        };
        let full_rows = self
            .store
            .party_rows_for_party_ids(vec![party_id], now)
            .await;
        let visible: HashSet<&String> = full_rows.iter().map(|r| &r.steam_id).collect();
        let reported = full_rows
            .iter()
            .filter_map(|r| normalize_party_size(r.party_size))
            .max()
            .unwrap_or(visible.len() as i64);
        let target = reported.max(raw_count).min(6);
        let unlinked = members
            .iter()
            .filter(|m| steam_map.get(m).map(|v| v.is_empty()).unwrap_or(true))
            .count() as i64;
        let inferred = (target - raw_count)
            .max(0)
            .min(unlinked)
            .min((members.len() as i64 - raw_count).max(0));
        (raw_count + inferred).min(6)
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply(
        self: &Arc<Self>,
        channel_id: u64,
        current_name: &str,
        base_name: &str,
        desired_suffix: Option<String>,
        stage_label: Option<String>,
        player_count: Option<i64>,
        minutes_value: Option<i64>,
        server_id: Option<String>,
    ) {
        let _ = server_id;
        let base_clean = base_name.trim_end();
        let target_name = match &desired_suffix {
            Some(suffix) => format!("{base_clean} - {suffix}"),
            None => base_clean.to_string(),
        };
        let (_, current_suffix) = split_suffix(current_name);
        let now = chrono::Utc::now().timestamp() as f64;

        let decision = {
            let states = self.states.lock().await;
            let state = states.get(&channel_id).cloned().unwrap_or_default();
            decide_rename(
                &state,
                current_name,
                &target_name,
                base_clean,
                desired_suffix.as_deref(),
                current_suffix.as_deref(),
                stage_label.as_deref(),
                player_count,
                minutes_value,
                now,
            )
        };

        let mut states = self.states.lock().await;
        let state = states.entry(channel_id).or_default();
        match decision {
            RenameDecision::NoopTargetMatches | RenameDecision::NoopNoMeaningfulChange => {
                state.stage = stage_label;
                state.suffix = desired_suffix;
                state.previous_member_count = player_count;
            }
            RenameDecision::Cooldown => {}
            RenameDecision::Rename => {
                drop(states);
                if let Err(err) = self.port.rename(channel_id, &target_name).await {
                    tracing::warn!(%err, channel_id, "VoiceStatus: Rename fehlgeschlagen");
                }
                let mut states = self.states.lock().await;
                let state = states.entry(channel_id).or_default();
                state.stage = stage_label;
                state.suffix = desired_suffix;
                state.last_rename = now;
                state.previous_member_count = player_count;
            }
        }
    }
}

pub fn spawn(worker: Arc<VoiceStatusWorker>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            worker.tick().await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn row(
        stage: &str,
        minutes: Option<i64>,
        localized: &str,
        updated: Option<i64>,
        seen: Option<i64>,
        strict: bool,
        server: Option<&str>,
        hint: Option<&str>,
    ) -> PresenceRow {
        PresenceRow {
            deadlock_stage: Some(stage.to_string()),
            deadlock_minutes: minutes,
            deadlock_localized: Some(localized.to_string()),
            deadlock_updated_at: updated,
            last_seen_ts: seen,
            in_match_now_strict: strict,
            last_server_id: server.map(str::to_string),
            deadlock_party_hint: hint.map(str::to_string),
        }
    }

    // Referenzwerte aus CPython (service/deadlock_voice_cohort.py)
    #[test]
    fn presence_auswertung_wie_python() {
        let now = 1_000_000;
        let r1 = row(
            "match",
            Some(17),
            "",
            Some(now - 10),
            None,
            true,
            Some("srv1"),
            None,
        );
        assert_eq!(
            evaluate_presence_row(&r1, now, 180),
            Some(("match".into(), Some(17), Some("srv1".into())))
        );
        let r2 = row(
            "",
            None,
            "Spiel (23. min.)",
            None,
            Some(now - 50),
            false,
            None,
            Some("party9"),
        );
        assert_eq!(
            evaluate_presence_row(&r2, now, 180),
            Some(("match".into(), Some(23), Some("party9".into())))
        );
        let r3 = row(
            "",
            None,
            "",
            Some(now - 100),
            None,
            false,
            Some("srv2"),
            None,
        );
        assert_eq!(
            evaluate_presence_row(&r3, now, 180),
            Some(("lobby".into(), None, Some("srv2".into())))
        );
        // stale (500s > 180s)
        let r4 = row(
            "match",
            Some(5),
            "",
            Some(now - 500),
            None,
            false,
            None,
            None,
        );
        assert_eq!(evaluate_presence_row(&r4, now, 180), None);

        let mut map = HashMap::new();
        map.insert("a".to_string(), r1);
        map.insert("b".to_string(), r2);
        map.insert("c".to_string(), r3);
        // b gewinnt: match mit 23 Minuten
        assert_eq!(
            select_best_presence(&["c".into(), "b".into(), "a".into()], &map, now, 180),
            Some(("match".into(), Some(23), Some("party9".into()), "b".into()))
        );
    }

    #[test]
    fn kohorte_wie_python() {
        let entries = vec![
            CohortEntry {
                member_id: 1,
                stage: "match".into(),
                minutes: 10,
                server_id: Some("s1".into()),
            },
            CohortEntry {
                member_id: 2,
                stage: "match".into(),
                minutes: 12,
                server_id: Some("s1".into()),
            },
            CohortEntry {
                member_id: 3,
                stage: "lobby".into(),
                minutes: 0,
                server_id: None,
            },
            CohortEntry {
                member_id: 4,
                stage: "match".into(),
                minutes: 3,
                server_id: Some("s2".into()),
            },
        ];
        let cohort = select_channel_cohort(&entries, 1).expect("cohort");
        assert_eq!(cohort.stage, "match");
        assert_eq!(cohort.server_id.as_deref(), Some("s1"));
        assert_eq!(cohort.member_ids, vec![1, 2]);
        assert_eq!(cohort.minute_values, vec![10, 12]);
    }

    #[test]
    fn suffix_split_wie_python() {
        assert_eq!(
            split_suffix("Lane 1 - im Match Min 17 (4/6)"),
            (
                "Lane 1".to_string(),
                Some("- im Match Min 17 (4/6)".to_string())
            )
        );
        assert_eq!(
            split_suffix("Chill 2 - in der Lobby"),
            ("Chill 2".to_string(), Some("- in der Lobby".to_string()))
        );
        assert_eq!(
            split_suffix("Phantom 3 - in der Lobby (2/6)"),
            (
                "Phantom 3".to_string(),
                Some("- in der Lobby (2/6)".to_string())
            )
        );
        assert_eq!(
            split_suffix("Lane 1 - irgendwas"),
            ("Lane 1 - irgendwas".to_string(), None)
        );
        assert_eq!(split_suffix("Lane 5"), ("Lane 5".to_string(), None));
    }

    #[test]
    fn rename_regeln() {
        let mut state = ChannelState::default();
        // Spielstatus-Wechsel → Rename (Cooldown frisch, last_rename=0, elapsed riesig)
        assert_eq!(
            decide_rename(
                &state,
                "Lane 1",
                "Lane 1 - in der Lobby",
                "Lane 1",
                Some("in der Lobby"),
                None,
                Some("lobby"),
                Some(2),
                None,
                1000.0
            ),
            RenameDecision::Rename
        );
        // Ziel = Ist → Noop
        assert_eq!(
            decide_rename(
                &state,
                "Lane 1 - in der Lobby",
                "Lane 1 - in der Lobby",
                "Lane 1",
                Some("in der Lobby"),
                Some("- in der Lobby"),
                Some("lobby"),
                Some(2),
                None,
                1000.0
            ),
            RenameDecision::NoopTargetMatches
        );
        // Nur Member-Zahl ohne Spielstatus → nie
        state.stage = None;
        state.previous_member_count = Some(2);
        assert_eq!(
            decide_rename(
                &state,
                "Lane 1",
                "Lane 1",
                "Lane 1",
                None,
                None,
                None,
                Some(3),
                None,
                1000.0
            ),
            RenameDecision::NoopTargetMatches
        );
        // Cooldown: Suffix-Änderung 100s nach letztem Rename
        state.stage = Some("match".into());
        state.suffix = Some("im Match Min 5 (4/6)".into());
        state.last_rename = 1000.0;
        assert_eq!(
            decide_rename(
                &state,
                "Lane 1 - im Match Min 5 (4/6)",
                "Lane 1 - im Match Min 10 (4/6)",
                "Lane 1",
                Some("im Match Min 10 (4/6)"),
                Some("- im Match Min 5 (4/6)"),
                Some("match"),
                Some(4),
                Some(10),
                1100.0
            ),
            RenameDecision::Cooldown
        );
        // Match-Ende umgeht Cooldown
        assert_eq!(
            decide_rename(
                &state,
                "Lane 1 - im Match Min 5 (4/6)",
                "Lane 1 - in der Lobby",
                "Lane 1",
                Some("in der Lobby"),
                Some("- im Match Min 5 (4/6)"),
                Some("lobby"),
                Some(4),
                None,
                1100.0
            ),
            RenameDecision::Rename
        );
        // Ab Minute 25: 600s-Cooldown (450s reichen nicht)
        assert_eq!(
            decide_rename(
                &state,
                "Lane 1 - im Match Min 5 (4/6)",
                "Lane 1 - im Match Min 30 (4/6)",
                "Lane 1",
                Some("im Match Min 30 (4/6)"),
                Some("- im Match Min 5 (4/6)"),
                Some("match"),
                Some(4),
                Some(30),
                1450.0
            ),
            RenameDecision::Cooldown
        );
    }

    #[test]
    fn party_auswahl() {
        let cohort: HashSet<String> = ["a".to_string(), "b".to_string()].into();
        let rows = vec![
            PartyRow {
                party_id: "p1".into(),
                steam_id: "a".into(),
                party_size: Some(4),
                seen_at: 100,
            },
            PartyRow {
                party_id: "p1".into(),
                steam_id: "b".into(),
                party_size: Some(4),
                seen_at: 110,
            },
            PartyRow {
                party_id: "p2".into(),
                steam_id: "a".into(),
                party_size: Some(6),
                seen_at: 90,
            },
            PartyRow {
                party_id: "p3".into(),
                steam_id: "x".into(),
                party_size: Some(6),
                seen_at: 200,
            },
        ];
        // p1 gewinnt: 2 Überlappungen schlagen p2 (1) trotz größerer Size; p3 hat keinen Überlapp
        assert_eq!(select_best_party(&cohort, &rows).as_deref(), Some("p1"));
    }
}
