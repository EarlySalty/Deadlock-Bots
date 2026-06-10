//! Rang-Konstanten und -Lookup (public_stats.py).
//!
//! Bewusste Aufräumung gegenüber dem Original: Die Co-Player-Heuristik
//! (`_estimate_rank_from_co_players`) lieferte `low/mid/high`-Buckets,
//! die anschließend an JEDER Verwendungsstelle durch `if rank in RANK_ORDER`-
//! Checks wieder herausgefiltert wurden — sie war wirkungslos (stiller Bug).
//! Der Port lässt sie weg; das Verhalten ist identisch.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

pub const RANK_ORDER: [&str; 11] = [
    "initiate",
    "seeker",
    "alchemist",
    "arcanist",
    "ritualist",
    "emissary",
    "archon",
    "oracle",
    "phantom",
    "ascendant",
    "eternus",
];

pub const RANK_COLORS: [(&str, &str); 11] = [
    ("initiate", "#8fa4b4"),
    ("seeker", "#72aa5a"),
    ("alchemist", "#3dbb44"),
    ("arcanist", "#18bba8"),
    ("ritualist", "#2288ee"),
    ("emissary", "#5055ee"),
    ("archon", "#8833dd"),
    ("oracle", "#cc33bb"),
    ("phantom", "#dd3344"),
    ("ascendant", "#ee9922"),
    ("eternus", "#f5cc11"),
];

/// Rank-Lookup mit Request-lokalem Cache. Das Original feuert dieselbe
/// Query pro Session-Zeile neu (N+1) — der Cache ist verhaltensgleich,
/// nur ohne die redundanten Roundtrips.
pub struct RankResolver<'c> {
    conn: &'c Connection,
    cache: HashMap<i64, Option<&'static str>>,
}

impl<'c> RankResolver<'c> {
    pub fn new(conn: &'c Connection) -> Self {
        Self {
            conn,
            cache: HashMap::new(),
        }
    }

    /// `_get_user_rank`: verifizierter Steam-Link, primärer Account zuerst,
    /// Name muss in RANK_ORDER liegen.
    pub fn rank_of(&mut self, user_id: i64) -> rusqlite::Result<Option<&'static str>> {
        if let Some(cached) = self.cache.get(&user_id) {
            return Ok(*cached);
        }
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT deadlock_rank_name FROM steam_links
                  WHERE user_id = ?1 AND verified = 1
                  ORDER BY primary_account DESC, deadlock_rank_updated_at DESC
                  LIMIT 1",
                [user_id],
                |row| row.get(0),
            )
            .optional()?;
        let resolved = raw
            .map(|s| s.to_lowercase())
            .and_then(|name| RANK_ORDER.iter().find(|r| **r == name).copied());
        self.cache.insert(user_id, resolved);
        Ok(resolved)
    }
}

/// `_detect_lane_from_name` — Reihenfolge der Checks ist Vertrag
/// ("off" matcht z. B. auch "Off-Lane", neue Checks erst danach).
pub fn detect_lane(name: Option<&str>) -> Option<&'static str> {
    let n = name?.to_lowercase();
    if n.contains("mid") {
        Some("mid")
    } else if n.contains("off") {
        Some("off")
    } else if n.contains("safe") || n.contains("carry") {
        Some("safe")
    } else if n.contains("jungle") || n.contains("jg") {
        Some("jungle")
    } else if n.contains("new") || n.contains("neue") || n.contains("🆕") {
        Some("new_player")
    } else {
        None
    }
}

pub const LANES: [&str; 6] = ["mid", "off", "safe", "jungle", "new_player", "unknown"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_erkennung() {
        assert_eq!(detect_lane(Some("Midlane #2")), Some("mid"));
        assert_eq!(detect_lane(Some("Off-Lane")), Some("off"));
        assert_eq!(detect_lane(Some("Carry Duo")), Some("safe"));
        assert_eq!(detect_lane(Some("🆕 Anfänger")), Some("new_player"));
        assert_eq!(detect_lane(Some("Lobby 3")), None);
        assert_eq!(detect_lane(None), None);
    }
}
