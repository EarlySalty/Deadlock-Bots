//! TempVoice-Persistenz über die Bestands-Tabellen (Verträge unverändert).

use sqlx::PgPool;

use crate::db::{
    i64_to_i32, i64_to_u64, opt_i64_to_u64, opt_u64_to_i64, u64_to_i64, VoiceDbResult,
};

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
    pub pool: PgPool,
}

impl TempVoiceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert/Update wie das Original (initial_owner/source bleiben erhalten).
    pub async fn upsert_lane(&self, lane: LaneRecord) -> VoiceDbResult<()> {
        let channel_id = u64_to_i64("tempvoice_lanes.channel_id", lane.channel_id)?;
        let guild_id = u64_to_i64("tempvoice_lanes.guild_id", lane.guild_id)?;
        let owner_id = u64_to_i64("tempvoice_lanes.owner_id", lane.owner_id)?;
        let initial_owner_id =
            opt_u64_to_i64("tempvoice_lanes.initial_owner_id", lane.initial_owner_id)?;
        let category_id = u64_to_i64("tempvoice_lanes.category_id", lane.category_id)?;
        let source_staging_id =
            opt_u64_to_i64("tempvoice_lanes.source_staging_id", lane.source_staging_id)?;

        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_lanes (
                channel_id, guild_id, owner_id, initial_owner_id,
                base_name, category_id, source_staging_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (channel_id) DO UPDATE SET
                owner_id = EXCLUDED.owner_id,
                initial_owner_id = COALESCE(
                    tempvoice_lanes.initial_owner_id,
                    EXCLUDED.initial_owner_id
                ),
                base_name = EXCLUDED.base_name,
                category_id = EXCLUDED.category_id,
                source_staging_id = COALESCE(
                    tempvoice_lanes.source_staging_id,
                    EXCLUDED.source_staging_id
                )
            "#,
            channel_id,
            guild_id,
            owner_id,
            initial_owner_id,
            lane.base_name,
            category_id,
            source_staging_id,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn set_owner(&self, channel_id: u64, owner_id: u64) -> VoiceDbResult<()> {
        let channel_id = u64_to_i64("tempvoice_lanes.channel_id", channel_id)?;
        let owner_id = u64_to_i64("tempvoice_lanes.owner_id", owner_id)?;
        sqlx::query!(
            r#"
            UPDATE voice.tempvoice_lanes
               SET owner_id = $1
             WHERE channel_id = $2
            "#,
            owner_id,
            channel_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_lane_base(&self, channel_id: u64, base_name: String) -> VoiceDbResult<()> {
        let channel_id = u64_to_i64("tempvoice_lanes.channel_id", channel_id)?;
        sqlx::query!(
            r#"
            UPDATE voice.tempvoice_lanes
               SET base_name = $1
             WHERE channel_id = $2
            "#,
            base_name,
            channel_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_lane_category_source(
        &self,
        channel_id: u64,
        category_id: u64,
        source_staging_id: Option<u64>,
    ) -> VoiceDbResult<()> {
        let channel_id = u64_to_i64("tempvoice_lanes.channel_id", channel_id)?;
        let category_id = u64_to_i64("tempvoice_lanes.category_id", category_id)?;
        let source_staging_id =
            opt_u64_to_i64("tempvoice_lanes.source_staging_id", source_staging_id)?;
        sqlx::query!(
            r#"
            UPDATE voice.tempvoice_lanes
               SET category_id = $1,
                   source_staging_id = $2
             WHERE channel_id = $3
            "#,
            category_id,
            source_staging_id,
            channel_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_lane(&self, channel_id: u64) -> VoiceDbResult<()> {
        let channel_id = u64_to_i64("tempvoice_lanes.channel_id", channel_id)?;
        sqlx::query!(
            r#"
            DELETE FROM voice.tempvoice_lanes
             WHERE channel_id = $1
            "#,
            channel_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Alle Lanes (für die Rehydrierung beim Start).
    pub async fn all_lanes(&self) -> VoiceDbResult<Vec<LaneRecord>> {
        let rows = sqlx::query!(
            r#"
            SELECT channel_id, guild_id, owner_id, initial_owner_id,
                   base_name, category_id, source_staging_id
              FROM voice.tempvoice_lanes
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(LaneRecord {
                    channel_id: i64_to_u64("tempvoice_lanes.channel_id", row.channel_id)?,
                    guild_id: i64_to_u64("tempvoice_lanes.guild_id", row.guild_id)?,
                    owner_id: i64_to_u64("tempvoice_lanes.owner_id", row.owner_id)?,
                    initial_owner_id: opt_i64_to_u64(
                        "tempvoice_lanes.initial_owner_id",
                        row.initial_owner_id,
                    )?,
                    base_name: row.base_name,
                    category_id: i64_to_u64("tempvoice_lanes.category_id", row.category_id)?,
                    source_staging_id: opt_i64_to_u64(
                        "tempvoice_lanes.source_staging_id",
                        row.source_staging_id,
                    )?,
                })
            })
            .collect()
    }

    // ── Lane-Tag-Filter (tempvoice_lane_tag_filter) ─────────────────────────

    pub async fn lane_tag_filter(&self, channel_id: u64) -> LaneTagFilter {
        let Ok(channel_id) = u64_to_i64("tempvoice_lane_tag_filter.channel_id", channel_id) else {
            return LaneTagFilter::default();
        };
        sqlx::query!(
            r#"
            SELECT min_age_tag, required_tone_tag,
                   COALESCE(deny_ragebaiter, FALSE) AS "deny_ragebaiter!"
              FROM voice.tempvoice_lane_tag_filter
             WHERE channel_id = $1
            "#,
            channel_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| LaneTagFilter {
            min_age_tag: row.min_age_tag,
            required_tone_tag: row.required_tone_tag,
            deny_ragebaiter: row.deny_ragebaiter,
        })
        .unwrap_or_default()
    }

    pub async fn set_lane_tag_filter(
        &self,
        channel_id: u64,
        filter: LaneTagFilter,
    ) -> VoiceDbResult<()> {
        let channel_id = u64_to_i64("tempvoice_lane_tag_filter.channel_id", channel_id)?;
        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_lane_tag_filter (
                channel_id, min_age_tag, required_tone_tag, deny_ragebaiter, updated_at
            )
            VALUES ($1, $2, $3, $4, NOW())
            ON CONFLICT (channel_id) DO UPDATE SET
                min_age_tag = EXCLUDED.min_age_tag,
                required_tone_tag = EXCLUDED.required_tone_tag,
                deny_ragebaiter = EXCLUDED.deny_ragebaiter,
                updated_at = NOW()
            "#,
            channel_id,
            filter.min_age_tag,
            filter.required_tone_tag,
            filter.deny_ragebaiter,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Bans (owner-gebunden, lane-übergreifend) ───────────────────────────

    pub async fn add_ban(&self, owner_id: u64, banned_id: u64) -> VoiceDbResult<()> {
        let owner_id = u64_to_i64("tempvoice_bans.owner_id", owner_id)?;
        let banned_id = u64_to_i64("tempvoice_bans.banned_id", banned_id)?;
        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_bans (owner_id, banned_id)
            VALUES ($1, $2)
            ON CONFLICT (owner_id, banned_id) DO NOTHING
            "#,
            owner_id,
            banned_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_ban(&self, owner_id: u64, banned_id: u64) -> VoiceDbResult<()> {
        let owner_id = u64_to_i64("tempvoice_bans.owner_id", owner_id)?;
        let banned_id = u64_to_i64("tempvoice_bans.banned_id", banned_id)?;
        sqlx::query!(
            r#"
            DELETE FROM voice.tempvoice_bans
             WHERE owner_id = $1
               AND banned_id = $2
            "#,
            owner_id,
            banned_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn is_banned_by_owner(&self, owner_id: u64, user_id: u64) -> VoiceDbResult<bool> {
        let owner_id = u64_to_i64("tempvoice_bans.owner_id", owner_id)?;
        let banned_id = u64_to_i64("tempvoice_bans.banned_id", user_id)?;
        let found = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                  FROM voice.tempvoice_bans
                 WHERE owner_id = $1
                   AND banned_id = $2
            ) AS "exists!"
            "#,
            owner_id,
            banned_id,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(found)
    }

    pub async fn list_bans(&self, owner_id: u64) -> VoiceDbResult<Vec<u64>> {
        let owner_id = u64_to_i64("tempvoice_bans.owner_id", owner_id)?;
        let rows = sqlx::query!(
            r#"
            SELECT banned_id
              FROM voice.tempvoice_bans
             WHERE owner_id = $1
             ORDER BY banned_id
            "#,
            owner_id,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| i64_to_u64("tempvoice_bans.banned_id", row.banned_id))
            .collect()
    }

    // ── Lurker (tempvoice_lurkers) ──────────────────────────────────────────

    pub async fn add_lurker(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
        original_nick: Option<String>,
    ) -> VoiceDbResult<()> {
        let guild_id = u64_to_i64("tempvoice_lurkers.guild_id", guild_id)?;
        let channel_id = u64_to_i64("tempvoice_lurkers.channel_id", channel_id)?;
        let user_id = u64_to_i64("tempvoice_lurkers.user_id", user_id)?;
        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_lurkers (
                guild_id, channel_id, user_id, original_nick
            )
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (channel_id, user_id) DO UPDATE SET
                original_nick = EXCLUDED.original_nick
            "#,
            guild_id,
            channel_id,
            user_id,
            original_nick,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Some(original_nick) wenn der User Lurker in diesem Kanal ist.
    pub async fn get_lurker(&self, channel_id: u64, user_id: u64) -> Option<Option<String>> {
        let channel_id = u64_to_i64("tempvoice_lurkers.channel_id", channel_id).ok()?;
        let user_id = u64_to_i64("tempvoice_lurkers.user_id", user_id).ok()?;
        sqlx::query!(
            r#"
            SELECT original_nick
              FROM voice.tempvoice_lurkers
             WHERE channel_id = $1
               AND user_id = $2
            "#,
            channel_id,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|row| row.original_nick)
    }

    pub async fn remove_lurker(&self, channel_id: u64, user_id: u64) -> VoiceDbResult<()> {
        let channel_id = u64_to_i64("tempvoice_lurkers.channel_id", channel_id)?;
        let user_id = u64_to_i64("tempvoice_lurkers.user_id", user_id)?;
        sqlx::query!(
            r#"
            DELETE FROM voice.tempvoice_lurkers
             WHERE channel_id = $1
               AND user_id = $2
            "#,
            channel_id,
            user_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Region-Präferenz (DE/EU) ───────────────────────────────────────────

    pub async fn region_pref(&self, owner_id: u64) -> VoiceDbResult<String> {
        let owner_id = u64_to_i64("tempvoice_owner_prefs.owner_id", owner_id)?;
        let row = sqlx::query!(
            r#"
            SELECT region
              FROM voice.tempvoice_owner_prefs
             WHERE owner_id = $1
            "#,
            owner_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|row| row.region)
            .unwrap_or_else(|| "EU".to_string()))
    }

    pub async fn set_region_pref(&self, owner_id: u64, region: &str) -> VoiceDbResult<()> {
        let owner_id = u64_to_i64("tempvoice_owner_prefs.owner_id", owner_id)?;
        let region = if region == "DE" { "DE" } else { "EU" }.to_string();
        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_owner_prefs (owner_id, region, updated_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (owner_id) DO UPDATE SET
                region = EXCLUDED.region,
                updated_at = NOW()
            "#,
            owner_id,
            region,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Rang-Präferenz (für prefix_from_rank-Lanes) ────────────────────────

    pub async fn rank_pref(&self, user_id: u64) -> VoiceDbResult<(String, i64)> {
        let user_id = u64_to_i64("tempvoice_rank_pref.user_id", user_id)?;
        let row = sqlx::query!(
            r#"
            SELECT rank, subrank
              FROM voice.tempvoice_rank_pref
             WHERE user_id = $1
            "#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|row| (row.rank, i64::from(row.subrank)))
            .unwrap_or_else(|| ("unknown".to_string(), 0)))
    }

    pub async fn set_rank_pref(&self, user_id: u64, rank: &str, subrank: i64) -> VoiceDbResult<()> {
        let user_id = u64_to_i64("tempvoice_rank_pref.user_id", user_id)?;
        let subrank = i64_to_i32("tempvoice_rank_pref.subrank", subrank)?;
        let rank = rank.to_lowercase();
        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_rank_pref (user_id, rank, subrank, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (user_id) DO UPDATE SET
                rank = EXCLUDED.rank,
                subrank = EXCLUDED.subrank,
                updated_at = NOW()
            "#,
            user_id,
            rank,
            subrank,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Presets (pro User + Kategorie) ─────────────────────────────────────

    pub async fn save_preset(&self, preset: PresetRecord) -> VoiceDbResult<()> {
        let PresetRecord {
            user_id,
            category_id,
            name,
            base_name,
            limit,
            min_rank,
            region,
        } = preset;
        let user_id = u64_to_i64("tempvoice_presets.user_id", user_id)?;
        let category_id = u64_to_i64("tempvoice_presets.category_id", category_id)?;
        let member_limit = i64_to_i32("tempvoice_presets.member_limit", limit)?;
        let region = if region == "DE" { "DE" } else { "EU" }.to_string();
        sqlx::query!(
            r#"
            INSERT INTO voice.tempvoice_presets (
                user_id, category_id, name, base_name, member_limit,
                min_rank, region, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
            ON CONFLICT (user_id, category_id, name) DO UPDATE SET
                base_name = EXCLUDED.base_name,
                member_limit = EXCLUDED.member_limit,
                min_rank = EXCLUDED.min_rank,
                region = EXCLUDED.region,
                updated_at = NOW()
            "#,
            user_id,
            category_id,
            name,
            base_name,
            member_limit,
            min_rank,
            region,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_presets(&self, user_id: u64, category_id: u64) -> VoiceDbResult<Vec<String>> {
        let user_id = u64_to_i64("tempvoice_presets.user_id", user_id)?;
        let category_id = u64_to_i64("tempvoice_presets.category_id", category_id)?;
        let rows = sqlx::query!(
            r#"
            SELECT name
              FROM voice.tempvoice_presets
             WHERE user_id = $1
               AND category_id = $2
             ORDER BY name
            "#,
            user_id,
            category_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| row.name).collect())
    }

    pub async fn get_preset(
        &self,
        user_id: u64,
        category_id: u64,
        name: &str,
    ) -> VoiceDbResult<Option<(String, i64, String, String)>> {
        let user_id = u64_to_i64("tempvoice_presets.user_id", user_id)?;
        let category_id = u64_to_i64("tempvoice_presets.category_id", category_id)?;
        let row = sqlx::query!(
            r#"
            SELECT base_name, member_limit, min_rank, region
              FROM voice.tempvoice_presets
             WHERE user_id = $1
               AND category_id = $2
               AND name = $3
            "#,
            user_id,
            category_id,
            name,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| {
            (
                row.base_name,
                i64::from(row.member_limit),
                row.min_rank,
                row.region,
            )
        }))
    }

    // ── Persistente Interface-Messages (tempvoice_interface) ───────────────

    pub async fn ensure_interface_schema(&self) -> VoiceDbResult<()> {
        // Zentraler Cutover: DDL und SQLite-Selbstmigration gehoeren dem
        // zentralen Migrator. Die alte tempvoice_interface_old-Migration ist
        // hier absichtlich entfernt.
        Ok(())
    }

    pub async fn record_interface_message(&self, record: InterfaceRecord) -> VoiceDbResult<()> {
        self.ensure_interface_schema().await?;
        let guild_id = u64_to_i64("tempvoice_interface.guild_id", record.guild_id)?;
        let channel_id = u64_to_i64("tempvoice_interface.channel_id", record.channel_id)?;
        let message_id = u64_to_i64("tempvoice_interface.message_id", record.message_id)?;
        let category_id = opt_u64_to_i64("tempvoice_interface.category_id", record.category_id)?;
        if let Some(lane_id) = record.lane_id {
            let lane_id = u64_to_i64("tempvoice_interface.lane_id", lane_id)?;
            sqlx::query!(
                r#"
                INSERT INTO voice.tempvoice_interface (
                    guild_id, channel_id, message_id, category_id, lane_id, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, NOW())
                ON CONFLICT (lane_id) DO UPDATE SET
                    channel_id = EXCLUDED.channel_id,
                    message_id = EXCLUDED.message_id,
                    category_id = EXCLUDED.category_id,
                    updated_at = NOW()
                "#,
                guild_id,
                channel_id,
                message_id,
                category_id,
                lane_id,
            )
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query!(
                r#"
                INSERT INTO voice.tempvoice_interface (
                    guild_id, channel_id, message_id, category_id, lane_id, updated_at
                )
                VALUES ($1, $2, $3, $4, NULL, NOW())
                ON CONFLICT (guild_id, message_id) DO UPDATE SET
                    channel_id = EXCLUDED.channel_id,
                    category_id = EXCLUDED.category_id,
                    updated_at = NOW()
                "#,
                guild_id,
                channel_id,
                message_id,
                category_id,
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn global_interface_messages(&self) -> VoiceDbResult<Vec<InterfaceRecord>> {
        self.ensure_interface_schema().await?;
        let rows = sqlx::query!(
            r#"
            SELECT guild_id, channel_id, message_id, category_id, lane_id
              FROM voice.tempvoice_interface
             WHERE lane_id IS NULL
             ORDER BY guild_id, message_id
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(InterfaceRecord {
                    guild_id: i64_to_u64("tempvoice_interface.guild_id", row.guild_id)?,
                    channel_id: i64_to_u64("tempvoice_interface.channel_id", row.channel_id)?,
                    message_id: i64_to_u64("tempvoice_interface.message_id", row.message_id)?,
                    category_id: opt_i64_to_u64(
                        "tempvoice_interface.category_id",
                        row.category_id,
                    )?,
                    lane_id: opt_i64_to_u64("tempvoice_interface.lane_id", row.lane_id)?,
                })
            })
            .collect()
    }

    pub async fn lane_interface_messages(&self) -> VoiceDbResult<Vec<InterfaceRecord>> {
        self.ensure_interface_schema().await?;
        let rows = sqlx::query!(
            r#"
            SELECT guild_id, channel_id, message_id, category_id, lane_id
              FROM voice.tempvoice_interface
             WHERE lane_id IS NOT NULL
             ORDER BY lane_id
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(InterfaceRecord {
                    guild_id: i64_to_u64("tempvoice_interface.guild_id", row.guild_id)?,
                    channel_id: i64_to_u64("tempvoice_interface.channel_id", row.channel_id)?,
                    message_id: i64_to_u64("tempvoice_interface.message_id", row.message_id)?,
                    category_id: opt_i64_to_u64(
                        "tempvoice_interface.category_id",
                        row.category_id,
                    )?,
                    lane_id: opt_i64_to_u64("tempvoice_interface.lane_id", row.lane_id)?,
                })
            })
            .collect()
    }

    pub async fn remove_lane_interface_record(&self, lane_id: u64) -> VoiceDbResult<()> {
        self.ensure_interface_schema().await?;
        let lane_id = u64_to_i64("tempvoice_interface.lane_id", lane_id)?;
        sqlx::query!(
            r#"
            DELETE FROM voice.tempvoice_interface
             WHERE lane_id = $1
            "#,
            lane_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_interface_record(
        &self,
        guild_id: u64,
        message_id: u64,
    ) -> VoiceDbResult<()> {
        self.ensure_interface_schema().await?;
        let guild_id = u64_to_i64("tempvoice_interface.guild_id", guild_id)?;
        let message_id = u64_to_i64("tempvoice_interface.message_id", message_id)?;
        sqlx::query!(
            r#"
            DELETE FROM voice.tempvoice_interface
             WHERE guild_id = $1
               AND message_id = $2
            "#,
            guild_id,
            message_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (dl_central_db::TestDb, TempVoiceStore) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let store = TempVoiceStore::new(db.pool().clone());
        (db, store)
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
