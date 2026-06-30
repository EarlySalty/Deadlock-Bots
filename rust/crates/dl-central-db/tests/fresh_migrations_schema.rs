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

async fn table_columns_in_schema(pool: &PgPool, schema: &str, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT column_name
           FROM information_schema.columns
          WHERE table_schema = $1
            AND table_name = $2
          ORDER BY ordinal_position",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|err| panic!("columns for {schema}.{table}: {err}"))
}

async fn table_columns(pool: &PgPool, table: &str) -> Vec<String> {
    table_columns_in_schema(pool, "core", table).await
}

async fn column_in_schema(pool: &PgPool, schema: &str, table: &str, name: &str) -> ColumnInfo {
    let (data_type, udt_name, is_nullable, column_default) = sqlx::query_as(
        "SELECT data_type, udt_name, is_nullable, column_default
           FROM information_schema.columns
          WHERE table_schema = $1
            AND table_name = $2
            AND column_name = $3",
    )
    .bind(schema)
    .bind(table)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("column {schema}.{table}.{name}: {err}"));

    ColumnInfo {
        data_type,
        udt_name,
        is_nullable,
        column_default,
    }
}

#[allow(clippy::too_many_arguments)]
async fn assert_column_in_schema(
    pool: &PgPool,
    schema: &str,
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
        column_in_schema(pool, schema, table, name).await,
        expected,
        "contract for {schema}.{table}.{name}"
    );
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
    assert_column_in_schema(
        pool,
        "core",
        table,
        name,
        data_type,
        udt_name,
        is_nullable,
        column_default,
    )
    .await;
}

async fn primary_key_columns_in_schema(pool: &PgPool, schema: &str, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT a.attname
           FROM pg_index i
           JOIN pg_class t ON t.oid = i.indrelid
           JOIN pg_namespace n ON n.oid = t.relnamespace
           JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
           JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
          WHERE n.nspname = $1
            AND t.relname = $2
            AND i.indisprimary
          ORDER BY k.ord",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|err| panic!("primary key columns for {schema}.{table}: {err}"))
}

async fn primary_key_columns(pool: &PgPool, table: &str) -> Vec<String> {
    primary_key_columns_in_schema(pool, "core", table).await
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

async fn steam_links_owner_unique_index_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM (
                SELECT array_agg(a.attname::text ORDER BY k.ord) AS columns,
                       pg_get_expr(i.indpred, i.indrelid) AS predicate
                  FROM pg_index i
                  JOIN pg_class t ON t.oid = i.indrelid
                  JOIN pg_namespace n ON n.oid = t.relnamespace
                  JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
                  JOIN pg_attribute a
                    ON a.attrelid = t.oid
                   AND a.attnum = k.attnum
                 WHERE n.nspname = 'core'
                   AND t.relname = 'steam_links'
                   AND i.indisunique
                   AND NOT i.indisprimary
                 GROUP BY i.indexrelid, i.indpred, i.indrelid
           ) indexes
          WHERE columns = ARRAY['steam_id']::text[]
            AND predicate LIKE '%discord_id <> 0%'",
    )
    .fetch_one(pool)
    .await
    .expect("steam_links owner unique index count")
}

async fn trigger_names(pool: &PgPool, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT tg.tgname
           FROM pg_trigger tg
           JOIN pg_class t ON t.oid = tg.tgrelid
           JOIN pg_namespace n ON n.oid = t.relnamespace
          WHERE n.nspname = 'core'
            AND t.relname = $1
            AND NOT tg.tgisinternal
          ORDER BY tg.tgname",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|err| panic!("trigger names for core.{table}: {err}"))
}

