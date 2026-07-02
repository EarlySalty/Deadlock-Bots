use std::{borrow::Cow, path::Path};

use dl_central_db::connect_pool;
use sqlx::{migrate::Migrator, PgPool};

const STEAM_LINKS_PRIMARY_MIGRATION: &str =
    include_str!("../migrations/0015_steam_links_one_primary.sql");

fn test_dsn() -> String {
    std::env::var("CENTRAL_TEST_DSN")
        .expect("CENTRAL_TEST_DSN must be set for ignored DB tests; do not silently skip")
}

fn fresh_db_name(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    format!("dlcentral_{label}_{}_{}", std::process::id(), nanos)
}

fn swap_db(dsn: &str, db: &str) -> String {
    let (base, _old) = dsn.rsplit_once('/').expect("DSN contains database path");
    format!("{base}/{db}")
}

async fn create_fresh_db(admin: &PgPool, dbname: &str) {
    sqlx::query(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .execute(admin)
        .await
        .ok();
    sqlx::query(&format!("CREATE DATABASE {dbname}"))
        .execute(admin)
        .await
        .expect("create fresh database");
}

async fn drop_db(admin: &PgPool, dbname: &str) {
    sqlx::query(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .execute(admin)
        .await
        .ok();
}

async fn run_migrations_through(pool: &PgPool, version: i64) {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let all = Migrator::new(migrations_path)
        .await
        .expect("resolve central migrations");
    let selected = all
        .migrations
        .iter()
        .filter(|migration| migration.version <= version)
        .cloned()
        .collect::<Vec<_>>();
    let migrator = Migrator {
        migrations: Cow::Owned(selected),
        ..Migrator::DEFAULT
    };

    migrator.run(pool).await.expect("run selected migrations");
}

async fn seed_core_user(pool: &PgPool, discord_id: i64) {
    sqlx::query("INSERT INTO core.users (discord_id) VALUES ($1)")
        .bind(discord_id)
        .execute(pool)
        .await
        .expect("seed core user");
}

#[allow(clippy::too_many_arguments)]
async fn insert_steam_link(
    pool: &PgPool,
    discord_id: i64,
    steam_id: &str,
    steam_id64: i64,
    verified: bool,
    primary_account: bool,
    linked_at: &str,
    updated_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO core.steam_links (
             discord_id,
             steam_id,
             steam_id64,
             verified,
             primary_account,
             linked_at,
             updated_at
         )
         VALUES ($1, $2, $3, $4, $5, $6::timestamptz, $7::timestamptz)",
    )
    .bind(discord_id)
    .bind(steam_id)
    .bind(steam_id64)
    .bind(verified)
    .bind(primary_account)
    .bind(linked_at)
    .bind(updated_at)
    .execute(pool)
    .await?;

    Ok(())
}

fn database_error_code(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(db_error) => db_error.code().map(|code| code.into_owned()),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn steam_links_primary_unique_index_rejects_second_primary_for_same_user() {
    let admin_dsn = test_dsn();
    let admin = connect_pool(&admin_dsn)
        .await
        .expect("connect admin database");
    let dbname = fresh_db_name("primary_guard");
    create_fresh_db(&admin, &dbname).await;
    let db_dsn = swap_db(&admin_dsn, &dbname);
    let pool = connect_pool(&db_dsn)
        .await
        .expect("connect fresh migrated database");

    run_migrations_through(&pool, 15).await;

    let discord_id = 9_150_001_i64;
    seed_core_user(&pool, discord_id).await;
    insert_steam_link(
        &pool,
        discord_id,
        "76561198000015001",
        7_656_119_800_015_001,
        true,
        true,
        "2026-07-02T10:00:00Z",
        "2026-07-02T10:00:00Z",
    )
    .await
    .expect("insert first primary steam link");

    let second_primary = insert_steam_link(
        &pool,
        discord_id,
        "76561198000015002",
        7_656_119_800_015_002,
        true,
        true,
        "2026-07-02T10:01:00Z",
        "2026-07-02T10:01:00Z",
    )
    .await;

    let error = second_primary.expect_err("second primary insert must fail");
    let code = database_error_code(&error);
    println!("zahn1 second primary insert error code: {code:?}");
    assert_eq!(code.as_deref(), Some("23505"));

    pool.close().await;
    drop_db(&admin, &dbname).await;
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn steam_links_primary_migration_dedupes_existing_duplicates_before_index() {
    let admin_dsn = test_dsn();
    let admin = connect_pool(&admin_dsn)
        .await
        .expect("connect admin database");
    let dbname = fresh_db_name("primary_dedupe");
    create_fresh_db(&admin, &dbname).await;
    let db_dsn = swap_db(&admin_dsn, &dbname);
    let pool = connect_pool(&db_dsn)
        .await
        .expect("connect fresh pre-0015 database");

    run_migrations_through(&pool, 14).await;

    let discord_id = 9_150_002_i64;
    seed_core_user(&pool, discord_id).await;
    insert_steam_link(
        &pool,
        discord_id,
        "76561198000015003",
        7_656_119_800_015_003,
        false,
        true,
        "2026-07-02T12:00:00Z",
        "2026-07-02T12:00:00Z",
    )
    .await
    .expect("insert newer unverified primary");
    insert_steam_link(
        &pool,
        discord_id,
        "76561198000015004",
        7_656_119_800_015_004,
        true,
        true,
        "2026-07-01T12:00:00Z",
        "2026-07-01T12:00:00Z",
    )
    .await
    .expect("insert older verified primary");
    insert_steam_link(
        &pool,
        0,
        "76561198000015005",
        7_656_119_800_015_005,
        true,
        true,
        "2026-07-02T12:00:00Z",
        "2026-07-02T12:00:00Z",
    )
    .await
    .expect("insert first sentinel primary");
    insert_steam_link(
        &pool,
        0,
        "76561198000015006",
        7_656_119_800_015_006,
        true,
        true,
        "2026-07-02T12:01:00Z",
        "2026-07-02T12:01:00Z",
    )
    .await
    .expect("insert second sentinel primary");

    sqlx::raw_sql(STEAM_LINKS_PRIMARY_MIGRATION)
        .execute(&pool)
        .await
        .expect("apply steam_links primary migration");
    sqlx::raw_sql(STEAM_LINKS_PRIMARY_MIGRATION)
        .execute(&pool)
        .await
        .expect("re-run steam_links primary migration");

    let true_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT
           FROM core.steam_links
          WHERE discord_id = $1
            AND primary_account",
    )
    .bind(discord_id)
    .fetch_one(&pool)
    .await
    .expect("count true primary links after migration");
    let kept_steam_id: String = sqlx::query_scalar(
        "SELECT steam_id
           FROM core.steam_links
          WHERE discord_id = $1
            AND primary_account",
    )
    .bind(discord_id)
    .fetch_one(&pool)
    .await
    .expect("fetch kept primary steam id");
    let sentinel_true_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT
           FROM core.steam_links
          WHERE discord_id = 0
            AND primary_account",
    )
    .fetch_one(&pool)
    .await
    .expect("count sentinel primary links after migration");

    println!(
        "zahn2 true_count={true_count}, kept_steam_id={kept_steam_id}, sentinel_true_count={sentinel_true_count}"
    );
    assert_eq!(true_count, 1);
    assert_eq!(kept_steam_id, "76561198000015004");
    assert_eq!(sentinel_true_count, 2);

    pool.close().await;
    drop_db(&admin, &dbname).await;
    admin.close().await;
}
