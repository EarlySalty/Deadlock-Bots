use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::Path,
};

use dl_central_etl::{
    reconcile_total, run, snapshot_db, source_snapshot_path, ColumnStatus, ConvertError,
    EngineError, Ledger, LedgerSet, TableLedger, TableResult, TargetError, DEADLOCK_SQLITE3_SOURCE,
};
use rusqlite::{
    types::{Value, ValueRef},
    Connection, OpenFlags,
};
use serde_json::Value as JsonValue;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tempfile::TempDir;

const SOURCE_DB: &str = "deadlock-sqlite3";
const LEDGER_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ledger/deadlock-sqlite3");

#[tokio::test]
#[ignore]
async fn deadlock_sqlite3_real_snapshot_loads_and_verifies() {
    let source_path = Path::new(DEADLOCK_SQLITE3_SOURCE);
    if !source_path.exists() {
        println!(
            "SKIP {SOURCE_DB}: Snapshot-Quelle fehlt: {}",
            source_path.display()
        );
        return;
    }

    let pool = test_pool().await;
    let snapshot_dir = TempDir::new().expect("create snapshot dir");
    let snapshot_path = source_snapshot_path(snapshot_dir.path(), SOURCE_DB);
    snapshot_db(source_path, &snapshot_path).expect("snapshot deadlock.sqlite3 read-only");

    let ledger_set = load_deadlock_ledger_set();
    let all_targets = mapped_targets(&ledger_set);
    clean_targets(&pool, &all_targets)
        .await
        .expect("clean mapped targets before ETL");

    let report = match run(&ledger_set, snapshot_dir.path(), &pool).await {
        Ok(report) => report,
        Err(err) => {
            let findings =
                diagnose_table_failures(&ledger_set, snapshot_dir.path(), &pool, &all_targets)
                    .await;
            panic!(
                "ECHTE-DATEN-BEFUND: engine::run fuer {SOURCE_DB} fehlgeschlagen: {err}\n{}",
                findings.join("\n")
            );
        }
    };

    let union_targets = union_targets();
    let non_union_results = report
        .tables
        .iter()
        .filter(|result| !union_targets.contains(result.target.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    for result in &non_union_results {
        assert_eq!(
            result.source_rows, result.loaded_rows,
            "non-union loaded_rows mismatch for {}",
            result.target
        );
    }
    reconcile_total(&pool, &non_union_results)
        .await
        .expect("non-union total reconciliation");

    let non_union_target_count = non_union_results
        .iter()
        .map(|result| result.target.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    println!(
        "non-union reconcile ok: targets={} source_tables={}",
        non_union_target_count,
        non_union_results.len()
    );

    verify_union_half_loaded(&pool, &report.tables, &union_targets).await;

    let source = open_snapshot(&snapshot_path);
    assert_persistent_views_round_trip(&source, &pool).await;
    assert_member_events_round_trip(&source, &pool).await;
    assert_voice_channel_settings_round_trip(&source, &pool).await;
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

fn load_deadlock_ledger_set() -> LedgerSet {
    let mut fragments = fs::read_dir(LEDGER_DIR)
        .expect("read deadlock-sqlite3 ledger dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("read deadlock-sqlite3 ledger entries");
    fragments.sort_by_key(|entry| entry.path());

    let mut combined = String::new();
    for entry in fragments {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        combined.push_str(&fs::read_to_string(&path).expect("read ledger fragment"));
        combined.push('\n');
    }

    let ledger = Ledger::from_toml_str(&combined).expect("deadlock-sqlite3 ledger parses");
    LedgerSet {
        sources: BTreeMap::from([(SOURCE_DB.to_string(), ledger)]),
    }
}

fn mapped_targets(ledger_set: &LedgerSet) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for ledger in ledger_set.sources.values() {
        for table_ledger in ledger.tables.values() {
            targets.extend(table_targets(table_ledger));
        }
    }
    targets
}

fn table_targets(table_ledger: &TableLedger) -> BTreeSet<String> {
    table_ledger
        .columns
        .values()
        .filter_map(|status| match status {
            ColumnStatus::Mapped { to } => Some(target_table(to)),
            ColumnStatus::Dropped { .. } => None,
        })
        .collect()
}

fn target_table(target_column: &str) -> String {
    let mut parts = target_column.split('.');
    let schema = parts.next().expect("target schema");
    let table = parts.next().expect("target table");
    format!("{schema}.{table}")
}

fn union_targets() -> BTreeSet<&'static str> {
    BTreeSet::from([
        "coaching.requests",
        "coaching.coaches",
        "coaching.coach_applications",
        "coaching.sessions",
        "coaching.surveys",
        "core.steam_links",
    ])
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

async fn diagnose_table_failures(
    ledger_set: &LedgerSet,
    snapshot_dir: &Path,
    pool: &PgPool,
    all_targets: &BTreeSet<String>,
) -> Vec<String> {
    let snapshot_path = source_snapshot_path(snapshot_dir, SOURCE_DB);
    let source = open_snapshot(&snapshot_path);
    let ledger = ledger_set
        .source(SOURCE_DB)
        .expect("deadlock-sqlite3 ledger source exists");

    let mut passed = 0_u64;
    let mut failures = Vec::new();

    for (table, table_ledger) in &ledger.tables {
        if let Err(err) = clean_targets(pool, all_targets).await {
            failures.push(format!("isolation clean failed before {table}: {err}"));
            break;
        }

        let single = LedgerSet {
            sources: BTreeMap::from([(
                SOURCE_DB.to_string(),
                Ledger {
                    tables: BTreeMap::from([(table.clone(), table_ledger.clone())]),
                },
            )]),
        };

        match run(&single, snapshot_dir, pool).await {
            Ok(report) => {
                passed += 1;
                for result in report.tables {
                    if result.source_rows != result.loaded_rows {
                        failures.push(format!(
                            "{table} -> {}: ReconcileMismatch source_rows={} loaded_rows={}",
                            result.target, result.source_rows, result.loaded_rows
                        ));
                    }
                }
            }
            Err(err) => failures.push(describe_engine_error(table, table_ledger, &source, &err)),
        }
    }

    let mut out = vec![format!(
        "isolation summary: ok_tables={} failed_tables={}",
        passed,
        failures.len()
    )];
    out.extend(failures);
    out
}

fn describe_engine_error(
    table: &str,
    table_ledger: &TableLedger,
    source: &Connection,
    err: &EngineError,
) -> String {
    match err {
        EngineError::NoConverter {
            table: err_table,
            column,
            source_affinity,
            target_type,
        } => format!(
            "{err_table}.{column}: NoConverter SQLite {source_affinity} -> PG {target_type}; source_rows={}",
            source_row_count(source, table).unwrap_or(0)
        ),
        EngineError::Conversion {
            table: err_table,
            column,
            source_pk,
            source: convert,
            ..
        } => {
            let (count, sample) = conversion_failure_count(source, table, column, convert.as_ref())
                .unwrap_or_else(|_| (1, vec![source_pk.clone()]));
            format!(
                "{err_table}.{column}: Conversion; source_pk={source_pk}; count={count}; source_pk_sample={}; error={convert}",
                sample.join(",")
            )
        }
        EngineError::InvalidSourceValueType {
            table: err_table,
            column,
            source_pk,
            value,
            ..
        } => format!(
            "{err_table}.{column}: InvalidSourceValueType; source_pk={source_pk}; count=1; value={value}"
        ),
        EngineError::Int4OutOfRange {
            table: err_table,
            column,
            source_pk,
            value,
            ..
        } => format!(
            "{err_table}.{column}: Int4OutOfRange; source_pk={source_pk}; count=1; value={value}"
        ),
        EngineError::Target(TargetError::Sqlx(sqlx::Error::Database(db))) => {
            let code = db.code().map(|code| code.into_owned()).unwrap_or_default();
            let pg_error = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>();
            let target_column = pg_error
                .and_then(|error| error.column())
                .unwrap_or("<unknown>");
            let target_table = db.table().unwrap_or("<unknown>");
            let constraint = db.constraint().unwrap_or("<unknown>");
            let source_context =
                source_context_for_target_error(source, table, table_ledger, target_column);
            format!(
                "{table}.{target_column}: TargetSqlx code={code} target_table={target_table} constraint={constraint}; {source_context}; error={}",
                db.message()
            )
        }
        other => format!(
            "{table}.<unknown>: {}; source_rows={} source_pk_sample={}",
            other,
            source_row_count(source, table).unwrap_or(0),
            source_pk_sample(source, table, 3).unwrap_or_default().join(",")
        ),
    }
}

fn source_context_for_target_error(
    source: &Connection,
    table: &str,
    table_ledger: &TableLedger,
    target_column: &str,
) -> String {
    if target_column != "<unknown>" {
        if let Some(source_column) = mapped_source_column(table_ledger, target_column) {
            let nulls = null_count(source, table, source_column).unwrap_or(0);
            let pk_sample = source_pk_sample_where_null(source, table, source_column, 3)
                .unwrap_or_default()
                .join(",");
            return format!(
                "mapped_source={table}.{source_column}; null_count={nulls}; source_pk_sample={pk_sample}"
            );
        }
    }

    format!(
        "source_rows={}; source_pk_sample={}",
        source_row_count(source, table).unwrap_or(0),
        source_pk_sample(source, table, 3)
            .unwrap_or_default()
            .join(",")
    )
}

fn conversion_failure_count(
    source: &Connection,
    table: &str,
    column: &str,
    convert: &ConvertError,
) -> rusqlite::Result<(u64, Vec<String>)> {
    let pk_columns = sqlite_primary_key_columns(source, table)?;
    let mut select_parts = Vec::new();
    if pk_columns.is_empty() {
        select_parts.push("rowid".to_string());
    } else {
        select_parts.extend(pk_columns.iter().map(|column| quote_sqlite_ident(column)));
    }
    select_parts.push(quote_sqlite_ident(column));

    let sql = format!(
        "SELECT {} FROM {}",
        select_parts.join(", "),
        quote_sqlite_ident(table)
    );
    let mut stmt = source.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let pk_len = if pk_columns.is_empty() {
            1
        } else {
            pk_columns.len()
        };
        let pk = if pk_columns.is_empty() {
            format!("rowid={}", row.get::<_, i64>(0)?)
        } else {
            let mut fields = Vec::with_capacity(pk_columns.len());
            for (index, pk_column) in pk_columns.iter().enumerate() {
                fields.push(format!(
                    "{pk_column}={}",
                    sqlite_value_to_string(row.get_ref(index)?)
                ));
            }
            fields.join(",")
        };
        Ok((pk, row.get::<_, Value>(pk_len)?))
    })?;

    let mut count = 0_u64;
    let mut sample = Vec::new();
    for row in rows {
        let (pk, value) = row?;
        if conversion_value_fails_like(&value, convert) {
            count += 1;
            if sample.len() < 3 {
                sample.push(pk);
            }
        }
    }

    Ok((count, sample))
}

fn conversion_value_fails_like(value: &Value, convert: &ConvertError) -> bool {
    if matches!(value, Value::Null) {
        return false;
    }

    match convert {
        ConvertError::InvalidUnixTimestamp { .. } => match value {
            Value::Integer(value) => dl_central_etl::unix_seconds_to_datetime(*value).is_err(),
            _ => false,
        },
        ConvertError::InvalidRealUnixTimestamp { .. } => match value {
            Value::Real(value) => dl_central_etl::real_unix_seconds_to_datetime(*value).is_err(),
            _ => false,
        },
        ConvertError::InvalidBoolInteger { .. } => match value {
            Value::Integer(value) => !matches!(value, 0 | 1),
            _ => false,
        },
        ConvertError::InvalidBigintText { .. } | ConvertError::BigintTextOutOfRange { .. } => {
            match value {
                Value::Text(value) => dl_central_etl::text_to_bigint(value).is_err(),
                _ => false,
            }
        }
        ConvertError::InvalidTimestampText { .. } => match value {
            Value::Text(value) => dl_central_etl::text_to_datetime(value).is_err(),
            Value::Integer(value) => dl_central_etl::unix_seconds_to_datetime(*value).is_err(),
            Value::Real(value) => dl_central_etl::real_unix_seconds_to_datetime(*value).is_err(),
            _ => false,
        },
        ConvertError::InvalidDateText { .. } => match value {
            Value::Text(value) => dl_central_etl::text_to_date(value).is_err(),
            _ => false,
        },
        ConvertError::Json(_) => match value {
            Value::Text(value) => serde_json::from_str::<JsonValue>(value).is_err(),
            _ => false,
        },
    }
}

fn mapped_source_column<'a>(table_ledger: &'a TableLedger, target_column: &str) -> Option<&'a str> {
    table_ledger
        .columns
        .iter()
        .find_map(|(source_column, status)| match status {
            ColumnStatus::Mapped { to } if to.rsplit('.').next() == Some(target_column) => {
                Some(source_column.as_str())
            }
            ColumnStatus::Mapped { .. } | ColumnStatus::Dropped { .. } => None,
        })
}