async fn migration_row_signature(pool: &PgPool, version: i64, description: &str) -> String {
    sqlx::query_scalar(
        "SELECT version::text
                || '|' || description
                || '|' || success::text
                || '|' || encode(checksum, 'hex')
                || '|' || execution_time::text
                || '|' || installed_on::text
           FROM _sqlx_migrations
          WHERE version = $1
            AND description = $2
            AND success",
    )
    .bind(version)
    .bind(description)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("migration version {version} row: {err}"))
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
          WHERE version BETWEEN 1 AND 11
            AND success",
    )
    .await;
    assert_eq!(migration_count_after_first, 11);
    let migration_1_signature_after_first =
        migration_row_signature(&pool, 1, "core and schemas").await;
    let migration_2_signature_after_first =
        migration_row_signature(&pool, 2, "sp1 schemas and core").await;
    let migration_11_signature_after_first =
        migration_row_signature(&pool, 11, "barrier orphans and cross fks").await;

    run_migrator(&db_dsn, "second run");

    let migration_count_after_second = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE version BETWEEN 1 AND 11
            AND success",
    )
    .await;
    assert_eq!(migration_count_after_second, 11);
    assert_eq!(
        migration_row_signature(&pool, 1, "core and schemas").await,
        migration_1_signature_after_first,
        "second migrator run must be a no-op for migration version 1"
    );
    assert_eq!(
        migration_row_signature(&pool, 2, "sp1 schemas and core").await,
        migration_2_signature_after_first,
        "second migrator run must be a no-op for migration version 2"
    );
    assert_eq!(
        migration_row_signature(&pool, 11, "barrier orphans and cross fks").await,
        migration_11_signature_after_first,
        "second migrator run must be a no-op for migration version 11"
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
              'activity',
              'voice',
              'tierlist',
              'moderation',
              'bot',
              'clips',
              'content'
          )",
    )
    .await;
    assert_eq!(schema_count, 13);

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
        vec![
            "discord_id",
            "steam_id64",
            "verified",
            "linked_at",
            "steam_id",
            "steam_display_name",
            "primary_account",
            "updated_at",
            "legacy_ref",
            "migrated_at",
            "deadlock_rank",
            "deadlock_subrank",
            "deadlock_badge_level",
            "deadlock_rank_name",
            "deadlock_rank_updated_at",
            "is_steam_friend"
        ]
    );
    assert_eq!(
        primary_key_columns(&pool, "steam_links").await,
        vec!["discord_id", "steam_id"]
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
        "YES",
        None,
    )
    .await;
    assert_column(&pool, "steam_links", "steam_id", "text", "text", "NO", None).await;
    assert_column(
        &pool,
        "steam_links",
        "steam_display_name",
        "text",
        "text",
        "YES",
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
        "primary_account",
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
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "updated_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "legacy_ref",
        "text",
        "text",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "migrated_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "deadlock_rank",
        "integer",
        "int4",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "deadlock_subrank",
        "integer",
        "int4",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "deadlock_badge_level",
        "integer",
        "int4",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "deadlock_rank_name",
        "text",
        "text",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "deadlock_rank_updated_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "is_steam_friend",
        "boolean",
        "bool",
        "NO",
        Some("false"),
    )
    .await;

    assert_eq!(
        trigger_names(&pool, "steam_links").await,
        vec![
            "trg_steam_links_owner_guard_insert".to_string(),
            "trg_steam_links_owner_guard_update".to_string(),
            "trg_steam_links_user_guard_insert".to_string(),
            "trg_steam_links_user_guard_update".to_string()
        ],
        "core.steam_links keeps owner guard and nonzero-user guard triggers"
    );

    assert_eq!(
        trigger_names(&pool, "users").await,
        vec!["trg_core_users_steam_links_delete_cascade".to_string()],
        "core.users delete must cascade to core.steam_links through the SP1 trigger"
    );

    assert_eq!(
        exact_steam_id64_btree_index_count(&pool).await,
        1,
        "expected exactly one non-PK btree index with columns exactly [steam_id64]"
    );
    assert_eq!(
        steam_links_owner_unique_index_count(&pool).await,
        1,
        "expected one partial unique owner index on steam_id where discord_id != 0"
    );

    assert_eq!(
        table_columns(&pool, "meta_users").await,
        vec![
            "id",
            "username",
            "display_name",
            "avatar_url",
            "role",
            "created_at"
        ]
    );
    assert_eq!(primary_key_columns(&pool, "meta_users").await, vec!["id"]);
    assert_column(&pool, "meta_users", "id", "bigint", "int8", "NO", None).await;
    assert_column(&pool, "meta_users", "username", "text", "text", "YES", None).await;
    assert_column(
        &pool,
        "meta_users",
        "display_name",
        "text",
        "text",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "meta_users",
        "avatar_url",
        "text",
        "text",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "meta_users",
        "role",
        "text",
        "text",
        "NO",
        Some("'user'::text"),
    )
    .await;
    assert_column(
        &pool,
        "meta_users",
        "created_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        Some("now()"),
    )
    .await;

    assert_eq!(
        table_columns(&pool, "user_privacy").await,
        vec!["user_id", "opted_out", "deleted_at", "reason", "updated_at"]
    );
    assert_eq!(
        primary_key_columns(&pool, "user_privacy").await,
        vec!["user_id"]
    );
    assert_column(
        &pool,
        "user_privacy",
        "user_id",
        "bigint",
        "int8",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_privacy",
        "opted_out",
        "boolean",
        "bool",
        "NO",
        Some("false"),
    )
    .await;
    assert_column(
        &pool,
        "user_privacy",
        "deleted_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column(&pool, "user_privacy", "reason", "text", "text", "YES", None).await;
    assert_column(
        &pool,
        "user_privacy",
        "updated_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        Some("now()"),
    )
    .await;

    assert_eq!(
        table_columns_in_schema(&pool, "activity", "live_player_state").await,
        vec![
            "steam_id",
            "last_gameid",
            "last_server_id",
            "last_seen_at",
            "in_deadlock_now",
            "in_match_now_strict",
            "deadlock_stage",
            "deadlock_minutes",
            "deadlock_localized",
            "deadlock_hero",
            "deadlock_party_hint",
            "deadlock_updated_at"
        ]
    );
    assert_eq!(
        primary_key_columns_in_schema(&pool, "activity", "live_player_state").await,
        vec!["steam_id"]
    );
    assert_column_in_schema(
        &pool,
        "activity",
        "live_player_state",
        "steam_id",
        "text",
        "text",
        "NO",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "activity",
        "live_player_state",
        "last_seen_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "activity",
        "live_player_state",
        "in_deadlock_now",
        "boolean",
        "bool",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "activity",
        "live_player_state",
        "in_match_now_strict",
        "boolean",
        "bool",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "activity",
        "live_player_state",
        "deadlock_minutes",
        "integer",
        "int4",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "activity",
        "live_player_state",
        "deadlock_updated_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;

    assert_eq!(
        table_columns(&pool, "user_data").await,
        vec![
            "user_id",
            "custom_interval",
            "paused_until",
            "created_at",
            "updated_at"
        ]
    );
    assert_eq!(
        primary_key_columns(&pool, "user_data").await,
        vec!["user_id"]
    );
    assert_column(&pool, "user_data", "user_id", "bigint", "int8", "NO", None).await;
    assert_column(
        &pool,
        "user_data",
        "custom_interval",
        "integer",
        "int4",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_data",
        "paused_until",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_data",
        "created_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_data",
        "updated_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;

    assert_eq!(
        table_columns(&pool, "user_mod_tags").await,
        vec![
            "user_id",
            "mod_tag",
            "set_by",
            "reason",
            "set_at",
            "expires_at"
        ]
    );
    assert_eq!(
        primary_key_columns(&pool, "user_mod_tags").await,
        vec!["user_id", "mod_tag"]
    );
    assert_column(
        &pool,
        "user_mod_tags",
        "user_id",
        "bigint",
        "int8",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_mod_tags",
        "mod_tag",
        "text",
        "text",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_mod_tags",
        "set_by",
        "bigint",
        "int8",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_mod_tags",
        "set_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "user_mod_tags",
        "expires_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;

    assert_eq!(
        table_columns(&pool, "user_tags").await,
        vec!["user_id", "tag_key", "tag_value", "set_at"]
    );
    assert_eq!(
        primary_key_columns(&pool, "user_tags").await,
        vec!["user_id", "tag_key"]
    );
    assert_column(&pool, "user_tags", "user_id", "bigint", "int8", "NO", None).await;
    assert_column(&pool, "user_tags", "tag_key", "text", "text", "NO", None).await;
    assert_column(&pool, "user_tags", "tag_value", "text", "text", "NO", None).await;
    assert_column(
        &pool,
        "user_tags",
        "set_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;

    pool.close().await;
    drop_db(&admin, &dbname).await;
}
