use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::Path,
};

use chrono::NaiveDate;
use dl_central_etl::plan::GeneratedColumn;
use dl_central_etl::{
    build_table_plan, real_unix_seconds_to_datetime, reconcile_total, run, snapshot_db,
    source_snapshot_path, sqlite_int_to_bool, sqlite_numeric_to_bool, text_json_to_value,
    text_to_bigint, text_to_date, text_to_datetime, unix_seconds_to_datetime, ColumnPlan,
    ColumnStatus, Converter, EngineError, Ledger, LedgerSet, SourceRow, SourceSqlite, TableLedger,
    TablePlan, WEBSITE_SOURCE,
};
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tempfile::TempDir;

const EXPECTED_WEBSITE_SOURCE: &str = "/home/naniadm/Documents/Website/builds/backend/deadlock.db";
const WEBSITE_SOURCE_DB: &str = "website";

#[tokio::test]
#[ignore]
async fn website_source_snapshot_loads_and_round_trips_real_data() {
    assert_eq!(WEBSITE_SOURCE, EXPECTED_WEBSITE_SOURCE);
    let source_path = Path::new(WEBSITE_SOURCE);
    if !source_path.exists() {
        println!("SKIP website source fehlt: {}", source_path.display());
        return;
    }

    let snapshot_dir = TempDir::new().expect("create website snapshot dir");
    let snapshot_path = source_snapshot_path(snapshot_dir.path(), WEBSITE_SOURCE_DB);
    snapshot_db(source_path, &snapshot_path).expect("snapshot website source");
    println!("website snapshot: {}", snapshot_path.display());

    let source = SourceSqlite::open_read_only(&snapshot_path).expect("open website snapshot");
    let ledger_set = website_ledger_set();
    let website_ledger = ledger_set
        .source(WEBSITE_SOURCE_DB)
        .expect("website ledger is present");
    assert_eq!(
        website_ledger.tables.len(),
        21,
        "website table ledger count"
    );

    let pool = test_pool().await;
    let all_targets = targets_by_source_table(website_ledger)
        .into_values()
        .collect::<BTreeSet<_>>();
    clean_targets(&pool, &all_targets)
        .await
        .expect("clean website mapped targets before ETL");
    let report = match run(&ledger_set, snapshot_dir.path(), &pool).await {
        Ok(report) => report,
        Err(error) => {
            println!("{}", describe_engine_error(&error));
            print_conversion_findings(&pool, &source, website_ledger).await;
            panic!("website ETL failed: {error}");
        }
    };

    assert_eq!(report.tables.len(), 21, "website report table count");
    assert_eq!(report.total_source_rows, report.total_loaded);
    for table in &report.tables {
        assert_eq!(
            table.source_rows, table.loaded_rows,
            "source_rows == loaded_rows for {}",
            table.target
        );
    }

    let expected_targets = targets_by_source_table(website_ledger);
    let report_counts = aggregate_report_counts(&report.tables);
    assert_source_targets_are_reported(&expected_targets, &report_counts);
    assert_non_union_targets_reconciled(&pool, &report_counts).await;
    assert_union_targets_loaded(&pool, &report_counts).await;
    reconcile_total(&pool, &report.tables)
        .await
        .expect("website-only total reconciliation passes");

    assert_goal_target_date_converter(&pool, &source, website_ledger).await;
    let sampled_tables = assert_round_trip_samples(&pool, &source, website_ledger).await;
    assert!(
        sampled_tables.len() >= 3,
        "round-trip samples need at least 3 non-empty website tables"
    );
    assert!(
        sampled_tables.contains("coaching_requests"),
        "coaching.requests website sample is covered"
    );
    assert!(
        sampled_tables.contains("meta_users"),
        "TEXT to BIGINT sample is covered"
    );

    println!(
        "website tables={} source_rows={} loaded_rows={}",
        report.tables.len(),
        report.total_source_rows,
        report.total_loaded
    );
    println!(
        "website non_union_targets={}",
        joined_targets(non_union_targets(&report_counts))
    );
    println!(
        "website union_targets={}",
        joined_targets(union_targets_present(&report_counts))
    );
    println!(
        "website round_trip_tables={}",
        sampled_tables.iter().cloned().collect::<Vec<_>>().join(",")
    );
    println!("ECHTE_DATEN_BEFUNDE=keine");
}

