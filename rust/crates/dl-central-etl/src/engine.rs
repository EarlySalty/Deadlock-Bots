use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::{
    convert::{
        integer_to_text, real_unix_seconds_to_datetime, sqlite_int_to_bool, sqlite_numeric_to_bool,
        text_json_to_value, text_to_bigint, text_to_date, unix_seconds_to_datetime, ConvertError,
    },
    ledger::{LedgerSet, TableLedger},
    plan::{build_table_plan, ColumnPlan, Converter, GeneratedColumn, TablePlan},
    snapshot::source_snapshot_path,
    source::{SourceError, SourceRow, SourceSqlite},
    target::{TargetError, TargetRow, TargetValue, TargetWriter},
    verify::{check_mapping_completeness, VerifyError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtlReport {
    pub tables: Vec<TableResult>,
    pub total_source_rows: u64,
    pub total_loaded: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableResult {
    pub target: String,
    pub source_rows: u64,
    pub loaded_rows: u64,
    pub dropped_cols: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Target(#[from] TargetError),
    #[error(transparent)]
    Verify(#[from] VerifyError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("kein Konverter fuer {table}.{column}: SQLite {source_affinity} -> PG {target_type}")]
    NoConverter {
        source_affinity: String,
        target_type: String,
        table: String,
        column: String,
    },
    #[error("ungueltiger Ledger-Zielpfad: {target}")]
    InvalidTargetPath { target: String },
    #[error("Quelltabelle {source_db}.{table} hat keine gemappten Spalten")]
    NoMappedColumns { source_db: String, table: String },
    #[error("Quelltabelle {source_db}.{table} mappt auf mehrere Ziele: {first}, {next}")]
    MixedTargetTables {
        source_db: String,
        table: String,
        first: String,
        next: String,
    },
    #[error("Target-Tabelle fehlt: {target}")]
    TargetTableNotFound { target: String },
    #[error("Target-Spalte fehlt: {target}.{column}")]
    MissingTargetColumn { target: String, column: String },
    #[error("Target-Spalte ist doppelt gemappt: {target}.{column}")]
    DuplicateTargetColumn { target: String, column: String },
    #[error("Target-Tabelle hat keinen Primary Key: {target}")]
    MissingPrimaryKey { target: String },
    #[error("Target-PK-Spalte ist nicht gemappt: {target}.{column}")]
    MissingPrimaryKeyColumn { target: String, column: String },
    #[error(
        "generierter Primary Key wird nicht unterstuetzt: {source_db}.{table} -> {target}.{column}"
    )]
    UnsupportedGeneratedPrimaryKey {
        source_db: String,
        table: String,
        target: String,
        column: String,
    },
    #[error("Quellspalte fehlt: {source_db}.{table}.{column}")]
    MissingSourceColumn {
        source_db: String,
        table: String,
        column: String,
    },
    #[error(
        "Quellwert hat falschen Typ: {source_db}.{table}.{column} row={source_pk} value={value}"
    )]
    InvalidSourceValueType {
        source_db: String,
        table: String,
        column: String,
        source_pk: String,
        value: Box<Value>,
    },
    #[error("INTEGER-Wert ausserhalb von int4: {source_db}.{table}.{column} row={source_pk} value={value}")]
    Int4OutOfRange {
        source_db: String,
        table: String,
        column: String,
        source_pk: String,
        value: i64,
    },
    #[error(
        "Konvertierung fehlgeschlagen: {source_db}.{table}.{column} row={source_pk}: {source}"
    )]
    Conversion {
        source_db: String,
        table: String,
        column: String,
        source_pk: String,
        #[source]
        source: Box<ConvertError>,
    },
}

