use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::Path,
};

use dl_central_etl::{
    conversion_for, reconcile_total, run, snapshot_known_sources, source_snapshot_path,
    text_to_datetime, ColumnStatus, LedgerSet, SourceSqlite, TableLedger, DEADLOCK_SQLITE3_SOURCE,
    TOURNAMENT_SOURCE, WEBSITE_SOURCE,
};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde_json::Value as JsonValue;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tempfile::TempDir;

const LEDGER_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ledger");
const DEADLOCK_SOURCE_DB: &str = "deadlock-sqlite3";
const WEBSITE_SOURCE_DB: &str = "website";

#[tokio::test]
#[ignore]
async fn barrier_full_real_run_reconciles_idempotently_and_preserves_schema() {
    assert_known_sources_exist();

    let pool = test_pool().await;
    let snapshot_dir = TempDir::new().expect("create barrier snapshot dir");
    snapshot_known_sources(snapshot_dir.path()).expect("snapshot all real ETL sources");
    let ledger_set = LedgerSet::from_dir(LEDGER_ROOT).expect("load complete ETL ledger set");
    let inventory = source_inventory(&ledger_set, snapshot_dir.path());
    let targets = mapped_targets(&ledger_set);

    clean_targets(&pool, &targets)
        .await
        .expect("clean all mapped targets before barrier ETL");

    let report = run(&ledger_set, snapshot_dir.path(), &pool)
        .await
        .expect("full barrier ETL run succeeds");
    assert_report_totals(&report);
    assert_eq!(
        inventory.mapped_source_rows, report.total_source_rows,
        "inventory mapped source rows match report"
    );
    assert_eq!(
        inventory.total_source_rows,
        report.total_source_rows + inventory.dropped_only_rows,
        "only dropped-only source rows explain full inventory/report difference"
    );
    reconcile_total(&pool, &report.tables)
        .await
        .expect("full barrier total reconciliation passes");

    let counts_after_first = target_counts(&pool, &targets).await;
    assert_target_counts_match_report(&counts_after_first, &report);
    assert_union_reconcile(&pool, &inventory, &report).await;
    assert_union_round_trip_samples(&pool, snapshot_dir.path()).await;
    assert_steam_links_orphans_loaded(&pool, &inventory, &report).await;
    assert_no_unexpected_orphan_reports(&report);
    assert_schema_intact(&pool).await;
    assert_value_correctness_samples(&pool, snapshot_dir.path()).await;
    let sample_before = barrier_sample_fingerprint(&pool).await;

    println!(
        "barrier totals: total_source_rows={} total_loaded={} dropped_only_rows={} dropped_columns={}",
        report.total_source_rows,
        report.total_loaded,
        inventory.dropped_only_rows,
        dropped_column_count(&report)
    );
    for dropped in &inventory.dropped_only_tables {
        println!(
            "barrier dropped_only: {}.{} rows={}",
            dropped.source_db, dropped.source_table, dropped.source_rows
        );
    }

    let second = run(&ledger_set, snapshot_dir.path(), &pool)
        .await
        .expect("second full barrier ETL run succeeds");
    assert_report_totals(&second);
    reconcile_total(&pool, &second.tables)
        .await
        .expect("second full barrier total reconciliation passes");
    let counts_after_second = target_counts(&pool, &targets).await;
    assert_eq!(
        counts_after_first, counts_after_second,
        "target row counts unchanged after second run"
    );
    assert_eq!(
        sample_before,
        barrier_sample_fingerprint(&pool).await,
        "round-trip sample unchanged after second run"
    );
    assert_schema_intact(&pool).await;
    println!(
        "barrier idempotence: targets={} counts_unchanged=true sample_unchanged=true schema_intact=true",
        counts_after_second.len()
    );
}

