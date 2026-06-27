//! TempVoice-Persistenz über die Bestands-Tabellen (Verträge unverändert).

use dl_db::{Db, DbError};
use rusqlite::OptionalExtension;

/// Tag-Filter einer Lane (Original: LaneTagFilter). `required_tone_tag`
/// wird wie im Original gespeichert, aber NICHT durchgesetzt —
/// `_member_block_reason` prüft nur min_age und ragebaiter (toter Zweig).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaneTagFilter {
    pub min_age_tag: Option<String>,
    pub required_tone_tag: Option<String>,
    pub deny_ragebaiter: bool,
}

impl LaneTagFilter {
    pub fn is_enabled(&self) -> bool {
        self.min_age_tag.is_some() || self.required_tone_tag.is_some() || self.deny_ragebaiter
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneRecord {
    pub channel_id: u64,
    pub guild_id: u64,
    pub owner_id: u64,
    pub initial_owner_id: Option<u64>,
    pub base_name: String,
    pub category_id: u64,
    pub source_staging_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PresetRecord {
    pub user_id: u64,
    pub category_id: u64,
    pub name: String,
    pub base_name: String,
    pub limit: i64,
    pub min_rank: String,
    pub region: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceRecord {
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub category_id: Option<u64>,
    pub lane_id: Option<u64>,
}

pub struct TempVoiceStore {
    pub db: Db,
}

impl TempVoiceStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Insert/Update wie das Original (initial_owner/source bleiben erhalten).
    pub async fn upsert_lane(&self, lane: LaneRecord) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO tempvoice_lanes(
                       channel_id, guild_id, owner_id, initial_owner_id,
                       base_name, category_id, source_staging_id
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7)
                     ON CONFLICT(channel_id) DO UPDATE SET
                       owner_id = excluded.owner_id,
                       initial_owner_id = COALESCE(tempvoice_lanes.initial_owner_id, excluded.initial_owner_id),
                       base_name = excluded.base_name,
                       category_id = excluded.category_id,
                       source_staging_id = COALESCE(tempvoice_lanes.source_staging_id, excluded.source_staging_id)",
                    rusqlite::params![
                        lane.channel_id,
                        lane.guild_id,
                        lane.owner_id,
                        lane.initial_owner_id,
                        lane.base_name,
                        lane.category_id,
                        lane.source_staging_id,
                    ],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn set_owner(&self, channel_id: u64, owner_id: u64) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE tempvoice_lanes SET owner_id = ?1 WHERE channel_id = ?2",
                    rusqlite::params![owner_id, channel_id],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn delete_lane(&self, channel_id: u64) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM tempvoice_lanes WHERE channel_id = ?1",
                    [channel_id],
                )
                .map(|_| ())
            })
            .await
    }

    /// Alle Lanes (für die Rehydrierung beim Start).
    pub async fn all_lanes(&self) -> Result<Vec<LaneRecord>, DbError> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT channel_id, guild_id, owner_id, initial_owner_id,
                            base_name, category_id, source_staging_id
                       FROM tempvoice_lanes",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(LaneRecord {
                        channel_id: row.get(0)?,
                        guild_id: row.get(1)?,
                        owner_id: row.get(2)?,
                        initial_owner_id: row.get(3)?,
                        base_name: row.get(4)?,
                        category_id: row.get(5)?,
                        source_staging_id: row.get(6)?,
                    })
                })?;
                rows.collect()
            })
            .await
    }

    // ── Lane-Tag-Filter (tempvoice_lane_tag_filter) ─────────────────────────

    pub async fn lane_tag_filter(&self, channel_id: u64) -> LaneTagFilter {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT min_age_tag, required_tone_tag, deny_ragebaiter
                       FROM tempvoice_lane_tag_filter WHERE channel_id = ?1",
                    [channel_id],
                    |row| {
                        Ok(LaneTagFilter {
                            min_age_tag: row.get(0)?,
                            required_tone_tag: row.get(1)?,
                            deny_ragebaiter: row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                        })
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub async fn set_lane_tag_filter(
        &self,
        channel_id: u64,
        filter: LaneTagFilter,
    ) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO tempvoice_lane_tag_filter(
                       channel_id, min_age_tag, required_tone_tag, deny_ragebaiter, updated_at
                     ) VALUES(?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)
                     ON CONFLICT(channel_id) DO UPDATE SET
                       min_age_tag = excluded.min_age_tag,
                       required_tone_tag = excluded.required_tone_tag,
                       deny_ragebaiter = excluded.deny_ragebaiter,
                       updated_at = CURRENT_TIMESTAMP",
                    rusqlite::params![
                        channel_id,
                        filter.min_age_tag,
                        filter.required_tone_tag,
                        i64::from(filter.deny_ragebaiter),
                    ],
                )
                .map(|_| ())
            })
            .await
    }

    // ── Bans (owner-gebunden, lane-übergreifend) ───────────────────────────

    pub async fn add_ban(&self, owner_id: u64, banned_id: u64) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO tempvoice_bans(owner_id, banned_id) VALUES(?1, ?2)",
                    rusqlite::params![owner_id, banned_id],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn remove_ban(&self, owner_id: u64, banned_id: u64) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM tempvoice_bans WHERE owner_id = ?1 AND banned_id = ?2",
                    rusqlite::params![owner_id, banned_id],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn is_banned_by_owner(&self, owner_id: u64, user_id: u64) -> Result<bool, DbError> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT 1 FROM tempvoice_bans WHERE owner_id = ?1 AND banned_id = ?2",
                    rusqlite::params![owner_id, user_id],
                    |_| Ok(()),
                )
                .optional()
            })
            .await
            .map(|found| found.is_some())
    }

    pub async fn list_bans(&self, owner_id: u64) -> Result<Vec<u64>, DbError> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT banned_id FROM tempvoice_bans WHERE owner_id = ?1 ORDER BY banned_id",
                )?;
                let rows = stmt.query_map([owner_id], |row| row.get(0))?;
                rows.collect()
            })
            .await
    }

    // ── Lurker (tempvoice_lurkers) ──────────────────────────────────────────

    pub async fn add_lurker(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
        original_nick: Option<String>,
    ) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO tempvoice_lurkers(guild_id, channel_id, user_id, original_nick)
                     VALUES(?1, ?2, ?3, ?4)
                     ON CONFLICT(channel_id, user_id) DO UPDATE SET
                       original_nick = excluded.original_nick",
                    rusqlite::params![guild_id, channel_id, user_id, original_nick],
                )
                .map(|_| ())
            })
            .await
    }

    /// Some(original_nick) wenn der User Lurker in diesem Kanal ist.
    pub async fn get_lurker(&self, channel_id: u64, user_id: u64) -> Option<Option<String>> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT original_nick FROM tempvoice_lurkers
                      WHERE channel_id = ?1 AND user_id = ?2",
                    rusqlite::params![channel_id, user_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    pub async fn remove_lurker(&self, channel_id: u64, user_id: u64) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM tempvoice_lurkers WHERE channel_id = ?1 AND user_id = ?2",
                    rusqlite::params![channel_id, user_id],
                )
                .map(|_| ())
            })
            .await
    }

    // ── Region-Präferenz (DE/EU) ───────────────────────────────────────────

    pub async fn region_pref(&self, owner_id: u64) -> Result<String, DbError> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT region FROM tempvoice_owner_prefs WHERE owner_id = ?1",
                    [owner_id],
                    |row| row.get(0),
                )
                .optional()
            })
            .await
            .map(|region: Option<String>| region.unwrap_or_else(|| "EU".to_string()))
    }

    pub async fn set_region_pref(&self, owner_id: u64, region: &str) -> Result<(), DbError> {
        let region = if region == "DE" { "DE" } else { "EU" }.to_string();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO tempvoice_owner_prefs(owner_id, region, updated_at)
                     VALUES(?1, ?2, CURRENT_TIMESTAMP)
                     ON CONFLICT(owner_id) DO UPDATE SET
                       region = excluded.region, updated_at = CURRENT_TIMESTAMP",
                    rusqlite::params![owner_id, region],
                )
                .map(|_| ())
            })
            .await
    }

    // ── Rang-Präferenz (für prefix_from_rank-Lanes) ────────────────────────

    pub async fn rank_pref(&self, user_id: u64) -> Result<(String, i64), DbError> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT rank, subrank FROM tempvoice_rank_pref WHERE user_id = ?1",
                    [user_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
            })
            .await
            .map(|pref| pref.unwrap_or_else(|| ("unknown".to_string(), 0)))
    }

    pub async fn set_rank_pref(
        &self,
        user_id: u64,
        rank: &str,
        subrank: i64,
    ) -> Result<(), DbError> {
        let rank = rank.to_lowercase();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO tempvoice_rank_pref(user_id, rank, subrank)
                     VALUES(?1, ?2, ?3)
                     ON CONFLICT(user_id) DO UPDATE SET
                       rank = excluded.rank, subrank = excluded.subrank",
                    rusqlite::params![user_id, rank, subrank],
                )
                .map(|_| ())
            })
            .await
    }

    // ── Presets (pro User + Kategorie) ─────────────────────────────────────

    pub async fn save_preset(&self, preset: PresetRecord) -> Result<(), DbError> {
        let PresetRecord {
            user_id,
            category_id,
            name,
            base_name,
            limit,
            min_rank,
            region,
        } = preset;
        let region = if region == "DE" { "DE" } else { "EU" }.to_string();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO tempvoice_presets(user_id, category_id, name, base_name, \"limit\", min_rank, region, updated_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,CURRENT_TIMESTAMP)
                     ON CONFLICT(user_id, category_id, name) DO UPDATE SET
                       base_name = excluded.base_name,
                       \"limit\" = excluded.\"limit\",
                       min_rank = excluded.min_rank,
                       region = excluded.region,
                       updated_at = CURRENT_TIMESTAMP",
                    rusqlite::params![user_id, category_id, name, base_name, limit, min_rank, region],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn list_presets(
        &self,
        user_id: u64,
        category_id: u64,
    ) -> Result<Vec<String>, DbError> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT name FROM tempvoice_presets
                      WHERE user_id = ?1 AND category_id = ?2 ORDER BY name",
                )?;
                let rows =
                    stmt.query_map(rusqlite::params![user_id, category_id], |row| row.get(0))?;
                rows.collect()
            })
            .await
    }

    pub async fn get_preset(
        &self,
        user_id: u64,
        category_id: u64,
        name: &str,
    ) -> Result<Option<(String, i64, String, String)>, DbError> {
        let name = name.to_string();
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT base_name, \"limit\", min_rank, region FROM tempvoice_presets
                      WHERE user_id = ?1 AND category_id = ?2 AND name = ?3",
                    rusqlite::params![user_id, category_id, name],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
            })
            .await
    }

    // ── Persistente Interface-Messages (tempvoice_interface) ───────────────

    pub async fn ensure_interface_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                let rows = {
                    let mut stmt = conn.prepare("PRAGMA table_info(tempvoice_interface)")?;
                    let rows = stmt.query_map([], |row| {
                        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
                    })?;
                    rows.collect::<Result<Vec<_>, _>>()?
                };

                if rows.is_empty() {
                    conn.execute_batch(INTERFACE_DDL)?;
                    return Ok(());
                }

                let col_names: std::collections::HashSet<&str> =
                    rows.iter().map(|(name, _)| name.as_str()).collect();
                let pk_cols: Vec<&str> = rows
                    .iter()
                    .filter(|(_, pk)| *pk > 0)
                    .map(|(name, _)| name.as_str())
                    .collect();
                let required = [
                    "guild_id",
                    "channel_id",
                    "message_id",
                    "category_id",
                    "lane_id",
                    "created_at",
                    "updated_at",
                ];
                if required.iter().all(|name| col_names.contains(name))
                    && pk_cols == ["guild_id", "message_id"]
                {
                    return Ok(());
                }

                let expr = |name: &str, fallback: &str| {
                    if col_names.contains(name) {
                        name.to_string()
                    } else {
                        fallback.to_string()
                    }
                };
                let select = format!(
                    "INSERT OR IGNORE INTO tempvoice_interface(
                       guild_id, channel_id, message_id, category_id, lane_id, created_at, updated_at
                     )
                     SELECT {}, {}, {}, {}, {}, {}, {}
                       FROM tempvoice_interface_old",
                    expr("guild_id", "0"),
                    expr("channel_id", "0"),
                    expr("message_id", "0"),
                    expr("category_id", "NULL"),
                    expr("lane_id", "NULL"),
                    expr("created_at", "CURRENT_TIMESTAMP"),
                    expr("updated_at", "CURRENT_TIMESTAMP"),
                );
                let tx = conn.transaction()?;
                tx.execute_batch(
                    "ALTER TABLE tempvoice_interface RENAME TO tempvoice_interface_old;",
                )?;
                tx.execute_batch(INTERFACE_DDL)?;
                tx.execute(&select, [])?;
                tx.execute_batch("DROP TABLE tempvoice_interface_old;")?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    pub async fn record_interface_message(&self, record: InterfaceRecord) -> Result<(), DbError> {
        self.ensure_interface_schema().await?;
        self.db
            .write(move |conn| {
                if let Some(lane_id) = record.lane_id {
                    conn.execute(
                        "INSERT INTO tempvoice_interface(
                           guild_id, channel_id, message_id, category_id, lane_id, updated_at
                         ) VALUES(?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
                         ON CONFLICT(lane_id) DO UPDATE SET
                           channel_id = excluded.channel_id,
                           message_id = excluded.message_id,
                           category_id = excluded.category_id,
                           updated_at = CURRENT_TIMESTAMP",
                        rusqlite::params![
                            record.guild_id,
                            record.channel_id,
                            record.message_id,
                            record.category_id,
                            lane_id,
                        ],
                    )
                } else {
                    conn.execute(
                        "INSERT INTO tempvoice_interface(
                           guild_id, channel_id, message_id, category_id, lane_id, updated_at
                         ) VALUES(?1, ?2, ?3, ?4, NULL, CURRENT_TIMESTAMP)
                         ON CONFLICT(guild_id, message_id) DO UPDATE SET
                           channel_id = excluded.channel_id,
                           category_id = excluded.category_id,
                           updated_at = CURRENT_TIMESTAMP",
                        rusqlite::params![
                            record.guild_id,
                            record.channel_id,
                            record.message_id,
                            record.category_id,
                        ],
                    )
                }
                .map(|_| ())
            })
            .await
    }

    pub async fn global_interface_messages(&self) -> Result<Vec<InterfaceRecord>, DbError> {
        self.ensure_interface_schema().await?;
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT guild_id, channel_id, message_id, category_id, lane_id
                       FROM tempvoice_interface
                      WHERE lane_id IS NULL
                      ORDER BY guild_id, message_id",
                )?;
                let rows = stmt.query_map([], interface_record_from_row)?;
                rows.collect()
            })
            .await
    }

    pub async fn lane_interface_messages(&self) -> Result<Vec<InterfaceRecord>, DbError> {
        self.ensure_interface_schema().await?;
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT guild_id, channel_id, message_id, category_id, lane_id
                       FROM tempvoice_interface
                      WHERE lane_id IS NOT NULL
                      ORDER BY lane_id",
                )?;
                let rows = stmt.query_map([], interface_record_from_row)?;
                rows.collect()
            })
            .await
    }

    pub async fn remove_lane_interface_record(&self, lane_id: u64) -> Result<(), DbError> {
        self.ensure_interface_schema().await?;
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM tempvoice_interface WHERE lane_id = ?1",
                    [lane_id],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn remove_interface_record(
        &self,
        guild_id: u64,
        message_id: u64,
    ) -> Result<(), DbError> {
        self.ensure_interface_schema().await?;
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM tempvoice_interface WHERE guild_id = ?1 AND message_id = ?2",
                    rusqlite::params![guild_id, message_id],
                )
                .map(|_| ())
            })
            .await
    }
}

