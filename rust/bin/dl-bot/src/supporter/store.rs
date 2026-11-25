use std::collections::HashSet;
use std::fmt;

use sqlx::{PgPool, Postgres, Transaction};

const KV_NS: &str = "supporter";
const MANUAL_IMPORT_KEY: &str = "manual_import_at";
const SHORTEST_SYNC_INTERVAL_MS: u64 =
    *dl_core::runtime_config::SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE.start() * 1_000;
pub(super) const TABLE_LOCKS_PER_TICK: u64 = 3;
const REST_OF_TICK_SHARES: u64 = 1;
const TABLE_LOCK_TIMEOUT_MS: u64 =
    SHORTEST_SYNC_INTERVAL_MS / (TABLE_LOCKS_PER_TICK + REST_OF_TICK_SHARES);
const _: () = assert!(TABLE_LOCK_TIMEOUT_MS > 0);
const LOCK_NOT_AVAILABLE: &str = "55P03";

#[cfg(test)]
tokio::task_local! {
    pub(super) static TABLE_LOCKS_TAKEN: std::cell::Cell<u64>;
}

#[derive(Debug)]
pub enum StoreError {
    Missing,
    GateDisabled,
    LockTimeout,
    TickDeadline(std::time::Duration),
    Db(sqlx::Error),
    Discord(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(
                f,
                "core.supporter_sources oder ihr Datenschutz-Trigger supporter_sources_privacy_gate fehlt, Abgleich pausiert, bis Tabelle und Trigger wieder angelegt sind"
            ),
            Self::LockTimeout => write!(
                f,
                "eine Datenbanksperre des Unterstützer-Abgleichs bleibt länger als {TABLE_LOCK_TIMEOUT_MS} ms von einer anderen Sitzung belegt, Abgleich versucht es beim nächsten Lauf erneut"
            ),
            Self::TickDeadline(deadline) => write!(
                f,
                "Abgleich nach {} ms abgebrochen",
                deadline.as_millis()
            ),
            Self::GateDisabled => write!(
                f,
                "Datenschutz-Trigger supporter_sources_privacy_gate ist nicht für normale Sitzungen aktiv, Abgleich pausiert bis ALTER TABLE core.supporter_sources ENABLE TRIGGER supporter_sources_privacy_gate"
            ),
            Self::Db(error) => write!(f, "Datenbank: {error}"),
            Self::Discord(error) => write!(f, "Discord: {error}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Db(error) => Some(error),
            Self::Missing
            | Self::GateDisabled
            | Self::LockTimeout
            | Self::TickDeadline(_)
            | Self::Discord(_) => None,
        }
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::Database(db) if db.code().as_deref() == Some(LOCK_NOT_AVAILABLE) => {
                Self::LockTimeout
            }
            _ => Self::Db(error),
        }
    }
}

pub async fn require_table(pool: &PgPool) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let gate = lock_gated_table(&mut tx).await;
    tx.rollback().await?;
    gate
}

async fn lock_gated_table(tx: &mut Transaction<'_, Postgres>) -> Result<(), StoreError> {
    lock_gated_table_within(tx, TABLE_LOCK_TIMEOUT_MS).await
}

async fn lock_gated_table_within(
    tx: &mut Transaction<'_, Postgres>,
    lock_timeout_ms: u64,
) -> Result<(), StoreError> {
    let present: bool =
        sqlx::query_scalar("SELECT to_regclass('core.supporter_sources') IS NOT NULL")
            .fetch_one(&mut **tx)
            .await?;
    if !present {
        return Err(StoreError::Missing);
    }
    sqlx::query("SELECT set_config('lock_timeout', $1, true)")
        .bind(format!("{lock_timeout_ms}ms"))
        .execute(&mut **tx)
        .await?;
    #[cfg(test)]
    let _ = TABLE_LOCKS_TAKEN.try_with(|taken| taken.set(taken.get() + 1));
    sqlx::query("LOCK TABLE core.supporter_sources IN ROW EXCLUSIVE MODE")
        .execute(&mut **tx)
        .await?;
    let state: Option<String> = sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_catalog.pg_trigger
          WHERE tgrelid = 'core.supporter_sources'::regclass
            AND tgname = 'supporter_sources_privacy_gate'",
    )
    .fetch_optional(&mut **tx)
    .await?;
    gate_state(state.as_deref())
}

