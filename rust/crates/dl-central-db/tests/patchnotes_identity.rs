#![cfg(feature = "testing")]

use dl_central_db::testing::test_pool;
use sqlx::PgPool;

const PATCHNOTES_IDENTITY_MIGRATION: &str =
    include_str!("../migrations/0012_patchnotes_identity_sequences.sql");

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN, DATABASE_URL or DEADLOCK_CENTRAL_DSN"]
async fn patchnotes_identity_columns_accept_default_and_explicit_ids() {
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    let generated_post_id = insert_changelog_post_without_id(pool, "generated-post")
        .await
        .expect("insert changelog post without id");
    assert!(generated_post_id > 0);

    let explicit_post_id = 400_i64;
    insert_changelog_post_with_id(pool, explicit_post_id, "explicit-post")
        .await
        .expect("insert changelog post with explicit id");
    assert_eq!(
        changelog_post_id(pool, explicit_post_id)
            .await
            .expect("fetch explicit changelog post id"),
        explicit_post_id
    );

    let generated_deadlock_id = insert_deadlock_changelog_without_id(pool, "generated-deadlock")
        .await
        .expect("insert deadlock changelog without id");
    assert!(generated_deadlock_id > 0);

    let explicit_deadlock_id = 401_i64;
    insert_deadlock_changelog_with_id(pool, explicit_deadlock_id, "explicit-deadlock")
        .await
        .expect("insert deadlock changelog with explicit id");
    assert_eq!(
        deadlock_changelog_id(pool, explicit_deadlock_id)
            .await
            .expect("fetch explicit deadlock changelog id"),
        explicit_deadlock_id
    );
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN, DATABASE_URL or DEADLOCK_CENTRAL_DSN"]
async fn patchnotes_identity_sequences_continue_after_existing_high_ids() {
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    reset_patchnotes_tables_to_pre_identity_shape(pool)
        .await
        .expect("reset identity columns");

    insert_changelog_post_with_id(pool, 500, "preexisting-post")
        .await
        .expect("insert preexisting high changelog post id");
    insert_deadlock_changelog_with_id(pool, 500, "preexisting-deadlock")
        .await
        .expect("insert preexisting high deadlock changelog id");

    sqlx::raw_sql(PATCHNOTES_IDENTITY_MIGRATION)
        .execute(pool)
        .await
        .expect("apply patchnotes identity migration");

    let next_post_id = insert_changelog_post_without_id(pool, "after-high-post")
        .await
        .expect("insert changelog post after high id");
    assert!(
        next_post_id >= 501,
        "generated changelog_posts id must continue after existing high id, got {next_post_id}"
    );

    let next_deadlock_id = insert_deadlock_changelog_without_id(pool, "after-high-deadlock")
        .await
        .expect("insert deadlock changelog after high id");
    assert!(
        next_deadlock_id >= 501,
        "generated deadlock_changelogs id must continue after existing high id, got {next_deadlock_id}"
    );
}

async fn reset_patchnotes_tables_to_pre_identity_shape(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(
        r#"
        TRUNCATE patchnotes.changelog_posts, patchnotes.deadlock_changelogs;
        ALTER TABLE patchnotes.changelog_posts ALTER COLUMN id DROP IDENTITY IF EXISTS;
        ALTER TABLE patchnotes.deadlock_changelogs ALTER COLUMN id DROP IDENTITY IF EXISTS;
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn insert_changelog_post_without_id(pool: &PgPool, slug: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO patchnotes.changelog_posts (title, url) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("title-{slug}"))
    .bind(format!("https://example.invalid/changelog-posts/{slug}"))
    .fetch_one(pool)
    .await
}

async fn insert_changelog_post_with_id(
    pool: &PgPool,
    id: i64,
    slug: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO patchnotes.changelog_posts (id, title, url) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(format!("title-{slug}"))
        .bind(format!("https://example.invalid/changelog-posts/{slug}"))
        .execute(pool)
        .await?;

    Ok(())
}

async fn insert_deadlock_changelog_without_id(
    pool: &PgPool,
    slug: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO patchnotes.deadlock_changelogs (title, url) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("title-{slug}"))
    .bind(format!(
        "https://example.invalid/deadlock-changelogs/{slug}"
    ))
    .fetch_one(pool)
    .await
}

async fn insert_deadlock_changelog_with_id(
    pool: &PgPool,
    id: i64,
    slug: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO patchnotes.deadlock_changelogs (id, title, url) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(format!("title-{slug}"))
        .bind(format!(
            "https://example.invalid/deadlock-changelogs/{slug}"
        ))
        .execute(pool)
        .await?;

    Ok(())
}

async fn changelog_post_id(pool: &PgPool, id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM patchnotes.changelog_posts WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
}

async fn deadlock_changelog_id(pool: &PgPool, id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM patchnotes.deadlock_changelogs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
}