#[tokio::test]
#[ignore]
async fn barrier_converter_matrix_covers_all_real_mapped_pairs() {
    assert_known_sources_exist();

    let pool = test_pool().await;
    let snapshot_dir = TempDir::new().expect("create matrix snapshot dir");
    snapshot_known_sources(snapshot_dir.path()).expect("snapshot all real ETL sources");
    let ledger_set = LedgerSet::from_dir(LEDGER_ROOT).expect("load complete ETL ledger set");
    let target_columns = load_target_column_types(&pool).await;

    let mut mapped_columns = 0_u64;
    let mut unique_pairs = BTreeSet::<(String, String)>::new();
    let mut missing = Vec::new();

    for (source_db, ledger) in &ledger_set.sources {
        let source_path = source_snapshot_path(snapshot_dir.path(), source_db);
        let source = SourceSqlite::open_read_only(&source_path)
            .unwrap_or_else(|err| panic!("open {source_db} matrix snapshot: {err}"));

        for (source_table, table_ledger) in &ledger.tables {
            let source_schema = source
                .table_schema(source_table)
                .unwrap_or_else(|err| panic!("read {source_db}.{source_table} schema: {err}"));
            let source_affinities = source_schema
                .iter()
                .map(|column| (column.name.as_str(), column.affinity.as_str()))
                .collect::<BTreeMap<_, _>>();

            for (source_column, status) in &table_ledger.columns {
                let ColumnStatus::Mapped { to } = status else {
                    continue;
                };
                let target_ref = TargetColumnRef::parse(to);
                let source_affinity = source_affinities
                    .get(source_column.as_str())
                    .unwrap_or_else(|| {
                        panic!("source column missing: {source_db}.{source_table}.{source_column}")
                    });
                let target_type = target_columns
                    .get(&target_ref.key())
                    .unwrap_or_else(|| panic!("target column missing: {to}"));

                mapped_columns += 1;
                unique_pairs.insert(((*source_affinity).to_string(), target_type.clone()));
                if let Err(err) = conversion_for(source_affinity, target_type) {
                    missing.push(format!(
                        "{source_db}.{source_table}.{source_column} -> {to}: {err}"
                    ));
                }
            }
        }
    }

    if !missing.is_empty() {
        panic!("matrix missing converters:\n{}", missing.join("\n"));
    }

    println!(
        "matrix_gate mapped_columns={} unique_pairs={} missing=0",
        mapped_columns,
        unique_pairs.len()
    );
}

#[tokio::test]
#[ignore]
async fn real_text_timestamptz_unix_seconds_text_fallback_is_limited_to_known_columns() {
    assert_known_sources_exist();

    let pool = test_pool().await;
    let snapshot_dir = TempDir::new().expect("create timestamp fallback scan snapshot dir");
    snapshot_known_sources(snapshot_dir.path()).expect("snapshot all real ETL sources");
    let ledger_set = LedgerSet::from_dir(LEDGER_ROOT).expect("load complete ETL ledger set");
    let target_columns = load_target_column_types(&pool).await;
    let expected_unix_text_columns = BTreeSet::from([
        "deadlock-sqlite3.changelog_posts.posted_at -> patchnotes.changelog_posts.posted_at"
            .to_string(),
        "deadlock-sqlite3.deadlock_changelogs.posted_at -> patchnotes.deadlock_changelogs.posted_at"
            .to_string(),
    ]);

    let mut scanned_columns = 0_u64;
    let mut unix_text_columns = BTreeSet::new();
    let mut unix_text_column_details = Vec::new();

    for (source_db, ledger) in &ledger_set.sources {
        let source_path = source_snapshot_path(snapshot_dir.path(), source_db);
        let source = SourceSqlite::open_read_only(&source_path)
            .unwrap_or_else(|err| panic!("open {source_db} timestamp scan snapshot: {err}"));
        let sqlite = open_snapshot(&source_path);

        for (source_table, table_ledger) in &ledger.tables {
            let source_schema = source
                .table_schema(source_table)
                .unwrap_or_else(|err| panic!("read {source_db}.{source_table} schema: {err}"));
            let source_affinities = source_schema
                .iter()
                .map(|column| (column.name.as_str(), column.affinity.as_str()))
                .collect::<BTreeMap<_, _>>();

            for (source_column, status) in &table_ledger.columns {
                let ColumnStatus::Mapped { to } = status else {
                    continue;
                };
                let target_ref = TargetColumnRef::parse(to);
                let source_affinity = source_affinities
                    .get(source_column.as_str())
                    .unwrap_or_else(|| {
                        panic!("source column missing: {source_db}.{source_table}.{source_column}")
                    });
                let target_type = target_columns
                    .get(&target_ref.key())
                    .unwrap_or_else(|| panic!("target column missing: {to}"));

                if *source_affinity != "TEXT" || target_type != "timestamptz" {
                    continue;
                }

                scanned_columns += 1;
                let count = sqlite_unix_seconds_text_count(&sqlite, source_table, source_column);
                if count > 0 {
                    let sample =
                        sqlite_unix_seconds_text_sample(&sqlite, source_table, source_column)
                            .expect("counted unix text column has sample");
                    let column = format!("{source_db}.{source_table}.{source_column} -> {to}");
                    unix_text_columns.insert(column.clone());
                    unix_text_column_details
                        .push(format!("{column}: count={count} sample={sample}"));
                }
            }
        }
    }

    assert_eq!(
        unix_text_columns,
        expected_unix_text_columns,
        "unexpected TEXT->timestamptz 10-digit Unix text columns:\n{}",
        unix_text_column_details.join("\n")
    );
    println!(
        "timestamp_fallback_scan text_timestamptz_columns={scanned_columns} unix_seconds_text_columns={} details={}",
        unix_text_columns.len(),
        unix_text_column_details.join("; ")
    );
}

