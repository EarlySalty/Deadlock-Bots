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

fn server_config_table_contracts() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            "desired_categories",
            vec![
                "guild_id",
                "category_id",
                "name",
                "position",
                "note",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "desired_channels",
            vec![
                "guild_id",
                "channel_id",
                "name",
                "channel_type",
                "topic",
                "position",
                "parent_category_id",
                "nsfw",
                "bitrate",
                "user_limit",
                "rate_limit_per_user",
                "status",
                "note",
                "created_at",
                "updated_at",
                "default_auto_archive_duration",
            ],
        ),
        (
            "desired_roles",
            vec![
                "guild_id",
                "role_id",
                "name",
                "color",
                "hoist",
                "mentionable",
                "managed",
                "permissions_bitmask",
                "position",
                "note",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "desired_permission_overwrites",
            vec![
                "guild_id",
                "channel_id",
                "target_type",
                "target_id",
                "allow_bits",
                "deny_bits",
                "note",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "desired_bot_messages",
            vec![
                "guild_id",
                "channel_id",
                "message_key",
                "message_kind",
                "message_id",
                "expected_hash",
                "metadata",
                "note",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "live_snapshots",
            vec![
                "snapshot_id",
                "guild_id",
                "captured_at",
                "source",
                "metadata",
            ],
        ),
        (
            "rollback_exports",
            vec![
                "rollback_export_id",
                "guild_id",
                "snapshot_id",
                "created_by_user_id",
                "artifact_hash",
                "artifact_json",
                "metadata",
                "created_at",
                "expires_at",
            ],
        ),
        (
            "live_snapshot_categories",
            vec![
                "snapshot_id",
                "captured_at",
                "guild_id",
                "category_id",
                "name",
                "position",
            ],
        ),
        (
            "live_snapshot_channels",
            vec![
                "snapshot_id",
                "captured_at",
                "guild_id",
                "channel_id",
                "name",
                "channel_type",
                "topic",
                "position",
                "parent_category_id",
                "nsfw",
                "bitrate",
                "user_limit",
                "rate_limit_per_user",
                "status",
                "default_auto_archive_duration",
            ],
        ),
        (
            "live_snapshot_roles",
            vec![
                "snapshot_id",
                "captured_at",
                "guild_id",
                "role_id",
                "name",
                "color",
                "hoist",
                "mentionable",
                "managed",
                "permissions_bitmask",
                "position",
            ],
        ),
        (
            "live_snapshot_permission_overwrites",
            vec![
                "snapshot_id",
                "captured_at",
                "guild_id",
                "channel_id",
                "target_type",
                "target_id",
                "allow_bits",
                "deny_bits",
            ],
        ),
        (
            "live_snapshot_bot_messages",
            vec![
                "snapshot_id",
                "captured_at",
                "guild_id",
                "channel_id",
                "message_key",
                "message_kind",
                "message_id",
                "observed_hash",
                "metadata",
            ],
        ),
        (
            "dynamic_namespaces",
            vec![
                "namespace_id",
                "guild_id",
                "namespace_key",
                "system_name",
                "object_kind",
                "match_rule_type",
                "match_rule",
                "foreign_writer",
                "active",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "documented_exceptions",
            vec![
                "exception_id",
                "guild_id",
                "exception_key",
                "exception_type",
                "object_kind",
                "object_id",
                "channel_id",
                "target_type",
                "target_id",
                "allow_bits",
                "deny_bits",
                "reason",
                "review_at",
                "expires_at",
                "metadata",
                "active",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "diff_previews",
            vec![
                "preview_id",
                "guild_id",
                "snapshot_id",
                "diff_hash",
                "diff_json",
                "human_summary",
                "created_by_user_id",
                "created_at",
                "applied_at",
            ],
        ),
        (
            "apply_runs",
            vec![
                "apply_run_id",
                "preview_id",
                "guild_id",
                "confirmed_diff_hash",
                "requested_by_user_id",
                "dry_run",
                "status",
                "result_json",
                "error_text",
                "started_at",
                "finished_at",
            ],
        ),
        (
            "auto_revert_whitelist",
            vec![
                "whitelist_id",
                "guild_id",
                "object_kind",
                "object_id",
                "target_type",
                "target_id",
                "permission_bits",
                "action",
                "reason",
                "active",
                "created_at",
            ],
        ),
        (
            "drift_events",
            vec![
                "drift_event_id",
                "guild_id",
                "snapshot_id",
                "preview_id",
                "detected_at",
                "last_seen_at",
                "resolved_at",
                "fingerprint",
                "event_kind",
                "severity",
                "object_kind",
                "object_id",
                "action",
                "diff_json",
                "filtered_dynamic",
                "documented_exception_id",
                "auto_revert_eligible",
                "status",
            ],
        ),
        (
            "adoption_events",
            vec![
                "adoption_event_id",
                "guild_id",
                "snapshot_id",
                "adopted_by_user_id",
                "object_kind",
                "object_id",
                "adopted_diff",
                "created_at",
            ],
        ),
    ]
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

async fn steam_links_one_primary_unique_index_count(pool: &PgPool) -> i64 {
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
          WHERE columns = ARRAY['discord_id']::text[]
            AND predicate LIKE '%primary_account%'
            AND predicate LIKE '%discord_id <> 0%'",
    )
    .fetch_one(pool)
    .await
    .expect("steam_links one-primary unique index count")
}

async fn rank_history_visibility_check_constraint_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM pg_constraint c
           JOIN pg_class t ON t.oid = c.conrelid
           JOIN pg_namespace n ON n.oid = t.relnamespace
          WHERE n.nspname = 'steam'
            AND t.relname = 'rank_history_visibility'
            AND c.contype = 'c'
            AND pg_get_constraintdef(c.oid) LIKE '%visibility%'
            AND pg_get_constraintdef(c.oid) LIKE '%''private''%'
            AND pg_get_constraintdef(c.oid) LIKE '%''members''%'
            AND pg_get_constraintdef(c.oid) LIKE '%''public''%'",
    )
    .fetch_one(pool)
    .await
    .expect("rank_history_visibility visibility check constraint count")
}

async fn rank_history_visibility_index_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM (
                SELECT idx.relname AS index_name,
                       array_agg(a.attname::text ORDER BY k.ord) AS columns
                  FROM pg_index i
                  JOIN pg_class t ON t.oid = i.indrelid
                  JOIN pg_namespace n ON n.oid = t.relnamespace
                  JOIN pg_class idx ON idx.oid = i.indexrelid
                  JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
                  JOIN pg_attribute a
                    ON a.attrelid = t.oid
                   AND a.attnum = k.attnum
                 WHERE n.nspname = 'steam'
                   AND t.relname = 'rank_history_visibility'
                   AND NOT i.indisprimary
                 GROUP BY i.indexrelid, idx.relname
           ) indexes
          WHERE index_name = 'rank_history_visibility_visibility_idx'
            AND columns = ARRAY['visibility']::text[]",
    )
    .fetch_one(pool)
    .await
    .expect("rank_history_visibility visibility index count")
}