fn source_row_count(source: &Connection, table: &str) -> rusqlite::Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {}", quote_sqlite_ident(table));
    source.query_row(&sql, [], |row| row.get::<_, u64>(0))
}

fn null_count(source: &Connection, table: &str, column: &str) -> rusqlite::Result<u64> {
    let sql = format!(
        "SELECT COUNT(*) FROM {} WHERE {} IS NULL",
        quote_sqlite_ident(table),
        quote_sqlite_ident(column)
    );
    source.query_row(&sql, [], |row| row.get::<_, u64>(0))
}

fn source_pk_sample(
    source: &Connection,
    table: &str,
    limit: usize,
) -> rusqlite::Result<Vec<String>> {
    source_pk_sample_with_where(source, table, None, limit)
}

fn source_pk_sample_where_null(
    source: &Connection,
    table: &str,
    column: &str,
    limit: usize,
) -> rusqlite::Result<Vec<String>> {
    source_pk_sample_with_where(
        source,
        table,
        Some(format!("{} IS NULL", quote_sqlite_ident(column))),
        limit,
    )
}

fn source_pk_sample_with_where(
    source: &Connection,
    table: &str,
    where_sql: Option<String>,
    limit: usize,
) -> rusqlite::Result<Vec<String>> {
    let pk_columns = sqlite_primary_key_columns(source, table)?;
    let where_clause = where_sql
        .as_ref()
        .map(|sql| format!(" WHERE {sql}"))
        .unwrap_or_default();

    if pk_columns.is_empty() {
        let sql = format!(
            "SELECT rowid FROM {}{} LIMIT {}",
            quote_sqlite_ident(table),
            where_clause,
            limit
        );
        let mut stmt = source.prepare(&sql)?;
        let rows = stmt.query_map([], |row| Ok(format!("rowid={}", row.get::<_, i64>(0)?)))?;
        return rows.collect();
    }

    let select = pk_columns
        .iter()
        .map(|column| quote_sqlite_ident(column))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {select} FROM {}{} LIMIT {}",
        quote_sqlite_ident(table),
        where_clause,
        limit
    );
    let mut stmt = source.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let mut fields = Vec::with_capacity(pk_columns.len());
        for (index, column) in pk_columns.iter().enumerate() {
            fields.push(format!(
                "{column}={}",
                sqlite_value_to_string(row.get_ref(index)?)
            ));
        }
        Ok(fields.join(","))
    })?;
    rows.collect()
}