async fn test_pool() -> PgPool {
    let dsn = env::var("CENTRAL_TEST_DSN").expect("CENTRAL_TEST_DSN is set by central_test_db.sh");
    assert!(
        !dsn.contains("127.0.0.1:5434") && !dsn.contains("localhost:5434"),
        "refuse to run ETL tests against the production central DB port"
    );
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&dsn)
        .await
        .expect("connect central test db")
}

fn assert_known_sources_exist() {
    for source in [DEADLOCK_SQLITE3_SOURCE, WEBSITE_SOURCE, TOURNAMENT_SOURCE] {
        assert!(
            Path::new(source).exists(),
            "real source path exists: {source}"
        );
    }
}

fn source_inventory(ledger_set: &LedgerSet, snapshot_dir: &Path) -> SourceInventory {
    let mut inventory = SourceInventory::default();

    for (source_db, ledger) in &ledger_set.sources {
        let path = source_snapshot_path(snapshot_dir, source_db);
        let source = open_snapshot(&path);

        for (source_table, table_ledger) in &ledger.tables {
            let source_rows = sqlite_count(&source, source_table);
            inventory.total_source_rows += source_rows;

            if let Some(target) = target_for_table(table_ledger) {
                inventory.mapped_source_rows += source_rows;
                *inventory
                    .contributions
                    .entry((source_db.clone(), target))
                    .or_default() += source_rows;
            } else {
                inventory.dropped_only_rows += source_rows;
                inventory.dropped_only_tables.push(DroppedOnlyTable {
                    source_db: source_db.clone(),
                    source_table: source_table.clone(),
                    source_rows,
                });
            }
        }
    }

    inventory
}

fn mapped_targets(ledger_set: &LedgerSet) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for ledger in ledger_set.sources.values() {
        for table_ledger in ledger.tables.values() {
            if let Some(target) = target_for_table(table_ledger) {
                targets.insert(target);
            }
        }
    }
    targets
}

fn target_for_table(table_ledger: &TableLedger) -> Option<String> {
    let mut target = None;

    for status in table_ledger.columns.values() {
        let ColumnStatus::Mapped { to } = status else {
            continue;
        };
        let Some((table_path, _column)) = to.rsplit_once('.') else {
            panic!("invalid target path in ledger: {to}");
        };
        match &target {
            Some(existing) => assert_eq!(existing, table_path, "mixed target table in ledger"),
            None => target = Some(table_path.to_string()),
        }
    }

    target
}

async fn clean_targets(pool: &PgPool, targets: &BTreeSet<String>) -> Result<(), sqlx::Error> {
    if targets.is_empty() {
        return Ok(());
    }

    let tables = targets
        .iter()
        .map(|target| quote_pg_path(target))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("TRUNCATE TABLE {tables} RESTART IDENTITY CASCADE");
    sqlx::query(&sql).execute(pool).await?;
    Ok(())
}

fn assert_report_totals(report: &dl_central_etl::EtlReport) {
    let source_sum = report
        .tables
        .iter()
        .map(|table| table.source_rows)
        .sum::<u64>();
    let loaded_sum = report
        .tables
        .iter()
        .map(|table| table.loaded_rows)
        .sum::<u64>();
    assert_eq!(source_sum, report.total_source_rows);
    assert_eq!(loaded_sum, report.total_loaded);
    assert_eq!(
        report.total_source_rows, report.total_loaded,
        "mapped source rows equal loaded rows"
    );
    for table in &report.tables {
        assert_eq!(
            table.source_rows, table.loaded_rows,
            "source_rows == loaded_rows for {}",
            table.target
        );
    }
}

async fn target_counts(pool: &PgPool, targets: &BTreeSet<String>) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for target in targets {
        counts.insert(target.clone(), target_count(pool, target).await);
    }
    counts
}

async fn target_count(pool: &PgPool, target: &str) -> u64 {
    let sql = format!(
        "SELECT COUNT(*)::bigint AS count FROM {}",
        quote_pg_path(target)
    );
    let row = sqlx::query(&sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("count target {target}: {err}"));
    i64_to_u64(row.try_get("count").expect("target count as i64"), target)
}