pub async fn migrate_table(
    source: &SourceSqlite,
    source_db: &str,
    table: &str,
    table_ledger: &TableLedger,
    plan: &TablePlan,
    target: &TargetWriter,
) -> Result<TableResult, EngineError> {
    let source_columns = source.table_columns(table)?;
    check_mapping_completeness(&source_columns, table_ledger)?;
    for ledger_column in table_ledger.columns.keys() {
        if !source_columns.contains(ledger_column) {
            return Err(EngineError::MissingSourceColumn {
                source_db: source_db.to_string(),
                table: table.to_string(),
                column: ledger_column.clone(),
            });
        }
    }

    let source_rows = source.read_rows(table)?;
    let target_rows = source_rows
        .iter()
        .map(|row| transform_row(source_db, table, plan, row))
        .collect::<Result<Vec<_>, _>>()?;
    let loaded_rows = target.upsert_rows(plan, &target_rows).await?;

    Ok(TableResult {
        target: plan.target.clone(),
        source_rows: source_rows.len() as u64,
        loaded_rows,
        dropped_cols: plan.dropped_cols.clone(),
    })
}

pub async fn run(
    ledger_set: &LedgerSet,
    snapshot_dir: &Path,
    pool: &PgPool,
) -> Result<EtlReport, EngineError> {
    let target = TargetWriter::new(pool.clone());
    let mut tables = Vec::new();
    let mut total_source_rows = 0;
    let mut total_loaded = 0;

    for (source_db, ledger) in &ledger_set.sources {
        let source_path = source_snapshot_path(snapshot_dir, source_db);
        let source = SourceSqlite::open_read_only(&source_path)?;

        for (table, table_ledger) in &ledger.tables {
            let plan = build_table_plan(&source, source_db, table, table_ledger, pool).await?;
            let result =
                migrate_table(&source, source_db, table, table_ledger, &plan, &target).await?;
            total_source_rows += result.source_rows;
            total_loaded += result.loaded_rows;
            tables.push(result);
        }
    }

    Ok(EtlReport {
        tables,
        total_source_rows,
        total_loaded,
    })
}

fn transform_row(
    source_db: &str,
    table: &str,
    plan: &TablePlan,
    row: &SourceRow,
) -> Result<TargetRow, EngineError> {
    let source_pk = source_pk(row, plan);
    let mut values = BTreeMap::new();

    for column in &plan.columns {
        let source_value = row.values.get(&column.source_column).ok_or_else(|| {
            EngineError::MissingSourceColumn {
                source_db: source_db.to_string(),
                table: table.to_string(),
                column: column.source_column.clone(),
            }
        })?;
        let target_value = if let Some(generated) = &column.generated {
            generate_value(source_db, table, generated, source_value)
        } else {
            convert_value(source_db, table, &source_pk, column, source_value)?
        };
        values.insert(column.target_column.clone(), target_value);
    }

    Ok(TargetRow { values })
}

