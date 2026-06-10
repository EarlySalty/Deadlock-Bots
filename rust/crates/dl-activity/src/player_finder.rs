//! Player-Finder — Port von `cogs/player_finder.py`.
//!
//! **Per Flag deaktiviert** (`PLAYER_FINDER_ENABLED`, Default AUS) — Nani
//! plant ein Redesign; der Kern ist vertragstreu portiert, damit das
//! Redesign auf Rust aufsetzen kann.
//!
//! Logik: LFG-Nachricht im Kanal → Lane des Suchenden → Kandidaten aus
//! verifizierten Steam-Links, gefiltert nach Zeit/Tag-Muster
//! (`user_activity_patterns`), Voice-Aktivität in der Lane-Kategorie
//! (14 Tage) und Rang ± 3 — sortiert nach Steam-Status (Lobby > Match >
//! im Spiel > Discord online).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use dl_db::Db;

pub const LFG_CHANNEL_ID: u64 = 1376335502919335936;
pub const ACTIVITY_LOOKBACK_DAYS: i64 = 14;
pub const RANK_TOLERANCE: f64 = 3.0;
pub const COOLDOWN_SECONDS: u64 = 60;
pub const MAX_SUGGESTIONS: usize = 5;
pub const PRESENCE_STALE_SECONDS: i64 = 120;

/// ENV-Flag — Default AUS (Redesign geplant).
pub fn enabled(lookup: impl Fn(&str) -> Option<String>) -> bool {
    lookup("PLAYER_FINDER_ENABLED")
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

// ── Pure Filter (wie das Original) ─────────────────────────────────────────

/// Keine Daten = keine Einschränkung; sonst muss die Stunde vorkommen.
pub fn passes_time_filter(typical_hours: &[i64], current_hour: i64) -> bool {
    typical_hours.is_empty() || typical_hours.contains(&current_hour)
}

pub fn passes_day_filter(typical_days: &[i64], current_day: i64) -> bool {
    typical_days.is_empty() || typical_days.contains(&current_day)
}

/// Rang passt bei |Kandidat − Lane-Durchschnitt| ≤ 3 (Unbekannt=0 zählt mit).
pub fn rank_matches(candidate_rank: i64, lane_avg: f64) -> bool {
    (candidate_rank as f64 - lane_avg).abs() <= RANK_TOLERANCE
}

/// Status-Label nach Steam-Presence bzw. Discord-Status (Texte wie Original).
pub fn status_label(steam: Option<(&str, Option<i64>)>, discord_status: &str) -> String {
    if let Some((stage, minutes)) = steam {
        return match stage {
            "lobby" => "🟢 In der Deadlock-Lobby".to_string(),
            "match" => match minutes {
                Some(m) => format!("🎮 Im Match (~{m}min)"),
                None => "🎮 Im Match".to_string(),
            },
            _ => "🟡 Im Spiel".to_string(),
        };
    }
    match discord_status {
        "online" => "💬 Auf Discord online".to_string(),
        "idle" => "🟠 Abwesend".to_string(),
        _ => "⚪ Offline".to_string(),
    }
}

/// Sortierreihenfolge der Vorschläge (kleiner = besser).
pub fn status_sort_key(status: &str) -> u8 {
    if status.starts_with("🟢") {
        0
    } else if status.starts_with("🎮") {
        1
    } else if status.starts_with("🟡") {
        2
    } else if status.starts_with("💬") {
        3
    } else if status.starts_with("🟠") {
        4
    } else {
        5
    }
}

// ── Daten-Zugriffe ─────────────────────────────────────────────────────────

pub struct FinderStore {
    pub db: Db,
}

impl FinderStore {
    /// Verifizierte Steam-Link-User.
    pub async fn verified_user_ids(&self) -> Vec<u64> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT user_id FROM steam_links
                      WHERE steam_id IS NOT NULL AND steam_id != '' AND verified = 1",
                )?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }

    /// (typical_hours, typical_days) — None wenn kein Muster existiert.
    pub async fn activity_pattern(&self, user_id: u64) -> Option<(Vec<i64>, Vec<i64>)> {
        use rusqlite::OptionalExtension;
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT typical_hours, typical_days FROM user_activity_patterns
                      WHERE user_id = ?1",
                    [user_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .map(|(hours, days)| {
                (
                    parse_json_list(hours.as_deref()),
                    parse_json_list(days.as_deref()),
                )
            })
    }

    /// Voice-Aktivität in den Kategorie-Kanälen innerhalb von 14 Tagen?
    pub async fn has_voice_activity(&self, user_id: u64, channel_ids: Vec<u64>) -> bool {
        if channel_ids.is_empty() {
            return false;
        }
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(ACTIVITY_LOOKBACK_DAYS))
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let channels_json = serde_json::to_string(&channel_ids).unwrap_or_default();
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM voice_session_log
                      WHERE started_at >= ?1 AND user_id = ?2
                        AND channel_id IN (SELECT CAST(value AS INTEGER) FROM json_each(?3))",
                    rusqlite::params![cutoff, user_id, channels_json],
                    |row| row.get::<_, i64>(0),
                )
            })
            .await
            .map(|count| count > 0)
            .unwrap_or(false)
    }

    /// Frische Steam-Presence je Discord-User: (stage, minutes).
    pub async fn steam_presence(&self) -> std::collections::HashMap<u64, (String, Option<i64>)> {
        let now = chrono::Utc::now().timestamp();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT l.user_id, p.deadlock_stage, p.deadlock_minutes,
                            COALESCE(p.deadlock_updated_at, p.last_seen_ts) AS fresh
                       FROM steam_links l
                       JOIN live_player_state p ON p.steam_id = l.steam_id
                      WHERE l.verified = 1",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                })?;
                let mut map = std::collections::HashMap::new();
                for row in rows {
                    let (user_id, stage, minutes, fresh) = row?;
                    let Some(stage) = stage.filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    if fresh
                        .map(|f| now - f > PRESENCE_STALE_SECONDS)
                        .unwrap_or(true)
                    {
                        continue;
                    }
                    map.insert(user_id, (stage, minutes));
                }
                Ok(map)
            })
            .await
            .unwrap_or_default()
    }
}