fn assert_target_counts_match_report(
    target_counts: &BTreeMap<String, u64>,
    report: &dl_central_etl::EtlReport,
) {
    for (target, (source_rows, _loaded_rows)) in aggregate_report_counts(&report.tables) {
        assert_eq!(
            target_counts.get(&target).copied(),
            Some(source_rows),
            "target row count matches report for {target}"
        );
    }
}

async fn assert_union_reconcile(
    pool: &PgPool,
    inventory: &SourceInventory,
    report: &dl_central_etl::EtlReport,
) {
    let report_counts = aggregate_report_counts(&report.tables);
    for target in union_targets() {
        let deadlock = inventory.contribution(DEADLOCK_SOURCE_DB, target);
        let website = inventory.contribution(WEBSITE_SOURCE_DB, target);
        let expected = deadlock + website;
        let actual = target_count(pool, target).await;
        let report_source = report_counts
            .get(target)
            .map(|(source_rows, _)| *source_rows)
            .unwrap_or_default();

        assert_eq!(
            expected, report_source,
            "union report source rows for {target}"
        );
        assert_eq!(expected, actual, "union target rows for {target}");
        println!(
            "union_reconcile {target}: deadlock_sqlite3_rows={deadlock} website_rows={website} target_rows={actual}"
        );
    }
}

async fn assert_union_round_trip_samples(pool: &PgPool, snapshot_dir: &Path) {
    for sample in union_samples() {
        let source = open_snapshot(&source_snapshot_path(snapshot_dir, sample.source_db));
        let source_rows = sqlite_count(&source, sample.source_table);
        if source_rows == 0 {
            println!(
                "union_round_trip {}.{} -> {} rows=0",
                sample.source_db, sample.source_table, sample.target
            );
            continue;
        }

        let (source_pk, source_check) = sqlite_sample_pair(
            &source,
            sample.source_table,
            sample.source_pk_column,
            sample.source_check_column,
        )
        .unwrap_or_else(|| {
            panic!(
                "expected sample row for {}.{}",
                sample.source_db, sample.source_table
            )
        });
        let target_pk = if sample.generated_request_uid {
            format!("{}:{}:{}", sample.source_db, sample.source_table, source_pk)
        } else {
            source_pk.clone()
        };
        let target_check = target_check_value(
            pool,
            sample.target,
            sample.target_pk_column,
            &target_pk,
            sample.target_check_column,
        )
        .await
        .unwrap_or_else(|| {
            panic!(
                "missing union target row {}.{}={target_pk}",
                sample.target, sample.target_pk_column
            )
        });

        if let Some(source_check) = source_check {
            assert_eq!(
                source_check, target_check,
                "union round-trip check for {}.{}",
                sample.source_db, sample.source_table
            );
        }
        println!(
            "union_round_trip {}.{} -> {} rows={} sample_pk={}",
            sample.source_db, sample.source_table, sample.target, source_rows, source_pk
        );
    }
}

async fn target_check_value(
    pool: &PgPool,
    target: &str,
    pk_column: &str,
    pk_value: &str,
    check_column: &str,
) -> Option<String> {
    let sql = format!(
        "SELECT COALESCE({}::text, 'NULL') AS check_value FROM {} WHERE {}::text = $1",
        quote_pg_ident(check_column),
        quote_pg_path(target),
        quote_pg_ident(pk_column)
    );
    let row = sqlx::query(&sql)
        .bind(pk_value)
        .fetch_optional(pool)
        .await
        .unwrap_or_else(|err| panic!("union target sample query failed for {target}: {err}"))?;
    Some(row.try_get("check_value").expect("check_value as text"))
}

async fn assert_steam_links_orphans_loaded(
    pool: &PgPool,
    inventory: &SourceInventory,
    report: &dl_central_etl::EtlReport,
) {
    let source_rows = inventory.contribution(DEADLOCK_SOURCE_DB, "core.steam_links");
    let target_rows = target_count(pool, "core.steam_links").await;
    assert_eq!(
        source_rows, target_rows,
        "core.steam_links target rows equal source rows"
    );

    let sql_count = steam_links_user_orphan_count(pool).await;
    let report_count = report
        .orphan_reports
        .iter()
        .filter(|orphan| orphan.table == "core.steam_links" && orphan.parent_table == "core.users")
        .map(|orphan| orphan.count)
        .sum::<u64>();
    assert_eq!(
        sql_count, report_count,
        "steam_links user orphan report matches SQL count"
    );
    println!(
        "orphan_report core.steam_links->core.users source_rows={source_rows} target_rows={target_rows} loaded_orphans={sql_count}"
    );
}

