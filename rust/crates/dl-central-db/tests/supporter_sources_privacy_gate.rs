#![cfg(feature = "testing")]
use std::time::Duration;

use dl_central_db::{lock_user_privacy, test_pool, testing::block_member};
use sqlx::{PgPool, Postgres, Transaction};

const STEP: Duration = Duration::from_secs(10);

const END_BY_REF: &str = "UPDATE core.supporter_sources
     SET valid_until = LEAST(COALESCE(valid_until, 'infinity'::timestamptz), now()),
         updated_at = now()
   WHERE source = 'stripe' AND source_ref = $1";

const TWITCH_UPSERT: &str = "INSERT INTO core.supporter_sources
         (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
     VALUES ($1, 'twitch_sub', 'earlysalty', NULL, now() + make_interval(days => $2), now())
     ON CONFLICT (discord_id, source, source_ref) DO UPDATE SET
         valid_until = EXCLUDED.valid_until,
         updated_at = now()";

const REOPEN_UPSERT: &str = "INSERT INTO core.supporter_sources
         (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
     VALUES ($1, 'twitch_sub', 'earlysalty', NULL, NULL, now())
     ON CONFLICT (discord_id, source, source_ref) DO UPDATE SET
         valid_until = NULL,
         updated_at = now()";

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn seed_row(pool: &PgPool, discord_id: i64, source: &str, source_ref: &str) -> TestResult {
    sqlx::query(
        "INSERT INTO core.supporter_sources (discord_id, source, source_ref, monthly_cents, valid_until)
         VALUES ($1, $2, $3, NULL, NULL)",
    )
    .bind(discord_id)
    .bind(source)
    .bind(source_ref)
    .execute(pool)
    .await?;
    Ok(())
}

async fn waiting_locks(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM pg_locks l
           JOIN pg_stat_activity a ON a.pid = l.pid
          WHERE NOT l.granted AND a.datname = current_database()",
    )
    .fetch_one(pool)
    .await
}

async fn wait_for_lock_wait(pool: &PgPool) -> TestResult {
    tokio::time::timeout(STEP, async {
        while waiting_locks(pool).await? == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<(), sqlx::Error>(())
    })
    .await??;
    Ok(())
}

async fn valid_until_state(pool: &PgPool, discord_id: i64) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT valid_until IS NOT NULL AND valid_until <= now() + interval '2 days'
           FROM core.supporter_sources WHERE discord_id = $1",
    )
    .bind(discord_id)
    .fetch_optional(pool)
    .await
}

async fn source_ended(pool: &PgPool, discord_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT valid_until <= now() FROM core.supporter_sources WHERE discord_id = $1",
    )
    .bind(discord_id)
    .fetch_one(pool)
    .await
}

#[tokio::test]
async fn ending_a_source_waits_on_no_privacy_lock_while_erasure_deletes() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool().clone();
    seed_row(&pool, 5001, "stripe", "sub_end").await?;

    let mut eraser: Transaction<'static, Postgres> = pool.begin().await?;
    lock_user_privacy(&mut eraser, 5001).await?;

    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        let mut tx = writer_pool.begin().await?;
        let ended = sqlx::query(END_BY_REF)
            .bind("sub_end")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        Ok::<_, sqlx::Error>((tx, ended))
    });
    tokio::time::timeout(STEP, async {
        while !writer.is_finished() && waiting_locks(&pool).await? == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<(), sqlx::Error>(())
    })
    .await??;

    let erasure = tokio::spawn(async move {
        let deleted = sqlx::query("DELETE FROM core.supporter_sources WHERE discord_id = $1")
            .bind(5001_i64)
            .execute(&mut *eraser)
            .await?
            .rows_affected();
        Ok::<_, sqlx::Error>((eraser, deleted))
    });
    let (writer_tx, ended) = tokio::time::timeout(STEP, writer).await???;
    assert_eq!(ended, 1);
    wait_for_lock_wait(&pool).await?;
    writer_tx.commit().await?;
    let (eraser, deleted) = tokio::time::timeout(STEP, erasure).await???;
    eraser.commit().await?;

    assert_eq!(deleted, 1);
    assert_eq!(valid_until_state(&pool, 5001).await?, None);
    Ok(())
}

#[tokio::test]
async fn extending_without_the_privacy_lock_fails_fast_under_contention() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool().clone();
    seed_row(&pool, 5002, "stripe", "sub_hold").await?;
    sqlx::query("UPDATE core.supporter_sources SET valid_until = now() WHERE discord_id = 5002")
        .execute(&pool)
        .await?;

    let mut eraser = pool.begin().await?;
    lock_user_privacy(&mut eraser, 5002).await?;
    let outcome = tokio::time::timeout(
        STEP,
        sqlx::query("UPDATE core.supporter_sources SET valid_until = NULL WHERE discord_id = 5002")
            .execute(&pool),
    )
    .await?;
    let code = match outcome {
        Err(sqlx::Error::Database(error)) => error.code().map(String::from),
        other => return Err(format!("extension must be refused, got {other:?}").into()),
    };
    assert_eq!(code.as_deref(), Some("40001"));
    eraser.rollback().await?;

    let reopened =
        sqlx::query("UPDATE core.supporter_sources SET valid_until = NULL WHERE discord_id = 5002")
            .execute(&pool)
            .await?
            .rows_affected();
    assert_eq!(reopened, 1);
    Ok(())
}

