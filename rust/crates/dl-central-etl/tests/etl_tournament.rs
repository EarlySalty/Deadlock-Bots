use std::{collections::BTreeMap, env, fs, path::Path};

use chrono::{DateTime, Utc};
use dl_central_etl::{
    build_table_plan, real_unix_seconds_to_datetime, reconcile_total, run, sample_round_trip,
    snapshot_db, source_snapshot_path, text_json_to_value, text_to_bigint, text_to_date,
    text_to_datetime, unix_seconds_to_datetime, ColumnStatus, ConvertError, Converter, EngineError,
    Ledger, LedgerSet, RoundTripFields, RoundTripRows, SourceRow, SourceSqlite, TOURNAMENT_SOURCE,
};
use serde_json::{Number, Value};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tempfile::TempDir;

const SOURCE_DB: &str = "tournament";
const EXPECTED_TOURNAMENT_SOURCE: &str =
    "/home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db";
const LEDGER_PATH: &str = "ledger/tournament/turnier.toml";
const LEDGER_TABLES: usize = 38;
const TARGET_TABLES: usize = 37;
const ROUND_TRIP_TABLES: &[&str] = &["audit_log", "rank_cache", "tournaments", "user_profiles"];

#[tokio::test]
#[ignore]
async fn tournament_snapshot_loads_and_verifies_real_data() {
    assert_eq!(TOURNAMENT_SOURCE, EXPECTED_TOURNAMENT_SOURCE);

    let source_path = Path::new(TOURNAMENT_SOURCE);
    if !source_path.exists() {
        println!("SKIP: tournament source fehlt: {}", source_path.display());
        return;
    }

    let pool = test_pool().await;
    let snapshot_dir = TempDir::new().expect("create tournament snapshot dir");
    let snapshot_path = source_snapshot_path(snapshot_dir.path(), SOURCE_DB);
    snapshot_db(source_path, &snapshot_path).unwrap_or_else(|err| {
        panic!(
            "BEFUND fuer Claude: source=tournament table=<snapshot> column=<snapshot> error_class={} source_pk=<none>",
            snapshot_error_class(&err)
        )
    });

    let source = SourceSqlite::open_read_only(&snapshot_path).expect("open tournament snapshot");
    let ledger = mapped_tournament_ledger();
    let ledger_set = LedgerSet {
        sources: BTreeMap::from([(SOURCE_DB.to_string(), ledger.clone())]),
    };

    assert_eq!(ledger_set.sources.len(), 1);
    assert_eq!(ledger.tables.len(), TARGET_TABLES);
    assert_source_tables_present(&source, &ledger);
    truncate_turnier_targets(&pool).await;

    let report = run(&ledger_set, snapshot_dir.path(), &pool)
        .await
        .unwrap_or_else(|err| panic!("{}", engine_finding(&err)));

    assert_eq!(report.tables.len(), TARGET_TABLES);
    assert_eq!(report.total_source_rows, report.total_loaded);
    for table in &report.tables {
        assert_eq!(
            table.source_rows, table.loaded_rows,
            "row count mismatch for {}",
            table.target
        );
    }
    reconcile_total(&pool, &report.tables)
        .await
        .expect("turnier total reconciliation passes");

    let counts = aggregate_table_counts(&report.tables);
    assert_eq!(counts.len(), TARGET_TABLES);
    for (target, (source_rows, loaded_rows)) in &counts {
        println!("{target}: source_rows={source_rows} loaded_rows={loaded_rows}");
    }

    verify_round_trip_samples(&source, &ledger, &pool)
        .await
        .unwrap_or_else(|message| panic!("{message}"));
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

fn mapped_tournament_ledger() -> Ledger {
    let ledger_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(LEDGER_PATH);
    let contents = fs::read_to_string(&ledger_path).expect("read tournament ledger");
    let full = Ledger::from_toml_str(&contents).expect("parse tournament ledger");
    assert_eq!(full.tables.len(), LEDGER_TABLES);

    let dropped_only = full
        .tables
        .iter()
        .filter_map(|(table, table_ledger)| {
            let has_mapped = table_ledger
                .columns
                .values()
                .any(|status| matches!(status, ColumnStatus::Mapped { .. }));
            (!has_mapped).then_some(table.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(dropped_only, vec!["_sqlx_migrations"]);

    let tables = full
        .tables
        .into_iter()
        .filter(|(_, table_ledger)| {
            table_ledger
                .columns
                .values()
                .any(|status| matches!(status, ColumnStatus::Mapped { .. }))
        })
        .collect::<BTreeMap<_, _>>();

    Ledger { tables }
}

fn assert_source_tables_present(source: &SourceSqlite, ledger: &Ledger) {
    for table in ledger.tables.keys() {
        source
            .table_schema(table)
            .unwrap_or_else(|_| panic!("source table missing from tournament snapshot: {table}"));
    }
}

async fn truncate_turnier_targets(pool: &PgPool) {
    let rows = sqlx::query(
        r#"
        SELECT quote_ident(table_schema) || '.' || quote_ident(table_name) AS table_path
        FROM information_schema.tables
        WHERE table_schema = 'turnier'
          AND table_type = 'BASE TABLE'
        ORDER BY table_name
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("list turnier target tables");

    let table_paths = rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("table_path").expect("table path"))
        .collect::<Vec<_>>();
    assert_eq!(table_paths.len(), TARGET_TABLES);

    let sql = format!(
        "TRUNCATE TABLE {} RESTART IDENTITY CASCADE",
        table_paths.join(", ")
    );
    sqlx::query(&sql)
        .execute(pool)
        .await
        .expect("truncate turnier targets");
}

async fn verify_round_trip_samples(
    source: &SourceSqlite,
    ledger: &Ledger,
    pool: &PgPool,
) -> Result<(), String> {
    for table in ROUND_TRIP_TABLES {
        let table_ledger = ledger
            .tables
            .get(*table)
            .ok_or_else(|| format!("round-trip ledger table missing: {table}"))?;
        let plan = build_table_plan(source, SOURCE_DB, table, table_ledger, pool)
            .await
            .map_err(|err| engine_finding(&err))?;
        let source_rows = source
            .read_rows(table)
            .map_err(|err| format!("round-trip source read failed for {table}: {err}"))?;
        if source_rows.len() < 3 {
            return Err(format!(
                "round-trip sample too small for tournament.{table}: rows={}",
                source_rows.len()
            ));
        }

        let expected = source_rows
            .iter()
            .take(3)
            .map(|row| expected_target_row(table, &plan, row))
            .collect::<Result<RoundTripRows, _>>()?;
        let target_rows = target_rows(pool, &plan).await?;
        let actual = expected
            .keys()
            .map(|key| {
                let fields = target_rows.get(key).cloned().ok_or_else(|| {
                    format!("round-trip target row missing for tournament.{table}: source_pk={key}")
                })?;
                Ok((key.clone(), fields))
            })
            .collect::<Result<RoundTripRows, String>>()?;

        sample_round_trip(&expected, &actual)
            .map_err(|err| format!("round-trip mismatch for tournament.{table}: {err}"))?;
        println!("round_trip_ok: tournament.{table} rows=3");
    }

    Ok(())
}

fn expected_target_row(
    table: &str,
    plan: &dl_central_etl::TablePlan,
    row: &SourceRow,
) -> Result<(String, RoundTripFields), String> {
    let mut fields = RoundTripFields::new();

    for column in &plan.columns {
        if column.generated.is_some() {
            return Err(format!(
                "unexpected generated column in tournament.{table}.{}",
                column.target_column
            ));
        }

        let source_value = row.values.get(&column.source_column).ok_or_else(|| {
            format!(
                "source column missing in tournament.{table}.{}",
                column.source_column
            )
        })?;
        let expected =
            expected_value(table, &column.source_column, column.converter, source_value)?;
        fields.insert(column.target_column.clone(), expected);
    }

    let key = target_key(plan, &fields)?;
    Ok((key, fields))
}

fn expected_value(
    table: &str,
    column: &str,
    converter: Converter,
    value: &Value,
) -> Result<Value, String> {
    if value.is_null() {
        return Ok(Value::Null);
    }

    match converter {
        Converter::TextToText => Ok(Value::String(source_text(table, column, value)?.to_string())),
        Converter::TextToJsonb => text_json_to_value(source_text(table, column, value)?)
            .map_err(|err| round_trip_convert_error(table, column, &err)),
        Converter::TextToBigint => text_to_bigint(source_text(table, column, value)?)
            .map(number_value)
            .map_err(|err| round_trip_convert_error(table, column, &err)),
        Converter::TextToTimestamptz => text_to_datetime(source_text(table, column, value)?)
            .map(timestamp_value)
            .map_err(|err| round_trip_convert_error(table, column, &err)),
        Converter::TextToDate => text_to_date(source_text(table, column, value)?)
            .map(|date| Value::String(date.to_string()))
            .map_err(|err| round_trip_convert_error(table, column, &err)),
        Converter::NumericToTimestamptz => numeric_to_timestamp(value)
            .map(timestamp_value)
            .map_err(|err| round_trip_convert_error(table, column, &err)),
        Converter::NumericToBool | Converter::IntegerToBool => match source_i64(table, column, value)? {
            0 => Ok(Value::Bool(false)),
            1 => Ok(Value::Bool(true)),
            _ => Err(format!(
                "round-trip conversion failed for tournament.{table}.{column}: error_class=invalid_bool_integer"
            )),
        },
        Converter::IntegerToInt8 => Ok(number_value(source_i64(table, column, value)?)),
        Converter::IntegerToInt4 => {
            let value = source_i64(table, column, value)?;
            let value = i32::try_from(value).map_err(|_| {
                format!(
                    "round-trip conversion failed for tournament.{table}.{column}: error_class=int4_out_of_range"
                )
            })?;
            Ok(number_value(i64::from(value)))
        }
        Converter::IntegerToText => Ok(Value::String(source_i64(table, column, value)?.to_string())),
        Converter::IntegerToTimestamptz => unix_seconds_to_datetime(source_i64(table, column, value)?)
            .map(timestamp_value)
            .map_err(|err| round_trip_convert_error(table, column, &err)),
        Converter::RealToFloat8 => Number::from_f64(source_f64(table, column, value)?)
            .map(Value::Number)
            .ok_or_else(|| {
                format!(
                    "round-trip conversion failed for tournament.{table}.{column}: error_class=non_finite_real"
                )
            }),
        Converter::RealToTimestamptz => {
            let seconds = source_f64(table, column, value)?;
            real_unix_seconds_to_datetime(seconds)
                .map(timestamp_value)
                .map_err(|err| round_trip_convert_error(table, column, &err))
        }
    }
}

fn numeric_to_timestamp(value: &Value) -> Result<DateTime<Utc>, ConvertError> {
    if let Some(text) = value.as_str() {
        return text_to_datetime(text);
    }

    if let Some(seconds) = value.as_i64() {
        return unix_seconds_to_datetime(seconds);
    }

    Err(ConvertError::InvalidTimestampText {
        value: value.to_string(),
    })
}

fn timestamp_value(value: DateTime<Utc>) -> Value {
    number_value(value.timestamp_micros())
}

fn number_value(value: i64) -> Value {
    Value::Number(Number::from(value))
}

fn source_text<'a>(table: &str, column: &str, value: &'a Value) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| {
        format!("round-trip source type mismatch for tournament.{table}.{column}: expected=text")
    })
}

fn source_i64(table: &str, column: &str, value: &Value) -> Result<i64, String> {
    value.as_i64().ok_or_else(|| {
        format!("round-trip source type mismatch for tournament.{table}.{column}: expected=integer")
    })
}

fn source_f64(table: &str, column: &str, value: &Value) -> Result<f64, String> {
    value.as_f64().ok_or_else(|| {
        format!("round-trip source type mismatch for tournament.{table}.{column}: expected=real")
    })
}

async fn target_rows(
    pool: &PgPool,
    plan: &dl_central_etl::TablePlan,
) -> Result<RoundTripRows, String> {
    let fields = plan
        .columns
        .iter()
        .map(|column| {
            format!(
                "{}, {}",
                quote_pg_string(&column.target_column),
                target_json_expr(&column.target_column, &column.target_pg_type)
            )
        })
        .collect::<Vec<_>>();
    let sql = format!(
        "SELECT jsonb_build_object({})::text AS row_json FROM {}",
        fields.join(", "),
        quote_pg_path(&plan.target_schema, &plan.target_table)
    );
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|err| format!("round-trip target query failed for {}: {err}", plan.target))?;

    let mut out = RoundTripRows::new();
    for row in rows {
        let row_json = row
            .try_get::<String, _>("row_json")
            .map_err(|err| format!("round-trip target row decode failed: {err}"))?;
        let value = serde_json::from_str::<Value>(&row_json)
            .map_err(|err| format!("round-trip target JSON parse failed: {err}"))?;
        let fields = object_fields(value)?;
        let key = target_key(plan, &fields)?;
        out.insert(key, fields);
    }

    Ok(out)
}

