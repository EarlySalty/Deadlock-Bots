//! Rang-Konstanten und -Lookup (public_stats.py).
//!
//! Bewusste Aufräumung gegenüber dem Original: Die Co-Player-Heuristik
//! (`_estimate_rank_from_co_players`) lieferte `low/mid/high`-Buckets,
//! die anschließend an JEDER Verwendungsstelle durch `if rank in RANK_ORDER`-
//! Checks wieder herausgefiltert wurden — sie war wirkungslos (stiller Bug).
//! Der Port lässt sie weg; das Verhalten ist identisch.

use std::collections::HashMap;

use sqlx::PgPool;

pub const RANK_ORDER: [&str; 11] = [
    "initiate",
    "seeker",
    "acolyte",
    "sentinel",
    "mystic",
    "ritualist",
    "emissary",
    "oracle",
    "phantom",
    "ascendant",
    "eternus",
];

pub const RANK_COLORS: [(&str, &str); 11] = [
    ("initiate", "#6A3E1E"),
    ("seeker", "#882355"),
    ("acolyte", "#5C6DAB"),
    ("sentinel", "#719C47"),
    ("mystic", "#DDA326"),
    ("ritualist", "#EE4F57"),
    ("emissary", "#B47FEB"),
    ("oracle", "#955138"),
    ("phantom", "#7C7C7C"),
    ("ascendant", "#C39751"),
    ("eternus", "#5CE9A9"),
];

/// Rank-Lookup mit Request-lokalem Cache. Das Original feuert dieselbe
/// Query pro Session-Zeile neu (N+1) — der Cache ist verhaltensgleich,
/// nur ohne die redundanten Roundtrips.
pub struct RankResolver<'c> {
    pool: &'c PgPool,
    cache: HashMap<i64, Option<&'static str>>,
}

impl<'c> RankResolver<'c> {
    pub fn new(pool: &'c PgPool) -> Self {
        Self {
            pool,
            cache: HashMap::new(),
        }
    }

    /// `_get_user_rank`: verifizierter Steam-Link, primärer Account zuerst,
    /// Name muss in RANK_ORDER liegen.
    pub async fn rank_of(&mut self, user_id: i64) -> Result<Option<&'static str>, sqlx::Error> {
        if let Some(cached) = self.cache.get(&user_id) {
            return Ok(*cached);
        }
        let raw = sqlx::query!(
            r#"
            SELECT deadlock_rank_name
            FROM core.steam_links
            WHERE discord_id = $1
              AND verified = TRUE
              AND deadlock_rank_name IS NOT NULL
            ORDER BY primary_account DESC, deadlock_rank_updated_at DESC NULLS LAST
            LIMIT 1
            "#,
            user_id,
        )
        .fetch_optional(self.pool)
        .await?
        .and_then(|row| row.deadlock_rank_name);
        let resolved = raw
            .map(|s| s.to_lowercase())
            .map(|name| match name.as_str() {
                "alchemist" => "acolyte".to_string(),
                "arcanist" => "sentinel".to_string(),
                "archon" => "emissary".to_string(),
                _ => name,
            })
            .and_then(|name| RANK_ORDER.iter().find(|r| *r == &name).copied());
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
