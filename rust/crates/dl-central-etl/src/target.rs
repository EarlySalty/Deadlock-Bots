use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{
    postgres::{PgArguments, PgQueryResult},
    query::Query,
    PgPool, Postgres, Transaction,
};

use crate::plan::TablePlan;

#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    #[error("ungueltiger Postgres-Identifier: {identifier}")]
    InvalidIdentifier { identifier: String },
    #[error("Upsert-Zeile enthaelt keinen Wert fuer Zielspalte {column}")]
    MissingColumnValue { column: String },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TargetValue {
    Text(Option<String>),
    Int8(Option<i64>),
    Int4(Option<i32>),
    Float8(Option<f64>),
    Bool(Option<bool>),
    Jsonb(Option<Value>),
    Timestamptz(Option<DateTime<Utc>>),
    Date(Option<NaiveDate>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetRow {
    pub values: BTreeMap<String, TargetValue>,
}

#[derive(Debug, Clone)]
pub struct TargetWriter {
    pool: PgPool,
}

impl TargetWriter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn upsert_rows(
        &self,
        plan: &TablePlan,
        rows: &[TargetRow],
    ) -> Result<u64, TargetError> {
        let mut tx = self.pool.begin().await?;
        let loaded = self.upsert_rows_in_tx(&mut tx, plan, rows).await?;
        tx.commit().await?;
        Ok(loaded)
    }

    pub(crate) async fn upsert_rows_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        plan: &TablePlan,
        rows: &[TargetRow],
    ) -> Result<u64, TargetError> {
        if plan.primary_key.is_empty() {
            return self.replace_rows_in_tx(tx, plan, rows).await;
        }

        if rows.is_empty() {
            return Ok(0);
        }

        let columns = plan
            .columns
            .iter()
            .map(|column| column.target_column.clone())
            .collect::<Vec<_>>();
        let sql = build_upsert_sql(plan, &columns, &rows[0])?;

        for row in rows {
            let mut query = sqlx::query(&sql);
            for column in &columns {
                let value =
                    row.values
                        .get(column)
                        .ok_or_else(|| TargetError::MissingColumnValue {
                            column: column.clone(),
                        })?;
                query = bind_value(query, value);
            }
            query.execute(&mut **tx).await?;
        }

        Ok(rows.len() as u64)
    }

    pub(crate) async fn insert_rows_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        plan: &TablePlan,
        rows: &[TargetRow],
    ) -> Result<u64, TargetError> {
        if rows.is_empty() {
            return Ok(0);
        }

        let columns = plan
            .columns
            .iter()
            .map(|column| column.target_column.clone())
            .collect::<Vec<_>>();
        let sql = build_insert_sql(plan, &columns, &rows[0])?;

        for row in rows {
            let mut query = sqlx::query(&sql);
            for column in &columns {
                let value =
                    row.values
                        .get(column)
                        .ok_or_else(|| TargetError::MissingColumnValue {
                            column: column.clone(),
                        })?;
                query = bind_value(query, value);
            }
            query.execute(&mut **tx).await?;
        }

        Ok(rows.len() as u64)
    }

    async fn replace_rows_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        plan: &TablePlan,
        rows: &[TargetRow],
    ) -> Result<u64, TargetError> {
        let table = quote_pg_path(&plan.target)?;
        let delete_sql = format!("DELETE FROM {table}");

        sqlx::query(&delete_sql).execute(&mut **tx).await?;

        if rows.is_empty() {
            return Ok(0);
        }

        let columns = plan
            .columns
            .iter()
            .map(|column| column.target_column.clone())
            .collect::<Vec<_>>();
        let sql = build_insert_sql(plan, &columns, &rows[0])?;

        for row in rows {
            let mut query = sqlx::query(&sql);
            for column in &columns {
                let value =
                    row.values
                        .get(column)
                        .ok_or_else(|| TargetError::MissingColumnValue {
                            column: column.clone(),
                        })?;
                query = bind_value(query, value);
            }
            query.execute(&mut **tx).await?;
        }

        Ok(rows.len() as u64)
    }

    pub async fn insert_text_value(
        &self,
        table: &str,
        column: &str,
        value: &str,
    ) -> Result<PgQueryResult, TargetError> {
        let table = quote_pg_path(table)?;
        let column = quote_pg_ident(column)?;
        let sql = format!("INSERT INTO {table} ({column}) VALUES ($1)");

        sqlx::query(&sql)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(TargetError::from)
    }

    pub async fn insert_default_row(&self, table: &str) -> Result<PgQueryResult, TargetError> {
        let table = quote_pg_path(table)?;
        let sql = format!("INSERT INTO {table} DEFAULT VALUES");

        sqlx::query(&sql)
            .execute(&self.pool)
            .await
            .map_err(TargetError::from)
    }
}