fn object_fields(value: Value) -> Result<RoundTripFields, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "round-trip target row is not a JSON object".to_string())?;
    Ok(object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn target_key(
    plan: &dl_central_etl::TablePlan,
    fields: &RoundTripFields,
) -> Result<String, String> {
    let mut parts = Vec::new();
    for pk_column in &plan.primary_key {
        let value = fields.get(pk_column).ok_or_else(|| {
            format!(
                "round-trip target key column missing for {}.{}",
                plan.target, pk_column
            )
        })?;
        parts.push(format!("{pk_column}={}", key_value(value)));
    }

    Ok(parts.join(","))
}

fn key_value(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn target_json_expr(column: &str, target_pg_type: &str) -> String {
    let column = quote_pg_ident(column);
    match normalized_target_type(target_pg_type).as_str() {
        "jsonb" => column,
        "timestamptz" => format!(
            "CASE WHEN {column} IS NULL THEN NULL ELSE to_jsonb(((EXTRACT(EPOCH FROM {column}) * 1000000)::bigint)) END"
        ),
        _ => format!("to_jsonb({column})"),
    }
}

fn normalized_target_type(target_pg_type: &str) -> String {
    match target_pg_type.trim().to_ascii_lowercase().as_str() {
        "timestamp with time zone" | "timestamptz" => "timestamptz".to_string(),
        other => other.to_string(),
    }
}

fn quote_pg_path(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_pg_ident(schema), quote_pg_ident(table))
}

fn quote_pg_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_pg_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn aggregate_table_counts(tables: &[dl_central_etl::TableResult]) -> BTreeMap<String, (u64, u64)> {
    let mut counts = BTreeMap::<String, (u64, u64)>::new();
    for table in tables {
        let entry = counts.entry(table.target.clone()).or_default();
        entry.0 += table.source_rows;
        entry.1 += table.loaded_rows;
    }
    counts
}

fn engine_finding(err: &EngineError) -> String {
    match err {
        EngineError::Conversion {
            source_db,
            table,
            column,
            source_pk,
            source,
        } => format!(
            "BEFUND fuer Claude: source={source_db} table={table} column={column} error_class={} source_pk={source_pk}",
            convert_error_class(source)
        ),
        EngineError::InvalidSourceValueType {
            source_db,
            table,
            column,
            source_pk,
            ..
        } => format!(
            "BEFUND fuer Claude: source={source_db} table={table} column={column} error_class=invalid_source_value_type source_pk={source_pk}"
        ),
        EngineError::Int4OutOfRange {
            source_db,
            table,
            column,
            source_pk,
            ..
        } => format!(
            "BEFUND fuer Claude: source={source_db} table={table} column={column} error_class=int4_out_of_range source_pk={source_pk}"
        ),
        EngineError::NoConverter {
            table,
            column,
            source_affinity,
            target_type,
        } => format!(
            "BEFUND fuer Claude: source=tournament table={table} column={column} error_class=no_converter:{source_affinity}->{target_type} source_pk=<planning>"
        ),
        EngineError::MissingSourceColumn {
            source_db,
            table,
            column,
        } => format!(
            "BEFUND fuer Claude: source={source_db} table={table} column={column} error_class=missing_source_column source_pk=<planning>"
        ),
        EngineError::MissingTargetColumn { target, column } => format!(
            "BEFUND fuer Claude: source=tournament table={target} column={column} error_class=missing_target_column source_pk=<planning>"
        ),
        EngineError::MissingPrimaryKeyColumn { target, column } => format!(
            "BEFUND fuer Claude: source=tournament table={target} column={column} error_class=missing_primary_key_column source_pk=<planning>"
        ),
        EngineError::TargetTableNotFound { target } => format!(
            "BEFUND fuer Claude: source=tournament table={target} column=<table> error_class=target_table_not_found source_pk=<planning>"
        ),
        EngineError::MissingPrimaryKey { target } => format!(
            "BEFUND fuer Claude: source=tournament table={target} column=<primary_key> error_class=missing_primary_key source_pk=<planning>"
        ),
        EngineError::NoMappedColumns { source_db, table } => format!(
            "BEFUND fuer Claude: source={source_db} table={table} column=<mapped> error_class=no_mapped_columns source_pk=<planning>"
        ),
        EngineError::MixedTargetTables {
            source_db, table, ..
        } => format!(
            "BEFUND fuer Claude: source={source_db} table={table} column=<ledger> error_class=mixed_target_tables source_pk=<planning>"
        ),
        EngineError::DuplicateTargetColumn { target, column } => format!(
            "BEFUND fuer Claude: source=tournament table={target} column={column} error_class=duplicate_target_column source_pk=<planning>"
        ),
        EngineError::UnsupportedGeneratedPrimaryKey {
            source_db,
            table,
            target,
            column,
        } => format!(
            "BEFUND fuer Claude: source={source_db} table={table}->{target} column={column} error_class=unsupported_generated_primary_key source_pk=<planning>"
        ),
        EngineError::Source(_) => {
            "BEFUND fuer Claude: source=tournament table=<source> column=<source> error_class=source_error source_pk=<unknown>".to_string()
        }
        EngineError::Target(_) => {
            "BEFUND fuer Claude: source=tournament table=<target> column=<target> error_class=target_error source_pk=<unknown>".to_string()
        }
        EngineError::Verify(_) => {
            "BEFUND fuer Claude: source=tournament table=<verify> column=<verify> error_class=verify_error source_pk=<unknown>".to_string()
        }
        EngineError::Sqlx(_) => {
            "BEFUND fuer Claude: source=tournament table=<postgres> column=<postgres> error_class=sqlx_error source_pk=<unknown>".to_string()
        }
        EngineError::InvalidTargetPath { target } => format!(
            "BEFUND fuer Claude: source=tournament table={target} column=<ledger> error_class=invalid_target_path source_pk=<planning>"
        ),
    }
}

fn convert_error_class(err: &ConvertError) -> &'static str {
    match err {
        ConvertError::InvalidUnixTimestamp { .. } => "invalid_unix_timestamp",
        ConvertError::InvalidRealUnixTimestamp { .. } => "invalid_real_unix_timestamp",
        ConvertError::InvalidBoolInteger { .. } => "invalid_bool_integer",
        ConvertError::InvalidBigintText { .. } => "invalid_bigint_text",
        ConvertError::BigintTextOutOfRange { .. } => "bigint_text_out_of_range",
        ConvertError::InvalidTimestampText { .. } => "invalid_timestamp_text",
        ConvertError::InvalidDateText { .. } => "invalid_date_text",
        ConvertError::Json(_) => "invalid_json",
    }
}

fn round_trip_convert_error(table: &str, column: &str, err: &ConvertError) -> String {
    format!(
        "round-trip conversion failed for tournament.{table}.{column}: error_class={}",
        convert_error_class(err)
    )
}

fn snapshot_error_class(err: &dl_central_etl::SnapshotError) -> &'static str {
    match err {
        dl_central_etl::SnapshotError::MissingSource { .. } => "missing_source",
        dl_central_etl::SnapshotError::NonUtf8Path { .. } => "non_utf8_path",
        dl_central_etl::SnapshotError::SourceEqualsDestination { .. } => {
            "source_equals_destination"
        }
        dl_central_etl::SnapshotError::Io { .. } => "snapshot_io",
        dl_central_etl::SnapshotError::Sqlite(_) => "snapshot_sqlite",
    }
}