const INTERFACE_DDL: &str = "
CREATE TABLE IF NOT EXISTS tempvoice_interface (
    guild_id    INTEGER NOT NULL,
    channel_id  INTEGER NOT NULL,
    message_id  INTEGER NOT NULL,
    category_id INTEGER,
    lane_id     INTEGER,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (guild_id, message_id),
    UNIQUE(lane_id)
);";

fn interface_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InterfaceRecord> {
    Ok(InterfaceRecord {
        guild_id: row.get(0)?,
        channel_id: row.get(1)?,
        message_id: row.get(2)?,
        category_id: row.get(3)?,
        lane_id: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDLS: [&str; 5] = [
        "CREATE TABLE tempvoice_lanes (channel_id INTEGER PRIMARY KEY, guild_id INTEGER NOT NULL, owner_id INTEGER NOT NULL, base_name TEXT NOT NULL, category_id INTEGER NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, source_staging_id INTEGER, initial_owner_id INTEGER)",
        "CREATE TABLE tempvoice_bans (owner_id BIGINT NOT NULL, banned_id BIGINT NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, PRIMARY KEY (owner_id, banned_id))",
        "CREATE TABLE tempvoice_owner_prefs (owner_id INTEGER PRIMARY KEY, region TEXT NOT NULL CHECK(region IN ('DE','EU')), updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP)",
        "CREATE TABLE tempvoice_rank_pref (user_id INTEGER PRIMARY KEY, rank TEXT NOT NULL, subrank INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE tempvoice_presets (user_id INTEGER NOT NULL, category_id INTEGER NOT NULL, name TEXT NOT NULL, base_name TEXT NOT NULL, \"limit\" INTEGER NOT NULL, min_rank TEXT NOT NULL DEFAULT 'unknown', region TEXT NOT NULL DEFAULT 'EU' CHECK(region IN ('DE','EU')), created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, PRIMARY KEY (user_id, category_id, name))",
    ];

    async fn store() -> (tempfile::TempDir, TempVoiceStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        (dir, TempVoiceStore::new(db))
    }

    #[tokio::test]
    async fn lane_upsert_erhaelt_initial_owner() {
        let (_dir, store) = store().await;
        let lane = LaneRecord {
            channel_id: 1,
            guild_id: 9,
            owner_id: 100,
            initial_owner_id: Some(100),
            base_name: "Lane 1".into(),
            category_id: 5,
            source_staging_id: Some(7),
        };
        store.upsert_lane(lane.clone()).await.expect("insert");

        // Owner-Wechsel per Upsert: initial_owner und Staging bleiben
        let mut transferred = lane.clone();
        transferred.owner_id = 200;
        transferred.initial_owner_id = Some(200);
        transferred.source_staging_id = None;
        store.upsert_lane(transferred).await.expect("update");

        let lanes = store.all_lanes().await.expect("lanes");
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].owner_id, 200);
        assert_eq!(lanes[0].initial_owner_id, Some(100));
        assert_eq!(lanes[0].source_staging_id, Some(7));

        store.delete_lane(1).await.expect("delete");
        assert!(store.all_lanes().await.expect("leer").is_empty());
    }

    #[tokio::test]
    async fn bans_und_prefs() {
        let (_dir, store) = store().await;
        store.add_ban(100, 666).await.expect("ban");
        store.add_ban(100, 666).await.expect("idempotent");
        assert!(store.is_banned_by_owner(100, 666).await.expect("check"));
        assert!(!store.is_banned_by_owner(100, 777).await.expect("check"));
        assert_eq!(store.list_bans(100).await.expect("list"), vec![666]);
        store.remove_ban(100, 666).await.expect("unban");
        assert!(!store.is_banned_by_owner(100, 666).await.expect("check"));

        assert_eq!(store.region_pref(100).await.expect("default"), "EU");
        store.set_region_pref(100, "DE").await.expect("set");
        assert_eq!(store.region_pref(100).await.expect("de"), "DE");
        store
            .set_region_pref(100, "quatsch")
            .await
            .expect("fallback");
        assert_eq!(store.region_pref(100).await.expect("eu"), "EU");

        assert_eq!(
            store.rank_pref(100).await.expect("default"),
            ("unknown".to_string(), 0)
        );
        store.set_rank_pref(100, "Phantom", 3).await.expect("set");
        assert_eq!(
            store.rank_pref(100).await.expect("gesetzt"),
            ("phantom".to_string(), 3)
        );
    }

    #[tokio::test]
    async fn presets_roundtrip() {
        let (_dir, store) = store().await;
        let preset = |name: &str, base: &str, limit, rank: &str, region: &str| PresetRecord {
            user_id: 100,
            category_id: 5,
            name: name.into(),
            base_name: base.into(),
            limit,
            min_rank: rank.into(),
            region: region.into(),
        };
        store
            .save_preset(preset("tryhard", "Lane 1", 6, "phantom", "DE"))
            .await
            .expect("save");
        store
            .save_preset(preset("chill", "Chill 1", 8, "unknown", "EU"))
            .await
            .expect("save2");
        assert_eq!(
            store.list_presets(100, 5).await.expect("list"),
            vec!["chill".to_string(), "tryhard".to_string()]
        );
        let preset = store
            .get_preset(100, 5, "tryhard")
            .await
            .expect("get")
            .expect("existiert");
        assert_eq!(
            preset,
            (
                "Lane 1".to_string(),
                6,
                "phantom".to_string(),
                "DE".to_string()
            )
        );
        assert!(store
            .get_preset(100, 5, "fehlt")
            .await
            .expect("none")
            .is_none());
    }

    #[tokio::test]
    async fn interface_messages_persistieren_global_und_lane_gebunden() {
        let (_dir, store) = store().await;
        store.ensure_interface_schema().await.expect("schema");

        store
            .record_interface_message(InterfaceRecord {
                guild_id: 9,
                channel_id: 20,
                message_id: 300,
                category_id: Some(40),
                lane_id: None,
            })
            .await
            .expect("global insert");
        store
            .record_interface_message(InterfaceRecord {
                guild_id: 9,
                channel_id: 21,
                message_id: 301,
                category_id: Some(41),
                lane_id: Some(9001),
            })
            .await
            .expect("lane insert");

        let global = store.global_interface_messages().await.expect("global");
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].channel_id, 20);
        assert_eq!(global[0].message_id, 300);
        assert_eq!(global[0].category_id, Some(40));
        assert_eq!(global[0].lane_id, None);

        let lane = store.lane_interface_messages().await.expect("lane");
        assert_eq!(lane.len(), 1);
        assert_eq!(lane[0].channel_id, 21);
        assert_eq!(lane[0].message_id, 301);
        assert_eq!(lane[0].category_id, Some(41));
        assert_eq!(lane[0].lane_id, Some(9001));

        store
            .record_interface_message(InterfaceRecord {
                guild_id: 9,
                channel_id: 22,
                message_id: 302,
                category_id: Some(42),
                lane_id: Some(9001),
            })
            .await
            .expect("lane upsert");

        let lane = store.lane_interface_messages().await.expect("lane update");
        assert_eq!(lane.len(), 1);
        assert_eq!(lane[0].channel_id, 22);
        assert_eq!(lane[0].message_id, 302);
        assert_eq!(lane[0].category_id, Some(42));

        store
            .remove_lane_interface_record(9001)
            .await
            .expect("lane delete");
        assert!(store
            .lane_interface_messages()
            .await
            .expect("lane empty")
            .is_empty());

        store
            .remove_interface_record(9, 300)
            .await
            .expect("global delete");
        assert!(store
            .global_interface_messages()
            .await
            .expect("global empty")
            .is_empty());
    }
}