async fn lfg_posts_unique_index_count(pool: &PgPool, column: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM (
                SELECT array_agg(a.attname::text ORDER BY k.ord) AS columns
                  FROM pg_index i
                  JOIN pg_class t ON t.oid = i.indrelid
                  JOIN pg_namespace n ON n.oid = t.relnamespace
                  JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
                  JOIN pg_attribute a
                    ON a.attrelid = t.oid
                   AND a.attnum = k.attnum
                 WHERE n.nspname = 'voice'
                   AND t.relname = 'lfg_posts'
                   AND i.indisunique
                   AND NOT i.indisprimary
                 GROUP BY i.indexrelid
           ) indexes
          WHERE columns = ARRAY[$1]::text[]",
    )
    .bind(column)
    .fetch_one(pool)
    .await
    .expect("lfg_posts unique index count")
}

async fn lfg_posts_owner_active_unique_index_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM (
                SELECT idx.relname AS index_name,
                       pg_get_expr(i.indpred, i.indrelid) AS predicate,
                       array_agg(a.attname::text ORDER BY k.ord) AS columns
                  FROM pg_index i
                  JOIN pg_class t ON t.oid = i.indrelid
                  JOIN pg_namespace n ON n.oid = t.relnamespace
                  JOIN pg_class idx ON idx.oid = i.indexrelid
                  JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
                  JOIN pg_attribute a
                    ON a.attrelid = t.oid
                   AND a.attnum = k.attnum
                 WHERE n.nspname = 'voice'
                   AND t.relname = 'lfg_posts'
                   AND i.indisunique
                   AND NOT i.indisprimary
                 GROUP BY i.indexrelid, idx.relname, i.indpred, i.indrelid
           ) indexes
          WHERE index_name = 'lfg_posts_owner_active_uidx'
            AND columns = ARRAY['owner_id']::text[]
            AND predicate LIKE '%status%'
            AND predicate LIKE '%creating%'
            AND predicate LIKE '%open%'",
    )
    .fetch_one(pool)
    .await
    .expect("lfg_posts owner active unique index count")
}

