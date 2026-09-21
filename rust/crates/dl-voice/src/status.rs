//! Deadlock-Voice-Status — Port von `cogs/deadlock_voice_status.py` +
//! `service/deadlock_voice_cohort.py`.
//!
//! Trackt Voice-Status aus `live_player_state` (Steam-Presence-Worker) und
//! `deadlock_party_members`. Kanalnamen werden nicht mehr automatisch geändert.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use dl_discord::{ChannelSender, Dispatcher};
use sqlx::PgPool;

use crate::db::{i64_to_u64, u64_to_i64, unix_to_utc};

pub const POLL_INTERVAL: Duration = Duration::from_secs(60);
pub const PRESENCE_STALE_SECONDS: i64 = 180;
pub const PARTY_MEMBER_STALE_SECONDS: i64 = 600;
pub const RENAME_COOLDOWN_SECONDS: f64 = 360.0;
pub const LATE_MATCH_COOLDOWN_SECONDS: f64 = 600.0;
pub const MIN_ACTIVE_PLAYERS: usize = 1;
pub const DEFAULT_MATCH_MINUTE_DISPLAY_OFFSET: i64 = 3;
pub const LOCALIZED_SLOTS_CACHE_SECONDS: i64 = 60 * 60;
pub const DLVS_ROOT_REPLY: &str = "🎙️ Voice-Status — Übersicht";
pub const DLVS_TRACE_REPLY: &str = "🎙️ Voice-Status — Trace";
pub const DLVS_SNAPSHOT_REPLY: &str = "🎙️ Voice-Status — Snapshot";

/// Überwachte Kategorien (VOICE_STATUS_CATEGORY_* = TempVoice-Kategorien).
pub const TARGET_CATEGORY_IDS: [u64; 3] = [
    1289721245281292290,
    1412804540994162789,
    1357422957017698478,
];
/// Permanenter Chill-Voice — Status wird hier aktiv entfernt.
pub const EXCLUDED_CHANNEL_IDS: [u64; 1] = [1493690350580138114];
pub const OFF_TOPIC_CHANNEL_ID: u64 = crate::adaptive::DUO_ANCHOR_CHANNEL_ID;
pub const VOICE_STATUS_ROUTER_ANCHOR: &str = "voice_status_router_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceStatusRoute {
    Deadlock,
    OffTopic,
    Unknown,
}

pub fn contains_deadlock_lobby_code(status: &str) -> bool {
    status
        .split(|c: char| !c.is_ascii_digit())
        .any(|part| part.len() == 5)
}

fn status_words(status: &str) -> Vec<String> {
    status
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

pub fn classify_voice_channel_status(status: Option<&str>) -> VoiceStatusRoute {
    let Some(status) = status.map(str::trim).filter(|status| !status.is_empty()) else {
        return VoiceStatusRoute::Unknown;
    };
    let words = status_words(status);
    let war_dogs = words.iter().any(|word| word == "wardogs")
        || words
            .windows(2)
            .any(|pair| pair[0] == "war" && pair[1] == "dogs");
    if war_dogs {
        return VoiceStatusRoute::OffTopic;
    }
    if words.iter().any(|word| word == "deadlock") || contains_deadlock_lobby_code(status) {
        return VoiceStatusRoute::Deadlock;
    }
    VoiceStatusRoute::Unknown
}

pub fn match_minute_display_offset() -> i64 {
    // NOTE(tempvoice-blocking-rework): config parity for this offset is deferred;
    // the fixed Python-default value is not a prod-risk for cutover.
    DEFAULT_MATCH_MINUTE_DISPLAY_OFFSET
}

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
                if after.trim_start().starts_with(')') {
                    return digits.parse().ok();
                }
            }
        }
        i = start;
    }
    None
}

fn parse_localized_slots(localized: &str) -> Option<i64> {
    let mut best = None;
    let mut start_from = 0;
    while let Some(open_offset) = localized[start_from..].find('(') {
        let open = start_from + open_offset + 1;
        let Some(close_offset) = localized[open..].find(')') else {
            break;
        };
        let close = open + close_offset;
        if let Some((left, right)) = localized[open..close].split_once('/') {
            if let (Ok(current), Ok(total)) =
                (left.trim().parse::<i64>(), right.trim().parse::<i64>())
            {
                if current >= 0 && total > 0 {
                    best = Some(total.max(current));
                }
            }
        }
        start_from = close + 1;
    }
    best
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
    pub pool: PgPool,
}