fn website_ledger_set() -> LedgerSet {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("ledger");
    let all = LedgerSet::from_dir(root).expect("load ETL ledger set");
    let website = all
        .source(WEBSITE_SOURCE_DB)
        .expect("website ledger exists")
        .clone();

    LedgerSet {
        sources: BTreeMap::from([(WEBSITE_SOURCE_DB.to_string(), website)]),
    }
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

fn targets_by_source_table(ledger: &Ledger) -> BTreeMap<String, String> {
    ledger
        .tables
        .iter()
        .map(|(table_name, table)| (table_name.clone(), target_for_table(table)))
        .collect()
}

fn target_for_table(table: &TableLedger) -> String {
    let mut target = None;

    for status in table.columns.values() {
        let ColumnStatus::Mapped { to } = status else {
            continue;
        };
        let Some((table_path, _column)) = to.rsplit_once('.') else {
            panic!("invalid target path in website ledger: {to}");
        };
        match &target {
            Some(existing) => assert_eq!(existing, table_path),
            None => target = Some(table_path.to_string()),
        }
    }

    target.expect("website source table has mapped target")
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

fn assert_source_targets_are_reported(
    expected_targets: &BTreeMap<String, String>,
    report_counts: &BTreeMap<String, (u64, u64)>,
) {
    for (source_table, target) in expected_targets {
        assert!(
            report_counts.contains_key(target),
            "report includes {source_table} target {target}"
        );
    }
}

async fn assert_non_union_targets_reconciled(
    pool: &PgPool,
    report_counts: &BTreeMap<String, (u64, u64)>,
) {
    for (target, (source_rows, loaded_rows)) in report_counts {
        if union_targets().contains(target.as_str()) {
            continue;
        }

        assert_eq!(source_rows, loaded_rows, "{target} load count");
        let actual = target_count(pool, target).await;
        assert_eq!(*source_rows, actual, "{target} target row count");
    }
}

async fn assert_union_targets_loaded(pool: &PgPool, report_counts: &BTreeMap<String, (u64, u64)>) {
    for target in union_targets() {
        let (source_rows, loaded_rows) = report_counts.get(target).copied().unwrap_or_default();
        assert_eq!(source_rows, loaded_rows, "{target} website load count");
        let actual = target_count(pool, target).await;
        assert_eq!(source_rows, actual, "{target} website target rows");
    }

    let website_requests = filtered_count(
        pool,
        "coaching.requests",
        "\"request_uid\" LIKE $1",
        "website:coaching_requests:%",
    )
    .await;
    let non_website_requests = filtered_count(
        pool,
        "coaching.requests",
        "\"request_uid\" NOT LIKE $1",
        "website:coaching_requests:%",
    )
    .await;
    let expected_requests = report_counts
        .get("coaching.requests")
        .map(|(source_rows, _)| *source_rows)
        .unwrap_or_default();
    assert_eq!(expected_requests, website_requests);
    assert_eq!(0, non_website_requests);

    let row = sqlx::query(
        r#"
        SELECT
            COUNT(*)::bigint AS rows,
            COUNT(DISTINCT request_uid)::bigint AS distinct_rows
        FROM coaching.requests
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("count request_uid distinctness");
    let rows: i64 = row.try_get("rows").expect("rows as i64");
    let distinct_rows: i64 = row.try_get("distinct_rows").expect("distinct_rows as i64");
    assert_eq!(rows, distinct_rows, "request_uid values are unique");
}

async fn assert_goal_target_date_converter(pool: &PgPool, source: &SourceSqlite, ledger: &Ledger) {
    let table = ledger
        .table("coaching_goals")
        .expect("coaching_goals ledger table");
    let plan = build_table_plan(source, WEBSITE_SOURCE_DB, "coaching_goals", table, pool)
        .await
        .expect("build coaching_goals plan");
    let column = plan
        .columns
        .iter()
        .find(|column| {
            column.source_column == "target_date" && column.target_column == "target_date"
        })
        .expect("coaching_goals.target_date is mapped");

    assert_eq!(column.source_affinity, "TEXT");
    assert_eq!(column.target_pg_type, "date");
    assert_eq!(column.converter, Converter::TextToDate);
}

async fn assert_round_trip_samples(
    pool: &PgPool,
    source: &SourceSqlite,
    ledger: &Ledger,
) -> BTreeSet<String> {
    let mut sampled_tables = BTreeSet::new();

    for (source_table, table_ledger) in &ledger.tables {
        let sample_count =
            assert_round_trip_table(pool, source, source_table, table_ledger, 3).await;
        if sample_count > 0 {
            sampled_tables.insert(source_table.clone());
        }
    }

    sampled_tables
}

async fn assert_round_trip_table(
    pool: &PgPool,
    source: &SourceSqlite,
    source_table: &str,
    table_ledger: &TableLedger,
    max_rows: usize,
) -> usize {
    let plan = build_table_plan(source, WEBSITE_SOURCE_DB, source_table, table_ledger, pool)
        .await
        .unwrap_or_else(|error| panic!("build {source_table} plan: {error}"));
    let mut rows = source
        .read_rows(source_table)
        .unwrap_or_else(|error| panic!("read source rows for {source_table}: {error}"));

    rows.sort_by_key(|row| target_pk_values(&plan, row).join("\u{1f}"));
    let sample_count = rows.len().min(max_rows);
    for row in rows.iter().take(sample_count) {
        assert_round_trip_row(pool, &plan, row).await;
    }

    if sample_count > 0 {
        println!(
            "round_trip {} -> {} rows={} columns={}",
            source_table,
            plan.target,
            sample_count,
            plan.columns.len()
        );
    }

    sample_count
}

async fn assert_round_trip_row(pool: &PgPool, plan: &TablePlan, row: &SourceRow) {
    let pk_values = target_pk_values(plan, row);
    let row_key = pk_values.join(",");

    for column in &plan.columns {
        let expected = expected_value(plan, column, row);
        assert_target_value(pool, plan, &pk_values, column, &expected, &row_key).await;
    }
}

fn target_pk_values(plan: &TablePlan, row: &SourceRow) -> Vec<String> {
    plan.primary_key
        .iter()
        .map(|pk_column| {
            let column = plan
                .columns
                .iter()
                .find(|column| column.target_column == *pk_column)
                .unwrap_or_else(|| panic!("primary key column {pk_column} is mapped"));
            expected_value(plan, column, row)
                .lookup_text()
                .unwrap_or_else(|| panic!("primary key {pk_column} has non-lookup value"))
        })
        .collect()
}

fn expected_value(plan: &TablePlan, column: &ColumnPlan, row: &SourceRow) -> ExpectedValue {
    let source_value = row
        .values
        .get(&column.source_column)
        .unwrap_or_else(|| panic!("source column {} is present", column.source_column));

    if matches!(
        column.generated.as_ref(),
        Some(GeneratedColumn::SourceQualifiedRequestUid)
    ) {
        return ExpectedValue::Text(Some(format!(
            "{}:{}:{}",
            plan.source_db,
            plan.source_table,
            format_source_value(source_value)
        )));
    }

    if source_value.is_null() {
        return ExpectedValue::null_for(column.converter);
    }

    match column.converter {
        Converter::TextToText => ExpectedValue::Text(Some(source_text(source_value).to_string())),
        Converter::TextToJsonb => ExpectedValue::Json(Some(
            text_json_to_value(source_text(source_value)).expect("source JSON parses"),
        )),
        Converter::TextToBigint => ExpectedValue::Int8(Some(
            text_to_bigint(source_text(source_value)).expect("source bigint text parses"),
        )),
        Converter::TextToTimestamptz => ExpectedValue::Timestamp(Some(
            text_to_datetime(source_text(source_value))
                .expect("source timestamp text parses")
                .timestamp_micros() as f64
                / 1_000_000_f64,
        )),
        Converter::TextToDate => ExpectedValue::Date(Some(
            text_to_date(source_text(source_value)).expect("source date text parses"),
        )),
        Converter::NumericToTimestamptz => {
            if let Some(text) = source_value.as_str() {
                ExpectedValue::Timestamp(Some(
                    text_to_datetime(text)
                        .expect("source numeric timestamp text parses")
                        .timestamp_micros() as f64
                        / 1_000_000_f64,
                ))
            } else {
                ExpectedValue::Timestamp(Some(
                    unix_seconds_to_datetime(source_i64(source_value))
                        .expect("source numeric unix timestamp parses")
                        .timestamp_micros() as f64
                        / 1_000_000_f64,
                ))
            }
        }
        Converter::NumericToBool => ExpectedValue::Bool(Some(
            sqlite_numeric_to_bool(source_i64(source_value)).expect("source numeric bool parses"),
        )),
        Converter::IntegerToInt8 => ExpectedValue::Int8(Some(source_i64(source_value))),
        Converter::IntegerToInt4 => ExpectedValue::Int4(Some(
            i32::try_from(source_i64(source_value)).expect("source int4 fits"),
        )),
        Converter::IntegerToText => ExpectedValue::Text(Some(source_i64(source_value).to_string())),
        Converter::IntegerToBool => ExpectedValue::Bool(Some(
            sqlite_int_to_bool(source_i64(source_value)).expect("source int bool parses"),
        )),
        Converter::IntegerToTimestamptz => ExpectedValue::Timestamp(Some(
            unix_seconds_to_datetime(source_i64(source_value))
                .expect("source integer unix timestamp parses")
                .timestamp_micros() as f64
                / 1_000_000_f64,
        )),
        Converter::RealToFloat8 => ExpectedValue::Float8(Some(source_f64(source_value))),
        Converter::RealToTimestamptz => ExpectedValue::Timestamp(Some(
            real_unix_seconds_to_datetime(source_f64(source_value))
                .expect("source real unix timestamp parses")
                .timestamp_micros() as f64
                / 1_000_000_f64,
        )),
    }
}

async fn assert_target_value(
    pool: &PgPool,
    plan: &TablePlan,
    pk_values: &[String],
    column: &ColumnPlan,
    expected: &ExpectedValue,
    row_key: &str,
) {
    let table = quote_pg_path(&plan.target);
    let expression = target_expression(column, expected);
    let where_clause = plan
        .primary_key
        .iter()
        .enumerate()
        .map(|(index, column)| format!("{}::text = ${}", quote_pg_ident(column), index + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("SELECT {expression} AS value FROM {table} WHERE {where_clause}");
    let mut query = sqlx::query(&sql);
    for value in pk_values {
        query = query.bind(value);
    }
    let row = query
        .fetch_optional(pool)
        .await
        .unwrap_or_else(|error| panic!("fetch {} row {row_key}: {error}", plan.target))
        .unwrap_or_else(|| panic!("missing {} row {row_key}", plan.target));

    match expected {
        ExpectedValue::Text(expected) => {
            let actual: Option<String> = row.try_get("value").expect("target text value");
            assert_eq!(actual, *expected, "{}.{row_key}", column.target_column);
        }
        ExpectedValue::Int8(expected) => {
            let actual: Option<i64> = row.try_get("value").expect("target int8 value");
            assert_eq!(actual, *expected, "{}.{row_key}", column.target_column);
        }
        ExpectedValue::Int4(expected) => {
            let actual: Option<i32> = row.try_get("value").expect("target int4 value");
            assert_eq!(actual, *expected, "{}.{row_key}", column.target_column);
        }
        ExpectedValue::Bool(expected) => {
            let actual: Option<bool> = row.try_get("value").expect("target bool value");
            assert_eq!(actual, *expected, "{}.{row_key}", column.target_column);
        }
        ExpectedValue::Json(expected) => {
            let actual: Option<String> = row.try_get("value").expect("target jsonb text");
            let actual = actual.map(|text| {
                serde_json::from_str::<Value>(&text).expect("target jsonb text parses")
            });
            assert_eq!(actual, *expected, "{}.{row_key}", column.target_column);
        }
        ExpectedValue::Float8(expected) => {
            let actual: Option<f64> = row.try_get("value").expect("target float8 value");
            assert_optional_f64(actual, *expected, &column.target_column, row_key);
        }
        ExpectedValue::Timestamp(expected) => {
            let actual: Option<f64> = row.try_get("value").expect("target timestamp epoch");
            assert_optional_f64(actual, *expected, &column.target_column, row_key);
        }
        ExpectedValue::Date(expected) => {
            let actual: Option<NaiveDate> = row.try_get("value").expect("target date value");
            assert_eq!(actual, *expected, "{}.{row_key}", column.target_column);
        }
    }
}

fn target_expression(column: &ColumnPlan, expected: &ExpectedValue) -> String {
    let column = quote_pg_ident(&column.target_column);
    match expected {
        ExpectedValue::Json(_) => format!("{column}::text"),
        ExpectedValue::Timestamp(_) => format!("EXTRACT(EPOCH FROM {column})::double precision"),
        ExpectedValue::Text(_)
        | ExpectedValue::Int8(_)
        | ExpectedValue::Int4(_)
        | ExpectedValue::Bool(_)
        | ExpectedValue::Float8(_)
        | ExpectedValue::Date(_) => column,
    }
}

fn assert_optional_f64(actual: Option<f64>, expected: Option<f64>, field: &str, row_key: &str) {
    match (actual, expected) {
        (Some(actual), Some(expected)) => assert!(
            (actual - expected).abs() < 0.001,
            "{field}.{row_key}: expected {expected}, got {actual}"
        ),
        (None, None) => {}
        other => panic!("{field}.{row_key}: f64 null mismatch {other:?}"),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ExpectedValue {
    Text(Option<String>),
    Int8(Option<i64>),
    Int4(Option<i32>),
    Bool(Option<bool>),
    Json(Option<Value>),
    Float8(Option<f64>),
    Timestamp(Option<f64>),
    Date(Option<NaiveDate>),
}

impl ExpectedValue {
    fn null_for(converter: Converter) -> Self {
        match converter {
            Converter::TextToText | Converter::IntegerToText => Self::Text(None),
            Converter::TextToJsonb => Self::Json(None),
            Converter::TextToBigint | Converter::IntegerToInt8 => Self::Int8(None),
            Converter::IntegerToInt4 => Self::Int4(None),
            Converter::NumericToBool | Converter::IntegerToBool => Self::Bool(None),
            Converter::RealToFloat8 => Self::Float8(None),
            Converter::TextToDate => Self::Date(None),
            Converter::TextToTimestamptz
            | Converter::NumericToTimestamptz
            | Converter::IntegerToTimestamptz
            | Converter::RealToTimestamptz => Self::Timestamp(None),
        }
    }

    fn lookup_text(&self) -> Option<String> {
        match self {
            Self::Text(Some(value)) => Some(value.clone()),
            Self::Int8(Some(value)) => Some(value.to_string()),
            Self::Int4(Some(value)) => Some(value.to_string()),
            Self::Bool(Some(value)) => Some(value.to_string()),
            Self::Date(Some(value)) => Some(value.to_string()),
            Self::Json(_) | Self::Float8(_) | Self::Timestamp(_) => None,
            Self::Text(None)
            | Self::Int8(None)
            | Self::Int4(None)
            | Self::Bool(None)
            | Self::Date(None) => None,
        }
    }
}

fn source_text(value: &Value) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("source value is text: {value}"))
}

fn source_i64(value: &Value) -> i64 {
    value
        .as_i64()
        .unwrap_or_else(|| panic!("source value is i64: {value}"))
}

fn source_f64(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("source value is f64: {value}"))
}

fn format_source_value(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

async fn print_conversion_findings(pool: &PgPool, source: &SourceSqlite, ledger: &Ledger) {
    let mut findings = Vec::new();

    for (source_table, table_ledger) in &ledger.tables {
        let plan =
            match build_table_plan(source, WEBSITE_SOURCE_DB, source_table, table_ledger, pool)
                .await
            {
                Ok(plan) => plan,
                Err(error) => {
                    println!("{}", describe_engine_error(&error));
                    continue;
                }
            };
        let rows = source
            .read_rows(source_table)
            .unwrap_or_else(|error| panic!("read source rows for {source_table}: {error}"));

        for row in &rows {
            let source_pk = source_pk_for_row(&plan, row);
            for column in &plan.columns {
                if let Err(finding) = validate_source_value(&plan, column, row, &source_pk) {
                    findings.push(*finding);
                }
            }
        }
    }

    if findings.is_empty() {
        println!("BEFUND_SCAN=keine weiteren Konvertierungsbefunde");
        return;
    }

    for finding in findings {
        println!(
            "BEFUND fuer Claude: {}.{}.{}; klasse={}; quell_pk={}; error={}",
            finding.source_db,
            finding.table,
            finding.column,
            finding.class,
            finding.source_pk,
            finding.error
        );
    }
}

fn validate_source_value(
    plan: &TablePlan,
    column: &ColumnPlan,
    row: &SourceRow,
    source_pk: &str,
) -> Result<(), Box<ConversionFinding>> {
    let source_value = row
        .values
        .get(&column.source_column)
        .unwrap_or_else(|| panic!("source column {} is present", column.source_column));

    if matches!(
        column.generated.as_ref(),
        Some(GeneratedColumn::SourceQualifiedRequestUid)
    ) || source_value.is_null()
    {
        return Ok(());
    }

    let result = match column.converter {
        Converter::TextToText => source_text_result(source_value).map(|_| ()),
        Converter::TextToJsonb => source_text_result(source_value).and_then(|value| {
            text_json_to_value(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
        Converter::TextToBigint => source_text_result(source_value).and_then(|value| {
            text_to_bigint(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
        Converter::TextToTimestamptz => source_text_result(source_value).and_then(|value| {
            text_to_datetime(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
        Converter::TextToDate => source_text_result(source_value).and_then(|value| {
            text_to_date(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
        Converter::NumericToTimestamptz => {
            if let Some(value) = source_value.as_str() {
                text_to_datetime(value)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            } else if let Some(value) = source_value.as_i64() {
                unix_seconds_to_datetime(value)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            } else {
                Err(format!("source value is not text or i64: {source_value}"))
            }
        }
        Converter::NumericToBool => source_i64_result(source_value).and_then(|value| {
            sqlite_numeric_to_bool(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
        Converter::IntegerToInt8 | Converter::IntegerToText => {
            source_i64_result(source_value).map(|_| ())
        }
        Converter::IntegerToInt4 => source_i64_result(source_value).and_then(|value| {
            i32::try_from(value)
                .map(|_| ())
                .map_err(|_| format!("INTEGER-Wert ausserhalb von int4: {value}"))
        }),
        Converter::IntegerToBool => source_i64_result(source_value).and_then(|value| {
            sqlite_int_to_bool(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
        Converter::IntegerToTimestamptz => source_i64_result(source_value).and_then(|value| {
            unix_seconds_to_datetime(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
        Converter::RealToFloat8 => source_f64_result(source_value).map(|_| ()),
        Converter::RealToTimestamptz => source_f64_result(source_value).and_then(|value| {
            real_unix_seconds_to_datetime(value)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }),
    };

    result.map_err(|error| {
        Box::new(ConversionFinding {
            source_db: plan.source_db.clone(),
            table: plan.source_table.clone(),
            column: column.source_column.clone(),
            source_pk: source_pk.to_string(),
            class: if error.starts_with("source value is") {
                "InvalidSourceValueType"
            } else if error.starts_with("INTEGER-Wert ausserhalb") {
                "Int4OutOfRange"
            } else {
                "Conversion"
            },
            error,
        })
    })
}

fn source_pk_for_row(plan: &TablePlan, row: &SourceRow) -> String {
    plan.primary_key
        .iter()
        .map(|pk_column| {
            let value = plan
                .columns
                .iter()
                .find(|column| column.target_column == pk_column.as_str())
                .and_then(|column| {
                    row.values
                        .get(&column.source_column)
                        .map(|value| source_pk_value(plan, column, value))
                })
                .unwrap_or_else(|| "<missing>".to_string());
            format!("{pk_column}={value}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn source_pk_value(plan: &TablePlan, column: &ColumnPlan, value: &Value) -> String {
    if matches!(
        column.generated.as_ref(),
        Some(GeneratedColumn::SourceQualifiedRequestUid)
    ) {
        format!(
            "{}:{}:{}",
            plan.source_db,
            plan.source_table,
            format_source_value(value)
        )
    } else {
        format_source_value(value)
    }
}

fn source_text_result(value: &Value) -> Result<&str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("source value is not text: {value}"))
}

fn source_i64_result(value: &Value) -> Result<i64, String> {
    value
        .as_i64()
        .ok_or_else(|| format!("source value is not i64: {value}"))
}

fn source_f64_result(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .ok_or_else(|| format!("source value is not f64: {value}"))
}

#[derive(Debug)]
struct ConversionFinding {
    source_db: String,
    table: String,
    column: String,
    source_pk: String,
    class: &'static str,
    error: String,
}

async fn target_count(pool: &PgPool, target: &str) -> u64 {
    let sql = format!(
        "SELECT COUNT(*)::bigint AS count FROM {}",
        quote_pg_path(target)
    );
    let row = sqlx::query(&sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("count {target}: {error}"));
    let count: i64 = row.try_get("count").expect("count as i64");
    u64::try_from(count).expect("non-negative count")
}

async fn filtered_count(pool: &PgPool, target: &str, filter: &str, value: &str) -> u64 {
    let sql = format!(
        "SELECT COUNT(*)::bigint AS count FROM {} WHERE {filter}",
        quote_pg_path(target)
    );
    let row = sqlx::query(&sql)
        .bind(value)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("filtered count {target}: {error}"));
    let count: i64 = row.try_get("count").expect("count as i64");
    u64::try_from(count).expect("non-negative filtered count")
}

fn union_targets() -> BTreeSet<&'static str> {
    BTreeSet::from([
        "coaching.requests",
        "coaching.coaches",
        "coaching.coach_applications",
    ])
}

fn non_union_targets(report_counts: &BTreeMap<String, (u64, u64)>) -> BTreeSet<String> {
    report_counts
        .keys()
        .filter(|target| !union_targets().contains(target.as_str()))
        .cloned()
        .collect()
}

fn union_targets_present(report_counts: &BTreeMap<String, (u64, u64)>) -> BTreeSet<String> {
    report_counts
        .keys()
        .filter(|target| union_targets().contains(target.as_str()))
        .cloned()
        .collect()
}

fn joined_targets(targets: BTreeSet<String>) -> String {
    targets.into_iter().collect::<Vec<_>>().join(",")
}

fn quote_pg_path(path: &str) -> String {
    path.split('.')
        .map(quote_pg_ident)
        .collect::<Vec<_>>()
        .join(".")
}

fn quote_pg_ident(identifier: &str) -> String {
    assert!(!identifier.trim().is_empty(), "empty identifier");
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn describe_engine_error(error: &EngineError) -> String {
    match error {
        EngineError::Conversion {
            source_db,
            table,
            column,
            source_pk,
            source,
        } => format!(
            "BEFUND fuer Claude: {source_db}.{table}.{column}; klasse=Conversion; quell_pk={source_pk}; error={source}"
        ),
        EngineError::InvalidSourceValueType {
            source_db,
            table,
            column,
            source_pk,
            value,
        } => format!(
            "BEFUND fuer Claude: {source_db}.{table}.{column}; klasse=InvalidSourceValueType; quell_pk={source_pk}; value={value}"
        ),
        EngineError::Int4OutOfRange {
            source_db,
            table,
            column,
            source_pk,
            value,
        } => format!(
            "BEFUND fuer Claude: {source_db}.{table}.{column}; klasse=Int4OutOfRange; quell_pk={source_pk}; value={value}"
        ),
        EngineError::NoConverter {
            table,
            column,
            source_affinity,
            target_type,
        } => format!(
            "BEFUND fuer Claude: {table}.{column}; klasse=NoConverter; quell_pk=<nicht verfuegbar>; {source_affinity}->{target_type}"
        ),
        EngineError::MissingSourceColumn {
            source_db,
            table,
            column,
        } => format!(
            "BEFUND fuer Claude: {source_db}.{table}.{column}; klasse=MissingSourceColumn; quell_pk=<nicht verfuegbar>"
        ),
        other => format!(
            "BEFUND fuer Claude: klasse={}; quell_pk=<nicht verfuegbar>; error={other}",
            std::any::type_name_of_val(other)
        ),
    }
}
