#![cfg(feature = "testing")]
use std::path::Path;

use dl_central_db::test_pool;
use dl_core::runtime_config::supporter_wire::{TIP_REF_PREFIX, TIP_VALID_DAYS};
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const PAID: &str = "2026-10-31 22:30:00+00";
const CONSTRAINT: &str = "supporter_sources_paid_at_tip_only";

const UNDO_MIGRATION: &str = "
    ALTER TABLE core.supporter_sources DROP CONSTRAINT IF EXISTS supporter_sources_paid_at_tip_only;
    DROP TRIGGER IF EXISTS supporter_sources_tip_paid_at ON core.supporter_sources;
    DROP FUNCTION IF EXISTS core.supporter_sources_tip_paid_at();
    ALTER TABLE core.supporter_sources DROP COLUMN IF EXISTS paid_at;";

fn migration_sql() -> std::io::Result<String> {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations")
            .join("20261116130000_supporter_sources_tip_paid_at.sql"),
    )
}

fn tip_ref(intent: &str) -> String {
    format!("{TIP_REF_PREFIX}{intent}")
}

async fn seed(
    pool: &PgPool,
    discord_id: i64,
    source: &str,
    source_ref: &str,
    valid: bool,
) -> TestResult {
    sqlx::query(
        "INSERT INTO core.supporter_sources
             (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
         VALUES ($1, $2, $3, 500,
                 CASE WHEN $5 THEN $4::timestamptz + make_interval(days => $6::int) END,
                 $4::timestamptz)",
    )
    .bind(discord_id)
    .bind(source)
    .bind(source_ref)
    .bind(PAID)
    .bind(valid)
    .bind(i32::try_from(TIP_VALID_DAYS)?)
    .execute(pool)
    .await?;
    Ok(())
}

async fn opt_out(pool: &PgPool, discord_id: i64) -> TestResult {
    sqlx::query(
        "INSERT INTO core.user_privacy(user_id, opted_out, reason, updated_at)
         VALUES ($1, true, 'test', now())",
    )
    .bind(discord_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn paid_at_matches_updated_at(
    pool: &PgPool,
    source_ref: &str,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT paid_at = updated_at FROM core.supporter_sources WHERE source_ref = $1",
    )
    .bind(source_ref)
    .fetch_one(pool)
    .await
}

async fn apply(pool: &PgPool, sql: &str) -> TestResult {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(sql).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn violated_constraint(pool: &PgPool, sql: &str) -> Option<String> {
    sqlx::query(sql)
        .execute(pool)
        .await
        .err()
        .and_then(|err| err.into_database_error())
        .and_then(|err| err.constraint().map(str::to_owned))
}

async fn privacy_gate_state(pool: &PgPool) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_catalog.pg_trigger
          WHERE tgrelid = 'core.supporter_sources'::regclass
            AND tgname = 'supporter_sources_privacy_gate'",
    )
    .fetch_one(pool)
    .await
}

#[tokio::test]
async fn backfill_reaches_every_tip_row_and_only_stripe_tips_carry_paid_at() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool();
    let sql = migration_sql()?;
    apply(pool, UNDO_MIGRATION).await?;

    let backfill = tip_ref("pi_backfill");
    let open_ended = tip_ref("pi_open_ended_opted_out");
    let manual_tip = tip_ref("manual_import");
    seed(pool, 7101, "stripe", &backfill, true).await?;
    seed(pool, 7102, "stripe", "sub_backfill", true).await?;
    seed(pool, 7103, "stripe", &open_ended, false).await?;
    seed(pool, 7104, "manual", &manual_tip, true).await?;
    opt_out(pool, 7103).await?;

    apply(pool, &sql).await?;
    apply(pool, &sql).await?;

    assert_eq!(privacy_gate_state(pool).await?, "O");
    assert_eq!(
        paid_at_matches_updated_at(pool, &backfill).await?,
        Some(true)
    );
    assert_eq!(
        paid_at_matches_updated_at(pool, &open_ended).await?,
        Some(true)
    );
    assert_eq!(
        paid_at_matches_updated_at(pool, "sub_backfill").await?,
        None
    );
    assert_eq!(paid_at_matches_updated_at(pool, &manual_tip).await?, None);

    assert_eq!(
        violated_constraint(
            pool,
            "UPDATE core.supporter_sources SET paid_at = now() WHERE source_ref = 'sub_backfill'",
        )
        .await
        .as_deref(),
        Some(CONSTRAINT)
    );
    assert_eq!(
        violated_constraint(
            pool,
            &format!(
                "UPDATE core.supporter_sources SET paid_at = now() WHERE source_ref = '{manual_tip}'"
            ),
        )
        .await
        .as_deref(),
        Some(CONSTRAINT)
    );
    assert_eq!(
        violated_constraint(
            pool,
            &format!(
                "UPDATE core.supporter_sources SET paid_at = 'infinity' WHERE source_ref = '{backfill}'"
            ),
        )
        .await
        .as_deref(),
        Some(CONSTRAINT)
    );
    Ok(())
}

#[tokio::test]
async fn a_tip_written_without_paid_at_gets_it_and_keeps_it() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool();

    let fresh = tip_ref("pi_fresh");
    seed(pool, 7201, "stripe", &fresh, true).await?;
    assert_eq!(paid_at_matches_updated_at(pool, &fresh).await?, Some(true));

    sqlx::query(
        "UPDATE core.supporter_sources
            SET paid_at = NULL, updated_at = $2::timestamptz + interval '1 day'
          WHERE source_ref = $1",
    )
    .bind(&fresh)
    .bind(PAID)
    .execute(pool)
    .await?;
    let kept: bool = sqlx::query_scalar(
        "SELECT paid_at = $2::timestamptz FROM core.supporter_sources WHERE source_ref = $1",
    )
    .bind(&fresh)
    .bind(PAID)
    .fetch_one(pool)
    .await?;
    assert!(kept);

    seed(pool, 7202, "boost", &tip_ref("boost_like"), false).await?;
    assert_eq!(
        paid_at_matches_updated_at(pool, &tip_ref("boost_like")).await?,
        None
    );
    Ok(())
}
