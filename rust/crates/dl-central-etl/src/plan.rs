use std::collections::{BTreeMap, BTreeSet};

use sqlx::{PgPool, Row};

use crate::{
    engine::EngineError,
    ledger::{ColumnStatus, TableLedger},
    source::SourceSqlite,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Converter {
    TextToText,
    TextToJsonb,
    TextToBigint,
    TextToTimestamptz,
    TextToDate,
    NumericToTimestamptz,
    NumericToBool,
    IntegerToInt8,
    IntegerToInt4,
    IntegerToText,
    IntegerToBool,
    IntegerToTimestamptz,
    RealToFloat8,
    RealToTimestamptz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnPlan {
    pub source_column: String,
    pub target_column: String,
    pub source_affinity: String,
    pub target_pg_type: String,
    pub converter: Converter,
    pub generated: Option<GeneratedColumn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePlan {
    pub source_db: String,
    pub source_table: String,
    pub target: String,
    pub target_schema: String,
    pub target_table: String,
    pub columns: Vec<ColumnPlan>,
    pub primary_key: Vec<String>,
    pub dropped_cols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedColumn {
    SourceQualifiedRequestUid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetColumn {
    pg_type: String,
}

pub fn conversion_for(
    source_affinity: &str,
    target_pg_type: &str,
) -> Result<Converter, EngineError> {
    conversion_for_context(source_affinity, target_pg_type, "", "")
}

pub async fn build_table_plan(
    source: &SourceSqlite,
    source_db: &str,
    table: &str,
    table_ledger: &TableLedger,
    pool: &PgPool,
) -> Result<TablePlan, EngineError> {
    let source_schema = source.table_schema(table)?;
    let source_affinities = source_schema
        .iter()
        .map(|column| (column.name.clone(), column.affinity.as_str().to_string()))
        .collect::<BTreeMap<_, _>>();
    let source_primary_key = source_schema
        .iter()
        .filter(|column| column.primary_key_ordinal > 0)
        .map(|column| (column.primary_key_ordinal, column.name.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();

    let mut target_schema = None;
    let mut target_table = None;
    let mut mapped = Vec::new();
    let mut dropped_cols = Vec::new();
    let mut seen_target_columns = BTreeSet::new();

    for (source_column, status) in &table_ledger.columns {
        match status {
            ColumnStatus::Dropped { .. } => dropped_cols.push(source_column.clone()),
            ColumnStatus::Mapped { to } => {
                let target_ref = TargetRef::parse(to)?;
                if let (Some(schema), Some(table_name)) = (&target_schema, &target_table) {
                    if schema != &target_ref.schema || table_name != &target_ref.table {
                        return Err(EngineError::MixedTargetTables {
                            source_db: source_db.to_string(),
                            table: table.to_string(),
                            first: format!("{schema}.{table_name}"),
                            next: format!("{}.{}", target_ref.schema, target_ref.table),
                        });
                    }
                } else {
                    target_schema = Some(target_ref.schema.clone());
                    target_table = Some(target_ref.table.clone());
                }

                if !seen_target_columns.insert(target_ref.column.clone()) {
                    return Err(EngineError::DuplicateTargetColumn {
                        target: format!("{}.{}", target_ref.schema, target_ref.table),
                        column: target_ref.column,
                    });
                }

                mapped.push((source_column.clone(), target_ref));
            }
        }
    }

    let Some(target_schema) = target_schema else {
        return Err(EngineError::NoMappedColumns {
            source_db: source_db.to_string(),
            table: table.to_string(),
        });
    };
    let Some(target_table) = target_table else {
        return Err(EngineError::NoMappedColumns {
            source_db: source_db.to_string(),
            table: table.to_string(),
        });
    };
    let target = format!("{target_schema}.{target_table}");

    let target_columns = load_target_columns(pool, &target_schema, &target_table).await?;
    let primary_key = load_primary_key(pool, &target_schema, &target_table).await?;

    let mut columns = Vec::with_capacity(mapped.len());
    for (source_column, target_ref) in mapped {
        let source_affinity = source_affinities
            .get(&source_column)
            .cloned()
            .ok_or_else(|| EngineError::MissingSourceColumn {
                source_db: source_db.to_string(),
                table: table.to_string(),
                column: source_column.clone(),
            })?;
        let target_column = target_columns.get(&target_ref.column).ok_or_else(|| {
            EngineError::MissingTargetColumn {
                target: target.clone(),
                column: target_ref.column.clone(),
            }
        })?;
        let converter = conversion_for_context(
            &source_affinity,
            &target_column.pg_type,
            &target,
            &target_ref.column,
        )?;

        columns.push(ColumnPlan {
            source_column,
            target_column: target_ref.column,
            source_affinity,
            target_pg_type: target_column.pg_type.clone(),
            converter,
            generated: None,
        });
    }

    add_supported_generated_columns(
        source_db,
        table,
        &target,
        &target_columns,
        &source_primary_key,
        &mut columns,
    )?;

    let target_column_set = columns
        .iter()
        .map(|column| column.target_column.as_str())
        .collect::<BTreeSet<_>>();
    for pk_column in &primary_key {
        if !target_column_set.contains(pk_column.as_str()) {
            return Err(EngineError::MissingPrimaryKeyColumn {
                target: target.clone(),
                column: pk_column.clone(),
            });
        }
    }

    Ok(TablePlan {
        source_db: source_db.to_string(),
        source_table: table.to_string(),
        target,
        target_schema,
        target_table,
        columns,
        primary_key,
        dropped_cols,
    })
}

fn conversion_for_context(
    source_affinity: &str,
    target_pg_type: &str,
    table: &str,
    column: &str,
) -> Result<Converter, EngineError> {
    let source = normalize_source_affinity(source_affinity);
    let target = normalize_target_type(target_pg_type);
    let converter = match (source.as_str(), target.as_str()) {
        ("TEXT", "text") => Converter::TextToText,
        ("TEXT", "jsonb") => Converter::TextToJsonb,
        ("TEXT", "int8") => Converter::TextToBigint,
        ("TEXT", "timestamptz") => Converter::TextToTimestamptz,
        ("TEXT", "date") => Converter::TextToDate,
        ("NUMERIC", "timestamptz") => Converter::NumericToTimestamptz,
        ("NUMERIC", "bool") => Converter::NumericToBool,
        ("INTEGER", "int8") => Converter::IntegerToInt8,
        ("INTEGER", "int4") => Converter::IntegerToInt4,
        ("INTEGER", "text") => Converter::IntegerToText,
        ("INTEGER", "bool") => Converter::IntegerToBool,
        ("INTEGER", "timestamptz") => Converter::IntegerToTimestamptz,
        ("REAL", "float8") => Converter::RealToFloat8,
        ("REAL", "timestamptz") => Converter::RealToTimestamptz,
        _ => {
            return Err(EngineError::NoConverter {
                source_affinity: source_affinity.to_string(),
                target_type: target_pg_type.to_string(),
                table: table.to_string(),
                column: column.to_string(),
            })
        }
    };

    Ok(converter)
}

fn add_supported_generated_columns(
    source_db: &str,
    table: &str,
    target: &str,
    target_columns: &BTreeMap<String, TargetColumn>,
    source_primary_key: &[String],
    columns: &mut Vec<ColumnPlan>,
) -> Result<(), EngineError> {
    if target != "coaching.requests"
        || columns
            .iter()
            .any(|column| column.target_column == "request_uid")
    {
        return Ok(());
    }

    if source_primary_key.len() != 1 {
        return Ok(());
    }

    let Some(target_column) = target_columns.get("request_uid") else {
        return Ok(());
    };

    let source_column = source_primary_key[0].clone();
    columns.insert(
        0,
        ColumnPlan {
            source_column,
            target_column: "request_uid".to_string(),
            source_affinity: "SYNTHETIC".to_string(),
            target_pg_type: target_column.pg_type.clone(),
            converter: Converter::TextToText,
            generated: Some(GeneratedColumn::SourceQualifiedRequestUid),
        },
    );

    if source_db != "deadlock-sqlite3" && source_db != "website" {
        return Err(EngineError::UnsupportedGeneratedPrimaryKey {
            source_db: source_db.to_string(),
            table: table.to_string(),
            target: target.to_string(),
            column: "request_uid".to_string(),
        });
    }

    Ok(())
}

fn normalize_source_affinity(source_affinity: &str) -> String {
    source_affinity.trim().to_ascii_uppercase()
}

fn normalize_target_type(target_pg_type: &str) -> String {
    match target_pg_type.trim().to_ascii_lowercase().as_str() {
        "bigint" | "int8" => "int8".to_string(),
        "integer" | "int4" => "int4".to_string(),
        "boolean" | "bool" => "bool".to_string(),
        "timestamp with time zone" | "timestamptz" => "timestamptz".to_string(),
        "double precision" | "float8" => "float8".to_string(),
        "character varying" | "varchar" | "character" | "bpchar" | "text" => "text".to_string(),
        other => other.to_string(),
    }
}

async fn load_target_columns(
    pool: &PgPool,
    schema: &str,
    table: &str,
) -> Result<BTreeMap<String, TargetColumn>, EngineError> {
    let rows = sqlx::query(
        r#"
        SELECT
            a.attname AS column_name,
            t.typname AS pg_type
        FROM pg_attribute a
        JOIN pg_class c ON c.oid = a.attrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_type t ON t.oid = a.atttypid
        WHERE n.nspname = $1
          AND c.relname = $2
          AND a.attnum > 0
          AND NOT a.attisdropped
        ORDER BY a.attnum
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Err(EngineError::TargetTableNotFound {
            target: format!("{schema}.{table}"),
        });
    }

    let mut columns = BTreeMap::new();
    for row in rows {
        let name: String = row.try_get("column_name")?;
        let pg_type: String = row.try_get("pg_type")?;
        columns.insert(name, TargetColumn { pg_type });
    }

    Ok(columns)
}

async fn load_primary_key(
    pool: &PgPool,
    schema: &str,
    table: &str,
) -> Result<Vec<String>, EngineError> {
    let rows = sqlx::query(
        r#"
        SELECT a.attname AS column_name
        FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS key(attnum, ord) ON true
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = key.attnum
        WHERE n.nspname = $1
          AND c.relname = $2
          AND i.indisprimary
        ORDER BY key.ord
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    let mut primary_key = Vec::with_capacity(rows.len());
    for row in rows {
        primary_key.push(row.try_get("column_name")?);
    }

    Ok(primary_key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetRef {
    schema: String,
    table: String,
    column: String,
}

impl TargetRef {
    fn parse(target: &str) -> Result<Self, EngineError> {
        let parts = target.split('.').collect::<Vec<_>>();
        if parts.len() != 3 || parts.iter().any(|part| part.trim().is_empty()) {
            return Err(EngineError::InvalidTargetPath {
                target: target.to_string(),
            });
        }

        Ok(Self {
            schema: parts[0].to_string(),
            table: parts[1].to_string(),
            column: parts[2].to_string(),
        })
    }
}