fn sqlite_primary_key_columns(source: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let sql = format!("PRAGMA table_info({})", quote_sqlite_ident(table));
    let mut stmt = source.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(5)?, row.get::<_, String>(1)?))
    })?;

    let mut columns = BTreeMap::new();
    for row in rows {
        let (pk_ordinal, name) = row?;
        if pk_ordinal > 0 {
            columns.insert(pk_ordinal, name);
        }
    }
    Ok(columns.into_values().collect())
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

async fn verify_union_half_loaded(
    pool: &PgPool,
    table_results: &[TableResult],
    union_targets: &BTreeSet<&str>,
) {
    let mut by_target = BTreeMap::<String, (u64, u64)>::new();
    for result in table_results {
        if union_targets.contains(result.target.as_str()) {
            let entry = by_target.entry(result.target.clone()).or_default();
            entry.0 += result.source_rows;
            entry.1 += result.loaded_rows;
        }
    }

    for target in union_targets {
        let (source_rows, loaded_rows) = by_target.get(*target).copied().unwrap_or_default();
        assert_eq!(
            source_rows, loaded_rows,
            "union half loaded_rows mismatch for {target}"
        );
        let actual = target_count(pool, target).await;
        if source_rows > 0 {
            assert!(actual > 0, "union target {target} has no loaded rows");
        }
        println!(
            "union half ok: {target} source_rows={source_rows} loaded_rows={loaded_rows} target_rows={actual}"
        );
    }
}