async fn steam_links_user_orphan_count(pool: &PgPool) -> u64 {
    let row = sqlx::query(
        r#"
        SELECT COUNT(*)::bigint AS count
        FROM core.steam_links s
        WHERE s.discord_id <> 0
          AND NOT EXISTS (
              SELECT 1
              FROM core.users u
              WHERE u.discord_id = s.discord_id
          )
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("count steam_links user orphans");
    i64_to_u64(
        row.try_get("count").expect("orphan count as i64"),
        "steam_links orphans",
    )
}

fn assert_no_unexpected_orphan_reports(report: &dl_central_etl::EtlReport) {
    let unexpected = report
        .orphan_reports
        .iter()
        .filter(|orphan| {
            !(orphan.table == "core.steam_links" && orphan.parent_table == "core.users")
        })
        .collect::<Vec<_>>();
    assert!(
        unexpected.is_empty(),
        "unexpected FK orphan reports: {unexpected:#?}"
    );
    println!("orphan_report unexpected_fk_orphans=0");
}

async fn assert_schema_intact(pool: &PgPool) {
    let trigger_rows = sqlx::query(
        r#"
        SELECT t.tgname, t.tgenabled::text AS tgenabled
        FROM pg_trigger t
        JOIN pg_class c ON c.oid = t.tgrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'core'
          AND c.relname = 'steam_links'
          AND NOT t.tgisinternal
        ORDER BY t.tgname
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("load core.steam_links trigger states");
    assert!(
        !trigger_rows.is_empty(),
        "core.steam_links has user triggers"
    );
    let mut trigger_states = Vec::new();
    for row in trigger_rows {
        let name: String = row.try_get("tgname").expect("trigger name");
        let enabled: String = row.try_get("tgenabled").expect("trigger enabled state");
        assert_eq!(enabled, "O", "trigger {name} is enabled");
        trigger_states.push(format!("{name}={enabled}"));
    }

    let invalid_fks = sqlx::query(
        r#"
        SELECT conrelid::regclass::text AS child_table, conname
        FROM pg_constraint
        WHERE contype = 'f'
          AND NOT convalidated
        ORDER BY child_table, conname
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("load invalid FKs");
    assert!(
        invalid_fks.is_empty(),
        "FKs left NOT VALID: {invalid_fks:#?}"
    );

    let fk_count_row =
        sqlx::query("SELECT COUNT(*)::bigint AS count FROM pg_constraint WHERE contype = 'f'")
            .fetch_one(pool)
            .await
            .expect("count FKs");
    let fk_count = i64_to_u64(fk_count_row.try_get("count").expect("FK count"), "FK count");
    println!(
        "schema_intact steam_links_triggers={} all_fks_valid=true fk_count={fk_count}",
        trigger_states.join(",")
    );
}

async fn assert_value_correctness_samples(pool: &PgPool, snapshot_dir: &Path) {
    let deadlock = open_snapshot(&source_snapshot_path(snapshot_dir, DEADLOCK_SOURCE_DB));
    let website = open_snapshot(&source_snapshot_path(snapshot_dir, WEBSITE_SOURCE_DB));

    let (offset_id, offset_text) = sqlite_sample_pair_where(
        &deadlock,
        "changelog_posts",
        "id",
        "posted_at",
        "posted_at GLOB '*[+-][0-9][0-9][0-9][0-9]'",
    )
    .expect("offset timestamp sample exists");
    let expected_offset = text_to_datetime(&offset_text)
        .expect("offset timestamp parses")
        .timestamp_micros();
    let actual_offset = target_timestamp_micros(
        pool,
        "patchnotes.changelog_posts",
        "id",
        &offset_id,
        "posted_at",
    )
    .await;
    assert_eq!(expected_offset, actual_offset);
    println!(
        "value_sample offset_timestamp patchnotes.changelog_posts id={offset_id} utc_micros={actual_offset}"
    );

    let (naive_id, naive_text) = sqlite_sample_pair_where(
        &website,
        "coaches",
        "id",
        "updated_at",
        "updated_at GLOB '*.*' AND updated_at NOT GLOB '*+*' AND updated_at NOT GLOB '*Z'",
    )
    .expect("naive fractional timestamp sample exists");
    let expected_naive = text_to_datetime(&naive_text)
        .expect("naive fractional timestamp parses")
        .timestamp_micros();
    let actual_naive =
        target_timestamp_micros(pool, "coaching.coaches", "id", &naive_id, "updated_at").await;
    assert_eq!(expected_naive, actual_naive);
    println!(
        "value_sample naive_fractional_timestamp coaching.coaches id={naive_id} utc_micros={actual_naive}"
    );

    let (csv_id, csv_text) = sqlite_sample_pair_where(
        &deadlock,
        "text_conversation_log",
        "id",
        "co_participant_ids",
        "co_participant_ids LIKE '%,%'",
    )
    .expect("co_participant_ids CSV sample exists");
    let expected_tokens = csv_text
        .split(',')
        .map(|token| token.trim().to_string())
        .collect::<Vec<_>>();
    let target_json_text = target_jsonb_text(
        pool,
        "activity.text_conversation_log",
        "id",
        &csv_id,
        "co_participant_ids",
    )
    .await;
    let actual_tokens = json_integer_tokens(&target_json_text);
    assert_eq!(expected_tokens, actual_tokens);
    let converted = dl_central_etl::text_json_or_integer_csv_list_to_value(&csv_text)
        .expect("CSV converter parses sample");
    assert_eq!(json_integer_tokens(&converted.to_string()), actual_tokens);
    println!(
        "value_sample csv_json_array activity.text_conversation_log id={csv_id} token_count={} snowflake_exact=true",
        actual_tokens.len()
    );
}

async fn target_timestamp_micros(
    pool: &PgPool,
    target: &str,
    pk_column: &str,
    pk_value: &str,
    timestamp_column: &str,
) -> i64 {
    let sql = format!(
        "SELECT (EXTRACT(EPOCH FROM {}) * 1000000)::bigint AS micros FROM {} WHERE {}::text = $1",
        quote_pg_ident(timestamp_column),
        quote_pg_path(target),
        quote_pg_ident(pk_column)
    );
    let row = sqlx::query(&sql)
        .bind(pk_value)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("timestamp sample query failed for {target}: {err}"));
    row.try_get("micros").expect("timestamp micros as i64")
}

async fn target_jsonb_text(
    pool: &PgPool,
    target: &str,
    pk_column: &str,
    pk_value: &str,
    json_column: &str,
) -> String {
    let sql = format!(
        "SELECT {}::text AS value FROM {} WHERE {}::text = $1",
        quote_pg_ident(json_column),
        quote_pg_path(target),
        quote_pg_ident(pk_column)
    );
    let row = sqlx::query(&sql)
        .bind(pk_value)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("jsonb sample query failed for {target}: {err}"));
    row.try_get("value").expect("jsonb text")
}

fn json_integer_tokens(json_text: &str) -> Vec<String> {
    let value = serde_json::from_str::<JsonValue>(json_text).expect("target JSON parses");
    let values = value.as_array().expect("target JSON is an array");
    values
        .iter()
        .map(|value| match value {
            JsonValue::Number(number) => number.to_string(),
            other => panic!("JSON array item is not a number: {other}"),
        })
        .collect()
}

async fn barrier_sample_fingerprint(pool: &PgPool) -> BTreeMap<String, String> {
    let queries = [
        (
            "offset_timestamp",
            r#"
            SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id)::text, '[]') AS sample
            FROM (
                SELECT id, (EXTRACT(EPOCH FROM posted_at) * 1000000)::bigint AS posted_at
                FROM patchnotes.changelog_posts
                ORDER BY id
                LIMIT 3
            ) t
            "#,
        ),
        (
            "naive_fractional_timestamp",
            r#"
            SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id)::text, '[]') AS sample
            FROM (
                SELECT id, (EXTRACT(EPOCH FROM updated_at) * 1000000)::bigint AS updated_at
                FROM coaching.coaches
                ORDER BY id
                LIMIT 3
            ) t
            "#,
        ),
        (
            "csv_json_array",
            r#"
            SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id)::text, '[]') AS sample
            FROM (
                SELECT id, co_participant_ids
                FROM activity.text_conversation_log
                WHERE co_participant_ids IS NOT NULL
                ORDER BY id
                LIMIT 3
            ) t
            "#,
        ),
        (
            "union_requests",
            r#"
            SELECT COALESCE(string_agg(request_uid || ':' || discord_user_id::text, ',' ORDER BY request_uid), '') AS sample
            FROM coaching.requests
            "#,
        ),
    ];

    let mut out = BTreeMap::new();
    for (name, sql) in queries {
        let row = sqlx::query(sql)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|err| panic!("idempotence sample {name} failed: {err}"));
        out.insert(
            name.to_string(),
            row.try_get::<String, _>("sample")
                .expect("idempotence sample as text"),
        );
    }
    out
}

