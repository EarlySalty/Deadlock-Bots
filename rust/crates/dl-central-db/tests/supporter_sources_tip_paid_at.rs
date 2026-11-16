#![cfg(feature = "testing")]
use std::path::Path;

use dl_central_db::test_pool;
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const PAID: &str = "2026-10-31 22:30:00+00";

fn migration_sql() -> std::io::Result<String> {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations")
            .join("20261116130000_supporter_sources_tip_paid_at.sql"),
    )
}

async fn seed(pool: &PgPool, discord_id: i64, source_ref: &str) -> TestResult {
    sqlx::query(
        "INSERT INTO core.supporter_sources
             (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
         VALUES ($1, 'stripe', $2, 500, $3::timestamptz + interval '31 days', $3::timestamptz)",
    )
    .bind(discord_id)
    .bind(source_ref)
    .bind(PAID)
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

#[tokio::test]
async fn tip_rows_get_paid_at_from_updated_at_and_other_rows_stay_without() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool();
    let sql = migration_sql()?;

    seed(pool, 7101, "tip:pi_backfill").await?;
    seed(pool, 7102, "sub_backfill").await?;
    seed(pool, 7103, "tip:pi_opted_out").await?;
    sqlx::query(
        "INSERT INTO core.user_privacy(user_id, opted_out, reason, updated_at)
         VALUES (7103, true, 'test', now())",
    )
    .execute(pool)
    .await?;

    apply(pool, &sql).await?;
    apply(pool, &sql).await?;

    assert_eq!(
        paid_at_matches_updated_at(pool, "tip:pi_backfill").await?,
        Some(true)
    );
    assert_eq!(
        paid_at_matches_updated_at(pool, "tip:pi_opted_out").await?,
        Some(true)
    );
    assert_eq!(
        paid_at_matches_updated_at(pool, "sub_backfill").await?,
        None
    );

    let on_subscription = sqlx::query(
        "UPDATE core.supporter_sources SET paid_at = now() WHERE source_ref = 'sub_backfill'",
    )
    .execute(pool)
    .await;
    let constraint = on_subscription
        .err()
        .and_then(|err| err.into_database_error())
        .and_then(|err| err.constraint().map(str::to_owned));
    assert_eq!(
        constraint.as_deref(),
        Some("supporter_sources_paid_at_tip_only")
    );

    let infinite = sqlx::query(
        "UPDATE core.supporter_sources SET paid_at = 'infinity' WHERE source_ref = 'tip:pi_backfill'",
    )
    .execute(pool)
    .await;
    assert!(infinite.is_err());
    Ok(())
}
