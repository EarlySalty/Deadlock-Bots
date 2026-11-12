use std::collections::HashSet;
use std::fmt;

use sqlx::PgPool;

const UNDEFINED_TABLE: &str = "42P01";
const KV_NS: &str = "supporter";
const MANUAL_IMPORT_KEY: &str = "manual_import_at";

#[derive(Debug)]
pub enum StoreError {
    Missing,
    Db(sqlx::Error),
    Discord(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "core.supporter_sources fehlt"),
            Self::Db(error) => write!(f, "Datenbank: {error}"),
            Self::Discord(error) => write!(f, "Discord: {error}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Db(error) => Some(error),
            Self::Missing | Self::Discord(_) => None,
        }
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        let missing = error
            .as_database_error()
            .and_then(|db| db.code())
            .is_some_and(|code| code == UNDEFINED_TABLE);
        if missing {
            Self::Missing
        } else {
            Self::Db(error)
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BoostSync {
    pub removed: u64,
}

pub async fn active_supporters(pool: &PgPool) -> Result<HashSet<u64>, StoreError> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT discord_id FROM core.active_supporters")
        .fetch_all(pool)
        .await?;
    Ok(ids
        .into_iter()
        .filter_map(|id| u64::try_from(id).ok())
        .collect())
}

pub async fn sync_boost_rows(pool: &PgPool, boosters: &[u64]) -> Result<BoostSync, StoreError> {
    let ids = to_db_ids(boosters);
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO core.supporter_sources
             (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
         SELECT id, 'boost', 'discord', NULL, NULL, now()
           FROM UNNEST($1::bigint[]) AS id
         ON CONFLICT (discord_id, source, source_ref)
         DO UPDATE SET valid_until = NULL, updated_at = now()",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    let removed = sqlx::query(
        "DELETE FROM core.supporter_sources
          WHERE source = 'boost'
            AND NOT (discord_id = ANY($1::bigint[]))",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(BoostSync { removed })
}

pub async fn manual_import_done(pool: &PgPool) -> Result<bool, StoreError> {
    let done: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bot.kv_store WHERE ns = $1 AND k = $2)")
            .bind(KV_NS)
            .bind(MANUAL_IMPORT_KEY)
            .fetch_one(pool)
            .await?;
    Ok(done)
}

pub async fn import_manual(pool: &PgPool, holders: &[u64]) -> Result<u64, StoreError> {
    let ids = to_db_ids(holders);
    let mut tx = pool.begin().await?;
    let imported = sqlx::query(
        "INSERT INTO core.supporter_sources
             (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
         SELECT id, 'manual', 'role_cutover', NULL, now() + make_interval(days => $2), now()
           FROM UNNEST($1::bigint[]) AS id
         ON CONFLICT (discord_id, source, source_ref) DO NOTHING",
    )
    .bind(&ids)
    .bind(dl_core::runtime_config::SUPPORTER_TIP_VALID_DAYS)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query(
        "INSERT INTO bot.kv_store(ns, k, v)
         VALUES($1, $2, now()::text)
         ON CONFLICT(ns, k) DO NOTHING",
    )
    .bind(KV_NS)
    .bind(MANUAL_IMPORT_KEY)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(imported)
}

fn to_db_ids(ids: &[u64]) -> Vec<i64> {
    ids.iter()
        .filter_map(|id| i64::try_from(*id).ok())
        .filter(|id| *id > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn jede_aktive_quelle_zaehlt_auch_plus_unter_fuenf_euro() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO core.supporter_sources
                 (discord_id, source, source_ref, monthly_cents, valid_until)
             VALUES (11, 'stripe', 'sub_plus', 300, NULL),
                    (12, 'stripe', 'tip:pi_old', 500, now() - interval '1 day'),
                    (13, 'twitch_sub', 'earlysalty', NULL, now() + interval '1 day')",
        )
        .execute(pool)
        .await?;
        assert_eq!(sync_boost_rows(pool, &[14, 0]).await?.removed, 0);
        let expected: HashSet<u64> = [11, 13, 14].into_iter().collect();
        assert_eq!(active_supporters(pool).await?, expected);
        assert_eq!(sync_boost_rows(pool, &[]).await?.removed, 1);
        assert!(!active_supporters(pool).await?.contains(&14));
        assert!(!manual_import_done(pool).await?);
        assert_eq!(import_manual(pool, &[15, 11]).await?, 2);
        assert_eq!(import_manual(pool, &[15]).await?, 0);
        assert!(manual_import_done(pool).await?);
        let expected: HashSet<u64> = [11, 13, 15].into_iter().collect();
        assert_eq!(active_supporters(pool).await?, expected);
        Ok(())
    }
}