fn gate_state(tgenabled: Option<&str>) -> Result<(), StoreError> {
    match tgenabled {
        None => Err(StoreError::Missing),
        Some("O" | "A") => Ok(()),
        Some(_) => Err(StoreError::GateDisabled),
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
    sync_boost_rows_within(pool, boosters, TABLE_LOCK_TIMEOUT_MS).await
}

async fn sync_boost_rows_within(
    pool: &PgPool,
    boosters: &[u64],
    lock_timeout_ms: u64,
) -> Result<BoostSync, StoreError> {
    let ids = to_db_ids(boosters);
    let mut tx = pool.begin().await?;
    lock_gated_table_within(&mut tx, lock_timeout_ms).await?;
    sqlx::query(
        "INSERT INTO core.supporter_sources
             (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
         SELECT id, 'boost', 'discord', NULL, NULL, now()
           FROM (SELECT DISTINCT id FROM UNNEST($1::bigint[]) AS id ORDER BY id) AS ids
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
    lock_gated_table(&mut tx).await?;
    let claimed = sqlx::query(
        "INSERT INTO bot.kv_store(ns, k, v)
         VALUES($1, $2, now()::text)
         ON CONFLICT(ns, k) DO NOTHING",
    )
    .bind(KV_NS)
    .bind(MANUAL_IMPORT_KEY)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed != 1 {
        tx.rollback().await?;
        return Ok(0);
    }
    let imported = sqlx::query(
        "INSERT INTO core.supporter_sources
             (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
         SELECT id, 'manual', 'role_cutover', NULL, now() + make_interval(days => $2), now()
           FROM (SELECT DISTINCT id FROM UNNEST($1::bigint[]) AS id ORDER BY id) AS ids
         ON CONFLICT (discord_id, source, source_ref) DO NOTHING",
    )
    .bind(&ids)
    .bind(dl_core::runtime_config::SUPPORTER_CUTOVER_GRACE_DAYS)
    .execute(&mut *tx)
    .await?
    .rows_affected();
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
    async fn jede_aktive_quelle_zaehlt() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO core.supporter_sources
                 (discord_id, source, source_ref, monthly_cents, valid_until)
             VALUES (11, 'stripe', 'sub_klein', 300, NULL),
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
        assert_eq!(import_manual(pool, &[15, 16]).await?, 0);
        assert!(manual_import_done(pool).await?);
        require_table(pool).await?;
        let expected: HashSet<u64> = [11, 13, 15].into_iter().collect();
        assert_eq!(active_supporters(pool).await?, expected);
        Ok(())
    }

    #[tokio::test]
    async fn require_table_meldet_jeden_triggerzustand() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        require_table(pool).await?;
        sqlx::query(
            "ALTER TABLE core.supporter_sources DISABLE TRIGGER supporter_sources_privacy_gate",
        )
        .execute(pool)
        .await?;
        assert!(matches!(
            require_table(pool).await,
            Err(StoreError::GateDisabled)
        ));
        sqlx::query(
            "ALTER TABLE core.supporter_sources ENABLE REPLICA TRIGGER supporter_sources_privacy_gate",
        )
        .execute(pool)
        .await?;
        assert!(matches!(
            require_table(pool).await,
            Err(StoreError::GateDisabled)
        ));
        sqlx::query(
            "ALTER TABLE core.supporter_sources ENABLE ALWAYS TRIGGER supporter_sources_privacy_gate",
        )
        .execute(pool)
        .await?;
        require_table(pool).await?;
        sqlx::query("DROP TRIGGER supporter_sources_privacy_gate ON core.supporter_sources")
            .execute(pool)
            .await?;
        assert!(matches!(
            require_table(pool).await,
            Err(StoreError::Missing)
        ));
        sqlx::query("DROP TABLE core.supporter_sources CASCADE")
            .execute(pool)
            .await?;
        assert!(matches!(
            require_table(pool).await,
            Err(StoreError::Missing)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn schreibpfade_pruefen_den_trigger_in_der_eigenen_transaktion() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        for state in ["DISABLE TRIGGER", "ENABLE REPLICA TRIGGER"] {
            sqlx::query(&format!(
                "ALTER TABLE core.supporter_sources {state} supporter_sources_privacy_gate"
            ))
            .execute(pool)
            .await?;
            assert!(
                matches!(
                    sync_boost_rows(pool, &[51]).await,
                    Err(StoreError::GateDisabled)
                ),
                "{state}"
            );
            assert!(
                matches!(
                    import_manual(pool, &[52]).await,
                    Err(StoreError::GateDisabled)
                ),
                "{state}"
            );
            assert_eq!(source_rows(pool, 51).await?, 0);
            assert_eq!(source_rows(pool, 52).await?, 0);
            assert!(!manual_import_done(pool).await?);
        }
        sqlx::query(
            "ALTER TABLE core.supporter_sources ENABLE TRIGGER supporter_sources_privacy_gate",
        )
        .execute(pool)
        .await?;
        sync_boost_rows(pool, &[51]).await?;
        sqlx::query("DROP TRIGGER supporter_sources_privacy_gate ON core.supporter_sources")
            .execute(pool)
            .await?;
        assert!(matches!(
            sync_boost_rows(pool, &[53]).await,
            Err(StoreError::Missing)
        ));
        assert!(matches!(
            import_manual(pool, &[52]).await,
            Err(StoreError::Missing)
        ));
        assert_eq!(source_rows(pool, 53).await?, 0);
        assert_eq!(source_rows(pool, 52).await?, 0);
        assert!(!manual_import_done(pool).await?);
        Ok(())
    }

    async fn alter_trigger_within(pool: &PgPool, lock_timeout: &str) -> Result<(), sqlx::Error> {
        let mut other = pool.begin().await?;
        sqlx::query("SELECT set_config('lock_timeout', $1, true)")
            .bind(lock_timeout)
            .execute(&mut *other)
            .await?;
        sqlx::query(
            "ALTER TABLE core.supporter_sources DISABLE TRIGGER supporter_sources_privacy_gate",
        )
        .execute(&mut *other)
        .await?;
        other.rollback().await
    }

    fn lock_not_available(result: Result<(), sqlx::Error>) -> bool {
        matches!(result, Err(sqlx::Error::Database(db)) if db.code().as_deref() == Some(LOCK_NOT_AVAILABLE))
    }

    #[tokio::test]
    async fn trigger_bleibt_bis_zum_ende_der_schreibtransaktion_gesperrt() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let mut writer = pool.begin().await?;
        lock_gated_table(&mut writer).await?;
        assert!(lock_not_available(
            alter_trigger_within(pool, "100ms").await
        ));
        writer.rollback().await?;
        alter_trigger_within(pool, "100ms").await?;
        Ok(())
    }

    #[tokio::test]
    async fn gesperrte_tabelle_endet_nach_der_wartegrenze() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let mut holder = pool.begin().await?;
        sqlx::query("LOCK TABLE core.supporter_sources IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *holder)
            .await?;
        let mut writer = pool.begin().await?;
        assert!(matches!(
            lock_gated_table_within(&mut writer, 100).await,
            Err(StoreError::LockTimeout)
        ));
        writer.rollback().await?;
        holder.rollback().await?;
        require_table(pool).await?;
        Ok(())
    }

    #[tokio::test]
    async fn gesperrte_datenschutz_sperre_beim_einfuegen_endet_als_sperrfehler() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let mut holder = pool.begin().await?;
        dl_central_db::lock_user_privacy(&mut holder, 61).await?;
        let blocked = sync_boost_rows_within(pool, &[61], 100).await;
        holder.rollback().await?;
        assert!(
            matches!(blocked, Err(StoreError::LockTimeout)),
            "{blocked:?}"
        );
        assert_eq!(source_rows(pool, 61).await?, 0);
        sync_boost_rows(pool, &[61]).await?;
        assert_eq!(source_rows(pool, 61).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn gesperrte_zeile_beim_loeschen_endet_als_sperrfehler() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        sync_boost_rows(pool, &[62]).await?;
        let mut holder = pool.begin().await?;
        sqlx::query("SELECT 1 FROM core.supporter_sources WHERE discord_id = 62 FOR UPDATE")
            .fetch_all(&mut *holder)
            .await?;
        let blocked = sync_boost_rows_within(pool, &[], 100).await;
        holder.rollback().await?;
        assert!(
            matches!(blocked, Err(StoreError::LockTimeout)),
            "{blocked:?}"
        );
        assert_eq!(source_rows(pool, 62).await?, 1);
        Ok(())
    }

    async fn source_rows(pool: &PgPool, user_id: i64) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM core.supporter_sources WHERE discord_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await
    }

    #[tokio::test]
    async fn boost_abgleich_legt_abgemeldete_und_geloeschte_nicht_wieder_an() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        dl_central_db::testing::block_member(pool, 31, false).await?;
        dl_central_db::testing::block_member(pool, 32, true).await?;
        sync_boost_rows(pool, &[33, 32, 31, 33]).await?;
        assert_eq!(source_rows(pool, 31).await?, 0);
        assert_eq!(source_rows(pool, 32).await?, 0);
        assert_eq!(source_rows(pool, 33).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn manueller_import_legt_abgemeldete_und_geloeschte_nicht_wieder_an() -> TestResult {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        dl_central_db::testing::block_member(pool, 41, false).await?;
        dl_central_db::testing::block_member(pool, 42, true).await?;
        assert_eq!(import_manual(pool, &[43, 42, 41]).await?, 1);
        assert_eq!(source_rows(pool, 41).await?, 0);
        assert_eq!(source_rows(pool, 42).await?, 0);
        assert_eq!(source_rows(pool, 43).await?, 1);
        Ok(())
    }
}