fn convert_value(
    source_db: &str,
    table: &str,
    source_pk: &str,
    column: &ColumnPlan,
    value: &Value,
) -> Result<TargetValue, EngineError> {
    if value.is_null() {
        return Ok(null_value(column.converter));
    }

    match column.converter {
        Converter::TextToText => Ok(TargetValue::Text(Some(
            value_as_text(source_db, table, source_pk, column, value)?.to_string(),
        ))),
        Converter::TextToJsonb => {
            text_json_to_value(value_as_text(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Jsonb(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
        Converter::TextToBigint => {
            text_to_bigint(value_as_text(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Int8(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
        Converter::TextToTimestamptz => {
            text_to_datetime(value_as_text(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Timestamptz(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
        Converter::TextToDate => {
            text_to_date(value_as_text(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Date(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
        Converter::NumericToTimestamptz => numeric_to_datetime(value)
            .map(|value| TargetValue::Timestamptz(Some(value)))
            .map_err(|source| conversion_error(source_db, table, source_pk, column, source)),
        Converter::NumericToBool => {
            sqlite_numeric_to_bool(value_as_i64(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Bool(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
        Converter::IntegerToInt8 => Ok(TargetValue::Int8(Some(value_as_i64(
            source_db, table, source_pk, column, value,
        )?))),
        Converter::IntegerToInt4 => {
            let value = value_as_i64(source_db, table, source_pk, column, value)?;
            let value = i32::try_from(value).map_err(|_| EngineError::Int4OutOfRange {
                source_db: source_db.to_string(),
                table: table.to_string(),
                column: column.source_column.clone(),
                source_pk: source_pk.to_string(),
                value,
            })?;
            Ok(TargetValue::Int4(Some(value)))
        }
        Converter::IntegerToText => Ok(TargetValue::Text(Some(integer_to_text(value_as_i64(
            source_db, table, source_pk, column, value,
        )?)))),
        Converter::IntegerToBool => {
            sqlite_int_to_bool(value_as_i64(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Bool(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
        Converter::IntegerToTimestamptz => {
            unix_seconds_to_datetime(value_as_i64(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Timestamptz(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
        Converter::RealToFloat8 => Ok(TargetValue::Float8(Some(value_as_f64(
            source_db, table, source_pk, column, value,
        )?))),
        Converter::RealToTimestamptz => {
            real_unix_seconds_to_datetime(value_as_f64(source_db, table, source_pk, column, value)?)
                .map(|value| TargetValue::Timestamptz(Some(value)))
                .map_err(|source| conversion_error(source_db, table, source_pk, column, source))
        }
    }
}

fn null_value(converter: Converter) -> TargetValue {
    match converter {
        Converter::TextToText => TargetValue::Text(None),
        Converter::TextToJsonb => TargetValue::Jsonb(None),
        Converter::TextToBigint | Converter::IntegerToInt8 => TargetValue::Int8(None),
        Converter::TextToDate => TargetValue::Date(None),
        Converter::TextToTimestamptz
        | Converter::NumericToTimestamptz
        | Converter::IntegerToTimestamptz
        | Converter::RealToTimestamptz => TargetValue::Timestamptz(None),
        Converter::IntegerToInt4 => TargetValue::Int4(None),
        Converter::NumericToBool | Converter::IntegerToBool => TargetValue::Bool(None),
        Converter::IntegerToText => TargetValue::Text(None),
        Converter::RealToFloat8 => TargetValue::Float8(None),
    }
}

fn generate_value(
    source_db: &str,
    table: &str,
    generated: &GeneratedColumn,
    source_value: &Value,
) -> TargetValue {
    match generated {
        GeneratedColumn::SourceQualifiedRequestUid => TargetValue::Text(Some(format!(
            "{source_db}:{table}:{}",
            format_source_value(source_value)
        ))),
    }
}

fn value_as_text<'a>(
    source_db: &str,
    table: &str,
    source_pk: &str,
    column: &ColumnPlan,
    value: &'a Value,
) -> Result<&'a str, EngineError> {
    value
        .as_str()
        .ok_or_else(|| invalid_source_type(source_db, table, source_pk, column, value))
}

fn value_as_i64(
    source_db: &str,
    table: &str,
    source_pk: &str,
    column: &ColumnPlan,
    value: &Value,
) -> Result<i64, EngineError> {
    value
        .as_i64()
        .ok_or_else(|| invalid_source_type(source_db, table, source_pk, column, value))
}

fn value_as_f64(
    source_db: &str,
    table: &str,
    source_pk: &str,
    column: &ColumnPlan,
    value: &Value,
) -> Result<f64, EngineError> {
    value
        .as_f64()
        .ok_or_else(|| invalid_source_type(source_db, table, source_pk, column, value))
}

fn invalid_source_type(
    source_db: &str,
    table: &str,
    source_pk: &str,
    column: &ColumnPlan,
    value: &Value,
) -> EngineError {
    EngineError::InvalidSourceValueType {
        source_db: source_db.to_string(),
        table: table.to_string(),
        column: column.source_column.clone(),
        source_pk: source_pk.to_string(),
        value: Box::new(value.clone()),
    }
}

fn text_to_datetime(text: &str) -> Result<DateTime<Utc>, ConvertError> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Ok(value.with_timezone(&Utc));
    }

    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(value.and_utc());
        }
    }

    Err(ConvertError::InvalidTimestampText {
        value: text.to_string(),
    })
}

fn numeric_to_datetime(value: &Value) -> Result<DateTime<Utc>, ConvertError> {
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

fn conversion_error(
    source_db: &str,
    table: &str,
    source_pk: &str,
    column: &ColumnPlan,
    source: ConvertError,
) -> EngineError {
    EngineError::Conversion {
        source_db: source_db.to_string(),
        table: table.to_string(),
        column: column.source_column.clone(),
        source_pk: source_pk.to_string(),
        source: Box::new(source),
    }
}

fn source_pk(row: &SourceRow, plan: &TablePlan) -> String {
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

fn format_source_value(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}
