use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::{
    convert::{
        integer_to_text, real_unix_seconds_to_datetime, sqlite_int_to_bool, sqlite_numeric_to_bool,
        text_json_or_integer_csv_list_to_value, text_json_to_value, text_to_bigint, text_to_date,
        text_to_datetime, unix_seconds_to_datetime, ConvertError,
    },
    ledger::{ColumnStatus, LedgerSet, TableLedger},
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
    pub orphan_reports: Vec<OrphanReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableResult {
    pub target: String,
    pub source_rows: u64,
    pub loaded_rows: u64,
    pub dropped_cols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanReport {
    pub table: String,
    pub parent_table: String,
    pub constraint: String,
    pub count: u64,
    pub sample_source_pk: String,
}

#[derive(Debug, Clone)]
struct LoadItem {
    source_db: String,
    source_table: String,
    table_ledger: TableLedger,
    plan: TablePlan,
    original_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForeignKeyConstraint {
    child_table: String,
    parent_table: String,
    name: String,
    definition: String,
    child_columns: Vec<String>,
    parent_columns: Vec<String>,
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
    let (source_rows, target_rows) =
        transform_table_rows(source, source_db, table, table_ledger, plan)?;
    let loaded_rows = target.upsert_rows(plan, &target_rows).await?;

    Ok(TableResult {
        target: plan.target.clone(),
        source_rows,
        loaded_rows,
        dropped_cols: plan.dropped_cols.clone(),
    })
}

async fn migrate_table_in_tx(
    source: &SourceSqlite,
    source_db: &str,
    table: &str,
    table_ledger: &TableLedger,
    plan: &TablePlan,
    target: &TargetWriter,
    tx: &mut Transaction<'_, Postgres>,
) -> Result<TableResult, EngineError> {
    let (source_rows, target_rows) =
        transform_table_rows(source, source_db, table, table_ledger, plan)?;
    let loaded_rows = target.upsert_rows_in_tx(tx, plan, &target_rows).await?;

    Ok(TableResult {
        target: plan.target.clone(),
        source_rows,
        loaded_rows,
        dropped_cols: plan.dropped_cols.clone(),
    })
}

fn transform_table_rows(
    source: &SourceSqlite,
    source_db: &str,
    table: &str,
    table_ledger: &TableLedger,
    plan: &TablePlan,
) -> Result<(u64, Vec<TargetRow>), EngineError> {
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

    Ok((source_rows.len() as u64, target_rows))
}

pub async fn run(
    ledger_set: &LedgerSet,
    snapshot_dir: &Path,
    pool: &PgPool,
) -> Result<EtlReport, EngineError> {
    let target = TargetWriter::new(pool.clone());
    let mut sources = BTreeMap::new();
    let mut load_items = Vec::new();

    for (source_db, ledger) in &ledger_set.sources {
        let source_path = source_snapshot_path(snapshot_dir, source_db);
        let source = SourceSqlite::open_read_only(&source_path)?;

        for (source_table, table_ledger) in &ledger.tables {
            if !has_mapped_columns(table_ledger) {
                validate_dropped_only_table(&source, source_db, source_table, table_ledger)?;
                continue;
            }

            let plan =
                build_table_plan(&source, source_db, source_table, table_ledger, pool).await?;
            load_items.push(LoadItem {
                source_db: source_db.clone(),
                source_table: source_table.clone(),
                table_ledger: table_ledger.clone(),
                plan,
                original_index: load_items.len(),
            });
        }

        sources.insert(source_db.clone(), source);
    }

    let target_tables = load_items
        .iter()
        .map(|item| item.plan.target.clone())
        .collect::<BTreeSet<_>>();
    let foreign_keys = load_foreign_keys(pool, &target_tables).await?;
    let target_rank = target_load_rank(&load_items, &foreign_keys);
    load_items.sort_by_key(|item| {
        (
            target_rank
                .get(&item.plan.target)
                .copied()
                .unwrap_or(usize::MAX),
            item.original_index,
        )
    });

    let foreign_keys_by_child = foreign_keys_by_child(&foreign_keys);
    let mut tables = Vec::new();
    let mut orphan_reports = Vec::new();
    let mut total_source_rows = 0;
    let mut total_loaded = 0;

    for item in &load_items {
        let source = sources
            .get(&item.source_db)
            .expect("source was opened while building load items");
        let (result, mut item_orphans) =
            migrate_load_item(source, item, &target, &foreign_keys_by_child).await?;
        total_source_rows += result.source_rows;
        total_loaded += result.loaded_rows;
        tables.push(result);
        orphan_reports.append(&mut item_orphans);
    }

    if let Some(report) = steam_links_user_orphan_report(pool, &load_items).await? {
        orphan_reports.push(report);
    }

    Ok(EtlReport {
        tables,
        total_source_rows,
        total_loaded,
        orphan_reports,
    })
}

fn has_mapped_columns(table_ledger: &TableLedger) -> bool {
    table_ledger
        .columns
        .values()
        .any(|status| matches!(status, ColumnStatus::Mapped { .. }))
}

fn validate_dropped_only_table(
    source: &SourceSqlite,
    source_db: &str,
    table: &str,
    table_ledger: &TableLedger,
) -> Result<(), EngineError> {
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

    Ok(())
}

async fn migrate_load_item(
    source: &SourceSqlite,
    item: &LoadItem,
    target: &TargetWriter,
    foreign_keys_by_child: &BTreeMap<String, Vec<ForeignKeyConstraint>>,
) -> Result<(TableResult, Vec<OrphanReport>), EngineError> {
    let error = match migrate_table_with_trigger_scope(source, item, target).await {
        Ok(result) => return Ok((result, Vec::new())),
        Err(error) => error,
    };

    if !is_foreign_key_violation(&error) {
        return Err(error);
    }

    let Some(foreign_keys) = foreign_keys_by_child.get(&item.plan.target) else {
        return Err(error);
    };
    if foreign_keys.is_empty() {
        return Err(error);
    }

    let mut tx = target.pool().begin().await?;
    let result =
        migrate_load_item_without_foreign_keys(source, item, target, foreign_keys, &mut tx).await;
    finish_transaction(tx, result).await
}

async fn migrate_load_item_without_foreign_keys(
    source: &SourceSqlite,
    item: &LoadItem,
    target: &TargetWriter,
    foreign_keys: &[ForeignKeyConstraint],
    tx: &mut Transaction<'_, Postgres>,
) -> Result<(TableResult, Vec<OrphanReport>), EngineError> {
    drop_foreign_keys(tx, foreign_keys).await?;
    let result = migrate_table_with_trigger_scope_in_tx(source, item, target, tx).await?;
    let orphan_reports = collect_orphan_reports(tx, item, foreign_keys).await?;
    restore_foreign_keys(tx, foreign_keys).await?;
    Ok((result, orphan_reports))
}

async fn migrate_table_with_trigger_scope(
    source: &SourceSqlite,
    item: &LoadItem,
    target: &TargetWriter,
) -> Result<TableResult, EngineError> {
    if item.plan.target == "core.steam_links" {
        let mut tx = target.pool().begin().await?;
        let result = migrate_table_with_trigger_scope_in_tx(source, item, target, &mut tx).await;
        return finish_transaction(tx, result).await;
    }

    migrate_table(
        source,
        &item.source_db,
        &item.source_table,
        &item.table_ledger,
        &item.plan,
        target,
    )
    .await
}

async fn migrate_table_with_trigger_scope_in_tx(
    source: &SourceSqlite,
    item: &LoadItem,
    target: &TargetWriter,
    tx: &mut Transaction<'_, Postgres>,
) -> Result<TableResult, EngineError> {
    let disable_user_triggers = item.plan.target == "core.steam_links";
    if disable_user_triggers {
        set_user_triggers(tx, &item.plan, false).await?;
    }

    let result = migrate_table_in_tx(
        source,
        &item.source_db,
        &item.source_table,
        &item.table_ledger,
        &item.plan,
        target,
        tx,
    )
    .await?;

    if disable_user_triggers {
        set_user_triggers(tx, &item.plan, true).await?;
    }

    Ok(result)
}

async fn set_user_triggers(
    tx: &mut Transaction<'_, Postgres>,
    plan: &TablePlan,
    enabled: bool,
) -> Result<(), EngineError> {
    let table = quote_pg_path(&plan.target);
    let state = if enabled { "ENABLE" } else { "DISABLE" };
    let sql = format!("ALTER TABLE {table} {state} TRIGGER USER");
    sqlx::query(&sql).execute(&mut **tx).await?;
    Ok(())
}

async fn finish_transaction<T>(
    tx: Transaction<'_, Postgres>,
    result: Result<T, EngineError>,
) -> Result<T, EngineError> {
    match result {
        Ok(value) => {
            tx.commit().await?;
            Ok(value)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}

async fn load_foreign_keys(
    pool: &PgPool,
    target_tables: &BTreeSet<String>,
) -> Result<Vec<ForeignKeyConstraint>, EngineError> {
    if target_tables.is_empty() {
        return Ok(Vec::new());
    }

    let targets = target_tables.iter().cloned().collect::<Vec<_>>();
    let rows = sqlx::query(
        r#"
        SELECT
            child_ns.nspname || '.' || child.relname AS child_table,
            parent_ns.nspname || '.' || parent.relname AS parent_table,
            con.conname AS constraint_name,
            pg_get_constraintdef(con.oid) AS constraint_def,
            array_agg(child_att.attname ORDER BY key.ord)::text[] AS child_columns,
            array_agg(parent_att.attname ORDER BY key.ord)::text[] AS parent_columns
        FROM pg_constraint con
        JOIN pg_class child ON child.oid = con.conrelid
        JOIN pg_namespace child_ns ON child_ns.oid = child.relnamespace
        JOIN pg_class parent ON parent.oid = con.confrelid
        JOIN pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
        JOIN LATERAL unnest(con.conkey, con.confkey)
            WITH ORDINALITY AS key(child_attnum, parent_attnum, ord) ON true
        JOIN pg_attribute child_att
          ON child_att.attrelid = child.oid
         AND child_att.attnum = key.child_attnum
        JOIN pg_attribute parent_att
          ON parent_att.attrelid = parent.oid
         AND parent_att.attnum = key.parent_attnum
        WHERE con.contype = 'f'
          AND child_ns.nspname || '.' || child.relname = ANY($1)
        GROUP BY con.oid, child_ns.nspname, child.relname, parent_ns.nspname, parent.relname, con.conname
        ORDER BY child_ns.nspname, child.relname, con.conname
        "#,
    )
    .bind(&targets)
    .fetch_all(pool)
    .await?;

    let mut foreign_keys = Vec::with_capacity(rows.len());
    for row in rows {
        foreign_keys.push(ForeignKeyConstraint {
            child_table: row.try_get("child_table")?,
            parent_table: row.try_get("parent_table")?,
            name: row.try_get("constraint_name")?,
            definition: row.try_get("constraint_def")?,
            child_columns: row.try_get("child_columns")?,
            parent_columns: row.try_get("parent_columns")?,
        });
    }

    Ok(foreign_keys)
}

fn foreign_keys_by_child(
    foreign_keys: &[ForeignKeyConstraint],
) -> BTreeMap<String, Vec<ForeignKeyConstraint>> {
    let mut by_child = BTreeMap::<String, Vec<ForeignKeyConstraint>>::new();
    for foreign_key in foreign_keys {
        by_child
            .entry(foreign_key.child_table.clone())
            .or_default()
            .push(foreign_key.clone());
    }
    by_child
}

fn target_load_rank(
    load_items: &[LoadItem],
    foreign_keys: &[ForeignKeyConstraint],
) -> BTreeMap<String, usize> {
    let mut first_index = BTreeMap::<String, usize>::new();
    for item in load_items {
        first_index
            .entry(item.plan.target.clone())
            .or_insert(item.original_index);
    }

    let mut dependencies = first_index
        .keys()
        .map(|target| (target.clone(), BTreeSet::<String>::new()))
        .collect::<BTreeMap<_, _>>();
    for foreign_key in foreign_keys {
        if foreign_key.child_table == foreign_key.parent_table {
            continue;
        }
        if first_index.contains_key(&foreign_key.parent_table) {
            dependencies
                .entry(foreign_key.child_table.clone())
                .or_default()
                .insert(foreign_key.parent_table.clone());
        }
    }

    let mut remaining = first_index.keys().cloned().collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(remaining.len());

    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .filter(|target| dependencies.get(*target).is_none_or(BTreeSet::is_empty))
            .min_by_key(|target| {
                (
                    first_index.get(*target).copied().unwrap_or(usize::MAX),
                    target.as_str(),
                )
            })
            .cloned();

        let Some(target) = ready else {
            let mut cycle_tail = remaining.into_iter().collect::<Vec<_>>();
            cycle_tail.sort_by_key(|target| {
                (
                    first_index.get(target).copied().unwrap_or(usize::MAX),
                    target.clone(),
                )
            });
            ordered.extend(cycle_tail);
            break;
        };

        remaining.remove(&target);
        for dependency_set in dependencies.values_mut() {
            dependency_set.remove(&target);
        }
        ordered.push(target);
    }

    ordered
        .into_iter()
        .enumerate()
        .map(|(index, target)| (target, index))
        .collect()
}

async fn drop_foreign_keys(
    tx: &mut Transaction<'_, Postgres>,
    foreign_keys: &[ForeignKeyConstraint],
) -> Result<(), EngineError> {
    for foreign_key in foreign_keys {
        let table = quote_pg_path(&foreign_key.child_table);
        let constraint = quote_pg_ident(&foreign_key.name);
        let sql = format!("ALTER TABLE {table} DROP CONSTRAINT {constraint}");
        sqlx::query(&sql).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn restore_foreign_keys(
    tx: &mut Transaction<'_, Postgres>,
    foreign_keys: &[ForeignKeyConstraint],
) -> Result<(), EngineError> {
    for foreign_key in foreign_keys {
        let table = quote_pg_path(&foreign_key.child_table);
        let constraint = quote_pg_ident(&foreign_key.name);
        let validity = if foreign_key
            .definition
            .to_ascii_uppercase()
            .contains("NOT VALID")
        {
            ""
        } else {
            " NOT VALID"
        };
        let sql = format!(
            "ALTER TABLE {table} ADD CONSTRAINT {constraint} {}{validity}",
            foreign_key.definition
        );
        sqlx::query(&sql).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn collect_orphan_reports(
    tx: &mut Transaction<'_, Postgres>,
    item: &LoadItem,
    foreign_keys: &[ForeignKeyConstraint],
) -> Result<Vec<OrphanReport>, EngineError> {
    let mut reports = Vec::new();

    for foreign_key in foreign_keys {
        let Some((count, sample_source_pk)) =
            foreign_key_orphan_sample(tx, item, foreign_key).await?
        else {
            continue;
        };
        reports.push(OrphanReport {
            table: foreign_key.child_table.clone(),
            parent_table: foreign_key.parent_table.clone(),
            constraint: foreign_key.name.clone(),
            count,
            sample_source_pk,
        });
    }

    Ok(reports)
}

async fn foreign_key_orphan_sample(
    tx: &mut Transaction<'_, Postgres>,
    item: &LoadItem,
    foreign_key: &ForeignKeyConstraint,
) -> Result<Option<(u64, String)>, EngineError> {
    let child = quote_pg_path(&foreign_key.child_table);
    let where_clause = foreign_key_orphan_where(foreign_key);
    let count_sql = format!("SELECT COUNT(*)::bigint AS count FROM {child} c WHERE {where_clause}");
    let count_row = sqlx::query(&count_sql).fetch_one(&mut **tx).await?;
    let count = count_row.try_get::<i64, _>("count")?;
    if count <= 0 {
        return Ok(None);
    }

    let sample_columns = sample_pk_columns(item, foreign_key);
    let sample_sql = sample_pk_sql(&child, &where_clause, &sample_columns);
    let sample_row = sqlx::query(&sample_sql).fetch_one(&mut **tx).await?;
    let sample_values = sample_row.try_get::<Vec<String>, _>("pk_values")?;
    let sample_source_pk =
        format_source_pk_from_target_values(&item.plan, &sample_columns, &sample_values);

    Ok(Some((count as u64, sample_source_pk)))
}

fn foreign_key_orphan_where(foreign_key: &ForeignKeyConstraint) -> String {
    let not_null = foreign_key
        .child_columns
        .iter()
        .map(|column| format!("c.{} IS NOT NULL", quote_pg_ident(column)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let matches = foreign_key
        .child_columns
        .iter()
        .zip(&foreign_key.parent_columns)
        .map(|(child_column, parent_column)| {
            format!(
                "p.{} = c.{}",
                quote_pg_ident(parent_column),
                quote_pg_ident(child_column)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "{not_null} AND NOT EXISTS (SELECT 1 FROM {} p WHERE {matches})",
        quote_pg_path(&foreign_key.parent_table)
    )
}

fn sample_pk_columns(item: &LoadItem, foreign_key: &ForeignKeyConstraint) -> Vec<String> {
    if item.plan.primary_key.is_empty() {
        foreign_key.child_columns.clone()
    } else {
        item.plan.primary_key.clone()
    }
}

fn sample_pk_sql(child: &str, where_clause: &str, sample_columns: &[String]) -> String {
    let select_values = sample_columns
        .iter()
        .map(|column| format!("COALESCE(c.{}::text, 'NULL')", quote_pg_ident(column)))
        .collect::<Vec<_>>()
        .join(", ");
    let order_by = if sample_columns.is_empty() {
        String::new()
    } else {
        format!(
            " ORDER BY {}",
            sample_columns
                .iter()
                .map(|column| format!("c.{}", quote_pg_ident(column)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    format!(
        "SELECT ARRAY[{select_values}]::text[] AS pk_values FROM {child} c WHERE {where_clause}{order_by} LIMIT 1"
    )
}

async fn steam_links_user_orphan_report(
    pool: &PgPool,
    load_items: &[LoadItem],
) -> Result<Option<OrphanReport>, EngineError> {
    let Some(item) = load_items
        .iter()
        .find(|item| item.plan.target == "core.steam_links")
    else {
        return Ok(None);
    };

    let count_row = sqlx::query(
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
    .await?;
    let count = count_row.try_get::<i64, _>("count")?;
    if count <= 0 {
        return Ok(None);
    }

    let sample_columns = if item.plan.primary_key.is_empty() {
        vec!["discord_id".to_string(), "steam_id".to_string()]
    } else {
        item.plan.primary_key.clone()
    };
    let select_values = sample_columns
        .iter()
        .map(|column| format!("COALESCE(s.{}::text, 'NULL')", quote_pg_ident(column)))
        .collect::<Vec<_>>()
        .join(", ");
    let order_by = sample_columns
        .iter()
        .map(|column| format!("s.{}", quote_pg_ident(column)))
        .collect::<Vec<_>>()
        .join(", ");
    let sample_sql = format!(
        r#"
        SELECT ARRAY[{select_values}]::text[] AS pk_values
        FROM core.steam_links s
        WHERE s.discord_id <> 0
          AND NOT EXISTS (
              SELECT 1
              FROM core.users u
              WHERE u.discord_id = s.discord_id
          )
        ORDER BY {order_by}
        LIMIT 1
        "#
    );
    let sample_row = sqlx::query(&sample_sql).fetch_one(pool).await?;
    let sample_values = sample_row.try_get::<Vec<String>, _>("pk_values")?;
    let sample_source_pk =
        format_source_pk_from_target_values(&item.plan, &sample_columns, &sample_values);

    Ok(Some(OrphanReport {
        table: "core.steam_links".to_string(),
        parent_table: "core.users".to_string(),
        constraint: "trg_steam_links_user_guard".to_string(),
        count: count as u64,
        sample_source_pk,
    }))
}

fn format_source_pk_from_target_values(
    plan: &TablePlan,
    target_columns: &[String],
    values: &[String],
) -> String {
    target_columns
        .iter()
        .zip(values)
        .map(|(target_column, value)| {
            let source_column = plan
                .columns
                .iter()
                .find(|column| column.target_column == *target_column)
                .map(|column| column.source_column.as_str())
                .unwrap_or(target_column.as_str());
            format!("{source_column}={value}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn is_foreign_key_violation(error: &EngineError) -> bool {
    match error {
        EngineError::Target(TargetError::Sqlx(sqlx::Error::Database(database)))
        | EngineError::Sqlx(sqlx::Error::Database(database)) => {
            database.code().as_deref() == Some("23503")
        }
        _ => false,
    }
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
        Converter::TextToJsonb => text_to_jsonb_value(
            source_db,
            table,
            column,
            value_as_text(source_db, table, source_pk, column, value)?,
        )
        .map(|value| TargetValue::Jsonb(Some(value)))
        .map_err(|source| conversion_error(source_db, table, source_pk, column, source)),
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

fn text_to_jsonb_value(
    source_db: &str,
    table: &str,
    column: &ColumnPlan,
    text: &str,
) -> Result<Value, ConvertError> {
    if source_db == "deadlock-sqlite3"
        && table == "text_conversation_log"
        && column.source_column == "co_participant_ids"
        && column.target_column == "co_participant_ids"
    {
        return text_json_or_integer_csv_list_to_value(text);
    }

    text_json_to_value(text)
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