impl StatusStore {
    /// user_id → Steam-IDs, sortiert primary > verified > frisch.
    pub async fn steam_ids_for(&self, user_ids: Vec<u64>) -> HashMap<u64, Vec<String>> {
        if user_ids.is_empty() {
            return HashMap::new();
        }
        let ids: Vec<i64> = match user_ids
            .into_iter()
            .map(|id| u64_to_i64("core.steam_links.discord_id", id))
            .collect()
        {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(%err, "VoiceStatus: User-ID-Liste ungueltig");
                return HashMap::new();
            }
        };
        let rows = match sqlx::query!(
            r#"
            SELECT discord_id, steam_id
              FROM core.steam_links
             WHERE discord_id = ANY($1)
               AND steam_id != ''
             ORDER BY primary_account DESC, verified DESC, updated_at DESC
            "#,
            &ids,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "VoiceStatus: Steam-IDs konnten nicht geladen werden");
                return HashMap::new();
            }
        };

        let mut map: HashMap<u64, Vec<String>> = HashMap::new();
        for row in rows {
            let Ok(uid) = i64_to_u64("core.steam_links.discord_id", row.discord_id) else {
                continue;
            };
            let bucket = map.entry(uid).or_default();
            if !bucket.contains(&row.steam_id) {
                bucket.push(row.steam_id);
            }
        }
        map
    }

    pub async fn presence_rows(&self, steam_ids: Vec<String>) -> HashMap<String, PresenceRow> {
        if steam_ids.is_empty() {
            return HashMap::new();
        }
        let rows = match sqlx::query!(
            r#"
            SELECT steam_id, deadlock_stage, deadlock_minutes, deadlock_localized,
                   deadlock_updated_at, last_seen_at, in_match_now_strict,
                   last_server_id, deadlock_party_hint
              FROM activity.live_player_state
             WHERE steam_id = ANY($1)
            "#,
            &steam_ids,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "VoiceStatus: Presence-Zeilen konnten nicht geladen werden");
                return HashMap::new();
            }
        };

        rows.into_iter()
            .map(|row| {
                (
                    row.steam_id,
                    PresenceRow {
                        deadlock_stage: row.deadlock_stage,
                        deadlock_minutes: row.deadlock_minutes.map(i64::from),
                        deadlock_localized: row.deadlock_localized,
                        deadlock_updated_at: row.deadlock_updated_at.map(|ts| ts.timestamp()),
                        last_seen_ts: row.last_seen_at.map(|ts| ts.timestamp()),
                        in_match_now_strict: row.in_match_now_strict.unwrap_or(false),
                        last_server_id: row.last_server_id,
                        deadlock_party_hint: row.deadlock_party_hint,
                    },
                )
            })
            .collect()
    }

    pub async fn party_rows_for_steam_ids(
        &self,
        steam_ids: Vec<String>,
        now: i64,
    ) -> Vec<PartyRow> {
        if steam_ids.is_empty() {
            return Vec::new();
        }
        let cutoff = match unix_to_utc(
            "deadlock_party_members.seen_at",
            now - PARTY_MEMBER_STALE_SECONDS,
        ) {
            Ok(cutoff) => cutoff,
            Err(err) => {
                tracing::warn!(%err, "VoiceStatus: Party-Cutoff ungueltig");
                return Vec::new();
            }
        };
        let rows = match sqlx::query!(
            r#"
            SELECT party_id, steam_id, party_size, seen_at
              FROM voice.deadlock_party_members
             WHERE steam_id = ANY($1)
               AND seen_at >= $2
            "#,
            &steam_ids,
            cutoff,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "VoiceStatus: Party-Zeilen konnten nicht geladen werden");
                return Vec::new();
            }
        };

        rows.into_iter()
            .map(|row| PartyRow {
                party_id: row.party_id,
                steam_id: row.steam_id,
                party_size: row.party_size.map(i64::from),
                seen_at: row.seen_at.timestamp(),
            })
            .collect()
    }

    pub async fn party_rows_for_party_ids(
        &self,
        party_ids: Vec<String>,
        now: i64,
    ) -> Vec<PartyRow> {
        if party_ids.is_empty() {
            return Vec::new();
        }
        let cutoff = match unix_to_utc(
            "deadlock_party_members.seen_at",
            now - PARTY_MEMBER_STALE_SECONDS,
        ) {
            Ok(cutoff) => cutoff,
            Err(err) => {
                tracing::warn!(%err, "VoiceStatus: Party-Cutoff ungueltig");
                return Vec::new();
            }
        };
        let rows = match sqlx::query!(
            r#"
            SELECT party_id, steam_id, party_size, seen_at
              FROM voice.deadlock_party_members
             WHERE party_id = ANY($1)
               AND seen_at >= $2
            "#,
            &party_ids,
            cutoff,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "VoiceStatus: Party-Zeilen konnten nicht geladen werden");
                return Vec::new();
            }
        };

        rows.into_iter()
            .map(|row| PartyRow {
                party_id: row.party_id,
                steam_id: row.steam_id,
                party_size: row.party_size.map(i64::from),
                seen_at: row.seen_at.timestamp(),
            })
            .collect()
    }

    /// `deadlock_voice_watch` synchronisieren (Voice-Standorte je Steam-ID).
    pub async fn persist_voice_watch(&self, entries: Vec<(String, u64, u64)>, now: i64) {
        let result = async {
            let updated_at = unix_to_utc("deadlock_voice_watch.updated_at", now)?;
            if entries.is_empty() {
                sqlx::query!("DELETE FROM voice.deadlock_voice_watch")
                    .execute(&self.pool)
                    .await?;
                return Ok::<(), crate::db::VoiceDbError>(());
            }
            let mut tx = self.pool.begin().await?;
            for (steam_id, guild_id, channel_id) in &entries {
                let guild_id = u64_to_i64("deadlock_voice_watch.guild_id", *guild_id)?;
                let channel_id = u64_to_i64("deadlock_voice_watch.channel_id", *channel_id)?;
                sqlx::query!(
                    r#"
                    INSERT INTO voice.deadlock_voice_watch (
                        steam_id, guild_id, channel_id, updated_at
                    )
                    VALUES ($1, $2, $3, $4)
                    ON CONFLICT (steam_id) DO UPDATE SET
                        guild_id = EXCLUDED.guild_id,
                        channel_id = EXCLUDED.channel_id,
                        updated_at = EXCLUDED.updated_at
                    "#,
                    steam_id,
                    guild_id,
                    channel_id,
                    updated_at,
                )
                .execute(&mut *tx)
                .await?;
            }
            let keep: Vec<String> = entries
                .iter()
                .map(|(steam_id, _, _)| steam_id.clone())
                .collect();
            sqlx::query!(
                r#"
                DELETE FROM voice.deadlock_voice_watch
                 WHERE NOT (steam_id = ANY($1))
                "#,
                &keep,
            )
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok::<(), crate::db::VoiceDbError>(())
        }
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
    async fn channel_status(&self, channel_id: u64) -> Option<String>;
    async fn move_member(&self, guild_id: u64, user_id: u64, channel_id: u64)
        -> Result<(), String>;
    async fn resolved_base_name(
        &self,
        _guild_id: u64,
        _channel_id: u64,
        _fallback_base: &str,
    ) -> Option<String> {
        None
    }
    async fn rename(&self, channel_id: u64, name: &str) -> Result<(), String>;
}

