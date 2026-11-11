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
         SELECT id, 'manual', 'role_cutover', NULL, now() + interval '31 days', now()
           FROM UNNEST($1::bigint[]) AS id
         ON CONFLICT (discord_id, source, source_ref) DO NOTHING",
    )
    .bind(&ids)
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