async fn target_count(pool: &PgPool, target: &str) -> i64 {
    let sql = format!(
        "SELECT COUNT(*)::bigint AS count FROM {}",
        quote_pg_path(target)
    );
    let row = sqlx::query(&sql)
        .fetch_one(pool)
        .await
        .expect("count target rows");
    row.try_get::<i64, _>("count").expect("count as i64")
}

fn open_snapshot(path: &Path) -> Connection {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open snapshot read-only")
}

async fn assert_persistent_views_round_trip(source: &Connection, pool: &PgPool) {
    let sql = r#"
        SELECT message_id, channel_id, guild_id, view_type, user_id, created_at
        FROM persistent_views
        ORDER BY CAST(message_id AS INTEGER)
        LIMIT 3
    "#;
    let mut stmt = source
        .prepare(sql)
        .expect("prepare persistent_views sample");
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .expect("read persistent_views sample")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect persistent_views sample");
    assert_eq!(rows.len(), 3, "persistent_views sample must have 3 rows");

    for (message_id, channel_id, guild_id, view_type, user_id, created_at) in rows {
        let source_pk = format!("message_id={message_id}");
        let message_id = parse_i64(&message_id, "persistent_views.message_id", &source_pk);
        let row = sqlx::query(
            r#"
            SELECT
                message_id,
                channel_id,
                guild_id,
                view_type,
                user_id,
                to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS created_at
            FROM bot.persistent_views
            WHERE message_id = $1
            "#,
        )
        .bind(message_id)
        .fetch_one(pool)
        .await
        .expect("persistent_views target row exists");

        assert_eq!(row.try_get::<i64, _>("message_id").unwrap(), message_id);
        assert_eq!(
            row.try_get::<i64, _>("channel_id").unwrap(),
            parse_i64(&channel_id, "persistent_views.channel_id", &source_pk)
        );
        assert_eq!(
            row.try_get::<i64, _>("guild_id").unwrap(),
            parse_i64(&guild_id, "persistent_views.guild_id", &source_pk)
        );
        assert_eq!(row.try_get::<String, _>("view_type").unwrap(), view_type);
        assert_eq!(
            row.try_get::<Option<i64>, _>("user_id").unwrap(),
            parse_optional_i64(user_id.as_deref(), "persistent_views.user_id", &source_pk)
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("created_at").unwrap(),
            created_at
        );
    }

    println!("round-trip ok: bot.persistent_views rows=3 shape=TEXT_TO_BIGINT");
}