async fn load_target_column_types(pool: &PgPool) -> BTreeMap<(String, String, String), String> {
    let rows = sqlx::query(
        r#"
        SELECT table_schema, table_name, column_name, udt_name
        FROM information_schema.columns
        WHERE table_schema NOT IN ('information_schema', 'pg_catalog')
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("load target column types");

    rows.into_iter()
        .map(|row| {
            let schema: String = row.try_get("table_schema").expect("table_schema");
            let table: String = row.try_get("table_name").expect("table_name");
            let column: String = row.try_get("column_name").expect("column_name");
            let pg_type: String = row.try_get("udt_name").expect("udt_name");
            ((schema, table, column), pg_type)
        })
        .collect()
}

fn aggregate_report_counts(tables: &[dl_central_etl::TableResult]) -> BTreeMap<String, (u64, u64)> {
    let mut counts = BTreeMap::<String, (u64, u64)>::new();
    for table in tables {
        let entry = counts.entry(table.target.clone()).or_default();
        entry.0 += table.source_rows;
        entry.1 += table.loaded_rows;
    }
    counts
}

fn dropped_column_count(report: &dl_central_etl::EtlReport) -> usize {
    report
        .tables
        .iter()
        .map(|table| table.dropped_cols.len())
        .sum()
}

fn sqlite_count(source: &Connection, table: &str) -> u64 {
    let sql = format!("SELECT COUNT(*) FROM {}", quote_sqlite_ident(table));
    source
        .query_row(&sql, [], |row| row.get::<_, u64>(0))
        .unwrap_or_else(|err| panic!("count SQLite table {table}: {err}"))
}

fn sqlite_unix_seconds_text_count(source: &Connection, table: &str, column: &str) -> u64 {
    let sql = format!(
        "SELECT COUNT(*) FROM {} WHERE typeof({}) = 'text' AND length({}) = 10 AND {} GLOB '[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]'",
        quote_sqlite_ident(table),
        quote_sqlite_ident(column),
        quote_sqlite_ident(column),
        quote_sqlite_ident(column)
    );
    source
        .query_row(&sql, [], |row| row.get::<_, u64>(0))
        .unwrap_or_else(|err| panic!("count SQLite unix timestamp text {table}.{column}: {err}"))
}

fn sqlite_unix_seconds_text_sample(
    source: &Connection,
    table: &str,
    column: &str,
) -> Option<String> {
    let sql = format!(
        "SELECT {} FROM {} WHERE typeof({}) = 'text' AND length({}) = 10 AND {} GLOB '[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]' ORDER BY {} LIMIT 1",
        quote_sqlite_ident(column),
        quote_sqlite_ident(table),
        quote_sqlite_ident(column),
        quote_sqlite_ident(column),
        quote_sqlite_ident(column),
        quote_sqlite_ident(column)
    );
    let mut stmt = source
        .prepare(&sql)
        .unwrap_or_else(|err| panic!("prepare SQLite unix timestamp text {table}.{column}: {err}"));
    let mut rows = stmt
        .query([])
        .unwrap_or_else(|err| panic!("query SQLite unix timestamp text {table}.{column}: {err}"));
    let row = rows
        .next()
        .unwrap_or_else(|err| panic!("read SQLite unix timestamp text {table}.{column}: {err}"))?;
    Some(sqlite_value_to_string(
        row.get_ref(0).expect("unix text sample"),
    ))
}

fn sqlite_sample_pair(
    source: &Connection,
    table: &str,
    pk_column: &str,
    check_column: &str,
) -> Option<(String, Option<String>)> {
    sqlite_sample_pair_where(source, table, pk_column, check_column, "1=1")
        .map(|(pk, check)| (pk, Some(check)))
}

fn sqlite_sample_pair_where(
    source: &Connection,
    table: &str,
    pk_column: &str,
    value_column: &str,
    where_clause: &str,
) -> Option<(String, String)> {
    let sql = format!(
        "SELECT {}, {} FROM {} WHERE {where_clause} ORDER BY {} LIMIT 1",
        quote_sqlite_ident(pk_column),
        quote_sqlite_ident(value_column),
        quote_sqlite_ident(table),
        quote_sqlite_ident(pk_column)
    );
    let mut stmt = source
        .prepare(&sql)
        .unwrap_or_else(|err| panic!("prepare SQLite sample {table}: {err}"));
    let mut rows = stmt
        .query([])
        .unwrap_or_else(|err| panic!("query SQLite sample {table}: {err}"));
    let row = rows
        .next()
        .unwrap_or_else(|err| panic!("read SQLite sample {table}: {err}"))?;
    Some((
        sqlite_value_to_string(row.get_ref(0).expect("sample pk")),
        sqlite_value_to_string(row.get_ref(1).expect("sample value")),
    ))
}

fn open_snapshot(path: &Path) -> Connection {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap_or_else(|err| panic!("open snapshot {}: {err}", path.display()))
}

fn sqlite_value_to_string(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        ValueRef::Blob(value) => format!("<blob:{}>", value.len()),
    }
}