#[tokio::test]
async fn upsert_may_end_but_never_extend_a_blocked_members_source() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool().clone();
    for (discord_id, deleted) in [(5101_i64, false), (5102, true)] {
        seed_row(&pool, discord_id, "twitch_sub", "earlysalty").await?;
        block_member(&pool, discord_id, deleted).await?;

        let shortened = sqlx::query(TWITCH_UPSERT)
            .bind(discord_id)
            .bind(1_i32)
            .execute(&pool)
            .await?
            .rows_affected();
        assert_eq!(shortened, 1, "{discord_id} shortens");
        assert_eq!(valid_until_state(&pool, discord_id).await?, Some(true));

        let ended = sqlx::query(TWITCH_UPSERT)
            .bind(discord_id)
            .bind(0_i32)
            .execute(&pool)
            .await?
            .rows_affected();
        assert_eq!(ended, 1, "{discord_id} ends");
        assert!(source_ended(&pool, discord_id).await?, "{discord_id} ended");

        let extended = sqlx::query(TWITCH_UPSERT)
            .bind(discord_id)
            .bind(30_i32)
            .execute(&pool)
            .await?
            .rows_affected();
        assert_eq!(extended, 0, "{discord_id} must not extend");
        let reopened = sqlx::query(REOPEN_UPSERT)
            .bind(discord_id)
            .execute(&pool)
            .await?
            .rows_affected();
        assert_eq!(reopened, 0, "{discord_id} must not reopen");
        assert_eq!(valid_until_state(&pool, discord_id).await?, Some(true));
    }

    block_member(&pool, 5103, false).await?;
    let granted = sqlx::query(TWITCH_UPSERT)
        .bind(5103_i64)
        .bind(1_i32)
        .execute(&pool)
        .await?
        .rows_affected();
    assert_eq!(granted, 0);
    assert_eq!(valid_until_state(&pool, 5103).await?, None);
    Ok(())
}

#[tokio::test]
async fn lock_free_shortening_never_raises_a_blocked_members_amount() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool().clone();
    sqlx::query(
        "INSERT INTO core.supporter_sources (discord_id, source, source_ref, monthly_cents, valid_until)
         VALUES (5301, 'stripe', 'sub_amount', 500, now() + interval '30 days'),
                (5302, 'stripe', 'sub_amount', NULL, now() + interval '30 days')",
    )
    .execute(&pool)
    .await?;
    for discord_id in [5301_i64, 5302] {
        block_member(&pool, discord_id, false).await?;
    }
    let raise = "UPDATE core.supporter_sources
                    SET monthly_cents = $2, valid_until = now() + interval '1 day'
                  WHERE discord_id = $1";
    for discord_id in [5301_i64, 5302] {
        let raised = sqlx::query(raise)
            .bind(discord_id)
            .bind(5_000_i32)
            .execute(&pool)
            .await?
            .rows_affected();
        assert_eq!(raised, 0, "{discord_id} must not raise the amount");
    }
    let mut eraser = pool.begin().await?;
    lock_user_privacy(&mut eraser, 5301).await?;
    let lowered = tokio::time::timeout(
        STEP,
        sqlx::query(raise)
            .bind(5301_i64)
            .bind(300_i32)
            .execute(&pool),
    )
    .await??
    .rows_affected();
    eraser.rollback().await?;
    assert_eq!(lowered, 1);
    let amounts: Vec<Option<i32>> = sqlx::query_scalar(
        "SELECT monthly_cents FROM core.supporter_sources WHERE discord_id IN (5301, 5302) ORDER BY discord_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(amounts, vec![Some(300), None]);
    Ok(())
}

#[tokio::test]
async fn shortening_upsert_never_recreates_a_row_deleted_meanwhile() -> TestResult {
    let db = test_pool().await?;
    let pool = db.pool().clone();
    seed_row(&pool, 5401, "twitch_sub", "earlysalty").await?;
    block_member(&pool, 5401, false).await?;

    let mut deleter = pool.begin().await?;
    let deleted = sqlx::query("DELETE FROM core.supporter_sources WHERE discord_id = $1")
        .bind(5401_i64)
        .execute(&mut *deleter)
        .await?
        .rows_affected();
    assert_eq!(deleted, 1);

    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        sqlx::query(TWITCH_UPSERT)
            .bind(5401_i64)
            .bind(1_i32)
            .execute(&writer_pool)
            .await
            .map(|done| done.rows_affected())
    });
    wait_for_lock_wait(&pool).await?;
    deleter.commit().await?;
    let written = tokio::time::timeout(STEP, writer).await???;

    assert_eq!(written, 0);
    assert_eq!(valid_until_state(&pool, 5401).await?, None);
    Ok(())
}