async fn assert_member_events_round_trip(source: &Connection, pool: &PgPool) {
    let sql = r#"
        SELECT
            id,
            user_id,
            guild_id,
            event_type,
            timestamp,
            display_name,
            account_created_at,
            join_position,
            metadata
        FROM member_events
        WHERE metadata IS NOT NULL
        ORDER BY id
        LIMIT 3
    "#;
    let mut stmt = source.prepare(sql).expect("prepare member_events sample");
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .expect("read member_events sample")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect member_events sample");
    assert_eq!(rows.len(), 3, "member_events sample must have 3 rows");

    for (
        id,
        user_id,
        guild_id,
        event_type,
        occurred_at,
        display_name,
        account_created_at,
        join_position,
        metadata,
    ) in rows
    {
        let row = sqlx::query(
            r#"
            SELECT
                id,
                user_id,
                guild_id,
                event_type,
                to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS occurred_at,
                display_name,
                to_char(account_created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS account_created_at,
                join_position,
                metadata::text AS metadata
            FROM activity.member_events
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("member_events target row exists");

        assert_eq!(row.try_get::<i64, _>("id").unwrap(), id);
        assert_eq!(row.try_get::<i64, _>("user_id").unwrap(), user_id);
        assert_eq!(row.try_get::<i64, _>("guild_id").unwrap(), guild_id);
        assert_eq!(row.try_get::<String, _>("event_type").unwrap(), event_type);
        assert_eq!(
            row.try_get::<Option<String>, _>("occurred_at").unwrap(),
            occurred_at
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("display_name").unwrap(),
            display_name
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("account_created_at")
                .unwrap(),
            account_created_at
        );
        assert_eq!(
            row.try_get::<Option<i32>, _>("join_position").unwrap(),
            join_position.map(|value| {
                i32::try_from(value).expect("member_events.join_position source value fits integer")
            })
        );
        assert_json_eq(
            metadata.as_deref(),
            row.try_get::<Option<String>, _>("metadata")
                .unwrap()
                .as_deref(),
            &format!("member_events.id={id}.metadata"),
        );
    }

    println!("round-trip ok: activity.member_events rows=3 shape=JSONB");
}

async fn assert_voice_channel_settings_round_trip(source: &Connection, pool: &PgPool) {
    let sql = r#"
        SELECT channel_id, guild_id, enabled, created_at, updated_at
        FROM voice_channel_settings
        ORDER BY channel_id
        LIMIT 3
    "#;
    let mut stmt = source
        .prepare(sql)
        .expect("prepare voice_channel_settings sample");
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .expect("read voice_channel_settings sample")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect voice_channel_settings sample");
    assert_eq!(
        rows.len(),
        3,
        "voice_channel_settings sample must have 3 rows"
    );

    for (channel_id, guild_id, enabled, created_at, updated_at) in rows {
        let row = sqlx::query(
            r#"
            SELECT
                channel_id,
                guild_id,
                enabled,
                to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS created_at,
                to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS updated_at
            FROM voice.voice_channel_settings
            WHERE channel_id = $1
            "#,
        )
        .bind(channel_id)
        .fetch_one(pool)
        .await
        .expect("voice_channel_settings target row exists");

        assert_eq!(row.try_get::<i64, _>("channel_id").unwrap(), channel_id);
        assert_eq!(row.try_get::<i64, _>("guild_id").unwrap(), guild_id);
        assert_eq!(
            row.try_get::<bool, _>("enabled").unwrap(),
            sqlite_bool(enabled, "voice_channel_settings.enabled")
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("created_at").unwrap(),
            created_at
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("updated_at").unwrap(),
            updated_at
        );
    }

    println!("round-trip ok: voice.voice_channel_settings rows=3 shape=INTEGER_TO_BOOL");
}

fn parse_i64(value: &str, column: &str, source_pk: &str) -> i64 {
    value
        .parse::<i64>()
        .unwrap_or_else(|err| panic!("{column} source_pk={source_pk} is not i64: {err}"))
}

fn parse_optional_i64(value: Option<&str>, column: &str, source_pk: &str) -> Option<i64> {
    value.map(|value| parse_i64(value, column, source_pk))
}

fn sqlite_bool(value: i64, column: &str) -> bool {
    match value {
        0 => false,
        1 => true,
        other => panic!("{column} is not SQLite bool 0/1: {other}"),
    }
}

fn assert_json_eq(source: Option<&str>, target: Option<&str>, label: &str) {
    let source = source.map(|value| {
        serde_json::from_str::<JsonValue>(value)
            .unwrap_or_else(|err| panic!("{label} source JSON invalid: {err}"))
    });
    let target = target.map(|value| {
        serde_json::from_str::<JsonValue>(value)
            .unwrap_or_else(|err| panic!("{label} target JSON invalid: {err}"))
    });
    assert_eq!(source, target, "{label}");
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