fn union_targets() -> [&'static str; 5] {
    [
        "coaching.requests",
        "coaching.coaches",
        "coaching.coach_applications",
        "coaching.sessions",
        "coaching.surveys",
    ]
}

fn union_samples() -> [UnionSample; 10] {
    [
        UnionSample::generated_request(DEADLOCK_SOURCE_DB, "coaching_requests", "discord_user_id"),
        UnionSample::generated_request(WEBSITE_SOURCE_DB, "coaching_requests", "discord_user_id"),
        UnionSample::id(
            DEADLOCK_SOURCE_DB,
            "coaches",
            "coaching.coaches",
            "discord_user_id",
        ),
        UnionSample::id(
            WEBSITE_SOURCE_DB,
            "coaches",
            "coaching.coaches",
            "discord_user_id",
        ),
        UnionSample::id(
            DEADLOCK_SOURCE_DB,
            "coach_applications",
            "coaching.coach_applications",
            "discord_user_id",
        ),
        UnionSample::id(
            WEBSITE_SOURCE_DB,
            "coach_applications",
            "coaching.coach_applications",
            "discord_user_id",
        ),
        UnionSample::id(
            DEADLOCK_SOURCE_DB,
            "coaching_sessions",
            "coaching.sessions",
            "discord_user_id",
        ),
        UnionSample::id(
            WEBSITE_SOURCE_DB,
            "coaching_sessions",
            "coaching.sessions",
            "discord_user_id",
        ),
        UnionSample::id(
            DEADLOCK_SOURCE_DB,
            "coaching_surveys",
            "coaching.surveys",
            "rating",
        ),
        UnionSample::id(
            WEBSITE_SOURCE_DB,
            "coaching_surveys",
            "coaching.surveys",
            "rating",
        ),
    ]
}