pub struct VoiceStatusWorker {
    pub store: StatusStore,
    pub port: Arc<dyn StatusPort>,
    states: tokio::sync::Mutex<HashMap<u64, ChannelState>>,
    localized_slots_cache: tokio::sync::Mutex<HashMap<u64, (i64, i64)>>,
}

impl VoiceStatusWorker {
    pub fn new(pool: PgPool, port: Arc<dyn StatusPort>) -> Arc<Self> {
        Arc::new(Self {
            store: StatusStore { pool },
            port,
            states: tokio::sync::Mutex::new(HashMap::new()),
            localized_slots_cache: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    async fn move_members_to_off_topic(
        &self,
        guild_id: u64,
        source_channel_id: u64,
        members: &[u64],
    ) {
        if source_channel_id == OFF_TOPIC_CHANNEL_ID {
            return;
        }
        for user_id in members {
            match self
                .port
                .move_member(guild_id, *user_id, OFF_TOPIC_CHANNEL_ID)
                .await
            {
                Ok(()) => tracing::info!(
                    anchor = VOICE_STATUS_ROUTER_ANCHOR,
                    guild_id,
                    source_channel_id,
                    target_channel_id = OFF_TOPIC_CHANNEL_ID,
                    user_id,
                    "VoiceStatusRouting moved member to off topic"
                ),
                Err(err) => tracing::warn!(
                    %err,
                    guild_id,
                    source_channel_id,
                    target_channel_id = OFF_TOPIC_CHANNEL_ID,
                    user_id,
                    "VoiceStatusRouting could not move member"
                ),
            }
        }
    }

    async fn monitored_members(&self, guild_id: u64, channel_id: u64) -> Option<Vec<u64>> {
        self.port
            .monitored_channels()
            .await
            .into_iter()
            .find(|(candidate_guild, candidate_channel, _, _)| {
                *candidate_guild == guild_id && *candidate_channel == channel_id
            })
            .map(|(_, _, _, members)| members)
    }

    pub async fn route_status_update(&self, guild_id: u64, channel_id: u64, status: Option<&str>) {
        if channel_id == OFF_TOPIC_CHANNEL_ID
            || classify_voice_channel_status(status) != VoiceStatusRoute::OffTopic
        {
            return;
        }
        let Some(members) = self.monitored_members(guild_id, channel_id).await else {
            return;
        };
        self.move_members_to_off_topic(guild_id, channel_id, &members)
            .await;
    }

    pub async fn route_voice_member(&self, guild_id: u64, user_id: u64, channel_id: u64) {
        if channel_id == OFF_TOPIC_CHANNEL_ID {
            return;
        }
        let Some(members) = self.monitored_members(guild_id, channel_id).await else {
            return;
        };
        if !members.contains(&user_id) {
            return;
        }
        let status = self.port.channel_status(channel_id).await;
        if classify_voice_channel_status(status.as_deref()) != VoiceStatusRoute::OffTopic {
            return;
        }
        self.move_members_to_off_topic(guild_id, channel_id, &[user_id])
            .await;
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
        let mut deadlock_channels = Vec::with_capacity(channels.len());
        for channel in channels {
            let status = self.port.channel_status(channel.1).await;
            if channel.1 != OFF_TOPIC_CHANNEL_ID
                && classify_voice_channel_status(status.as_deref()) == VoiceStatusRoute::OffTopic
            {
                self.move_members_to_off_topic(channel.0, channel.1, &channel.3)
                    .await;
                continue;
            }
            deadlock_channels.push(channel);
        }
        let channels = deadlock_channels;
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
            self.process_channel(
                *guild_id,
                *channel_id,
                name,
                members,
                &steam_map,
                &presence_map,
                now,
            )
            .await;
        }
        self.store
            .persist_voice_watch(watch.into_values().collect(), now)
            .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_channel(
        self: &Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        name: &str,
        members: &[u64],
        steam_map: &HashMap<u64, Vec<String>>,
        presence_map: &HashMap<String, PresenceRow>,
        now: i64,
    ) {
        let (fallback_base, _) = split_suffix(name);
        let base_name = self
            .port
            .resolved_base_name(guild_id, channel_id, &fallback_base)
            .await
            .unwrap_or(fallback_base);
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
        let localized_slots = self
            .localized_voice_slots(channel_id, &cohort_steam_ids, presence_map, now)
            .await;
        let voice_slots = localized_slots
            .map(|slots| slots.max(player_count))
            .unwrap_or_else(|| (members.len() as i64).max(player_count));

        if cohort.stage == "lobby" {
            let suffix = if localized_slots.is_some_and(|slots| slots > player_count) {
                format!("in der Lobby ({player_count}/{voice_slots})")
            } else {
                "in der Lobby".to_string()
            };
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
            let display_minutes = max_minutes + match_minute_display_offset();
            let suffix = format!("im Match Min {display_minutes} ({player_count}/{voice_slots})");
            self.apply(
                channel_id,
                name,
                &base_name,
                Some(suffix),
                Some("match".into()),
                Some(player_count),
                Some(display_minutes),
                cohort.server_id.clone(),
            )
            .await;
        }
    }

    async fn localized_voice_slots(
        &self,
        channel_id: u64,
        cohort_steam_ids: &HashSet<String>,
        presence_map: &HashMap<String, PresenceRow>,
        now: i64,
    ) -> Option<i64> {
        let parsed = cohort_steam_ids
            .iter()
            .filter_map(|steam_id| presence_map.get(steam_id))
            .filter_map(|row| row.deadlock_localized.as_deref())
            .filter_map(parse_localized_slots)
            .max();
        if let Some(slots) = parsed {
            self.localized_slots_cache
                .lock()
                .await
                .insert(channel_id, (slots, now + LOCALIZED_SLOTS_CACHE_SECONDS));
            return Some(slots);
        }
        let mut cache = self.localized_slots_cache.lock().await;
        match cache.get(&channel_id).copied() {
            Some((slots, expires_at)) if expires_at >= now => Some(slots),
            Some(_) => {
                cache.remove(&channel_id);
                None
            }
            None => None,
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
        let _ = (current_name, base_name, minutes_value, server_id);
        let mut states = self.states.lock().await;
        let state = states.entry(channel_id).or_default();
        state.stage = stage_label;
        state.suffix = desired_suffix;
        state.previous_member_count = player_count;
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

pub fn spawn_routing(
    worker: Arc<VoiceStatusWorker>,
    dispatcher: &Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut channel_events = dispatcher.subscribe_channels();
    let mut voice_events = dispatcher.subscribe_voice();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                event = channel_events.recv() => match event {
                    Ok(dl_discord::ChannelEvent::VoiceChannelStatusUpdated {
                        guild_id,
                        channel_id,
                        status,
                        ..
                    }) => {
                        worker
                            .route_status_update(guild_id, channel_id, status.as_deref())
                            .await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "VoiceStatusRouting channel events lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                event = voice_events.recv() => match event {
                    Ok(dl_discord::VoiceEvent::Join {
                        guild_id,
                        user_id,
                        channel_id,
                    }) => {
                        worker.route_voice_member(guild_id, user_id, channel_id).await;
                    }
                    Ok(dl_discord::VoiceEvent::Move {
                        guild_id,
                        user_id,
                        to_channel_id,
                        ..
                    }) => {
                        worker
                            .route_voice_member(guild_id, user_id, to_channel_id)
                            .await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "VoiceStatusRouting voice events lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    })
}

pub struct StatusCommands {
    worker: Arc<VoiceStatusWorker>,
}

impl StatusCommands {
    pub fn new(worker: Arc<VoiceStatusWorker>) -> Arc<Self> {
        Arc::new(Self { worker })
    }

    async fn reply_for(&self, content: &str) -> Option<String> {
        let mut parts = content.split_whitespace();
        let root = parts.next()?.to_lowercase();
        if root != "dlvs" && root != "!dlvs" {
            return None;
        }
        match parts.next().unwrap_or_default().to_lowercase().as_str() {
            "trace" => Some(self.trace_reply().await),
            "snapshot" => Some(self.snapshot_reply(parts.next()).await),
            _ => Some(self.root_reply().await),
        }
    }

    async fn root_reply(&self) -> String {
        let state_count = self.worker.states.lock().await.len();
        format!("{DLVS_ROOT_REPLY}\nstate_count={state_count}")
    }

    async fn trace_reply(&self) -> String {
        let states = self.state_rows(None).await;
        let mut reply = format!("{DLVS_TRACE_REPLY}\nstate_count={}", states.len());
        for row in states {
            let _ = write!(reply, "\n{row}");
        }
        reply
    }

    async fn snapshot_reply(&self, channel_arg: Option<&str>) -> String {
        let channel_id = channel_arg.and_then(parse_channel_id);
        let states = self.state_rows(channel_id).await;
        let mut reply = format!("{DLVS_SNAPSHOT_REPLY}\nstate_count={}", states.len());
        for row in states {
            let _ = write!(reply, "\n{row}");
        }
        reply
    }

    async fn state_rows(&self, channel_id: Option<u64>) -> Vec<String> {
        let mut states: Vec<(u64, ChannelState)> = self
            .worker
            .states
            .lock()
            .await
            .iter()
            .filter(|(id, _)| channel_id.is_none_or(|target| target == **id))
            .map(|(id, state)| (*id, state.clone()))
            .collect();
        states.sort_by_key(|(id, _)| *id);
        states
            .into_iter()
            .map(|(id, state)| {
                format!(
                    "channel_id={id} stage={} suffix={} players={} last_rename={:.0}",
                    state.stage.unwrap_or_else(|| "-".to_string()),
                    state.suffix.unwrap_or_else(|| "-".to_string()),
                    state
                        .previous_member_count
                        .map(|count| count.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    state.last_rename,
                )
            })
            .collect()
    }
}

fn parse_channel_id(raw: &str) -> Option<u64> {
    raw.trim()
        .strip_prefix("<#")
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or_else(|| raw.trim())
        .parse()
        .ok()
}

pub fn spawn_command(
    commands: Arc<StatusCommands>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    if !event.author_can_manage_guild {
                        continue;
                    }
                    let Some(reply) = commands.reply_for(event.content.trim()).await else {
                        continue;
                    };
                    let _ = sender
                        .send_to_channel(event.channel_id, Some(&reply), &[])
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
    use std::sync::Mutex as StdMutex;

    use sqlx::postgres::PgPoolOptions;

    #[derive(Default)]
    struct MockStatusPort {
        renamed: StdMutex<Vec<(u64, String)>>,
        monitored: StdMutex<Vec<(u64, u64, String, Vec<u64>)>>,
        statuses: StdMutex<HashMap<u64, String>>,
        moved: StdMutex<Vec<(u64, u64, u64)>>,
    }

    #[async_trait::async_trait]
    impl StatusPort for MockStatusPort {
        async fn monitored_channels(&self) -> Vec<(u64, u64, String, Vec<u64>)> {
            self.monitored.lock().expect("lock").clone()
        }

        async fn channel_info(&self, _channel_id: u64) -> Option<(u64, String, Vec<u64>)> {
            None
        }

        async fn channel_status(&self, channel_id: u64) -> Option<String> {
            self.statuses
                .lock()
                .expect("lock")
                .get(&channel_id)
                .cloned()
        }

        async fn move_member(
            &self,
            guild_id: u64,
            user_id: u64,
            channel_id: u64,
        ) -> Result<(), String> {
            self.moved
                .lock()
                .expect("lock")
                .push((guild_id, user_id, channel_id));
            Ok(())
        }

        async fn rename(&self, channel_id: u64, name: &str) -> Result<(), String> {
            self.renamed
                .lock()
                .expect("lock")
                .push((channel_id, name.to_string()));
            Ok(())
        }
    }

    fn lazy_pool() -> PgPool {
        PgPoolOptions::new()
            .connect_lazy("postgres://voice-status-test.invalid/deadlock")
            .expect("lazy pg pool")
    }

    #[test]
    fn voice_status_routing_erkennt_war_dogs_und_deadlock_codes() {
        assert_eq!(
            classify_voice_channel_status(Some("War Dogs")),
            VoiceStatusRoute::OffTopic
        );
        assert_eq!(
            classify_voice_channel_status(Some("heute WAR-DOGS zocken")),
            VoiceStatusRoute::OffTopic
        );
        assert_eq!(
            classify_voice_channel_status(Some("Lobby 12345 EU")),
            VoiceStatusRoute::Deadlock
        );
        assert_eq!(
            classify_voice_channel_status(Some("Deadlock Ranked")),
            VoiceStatusRoute::Deadlock
        );
        assert_eq!(
            classify_voice_channel_status(Some("Lobby 123456")),
            VoiceStatusRoute::Unknown
        );
        assert_eq!(
            classify_voice_channel_status(Some("War Dogs 12345")),
            VoiceStatusRoute::OffTopic
        );
    }

    #[tokio::test]
    async fn war_dogs_status_verschiebt_mitglieder_in_off_topic() {
        let port = Arc::new(MockStatusPort::default());
        port.monitored.lock().expect("lock").push((
            1289721245281292288,
            1555000000000000001,
            "Chill Lane".to_string(),
            vec![11, 22],
        ));
        let worker = VoiceStatusWorker::new(lazy_pool(), port.clone());

        worker
            .route_status_update(1289721245281292288, 1555000000000000001, Some("War Dogs"))
            .await;

        assert_eq!(
            *port.moved.lock().expect("lock"),
            vec![
                (1289721245281292288, 11, OFF_TOPIC_CHANNEL_ID),
                (1289721245281292288, 22, OFF_TOPIC_CHANNEL_ID),
            ]
        );
    }

    #[tokio::test]
    async fn war_dogs_status_verschiebt_neu_beigetretenes_mitglied() {
        let port = Arc::new(MockStatusPort::default());
        port.monitored.lock().expect("lock").push((
            1289721245281292288,
            1555000000000000001,
            "Chill Lane".to_string(),
            vec![11, 22],
        ));
        port.statuses
            .lock()
            .expect("lock")
            .insert(1555000000000000001, "War Dogs".to_string());
        let worker = VoiceStatusWorker::new(lazy_pool(), port.clone());

        worker
            .route_voice_member(1289721245281292288, 22, 1555000000000000001)
            .await;

        assert_eq!(
            *port.moved.lock().expect("lock"),
            vec![(1289721245281292288, 22, OFF_TOPIC_CHANNEL_ID)]
        );
    }

    #[tokio::test]
    async fn fuenfstelliger_lobby_code_bleibt_in_deadlock_lane() {
        let port = Arc::new(MockStatusPort::default());
        port.monitored.lock().expect("lock").push((
            1289721245281292288,
            1555000000000000001,
            "Chill Lane".to_string(),
            vec![11],
        ));
        port.statuses
            .lock()
            .expect("lock")
            .insert(1555000000000000001, "Lobby 54321".to_string());
        let worker = VoiceStatusWorker::new(lazy_pool(), port.clone());

        worker
            .route_voice_member(1289721245281292288, 11, 1555000000000000001)
            .await;

        assert!(port.moved.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn unbekannter_status_loest_keinen_move_aus() {
        let port = Arc::new(MockStatusPort::default());
        port.monitored.lock().expect("lock").push((
            1289721245281292288,
            1555000000000000001,
            "Chill Lane".to_string(),
            vec![11],
        ));
        port.statuses
            .lock()
            .expect("lock")
            .insert(1555000000000000001, "Abends entspannt".to_string());
        let worker = VoiceStatusWorker::new(lazy_pool(), port.clone());

        worker
            .route_voice_member(1289721245281292288, 11, 1555000000000000001)
            .await;

        assert!(port.moved.lock().expect("lock").is_empty());
    }

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
    fn lokalisierte_minuten_und_slots_sind_tolerant() {
        assert_eq!(parse_localized_minutes("Spiel (23 min )"), Some(23));
        assert_eq!(parse_localized_minutes("Spiel (23. min. )"), Some(23));
        assert_eq!(parse_localized_slots("Lobby (4/6)"), Some(6));
        assert_eq!(parse_localized_slots("Spiel (23 min ) (5/6)"), Some(6));
        assert_eq!(match_minute_display_offset(), 3);
    }

    #[test]
    fn dlvs_texte_sind_final() {
        for text in [DLVS_ROOT_REPLY, DLVS_TRACE_REPLY, DLVS_SNAPSHOT_REPLY] {
            assert_ne!(text, "Platzhalter");
            assert!(!text.is_empty());
        }
    }

    #[tokio::test]
    async fn status_worker_aendert_keine_channelnamen_mehr() {
        let port = Arc::new(MockStatusPort::default());
        let worker = Arc::new(VoiceStatusWorker {
            store: StatusStore { pool: lazy_pool() },
            port: port.clone(),
            states: tokio::sync::Mutex::new(HashMap::new()),
            localized_slots_cache: tokio::sync::Mutex::new(HashMap::new()),
        });

        worker
            .apply(
                42,
                "Chill Lane 1",
                "Chill Lane 1",
                Some("in der Lobby".to_string()),
                Some("lobby".to_string()),
                Some(2),
                None,
                None,
            )
            .await;

        assert!(port.renamed.lock().expect("lock").is_empty());
        let states = worker.states.lock().await;
        assert_eq!(
            states.get(&42).and_then(|s| s.stage.as_deref()),
            Some("lobby")
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

    #[tokio::test]
    async fn status_store_any_arrays_und_watch_pruning() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let store = StatusStore { pool: pool.clone() };
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::seconds(PARTY_MEMBER_STALE_SECONDS + 30);

        sqlx::query!(
            r#"
            INSERT INTO core.users (discord_id)
            VALUES (100), (200)
            "#
        )
        .execute(&pool)
        .await
        .expect("users");
        sqlx::query!(
            r#"
            INSERT INTO core.steam_links (
                discord_id, steam_id, steam_display_name, verified, primary_account, updated_at
            )
            VALUES
                (100, 'steam-a', 'A', TRUE, TRUE, $1),
                (100, 'steam-b', 'B', TRUE, FALSE, $1),
                (200, 'steam-c', 'C', FALSE, FALSE, $1)
            "#,
            now,
        )
        .execute(&pool)
        .await
        .expect("steam links");
        sqlx::query!(
            r#"
            INSERT INTO activity.live_player_state (
                steam_id, deadlock_stage, deadlock_minutes, deadlock_localized,
                deadlock_updated_at, last_seen_at, in_match_now_strict,
                last_server_id, deadlock_party_hint
            )
            VALUES
                ('steam-a', 'match', 12, 'Spiel (12. min.)', $1, $1, TRUE, 'srv-1', NULL),
                ('steam-c', 'lobby', NULL, 'Lobby (2/6)', $1, $1, FALSE, 'srv-2', NULL)
            "#,
            now,
        )
        .execute(&pool)
        .await
        .expect("presence");
        sqlx::query!(
            r#"
            INSERT INTO voice.deadlock_party_members (party_id, steam_id, party_size, seen_at)
            VALUES
                ('party-1', 'steam-a', 4, $1),
                ('party-1', 'steam-b', 4, $1),
                ('party-old', 'steam-a', 6, $2),
                ('party-2', 'steam-c', 2, $1)
            "#,
            now,
            old,
        )
        .execute(&pool)
        .await
        .expect("party rows");

        assert!(store.steam_ids_for(Vec::new()).await.is_empty());
        let steam = store.steam_ids_for(vec![100, 200, 999]).await;
        assert_eq!(
            steam.get(&100).cloned().unwrap_or_default(),
            vec!["steam-a".to_string(), "steam-b".to_string()]
        );
        assert_eq!(
            steam.get(&200).cloned().unwrap_or_default(),
            vec!["steam-c".to_string()]
        );

        let presence = store
            .presence_rows(vec!["steam-a".to_string(), "missing".to_string()])
            .await;
        assert_eq!(presence.len(), 1);
        assert_eq!(presence["steam-a"].deadlock_minutes, Some(12));
        assert!(presence["steam-a"].in_match_now_strict);

        let party_rows = store
            .party_rows_for_steam_ids(
                vec!["steam-a".to_string(), "steam-b".to_string()],
                now.timestamp(),
            )
            .await;
        assert_eq!(party_rows.len(), 2);
        assert!(party_rows.iter().all(|row| row.party_id == "party-1"));

        let full_party = store
            .party_rows_for_party_ids(vec!["party-1".to_string()], now.timestamp())
            .await;
        assert_eq!(full_party.len(), 2);

        store
            .persist_voice_watch(
                vec![
                    ("steam-a".to_string(), 1, 10),
                    ("steam-c".to_string(), 1, 20),
                ],
                now.timestamp(),
            )
            .await;
        store
            .persist_voice_watch(vec![("steam-a".to_string(), 2, 30)], now.timestamp())
            .await;
        let rows = sqlx::query!(
            r#"
            SELECT steam_id, guild_id, channel_id
              FROM voice.deadlock_voice_watch
             ORDER BY steam_id
            "#
        )
        .fetch_all(&pool)
        .await
        .expect("watch rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].steam_id, "steam-a");
        assert_eq!(rows[0].guild_id, Some(2));
        assert_eq!(rows[0].channel_id, Some(30));

        store.persist_voice_watch(Vec::new(), now.timestamp()).await;
        let count =
            sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM voice.deadlock_voice_watch"#)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count, 0);
    }
}