async fn lfg_posts_status_check_constraint_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM pg_constraint c
           JOIN pg_class t ON t.oid = c.conrelid
           JOIN pg_namespace n ON n.oid = t.relnamespace
          WHERE n.nspname = 'voice'
            AND t.relname = 'lfg_posts'
            AND c.contype = 'c'
            AND pg_get_constraintdef(c.oid) LIKE '%status%'
            AND pg_get_constraintdef(c.oid) LIKE '%''creating''%'
            AND pg_get_constraintdef(c.oid) LIKE '%''open''%'
            AND pg_get_constraintdef(c.oid) LIKE '%''closed''%'
            AND pg_get_constraintdef(c.oid) LIKE '%''expired''%'",
    )
    .fetch_one(pool)
    .await
    .expect("lfg_posts status check constraint count")
}

async fn steam_rank_history_covering_index_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM (
                SELECT idx.relname AS index_name,
                       pg_get_indexdef(i.indexrelid) AS definition,
                       pg_get_expr(i.indpred, i.indrelid) AS predicate,
                       array_agg(a.attname::text ORDER BY k.ord)
                           FILTER (WHERE k.ord <= i.indnkeyatts) AS key_columns,
                       array_agg(a.attname::text ORDER BY k.ord)
                           FILTER (WHERE k.ord > i.indnkeyatts) AS include_columns
                  FROM pg_index i
                  JOIN pg_class t ON t.oid = i.indrelid
                  JOIN pg_namespace n ON n.oid = t.relnamespace
                  JOIN pg_class idx ON idx.oid = i.indexrelid
                  JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
                  JOIN pg_attribute a
                    ON a.attrelid = t.oid
                   AND a.attnum = k.attnum
                 WHERE n.nspname = 'steam'
                   AND t.relname = 'steam_rank_history'
                   AND NOT i.indisprimary
                 GROUP BY i.indexrelid, i.indrelid, i.indpred, i.indnkeyatts, idx.relname
           ) indexes
          WHERE index_name = 'steam_rank_history_user_captured_visible_cover_idx'
            AND key_columns = ARRAY['user_id', 'captured_at']::text[]
            AND include_columns = ARRAY['badge_level', 'rank_name']::text[]
            AND definition LIKE '%captured_at DESC%'
            AND predicate LIKE '%badge_level IS NOT NULL%'
            AND predicate LIKE '%rank_name IS NOT NULL%'",
    )
    .fetch_one(pool)
    .await
    .expect("steam_rank_history covering index count")
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

async fn trigger_definition(pool: &PgPool, table: &str, trigger: &str) -> String {
    sqlx::query_scalar(
        "SELECT pg_get_triggerdef(tg.oid)
           FROM pg_trigger tg
           JOIN pg_class t ON t.oid = tg.tgrelid
           JOIN pg_namespace n ON n.oid = t.relnamespace
          WHERE n.nspname = 'core'
            AND t.relname = $1
            AND tg.tgname = $2
            AND NOT tg.tgisinternal",
    )
    .bind(table)
    .bind(trigger)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("trigger definition for core.{table}.{trigger}: {err}"))
}