fn build_upsert_sql(
    plan: &TablePlan,
    columns: &[String],
    sample_row: &TargetRow,
) -> Result<String, TargetError> {
    let insert_sql = build_insert_sql(plan, columns, sample_row)?;
    let conflict_columns = plan
        .primary_key
        .iter()
        .map(|column| quote_pg_ident(column))
        .collect::<Result<Vec<_>, _>>()?;
    let pk_set = plan
        .primary_key
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let update_columns = columns
        .iter()
        .filter(|column| !pk_set.contains(column.as_str()))
        .map(|column| {
            let quoted = quote_pg_ident(column)?;
            Ok(format!("{quoted} = EXCLUDED.{quoted}"))
        })
        .collect::<Result<Vec<_>, TargetError>>()?;

    let conflict_action = if update_columns.is_empty() {
        "DO NOTHING".to_string()
    } else {
        format!("DO UPDATE SET {}", update_columns.join(", "))
    };

    Ok(format!(
        "{insert_sql} ON CONFLICT ({}) {conflict_action}",
        conflict_columns.join(", ")
    ))
}

fn build_insert_sql(
    plan: &TablePlan,
    columns: &[String],
    sample_row: &TargetRow,
) -> Result<String, TargetError> {
    let table = quote_pg_path(&plan.target)?;
    let quoted_columns = columns
        .iter()
        .map(|column| quote_pg_ident(column))
        .collect::<Result<Vec<_>, _>>()?;
    let placeholders =
        columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                let value = sample_row.values.get(column).ok_or_else(|| {
                    TargetError::MissingColumnValue {
                        column: column.clone(),
                    }
                })?;
                Ok(format!("${}{}", index + 1, value.pg_cast()))
            })
            .collect::<Result<Vec<_>, TargetError>>()?;

    Ok(format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        quoted_columns.join(", "),
        placeholders.join(", ")
    ))
}

fn bind_value<'q>(
    query: Query<'q, Postgres, PgArguments>,
    value: &TargetValue,
) -> Query<'q, Postgres, PgArguments> {
    match value {
        TargetValue::Text(value) => query.bind(value.clone()),
        TargetValue::Int8(value) => query.bind(*value),
        TargetValue::Int4(value) => query.bind(*value),
        TargetValue::Float8(value) => query.bind(*value),
        TargetValue::Bool(value) => query.bind(*value),
        TargetValue::Jsonb(value) => query.bind(value.as_ref().map(Value::to_string)),
        TargetValue::Timestamptz(value) => query.bind(*value),
        TargetValue::Date(value) => query.bind(*value),
    }
}

impl TargetValue {
    fn pg_cast(&self) -> &'static str {
        match self {
            Self::Text(_) => "::text",
            Self::Int8(_) => "::bigint",
            Self::Int4(_) => "::integer",
            Self::Float8(_) => "::double precision",
            Self::Bool(_) => "::boolean",
            Self::Jsonb(_) => "::jsonb",
            Self::Timestamptz(_) => "::timestamptz",
            Self::Date(_) => "::date",
        }
    }
}

fn quote_pg_path(path: &str) -> Result<String, TargetError> {
    let mut parts = Vec::new();
    for part in path.split('.') {
        parts.push(quote_pg_ident(part)?);
    }

    if parts.is_empty() {
        return Err(TargetError::InvalidIdentifier {
            identifier: path.to_string(),
        });
    }

    Ok(parts.join("."))
}

fn quote_pg_ident(identifier: &str) -> Result<String, TargetError> {
    if identifier.trim().is_empty() {
        return Err(TargetError::InvalidIdentifier {
            identifier: identifier.to_string(),
        });
    }

    Ok(format!("\"{}\"", identifier.replace('"', "\"\"")))
}
