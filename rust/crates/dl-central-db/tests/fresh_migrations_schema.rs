use std::{
    path::{Path, PathBuf},
    process::Command,
};

use dl_central_db::connect_pool;
use sqlx::PgPool;

// DB-Tests sind bewusst #[ignore]: CI Phase D (central_ci.sh) startet eine
// Wegwerf-Timescale-DB und fuehrt sie mit `cargo test -- --ignored` + echter DSN aus.
#[derive(Debug, PartialEq, Eq)]
struct ColumnInfo {
    data_type: String,
    udt_name: String,
    is_nullable: String,
    column_default: Option<String>,
}

fn test_dsn() -> String {
    std::env::var("CENTRAL_TEST_DSN")
        .expect("CENTRAL_TEST_DSN muss fuer ignored DB-Tests gesetzt sein; kein stilles Skippen")
}

fn fresh_db_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    format!("dlcentral_{}_{}", std::process::id(), nanos)
}

fn swap_db(dsn: &str, db: &str) -> String {
    let (base, _old) = dsn.rsplit_once('/').expect("DSN contains database path");
    format!("{base}/{db}")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("dl-central-db is under rust/crates")
        .to_path_buf()
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

async fn scalar_i64(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .expect("fetch scalar i64")
}

async fn table_columns(pool: &PgPool, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT column_name
           FROM information_schema.columns
          WHERE table_schema = 'core'
            AND table_name = $1
          ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|err| panic!("columns for core.{table}: {err}"))
}

async fn column(pool: &PgPool, table: &str, name: &str) -> ColumnInfo {
    let (data_type, udt_name, is_nullable, column_default) = sqlx::query_as(
        "SELECT data_type, udt_name, is_nullable, column_default
           FROM information_schema.columns
          WHERE table_schema = 'core'
            AND table_name = $1
            AND column_name = $2",
    )
    .bind(table)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("column core.{table}.{name}: {err}"));

    ColumnInfo {
        data_type,
        udt_name,
        is_nullable,
        column_default,
    }
}

async fn assert_column(
    pool: &PgPool,
    table: &str,
    name: &str,
    data_type: &str,
    udt_name: &str,
    is_nullable: &str,
    column_default: Option<&str>,
) {
    let expected = ColumnInfo {
        data_type: data_type.to_string(),
        udt_name: udt_name.to_string(),
        is_nullable: is_nullable.to_string(),
        column_default: column_default.map(str::to_string),
    };

    assert_eq!(
        column(pool, table, name).await,
        expected,
        "contract for core.{table}.{name}"
    );
}

async fn primary_key_columns(pool: &PgPool, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT a.attname
           FROM pg_index i
           JOIN pg_class t ON t.oid = i.indrelid
           JOIN pg_namespace n ON n.oid = t.relnamespace
           JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
           JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
          WHERE n.nspname = 'core'
            AND t.relname = $1
            AND i.indisprimary
          ORDER BY k.ord",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|err| panic!("primary key columns for core.{table}: {err}"))
}

async fn steam_links_user_fk_contract(pool: &PgPool) -> Vec<(Vec<String>, Vec<String>, String)> {
    sqlx::query_as(
        "SELECT array_agg(child_attr.attname::text ORDER BY child_key.ord) AS child_columns,
                array_agg(parent_attr.attname::text ORDER BY child_key.ord) AS parent_columns,
                c.confdeltype::text AS on_delete
           FROM pg_constraint c
           JOIN pg_class child ON child.oid = c.conrelid
           JOIN pg_namespace child_ns ON child_ns.oid = child.relnamespace
           JOIN pg_class parent ON parent.oid = c.confrelid
           JOIN pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
           JOIN unnest(c.conkey) WITH ORDINALITY AS child_key(attnum, ord) ON true
           JOIN pg_attribute child_attr
             ON child_attr.attrelid = child.oid
            AND child_attr.attnum = child_key.attnum
           JOIN unnest(c.confkey) WITH ORDINALITY AS parent_key(attnum, ord)
             ON parent_key.ord = child_key.ord
           JOIN pg_attribute parent_attr
             ON parent_attr.attrelid = parent.oid
            AND parent_attr.attnum = parent_key.attnum
          WHERE c.contype = 'f'
            AND child_ns.nspname = 'core'
            AND child.relname = 'steam_links'
            AND parent_ns.nspname = 'core'
            AND parent.relname = 'users'
          GROUP BY c.oid, c.confdeltype
          ORDER BY c.conname",
    )
    .fetch_all(pool)
    .await
    .expect("steam_links -> users FK contract")
}

async fn exact_steam_id64_btree_index_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM (
                SELECT array_agg(a.attname::text ORDER BY k.ord) AS columns
                  FROM pg_index i
                  JOIN pg_class t ON t.oid = i.indrelid
                  JOIN pg_namespace n ON n.oid = t.relnamespace
                  JOIN pg_class idx ON idx.oid = i.indexrelid
                  JOIN pg_am am ON am.oid = idx.relam
                  JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
                  JOIN pg_attribute a
                    ON a.attrelid = t.oid
                   AND a.attnum = k.attnum
                 WHERE n.nspname = 'core'
                   AND t.relname = 'steam_links'
                   AND NOT i.indisprimary
                   AND am.amname = 'btree'
                 GROUP BY i.indexrelid
           ) indexes
          WHERE columns = ARRAY['steam_id64']::text[]",
    )
    .fetch_one(pool)
    .await
    .expect("exact steam_id64 btree index count")
}