async fn assert_steam_link_reassign_enqueues_old_and_new_discord_ids(pool: &PgPool) {
    let old_discord_id = 9_960_001_i64;
    let new_discord_id = 9_960_002_i64;
    let steam_id = "fresh-reassign-linked-role";
    let steam_id64 = 9_960_001_i64;

    sqlx::query(
        "INSERT INTO core.users(discord_id, username)
         VALUES ($1, 'linked-role-old'), ($2, 'linked-role-new')
         ON CONFLICT(discord_id) DO NOTHING",
    )
    .bind(old_discord_id)
    .bind(new_discord_id)
    .execute(pool)
    .await
    .expect("seed reassign core users");
    sqlx::query(
        "INSERT INTO core.steam_links
             (discord_id, steam_id, steam_id64, verified, primary_account, updated_at)
         VALUES ($1, $2, $3, TRUE, TRUE, now())",
    )
    .bind(old_discord_id)
    .bind(steam_id)
    .bind(steam_id64)
    .execute(pool)
    .await
    .expect("seed reassign steam link");
    sqlx::query(
        "DELETE FROM core.discord_role_connection_sync_state
          WHERE discord_id IN ($1, $2)",
    )
    .bind(old_discord_id)
    .bind(new_discord_id)
    .execute(pool)
    .await
    .expect("clear initial trigger rows");

    sqlx::query(
        "UPDATE core.steam_links
            SET discord_id=$2
          WHERE discord_id=$1
            AND steam_id=$3",
    )
    .bind(old_discord_id)
    .bind(new_discord_id)
    .bind(steam_id)
    .execute(pool)
    .await
    .expect("reassign steam link");

    let rows: Vec<(i64, bool, String)> = sqlx::query_as(
        "SELECT discord_id, pending, reason
           FROM core.discord_role_connection_sync_state
          WHERE discord_id IN ($1, $2)
          ORDER BY discord_id",
    )
    .bind(old_discord_id)
    .bind(new_discord_id)
    .fetch_all(pool)
    .await
    .expect("reassign sync rows");

    assert_eq!(
        rows,
        vec![
            (old_discord_id, true, "steam_link_removed".to_string()),
            (new_discord_id, true, "steam_link_changed".to_string())
        ],
        "steam_links.discord_id reassignment enqueues old and new linked-role sync targets"
    );
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
          WHERE (version BETWEEN 1 AND 15 OR version IN (2026070311, 2026070312, 2026070320, 2026070330, 2026070335))
            AND success",
    )
    .await;
    assert_eq!(migration_count_after_first, 20);
    let journey_migration_count_after_first = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE version = 2026070220
            AND success",
    )
    .await;
    assert_eq!(journey_migration_count_after_first, 1);
    let rank_history_visibility_migration_count_after_first = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE version = 2026070260
            AND success",
    )
    .await;
    assert_eq!(rank_history_visibility_migration_count_after_first, 1);
    let migration_1_signature_after_first =
        migration_row_signature(&pool, 1, "core and schemas").await;
    let migration_2_signature_after_first =
        migration_row_signature(&pool, 2, "sp1 schemas and core").await;
    let migration_11_signature_after_first =
        migration_row_signature(&pool, 11, "barrier orphans and cross fks").await;
    let migration_12_signature_after_first =
        migration_row_signature(&pool, 12, "brain knowledge timeline").await;
    let migration_13_signature_after_first =
        migration_row_signature(&pool, 13, "brain insight records").await;
    let migration_14_signature_after_first =
        migration_row_signature(&pool, 14, "patchnotes identity sequences").await;
    let migration_15_signature_after_first =
        migration_row_signature(&pool, 15, "steam links one primary").await;
    let migration_2026070210_signature_after_first =
        migration_row_signature(&pool, 2026070210, "server config schema").await;
    let migration_2026070220_signature_after_first =
        migration_row_signature(&pool, 2026070220, "journey ingestion analytics").await;
    let migration_2026070260_signature_after_first =
        migration_row_signature(&pool, 2026070260, "rank history visibility").await;
    let migration_2026070311_signature_after_first =
        migration_row_signature(&pool, 2026070311, "steam rank history account scope").await;
    let migration_2026070312_signature_after_first =
        migration_row_signature(&pool, 2026070312, "steam friend requests task link").await;
    let migration_2026070320_signature_after_first =
        migration_row_signature(&pool, 2026070320, "lfg posts").await;
    let migration_2026070330_signature_after_first =
        migration_row_signature(&pool, 2026070330, "discord role connections").await;
    let migration_2026070335_signature_after_first =
        migration_row_signature(&pool, 2026070335, "lfg post ids").await;

    run_migrator(&db_dsn, "second run");

    let migration_count_after_second = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE (version BETWEEN 1 AND 15 OR version IN (2026070311, 2026070312, 2026070320, 2026070330, 2026070335))
            AND success",
    )
    .await;
    assert_eq!(migration_count_after_second, 20);
    let journey_migration_count_after_second = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE version = 2026070220
            AND success",
    )
    .await;
    assert_eq!(journey_migration_count_after_second, 1);
    let rank_history_visibility_migration_count_after_second = scalar_i64(
        &pool,
        "SELECT count(*)
           FROM _sqlx_migrations
          WHERE version = 2026070260
            AND success",
    )
    .await;
    assert_eq!(rank_history_visibility_migration_count_after_second, 1);
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
    assert_eq!(
        migration_row_signature(&pool, 12, "brain knowledge timeline").await,
        migration_12_signature_after_first,
        "second migrator run must be a no-op for migration version 12"
    );
    assert_eq!(
        migration_row_signature(&pool, 13, "brain insight records").await,
        migration_13_signature_after_first,
        "second migrator run must be a no-op for migration version 13"
    );
    assert_eq!(
        migration_row_signature(&pool, 14, "patchnotes identity sequences").await,
        migration_14_signature_after_first,
        "second migrator run must be a no-op for migration version 14"
    );
    assert_eq!(
        migration_row_signature(&pool, 15, "steam links one primary").await,
        migration_15_signature_after_first,
        "second migrator run must be a no-op for migration version 15"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070210, "server config schema").await,
        migration_2026070210_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070210"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070220, "journey ingestion analytics").await,
        migration_2026070220_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070220"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070260, "rank history visibility").await,
        migration_2026070260_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070260"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070311, "steam rank history account scope").await,
        migration_2026070311_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070311"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070312, "steam friend requests task link").await,
        migration_2026070312_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070312"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070320, "lfg posts").await,
        migration_2026070320_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070320"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070330, "discord role connections").await,
        migration_2026070330_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070330"
    );
    assert_eq!(
        migration_row_signature(&pool, 2026070335, "lfg post ids").await,
        migration_2026070335_signature_after_first,
        "second migrator run must be a no-op for migration version 2026070335"
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
              'content',
              'brain',
              'server_config'
          )",
    )
    .await;
    assert_eq!(schema_count, 15);

    let timescaledb_count = scalar_i64(
        &pool,
        "SELECT count(*) FROM pg_extension WHERE extname = 'timescaledb'",
    )
    .await;
    assert_eq!(timescaledb_count, 1);

    assert_eq!(
        table_columns_in_schema(&pool, "brain", "knowledge_events").await,
        vec![
            "id",
            "event_hash",
            "event_source",
            "source_table",
            "source_legacy_id",
            "source_document_id",
            "snapshot_id",
            "patch_event_id",
            "forum_claim_id",
            "entity_type",
            "entity_name",
            "subject",
            "event_type",
            "validity_status",
            "currentness",
            "trust_tier",
            "source_url",
            "occurred_at",
            "observed_at",
            "effective_from",
            "effective_to",
            "raw_text",
            "normalized_text",
            "evidence_quote",
            "old_value",
            "new_value",
            "confidence",
            "safety_labels",
            "source_references",
            "payload",
            "metadata",
            "created_at",
            "updated_at",
        ]
    );
    assert_eq!(
        primary_key_columns_in_schema(&pool, "brain", "knowledge_events").await,
        vec!["id"]
    );
    assert_column_in_schema(
        &pool,
        "brain",
        "knowledge_events",
        "payload",
        "jsonb",
        "jsonb",
        "NO",
        Some("'{}'::jsonb"),
    )
    .await;
    let server_config_tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name
           FROM information_schema.tables
          WHERE table_schema = 'server_config'
            AND table_type = 'BASE TABLE'
          ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .expect("server_config tables");
    assert_eq!(
        server_config_tables,
        vec![
            "adoption_events",
            "apply_runs",
            "auto_revert_whitelist",
            "desired_bot_messages",
            "desired_categories",
            "desired_channels",
            "desired_permission_overwrites",
            "desired_roles",
            "diff_previews",
            "documented_exceptions",
            "drift_events",
            "dynamic_namespaces",
            "live_snapshot_bot_messages",
            "live_snapshot_categories",
            "live_snapshot_channels",
            "live_snapshot_permission_overwrites",
            "live_snapshot_roles",
            "live_snapshots",
            "rollback_exports",
        ]
    );
    for (table, columns) in server_config_table_contracts() {
        assert_eq!(
            table_columns_in_schema(&pool, "server_config", table).await,
            columns,
            "server_config.{table} columns"
        );
    }
    assert_eq!(
        primary_key_columns_in_schema(&pool, "server_config", "desired_channels").await,
        vec!["guild_id", "channel_id"]
    );
    assert_column_in_schema(
        &pool,
        "brain",
        "knowledge_events",
        "currentness",
        "text",
        "text",
        "NO",
        None,
    )
    .await;
    assert_eq!(
        table_columns_in_schema(&pool, "brain", "current_entity_state").await,
        vec![
            "id",
            "entity_type",
            "entity_name",
            "state_kind",
            "winning_event_id",
            "winning_snapshot_id",
            "source",
            "source_priority",
            "valid_from",
            "observed_at",
            "content_hash",
            "payload",
            "metadata",
            "updated_at",
        ]
    );
    assert_eq!(
        table_columns_in_schema(&pool, "brain", "insight_records").await,
        vec![
            "id",
            "insight_hash",
            "insight_type",
            "entity_type",
            "entity_name",
            "subject",
            "summary",
            "reason",
            "validity_status",
            "currentness",
            "trust_tier",
            "confidence",
            "occurred_at",
            "observed_at",
            "source_patch_event_ids",
            "source_urls",
            "source_references",
            "payload",
            "metadata",
            "created_at",
            "updated_at",
        ]
    );
    assert_eq!(
        primary_key_columns_in_schema(&pool, "brain", "insight_records").await,
        vec!["id"]
    );
    assert_column_in_schema(
        &pool,
        "brain",
        "insight_records",
        "source_patch_event_ids",
        "ARRAY",
        "_int8",
        "NO",
        Some("'{}'::bigint[]"),
    )
    .await;
    assert_column_in_schema(
        &pool,
        "brain",
        "insight_records",
        "source_references",
        "jsonb",
        "jsonb",
        "NO",
        Some("'[]'::jsonb"),
    )
    .await;

    assert_eq!(
        table_columns_in_schema(&pool, "activity", "journey_events").await,
        vec![
            "id",
            "user_id",
            "guild_id",
            "event_type",
            "event_source",
            "actor_kind",
            "occurred_at",
            "channel_id",
            "message_id",
            "metadata",
        ]
    );
    assert_eq!(
        primary_key_columns_in_schema(&pool, "activity", "journey_events").await,
        vec!["id"]
    );
    assert_column_in_schema(
        &pool,
        "activity",
        "journey_events",
        "metadata",
        "jsonb",
        "jsonb",
        "NO",
        Some("'{}'::jsonb"),
    )
    .await;
    assert_eq!(
        table_columns_in_schema(&pool, "activity", "journey_user_state").await,
        vec![
            "user_id",
            "guild_id",
            "joined_at",
            "screening_completed_at",
            "native_onboarding_completed_at",
            "weiche_choice",
            "steam_linked_at",
            "invite_friend_request_sent_at",
            "invite_friend_request_accepted_at",
            "invite_sent_at",
            "invite_accepted_at",
            "invite_actor_kind",
            "first_message_at",
            "first_voice_at",
            "first_match_at",
            "squad_joined_at",
            "first_interaction_at",
            "streamer_contact_activated_at",
            "opt_out_at",
            "last_event_at",
            "last_event_type",
            "metadata",
            "updated_at",
        ]
    );
    assert_eq!(
        primary_key_columns_in_schema(&pool, "activity", "journey_user_state").await,
        vec!["user_id", "guild_id"]
    );
    assert_eq!(
        table_columns_in_schema(&pool, "activity", "message_metadata_events").await,
        vec![
            "id",
            "user_id",
            "guild_id",
            "channel_id",
            "message_id",
            "occurred_at",
            "message_length",
            "has_attachment",
            "attachment_count",
            "is_reply",
        ]
    );
    assert_eq!(
        table_columns_in_schema(&pool, "activity", "voice_metadata_events").await,
        vec![
            "id",
            "user_id",
            "guild_id",
            "channel_id",
            "event_type",
            "occurred_at",
            "duration_seconds",
            "from_channel_id",
            "to_channel_id",
        ]
    );
    assert_eq!(
        table_columns_in_schema(&pool, "activity", "voice_open_sessions").await,
        vec![
            "user_id",
            "guild_id",
            "channel_id",
            "joined_at",
            "updated_at"
        ]
    );
    assert_eq!(
        table_columns_in_schema(&pool, "activity", "interaction_events").await,
        vec![
            "id",
            "user_id",
            "guild_id",
            "channel_id",
            "message_id",
            "interaction_id",
            "interaction_kind",
            "route",
            "occurred_at",
        ]
    );
    assert_eq!(
        table_columns_in_schema(&pool, "activity", "message_daily_aggregates").await,
        vec![
            "day",
            "guild_id",
            "channel_id",
            "message_count",
            "total_message_length",
            "attachment_message_count",
            "reply_message_count",
            "distinct_user_count",
        ]
    );
    assert!(
        !table_columns_in_schema(&pool, "activity", "message_daily_aggregates")
            .await
            .contains(&"user_id".to_string())
    );

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
            "is_steam_friend",
            "friend_bot_account_id",
            "unlink_reason",
            "refriend_attempted_at"
        ]
    );
    assert_eq!(
        primary_key_columns(&pool, "steam_links").await,
        vec!["discord_id", "steam_id"]
    );
    // Multi-Bot: welcher Bot-Account ist mit diesem Nutzer befreundet (NULL = keiner).
    assert_column(
        &pool,
        "steam_links",
        "friend_bot_account_id",
        "smallint",
        "int2",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "unlink_reason",
        "text",
        "text",
        "YES",
        None,
    )
    .await;
    assert_column(
        &pool,
        "steam_links",
        "refriend_attempted_at",
        "timestamp with time zone",
        "timestamptz",
        "YES",
        None,
    )
    .await;
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
            "trg_steam_links_role_connection_sync_delete".to_string(),
            "trg_steam_links_role_connection_sync_insert".to_string(),
            "trg_steam_links_role_connection_sync_update".to_string(),
            "trg_steam_links_user_guard_insert".to_string(),
            "trg_steam_links_user_guard_update".to_string()
        ],
        "core.steam_links keeps owner/user guards and linked-role sync triggers"
    );
    assert!(
        trigger_definition(
            &pool,
            "steam_links",
            "trg_steam_links_role_connection_sync_update"
        )
        .await
        .contains("UPDATE OF discord_id, verified, primary_account"),
        "linked-role sync update trigger must fire on discord_id changes"
    );
    assert_steam_link_reassign_enqueues_old_and_new_discord_ids(&pool).await;

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
        steam_links_one_primary_unique_index_count(&pool).await,
        1,
        "expected one partial unique primary index on discord_id where primary_account and discord_id != 0"
    );

    assert_column_in_schema(
        &pool,
        "steam",
        "steam_rank_history",
        "steam_id",
        "bigint",
        "int8",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "steam",
        "steam_friend_requests",
        "task_id",
        "bigint",
        "int8",
        "YES",
        None,
    )
    .await;

    assert_eq!(
        table_columns_in_schema(&pool, "voice", "lfg_posts").await,
        vec![
            "guild_id",
            "forum_channel_id",
            "thread_id",
            "starter_message_id",
            "lane_id",
            "owner_id",
            "mode",
            "rank_min",
            "rank_max",
            "requested_slots",
            "status",
            "created_at",
            "updated_at",
            "expires_at",
            "closed_at",
            "last_render_hash",
            "last_post_edit_at",
            "id",
            "play_window",
        ]
    );
    assert_column_in_schema(
        &pool,
        "voice",
        "lfg_posts",
        "id",
        "bigint",
        "int8",
        "NO",
        Some("nextval('voice.lfg_posts_id_seq'::regclass)"),
    )
    .await;
    assert_column_in_schema(
        &pool,
        "voice",
        "lfg_posts",
        "thread_id",
        "bigint",
        "int8",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "voice",
        "lfg_posts",
        "status",
        "text",
        "text",
        "NO",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "voice",
        "lfg_posts",
        "starter_message_id",
        "bigint",
        "int8",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "voice",
        "lfg_posts",
        "lane_id",
        "bigint",
        "int8",
        "YES",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "voice",
        "lfg_posts",
        "expires_at",
        "timestamp with time zone",
        "timestamptz",
        "NO",
        None,
    )
    .await;
    assert_eq!(
        lfg_posts_unique_index_count(&pool, "thread_id").await,
        1,
        "voice.lfg_posts.thread_id remains unique"
    );
    assert_eq!(
        lfg_posts_unique_index_count(&pool, "lane_id").await,
        1,
        "voice.lfg_posts.lane_id remains unique without FK to tempvoice_lanes"
    );
    assert_eq!(
        lfg_posts_owner_active_unique_index_count(&pool).await,
        1,
        "voice.lfg_posts.owner_id has partial unique index for creating/open posts"
    );
    assert_eq!(
        lfg_posts_status_check_constraint_count(&pool).await,
        1,
        "voice.lfg_posts.status keeps creating/open/closed/expired CHECK"
    );

    assert_eq!(
        table_columns_in_schema(&pool, "steam", "rank_history_visibility").await,
        vec!["user_id", "visibility", "updated_at"]
    );
    assert_eq!(
        primary_key_columns_in_schema(&pool, "steam", "rank_history_visibility").await,
        vec!["user_id"]
    );
    assert_eq!(
        rank_history_visibility_check_constraint_count(&pool).await,
        1,
        "rank_history_visibility.visibility keeps the private/members/public CHECK"
    );
    assert_eq!(
        rank_history_visibility_index_count(&pool).await,
        1,
        "rank_history_visibility visibility index exists"
    );
    assert_eq!(
        steam_rank_history_covering_index_count(&pool).await,
        1,
        "steam_rank_history has the partial covering index for visible leaderboard lookups"
    );
    assert_column_in_schema(
        &pool,
        "steam",
        "rank_history_visibility",
        "user_id",
        "bigint",
        "int8",
        "NO",
        None,
    )
    .await;
    assert_column_in_schema(
        &pool,
        "steam",
        "rank_history_visibility",
        "visibility",
        "text",
        "text",
        "NO",
        Some("'private'::text"),
    )
    .await;
    assert_column_in_schema(
        &pool,
        "steam",
        "rank_history_visibility",
        "updated_at",
        "timestamp with time zone",
        "timestamptz",
        "NO",
        Some("now()"),
    )
    .await;

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
        table_columns(&pool, "discord_role_connection_tokens").await,
        vec![
            "discord_id",
            "access_token",
            "refresh_token",
            "token_type",
            "scope",
            "expires_at",
            "token_version",
            "active",
            "invalidated_at",
            "invalidation_reason",
            "last_refresh_at",
            "last_push_at",
            "last_push_error",
            "created_at",
            "updated_at"
        ]
    );
    assert_eq!(
        primary_key_columns(&pool, "discord_role_connection_tokens").await,
        vec!["discord_id"]
    );
    assert_column(
        &pool,
        "discord_role_connection_tokens",
        "discord_id",
        "bigint",
        "int8",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "discord_role_connection_tokens",
        "access_token",
        "bytea",
        "bytea",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "discord_role_connection_tokens",
        "refresh_token",
        "bytea",
        "bytea",
        "NO",
        None,
    )
    .await;
    assert_column(
        &pool,
        "discord_role_connection_tokens",
        "active",
        "boolean",
        "bool",
        "NO",
        Some("true"),
    )
    .await;

    assert_eq!(
        table_columns(&pool, "discord_role_connection_sync_state").await,
        vec![
            "discord_id",
            "pending",
            "reason",
            "attempts",
            "next_attempt_at",
            "locked_at",
            "last_error",
            "created_at",
            "updated_at"
        ]
    );
    assert_eq!(
        primary_key_columns(&pool, "discord_role_connection_sync_state").await,
        vec!["discord_id"]
    );
    assert_column(
        &pool,
        "discord_role_connection_sync_state",
        "pending",
        "boolean",
        "bool",
        "NO",
        Some("true"),
    )
    .await;
    assert_column(
        &pool,
        "discord_role_connection_sync_state",
        "next_attempt_at",
        "timestamp with time zone",
        "timestamptz",
        "NO",
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