#[derive(Debug, Clone, Default)]
struct SourceInventory {
    total_source_rows: u64,
    mapped_source_rows: u64,
    dropped_only_rows: u64,
    dropped_only_tables: Vec<DroppedOnlyTable>,
    contributions: BTreeMap<(String, String), u64>,
}

impl SourceInventory {
    fn contribution(&self, source_db: &str, target: &str) -> u64 {
        self.contributions
            .get(&(source_db.to_string(), target.to_string()))
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
struct DroppedOnlyTable {
    source_db: String,
    source_table: String,
    source_rows: u64,
}

#[derive(Debug, Clone, Copy)]
struct UnionSample {
    source_db: &'static str,
    source_table: &'static str,
    source_pk_column: &'static str,
    source_check_column: &'static str,
    target: &'static str,
    target_pk_column: &'static str,
    target_check_column: &'static str,
    generated_request_uid: bool,
}

impl UnionSample {
    const fn generated_request(
        source_db: &'static str,
        source_table: &'static str,
        check_column: &'static str,
    ) -> Self {
        Self {
            source_db,
            source_table,
            source_pk_column: "id",
            source_check_column: check_column,
            target: "coaching.requests",
            target_pk_column: "request_uid",
            target_check_column: check_column,
            generated_request_uid: true,
        }
    }

    const fn id(
        source_db: &'static str,
        source_table: &'static str,
        target: &'static str,
        check_column: &'static str,
    ) -> Self {
        Self {
            source_db,
            source_table,
            source_pk_column: "id",
            source_check_column: check_column,
            target,
            target_pk_column: "id",
            target_check_column: check_column,
            generated_request_uid: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TargetColumnRef {
    schema: String,
    table: String,
    column: String,
}

impl TargetColumnRef {
    fn parse(path: &str) -> Self {
        let parts = path.split('.').collect::<Vec<_>>();
        assert_eq!(parts.len(), 3, "invalid target path: {path}");
        Self {
            schema: parts[0].to_string(),
            table: parts[1].to_string(),
            column: parts[2].to_string(),
        }
    }

    fn key(&self) -> (String, String, String) {
        (self.schema.clone(), self.table.clone(), self.column.clone())
    }
}

fn i64_to_u64(value: i64, label: &str) -> u64 {
    u64::try_from(value).unwrap_or_else(|_| panic!("{label} is negative: {value}"))
}

fn quote_pg_path(path: &str) -> String {
    path.split('.')
        .map(quote_pg_ident)
        .collect::<Vec<_>>()
        .join(".")
}

fn quote_pg_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_sqlite_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