async fn migration_row_signature(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "SELECT version::text
                || '|' || description
                || '|' || success::text
                || '|' || encode(checksum, 'hex')
                || '|' || execution_time::text
                || '|' || installed_on::text
           FROM _sqlx_migrations
          WHERE version = 1
            AND description = 'core and schemas'
            AND success",
    )
    .fetch_one(pool)
    .await
    .expect("migration version 1 row")
}

fn migrator_command() -> Command {
    if let Ok(binary) = std::env::var("CARGO_BIN_EXE_dl-central-migrate") {
        return Command::new(binary);
    }

    if let Some(binary) = option_env!("CARGO_BIN_EXE_dl-central-migrate") {
        return Command::new(binary);
    }

    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
    command.current_dir(workspace_root()).args([
        "run",
        "--quiet",
        "-p",
        "dl-central-migrate",
        "--",
    ]);
    command
}

fn run_migrator(dsn: &str, label: &str) {
    let output = migrator_command()
        .env("DEADLOCK_CENTRAL_DSN", dsn)
        .output()
        .unwrap_or_else(|err| panic!("{label}: start dl-central-migrate: {err}"));

    assert!(
        output.status.success(),
        "{label}: dl-central-migrate exited with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN; via Test-Skript/CI laufen lassen"]
async fn pool_connects_and_pings() {
    let dsn = test_dsn();
    let pool = connect_pool(&dsn).await.expect("connect test database");
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("select 1");
    assert_eq!(one, 1);
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN; via Test-Skript/CI laufen lassen"]
async fn dl_central_migrate_builds_contract_schema_and_is_idempotent() {
    let admin_dsn = test_dsn();
    let admin = connect_pool(&admin_dsn)
        .await
        .expect("connect admin database");
    let dbname = fresh_db_name();

    create_fresh_db(&admin, &dbname).await;
    let db_dsn = swap_db(&admin_dsn, &dbname);

    run_migrator(&db_dsn, "first run");
    let pool = connect_pool(&db_dsn)
        .await
        .expect("connect fresh migrated database");

    let migration_count_after_first = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE version = 1
            AND description = 'core and schemas'
            AND success",
    )
    .await;
    assert_eq!(migration_count_after_first, 1);
    let migration_signature_after_first = migration_row_signature(&pool).await;

    run_migrator(&db_dsn, "second run");

    let migration_count_after_second = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE version = 1
            AND description = 'core and schemas'
            AND success",
    )
    .await;
    assert_eq!(migration_count_after_second, 1);
    assert_eq!(
        migration_row_signature(&pool).await,
        migration_signature_after_first,
        "second migrator run must be a no-op for migration version 1"
    );

    let schema_count = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM information_schema.schemata
          WHERE schema_name IN (
              'core',
              'coaching',
              'scrim',
              'steam',
              'turnier',
              'patchnotes',
              'activity'
          )",
    )
    .await;
    assert_eq!(schema_count, 7);

    let timescaledb_count = scalar_i64(
        &pool,
        "SELECT count(*) FROM pg_extension WHERE extname = 'timescaledb'",
    )
    .await;
    assert_eq!(timescaledb_count, 1);

    assert_eq!(
        table_columns(&pool, "users").await,
        vec![
            "discord_id",
            "username",
            "global_name",
            "avatar",
            "first_seen",
            "last_seen",
            "raw"
        ]
    );
    assert_eq!(
        primary_key_columns(&pool, "users").await,
        vec!["discord_id"]
    );
    assert_column(&pool, "users", "discord_id", "bigint", "int8", "NO", None).await;
    assert_column(&pool, "users", "username", "text", "text", "YES", None).await;
    assert_column(&pool, "users", "global_name", "text", "text", "YES", None).await;
    assert_column(&pool, "users", "avatar", "text", "text", "YES", None).await;
    assert_column(
        &pool,
        "users",
        "first_seen",
        "timestamp with time zone",
        "timestamptz",
        "NO",
        Some("now()"),
    )
    .await;
    assert_column(
        &pool,
        "users",
        "last_seen",
        "timestamp with time zone",
        "timestamptz",
        "NO",
        Some("now()"),
    )
    .await;
    assert_column(&pool, "users", "raw", "jsonb", "jsonb", "YES", None).await;

    assert_eq!(
        table_columns(&pool, "steam_links").await,
        vec!["discord_id", "steam_id64", "verified", "linked_at"]
    );
    assert_eq!(
        primary_key_columns(&pool, "steam_links").await,
        vec!["discord_id", "steam_id64"]
    );
    assert_column(
        &pool,
        "steam_links",
        "discord_id",
        "bigint",
        "int8",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "steam_id64",
        "bigint",
        "int8",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "verified",
        "boolean",
        "bool",
        "NO",
        Some("false"),
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "linked_at",
        "timestamp with time zone",
        "timestamptz",
        "NO",
        Some("now()"),
    )
    .await;

    assert_eq!(
        steam_links_user_fk_contract(&pool).await,
        vec![(
            vec!["discord_id".to_string()],
            vec!["discord_id".to_string()],
            "c".to_string()
        )],
        "core.steam_links.discord_id must FK to core.users.discord_id ON DELETE CASCADE"
    );

    assert_eq!(
        exact_steam_id64_btree_index_count(&pool).await,
        1,
        "expected exactly one non-PK btree index with columns exactly [steam_id64]"
    );

    pool.close().await;
    drop_db(&admin, &dbname).await;
}