fn parse_json_list(raw: Option<&str>) -> Vec<i64> {
    raw.and_then(|r| serde_json::from_str::<Vec<i64>>(r).ok())
        .unwrap_or_default()
}

// ── Kandidaten-Auswahl ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CandidateInput {
    pub user_id: u64,
    pub rank_value: i64,
    pub in_voice: bool,
    pub is_bot: bool,
    /// "online" | "idle" | sonst
    pub discord_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub user_id: u64,
    pub status: String,
}

pub struct PlayerFinder {
    pub store: FinderStore,
    cooldown: tokio::sync::Mutex<std::collections::HashMap<u64, std::time::Instant>>,
}

impl PlayerFinder {
    pub fn new(db: Db) -> Arc<Self> {
        Arc::new(Self {
            store: FinderStore { db },
            cooldown: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub async fn check_cooldown(&self, user_id: u64) -> bool {
        let mut cooldown = self.cooldown.lock().await;
        let now = std::time::Instant::now();
        if let Some(last) = cooldown.get(&user_id) {
            if now.duration_since(*last) < Duration::from_secs(COOLDOWN_SECONDS) {
                return false;
            }
        }
        cooldown.insert(user_id, now);
        true
    }

    /// Kandidaten für eine Lane: Filterkette wie das Original
    /// (verlinkt+verifiziert, nicht in der Lane, nicht im Voice, Rang ± 3,
    /// Zeit/Tag-Muster, 14-Tage-Aktivität, mindestens EIN Lebenszeichen).
    pub async fn candidates_for_lane(
        &self,
        lane_member_ids: &HashSet<u64>,
        lane_avg_rank: f64,
        category_channel_ids: Vec<u64>,
        members: impl Fn(u64) -> Option<CandidateInput>,
    ) -> Vec<Suggestion> {
        let now = chrono::Utc::now();
        let current_hour = chrono::Timelike::hour(&now) as i64;
        let current_day = chrono::Datelike::weekday(&now).num_days_from_monday() as i64;
        let presence = self.store.steam_presence().await;

        let mut suggestions: Vec<Suggestion> = Vec::new();
        for user_id in self.store.verified_user_ids().await {
            if lane_member_ids.contains(&user_id) {
                continue;
            }
            let Some(candidate) = members(user_id) else {
                continue;
            };
            if candidate.is_bot || candidate.in_voice {
                continue;
            }
            if !rank_matches(candidate.rank_value, lane_avg_rank) {
                continue;
            }
            let pattern = self.store.activity_pattern(user_id).await;
            if let Some((hours, days)) = &pattern {
                if !passes_time_filter(hours, current_hour) || !passes_day_filter(days, current_day)
                {
                    continue;
                }
            }
            if !category_channel_ids.is_empty()
                && !self
                    .store
                    .has_voice_activity(user_id, category_channel_ids.clone())
                    .await
            {
                continue;
            }
            let steam = presence.get(&user_id);
            let has_discord_online = matches!(candidate.discord_status.as_str(), "online" | "idle");
            if steam.is_none() && !has_discord_online && pattern.is_none() {
                continue; // kein einziges Lebenszeichen
            }
            let status = status_label(
                steam.map(|(stage, minutes)| (stage.as_str(), *minutes)),
                &candidate.discord_status,
            );
            suggestions.push(Suggestion { user_id, status });
        }
        suggestions.sort_by_key(|s| status_sort_key(&s.status));
        suggestions.truncate(MAX_SUGGESTIONS);
        suggestions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_wie_python() {
        assert!(passes_time_filter(&[], 14)); // keine Daten = keine Einschränkung
        assert!(passes_time_filter(&[13, 14, 15], 14));
        assert!(!passes_time_filter(&[8, 9], 14));
        assert!(passes_day_filter(&[], 2));
        assert!(passes_day_filter(&[0, 2], 2));
        assert!(!passes_day_filter(&[5, 6], 2));
        assert!(rank_matches(9, 7.0)); // |9-7| = 2 ≤ 3
        assert!(!rank_matches(11, 7.0)); // 4 > 3
    }

    #[test]
    fn status_labels_und_sortierung() {
        assert_eq!(
            status_label(Some(("lobby", None)), "offline"),
            "🟢 In der Deadlock-Lobby"
        );
        assert_eq!(
            status_label(Some(("match", Some(17))), "offline"),
            "🎮 Im Match (~17min)"
        );
        assert_eq!(status_label(Some(("menu", None)), "offline"), "🟡 Im Spiel");
        assert_eq!(status_label(None, "online"), "💬 Auf Discord online");
        assert_eq!(status_label(None, "idle"), "🟠 Abwesend");
        assert_eq!(status_label(None, "dnd"), "⚪ Offline");
        // Sortierung: Lobby vor Match vor Spiel vor Online
        let mut statuses = [
            "💬 Auf Discord online".to_string(),
            "🎮 Im Match".to_string(),
            "🟢 In der Deadlock-Lobby".to_string(),
        ];
        statuses.sort_by_key(|s| status_sort_key(s));
        assert!(statuses[0].starts_with("🟢"));
        assert!(statuses[1].starts_with("🎮"));
    }

    #[test]
    fn flag_default_aus() {
        assert!(!enabled(|_| None));
        assert!(!enabled(|_| Some("0".to_string())));
        assert!(enabled(|_| Some("1".to_string())));
        assert!(enabled(|_| Some("true".to_string())));
    }
}
