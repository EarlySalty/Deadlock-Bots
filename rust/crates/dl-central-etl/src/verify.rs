use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::ledger::{ColumnStatus, Ledger, LedgerSet, TableLedger};

pub type RoundTripFields = BTreeMap<String, Value>;
pub type RoundTripRows = BTreeMap<String, RoundTripFields>;
pub type SourceTableColumns = BTreeMap<String, Vec<String>>;
pub type SourceSchemas = BTreeMap<String, SourceTableColumns>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowCountFactor {
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VerifyError {
    #[error("Quellspalte ist weder gemappt noch gedroppt: {column}")]
    UnmappedColumn { column: String },
    #[error("Ledger-Quelle fehlt im Quellschema: {source_db}")]
    MissingSourceSchema { source_db: String },
    #[error("Ledger-Tabelle fehlt im Quellschema: {source_db}.{table}")]
    MissingSourceTable { source_db: String, table: String },
    #[error("Quellspalte ist weder gemappt noch gedroppt: {source_db}.{table}.{column}")]
    UnmappedSourceColumn {
        source_db: String,
        table: String,
        column: String,
    },
    #[error("gemappte Spalte fehlt im Round-Trip-Sample: {column} in row={row_key}")]
    MappedColumnMissingFromSample { column: String, row_key: String },
    #[error("Row-Count-Mismatch: erwartet {expected}, bekam {actual}")]
    RowCountMismatch { expected: u64, actual: u64 },
    #[error("ungueltiger Row-Count-Faktor {numerator}/{denominator}")]
    InvalidRowCountFactor { numerator: u64, denominator: u64 },
    #[error("Round-Trip-Mismatch fuer key={key}, field={field}")]
    RoundTripMismatch {
        key: String,
        field: String,
        source_value: Option<Value>,
        target_value: Option<Value>,
    },
}

pub fn check_mapping_completeness(
    source_columns: &[String],
    ledger: &TableLedger,
) -> Result<(), VerifyError> {
    for column in source_columns {
        if ledger.column_status(column).is_none() {
            return Err(VerifyError::UnmappedColumn {
                column: column.clone(),
            });
        }
    }

    Ok(())
}

pub fn check_source_mapping_completeness(
    source: &str,
    source_tables: &SourceTableColumns,
    ledger: &Ledger,
) -> Result<(), VerifyError> {
    for (table_name, table_ledger) in &ledger.tables {
        let source_columns =
            source_tables
                .get(table_name)
                .ok_or_else(|| VerifyError::MissingSourceTable {
                    source_db: source.to_string(),
                    table: table_name.clone(),
                })?;

        for column in source_columns {
            if table_ledger.column_status(column).is_none() {
                return Err(VerifyError::UnmappedSourceColumn {
                    source_db: source.to_string(),
                    table: table_name.clone(),
                    column: column.clone(),
                });
            }
        }
    }

    Ok(())
}

pub fn check_ledger_set_mapping_completeness(
    source_schemas: &SourceSchemas,
    ledger_set: &LedgerSet,
) -> Result<(), VerifyError> {
    for (source_name, ledger) in &ledger_set.sources {
        let source_tables =
            source_schemas
                .get(source_name)
                .ok_or_else(|| VerifyError::MissingSourceSchema {
                    source_db: source_name.clone(),
                })?;

        check_source_mapping_completeness(source_name, source_tables, ledger)?;
    }

    Ok(())
}

pub fn check_sample_covers_mapped_columns(
    table_ledger: &TableLedger,
    sample_rows: &RoundTripRows,
) -> Result<(), VerifyError> {
    for (column, status) in &table_ledger.columns {
        if !matches!(status, ColumnStatus::Mapped { .. }) {
            continue;
        }

        for (row_key, fields) in sample_rows {
            if !fields.contains_key(column) {
                return Err(VerifyError::MappedColumnMissingFromSample {
                    column: column.clone(),
                    row_key: row_key.clone(),
                });
            }
        }
    }

    Ok(())
}

pub fn check_row_counts(expected: u64, actual: u64) -> Result<(), VerifyError> {
    if expected == actual {
        return Ok(());
    }

    Err(VerifyError::RowCountMismatch { expected, actual })
}

/// Checks row counts with cross multiplication as the source of truth.
///
/// If the scaled expectation is not an integer, no integer actual count can
/// satisfy the factor exactly; the returned `expected` is rounded up for
/// diagnostics only.
pub fn check_row_counts_with_factor(
    source_rows: u64,
    actual: u64,
    factor: RowCountFactor,
) -> Result<(), VerifyError> {
    if factor.numerator == 0 || factor.denominator == 0 {
        return Err(VerifyError::InvalidRowCountFactor {
            numerator: factor.numerator,
            denominator: factor.denominator,
        });
    }

    let expected_scaled = u128::from(source_rows) * u128::from(factor.numerator);
    let actual_scaled = u128::from(actual) * u128::from(factor.denominator);
    if expected_scaled == actual_scaled {
        return Ok(());
    }

    let denominator = u128::from(factor.denominator);
    let expected = expected_scaled
        .div_ceil(denominator)
        .min(u128::from(u64::MAX)) as u64;

    Err(VerifyError::RowCountMismatch { expected, actual })
}

pub fn sample_round_trip(
    source_rows: &RoundTripRows,
    target_rows: &RoundTripRows,
) -> Result<(), VerifyError> {
    for (key, source_fields) in source_rows {
        let Some(target_fields) = target_rows.get(key) else {
            return Err(VerifyError::RoundTripMismatch {
                key: key.clone(),
                field: "<row>".to_string(),
                source_value: Some(fields_to_value(source_fields)),
                target_value: None,
            });
        };

        compare_fields(key, source_fields, target_fields)?;
    }

    for (key, target_fields) in target_rows {
        if !source_rows.contains_key(key) {
            return Err(VerifyError::RoundTripMismatch {
                key: key.clone(),
                field: "<row>".to_string(),
                source_value: None,
                target_value: Some(fields_to_value(target_fields)),
            });
        }
    }

    Ok(())
}

fn compare_fields(
    key: &str,
    source_fields: &RoundTripFields,
    target_fields: &RoundTripFields,
) -> Result<(), VerifyError> {
    let mut field_names = BTreeSet::new();
    field_names.extend(source_fields.keys());
    field_names.extend(target_fields.keys());

    for field in field_names {
        let source = source_fields.get(field);
        let target = target_fields.get(field);

        if source != target {
            return Err(VerifyError::RoundTripMismatch {
                key: key.to_string(),
                field: field.to_string(),
                source_value: source.cloned(),
                target_value: target.cloned(),
            });
        }
    }

    Ok(())
}

fn fields_to_value(fields: &RoundTripFields) -> Value {
    let mut map = Map::new();
    for (key, value) in fields {
        map.insert(key.clone(), value.clone());
    }
    Value::Object(map)
}
