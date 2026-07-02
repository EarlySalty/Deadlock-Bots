use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Number, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::{
    build_table_plan,
    engine::{transform_source_row, EngineError},
    ledger::{Ledger, LedgerError, TableLedger},
    reconciliation_manifest::{ReconciliationManifest, TableClassification},
    snapshot::source_snapshot_path,
    source::{SourceError, SourceSqlite},
    target::{TargetError, TargetRow, TargetValue, TargetWriter},
    ColumnPlan, Converter, TablePlan,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateOptions {
    pub manifest_path: PathBuf,
    pub ledger_dir: PathBuf,
    pub original_snapshot_dir: Option<PathBuf>,
    pub current_sources: BTreeMap<String, PathBuf>,
    pub generated_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationOptions {
    pub candidate_options: CandidateOptions,
    pub mode: ReconciliationMode,
    pub abort_table_on_conflict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationMode {
    DryRun,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateReport {
    pub generated_at: String,
    pub mode: String,
    pub cutoff: String,
    pub original_snapshot_dir: String,
    pub source_files: Vec<SourceFileReport>,
    pub totals: CandidateCounters,
    pub tables: Vec<TableCandidateReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceFileReport {
    pub source_db: String,
    pub current_path: String,
    pub original_snapshot_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CandidateCounters {
    pub source_new: u64,
    pub source_changed: u64,
    pub insert_allowed: u64,
    pub update_allowed: u64,
    pub noop: u64,
    pub conflict: u64,
    pub manual: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableCandidateReport {
    pub source_db: String,
    pub ledger_file: String,
    pub source_table: String,
    pub target: Option<String>,
    pub classification: TableClassification,
    pub scope_filter: Option<String>,
    pub source_rows: u64,
    pub original_rows: u64,
    pub target_rows_read: u64,
    pub counters: CandidateCounters,
    pub candidates: Vec<RowCandidateReport>,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowCandidateReport {
    pub source: String,
    pub target_pk: String,
    pub source_hash: String,
    pub original_hash: Option<String>,
    pub target_hash_before: Option<String>,
    pub action: SourceDelta,
    pub decision: MergeDecision,
    pub conflict_target_pk: Option<String>,
    pub conflict_target_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationAuditReport {
    pub generated_at: String,
    pub mode: String,
    pub cutoff: String,
    pub original_snapshot_dir: String,
    pub source_files: Vec<SourceFileReport>,
    pub totals: AuditCounters,
    pub tables: Vec<TableAuditReport>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AuditCounters {
    pub applied: u64,
    pub noop: u64,
    pub conflict: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableAuditReport {
    pub source_db: String,
    pub ledger_file: String,
    pub source_table: String,
    pub target: Option<String>,
    pub classification: TableClassification,
    pub scope_filter: Option<String>,
    pub source_rows: u64,
    pub original_rows: u64,
    pub target_rows_read: u64,
    pub counters: AuditCounters,
    pub rows: Vec<RowAuditReport>,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowAuditReport {
    pub source: String,
    pub target_pk: String,
    pub source_hash: String,
    pub target_hash_before: Option<String>,
    pub action: AuditAction,
    pub reason: String,
    pub source_delta: SourceDelta,
    pub merge_decision: MergeDecision,
    pub original_hash: Option<String>,
    pub conflict_target_pk: Option<String>,
    pub conflict_target_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    Applied,
    Noop,
    Conflict,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceDelta {
    New,
    Changed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeDecision {
    InsertAllowed,
    UpdateAllowed,
    Noop,
    Conflict,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateClassification {
    pub delta: SourceDelta,
    pub decision: MergeDecision,
}

#[derive(Debug, thiserror::Error)]
pub enum CandidateError {
    #[error("Kandidaten-I/O fehlgeschlagen fuer {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("T0-Manifest {path} konnte nicht gelesen werden")]
    ManifestJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "T0-Manifest enthaelt kein selected_snapshot_dir; bitte --original-snapshot-dir setzen"
    )]
    MissingOriginalSnapshotDir,
    #[error("aktueller SQLite-Pfad fuer Source {source_db} fehlt")]
    MissingCurrentSource { source_db: String },
    #[error("Ledger {path} konnte nicht geladen werden")]
    Ledger {
        path: PathBuf,
        #[source]
        source: LedgerError,
    },
    #[error("Ledger-Tabelle fehlt: {source_db}/{ledger_file}:{source_table}")]
    MissingLedgerTable {
        source_db: String,
        ledger_file: String,
        source_table: String,
    },
    #[error("SQLite-Quelle {source_db} konnte nicht gelesen werden: {path}")]
    Source {
        source_db: String,
        path: PathBuf,
        #[source]
        source: SourceError,
    },
    #[error("ETL-Plan/Transformation fehlgeschlagen fuer {source_db}.{source_table}")]
    Engine {
        source_db: String,
        source_table: String,
        #[source]
        source: Box<EngineError>,
    },
    #[error("Postgres-Lesequery fehlgeschlagen fuer {target}")]
    TargetRead {
        target: String,
        #[source]
        source: Box<sqlx::Error>,
    },
    #[error("Postgres-Schreibquery fehlgeschlagen fuer {target}")]
    TargetWrite {
        target: String,
        #[source]
        source: Box<TargetError>,
    },
    #[error("JSON-Normalisierung fehlgeschlagen")]
    Json(#[from] serde_json::Error),
    #[error("Target-Zeile enthaelt keinen Wert fuer PK-Spalte {target}.{column}")]
    MissingTargetPrimaryKeyValue { target: String, column: String },
    #[error("doppelter Primary Key beim Hash-Aufbau fuer {scope} {source_db}.{source_table}")]
    DuplicatePrimaryKey {
        scope: &'static str,
        source_db: String,
        source_table: String,
    },
    #[error("nicht-finites float8 kann nicht kanonisch gehasht werden: {column}")]
    NonFiniteFloat { column: String },
}

pub async fn build_candidate_report(
    options: &CandidateOptions,
    pool: &PgPool,
) -> Result<CandidateReport, CandidateError> {
    let manifest = load_manifest(&options.manifest_path)?;
    let original_snapshot_dir = options
        .original_snapshot_dir
        .clone()
        .or_else(|| {
            manifest
                .cutoff
                .selected_snapshot_dir
                .as_ref()
                .map(PathBuf::from)
        })
        .ok_or(CandidateError::MissingOriginalSnapshotDir)?;
    let source_pairs = open_source_pairs(&manifest, options, &original_snapshot_dir)?;
    let ledgers = load_manifest_ledgers(&manifest, &options.ledger_dir)?;

    let mut tables = Vec::new();
    let mut totals = CandidateCounters::default();

    for table in manifest
        .tables
        .iter()
        .filter(|table| source_pairs.contains_key(&table.source_db))
    {
        let ledger_key = (table.source_db.clone(), table.ledger_file.clone());
        let ledger =
            ledgers
                .get(&ledger_key)
                .ok_or_else(|| CandidateError::MissingLedgerTable {
                    source_db: table.source_db.clone(),
                    ledger_file: table.ledger_file.clone(),
                    source_table: table.source_table.clone(),
                })?;
        let table_ledger = ledger.table(&table.source_table).ok_or_else(|| {
            CandidateError::MissingLedgerTable {
                source_db: table.source_db.clone(),
                ledger_file: table.ledger_file.clone(),
                source_table: table.source_table.clone(),
            }
        })?;
        let pair = source_pairs
            .get(&table.source_db)
            .expect("source_pairs was filtered above");
        let table_report =
            reconcile_table(table, table_ledger, &pair.current, &pair.original, pool).await?;
        totals.add(&table_report.counters);
        tables.push(table_report);
    }

    Ok(CandidateReport {
        generated_at: system_time_rfc3339(options.generated_at),
        mode: "read_only_candidate_detection_no_apply".to_string(),
        cutoff: manifest.cutoff.selected_cutoff,
        original_snapshot_dir: path_string(&original_snapshot_dir),
        source_files: source_pairs
            .into_iter()
            .map(|(source_db, pair)| SourceFileReport {
                source_db,
                current_path: path_string(&pair.current_path),
                original_snapshot_path: path_string(&pair.original_path),
            })
            .collect(),
        totals,
        tables,
    })
}

pub async fn run_reconciliation(
    options: &ReconciliationOptions,
    pool: &PgPool,
) -> Result<ReconciliationAuditReport, CandidateError> {
    match options.mode {
        ReconciliationMode::DryRun => {
            let report = build_candidate_report(&options.candidate_options, pool).await?;
            Ok(candidate_report_to_audit(
                report,
                ReconciliationMode::DryRun,
                false,
            ))
        }
        ReconciliationMode::Apply => apply_reconciliation_report(options, pool).await,
    }
}

async fn apply_reconciliation_report(
    options: &ReconciliationOptions,
    pool: &PgPool,
) -> Result<ReconciliationAuditReport, CandidateError> {
    let manifest = load_manifest(&options.candidate_options.manifest_path)?;
    let original_snapshot_dir = options
        .candidate_options
        .original_snapshot_dir
        .clone()
        .or_else(|| {
            manifest
                .cutoff
                .selected_snapshot_dir
                .as_ref()
                .map(PathBuf::from)
        })
        .ok_or(CandidateError::MissingOriginalSnapshotDir)?;
    let source_pairs = open_source_pairs(
        &manifest,
        &options.candidate_options,
        &original_snapshot_dir,
    )?;
    let source_files = source_pairs
        .iter()
        .map(|(source_db, pair)| SourceFileReport {
            source_db: source_db.clone(),
            current_path: path_string(&pair.current_path),
            original_snapshot_path: path_string(&pair.original_path),
        })
        .collect::<Vec<_>>();
    let ledgers = load_manifest_ledgers(&manifest, &options.candidate_options.ledger_dir)?;
    let writer = TargetWriter::new(pool.clone());

    let mut tables = Vec::new();
    let mut totals = AuditCounters::default();

    for table in manifest
        .tables
        .iter()
        .filter(|table| source_pairs.contains_key(&table.source_db))
    {
        let ledger_key = (table.source_db.clone(), table.ledger_file.clone());
        let ledger =
            ledgers
                .get(&ledger_key)
                .ok_or_else(|| CandidateError::MissingLedgerTable {
                    source_db: table.source_db.clone(),
                    ledger_file: table.ledger_file.clone(),
                    source_table: table.source_table.clone(),
                })?;
        let table_ledger = ledger.table(&table.source_table).ok_or_else(|| {
            CandidateError::MissingLedgerTable {
                source_db: table.source_db.clone(),
                ledger_file: table.ledger_file.clone(),
                source_table: table.source_table.clone(),
            }
        })?;
        let pair = source_pairs
            .get(&table.source_db)
            .expect("source_pairs was filtered above");
        let table_report = apply_table(
            table,
            table_ledger,
            &pair.current,
            &pair.original,
            pool,
            &writer,
            options.abort_table_on_conflict,
        )
        .await?;
        totals.add(&table_report.counters);
        tables.push(table_report);
    }

    Ok(ReconciliationAuditReport {
        generated_at: system_time_rfc3339(options.candidate_options.generated_at),
        mode: "apply_table_transactions".to_string(),
        cutoff: manifest.cutoff.selected_cutoff,
        original_snapshot_dir: path_string(&original_snapshot_dir),
        source_files,
        totals,
        tables,
    })
}

async fn apply_table(
    table: &crate::reconciliation_manifest::TableManifest,
    table_ledger: &TableLedger,
    current: &SourceSqlite,
    original: &SourceSqlite,
    pool: &PgPool,
    writer: &TargetWriter,
    abort_table_on_conflict: bool,
) -> Result<TableAuditReport, CandidateError> {
    let candidate_table = reconcile_table(table, table_ledger, current, original, pool).await?;
    if candidate_table.candidates.is_empty() || candidate_table.skipped_reason.is_some() {
        return Ok(candidate_table_to_audit(
            candidate_table,
            ReconciliationMode::Apply,
            false,
        ));
    }

    let has_blocking_decision = candidate_table.candidates.iter().any(|candidate| {
        matches!(
            candidate.decision,
            MergeDecision::Conflict | MergeDecision::Manual
        )
    });
    if abort_table_on_conflict && has_blocking_decision {
        return Ok(candidate_table_to_audit(
            candidate_table,
            ReconciliationMode::Apply,
            true,
        ));
    }

    let plan = build_table_plan(
        current,
        &table.source_db,
        &table.source_table,
        table_ledger,
        pool,
    )
    .await
    .map_err(|source| CandidateError::Engine {
        source_db: table.source_db.clone(),
        source_table: table.source_table.clone(),
        source: Box::new(source),
    })?;
    let current_target_rows = match source_target_rows(current, &plan, "source") {
        Ok(rows) => rows,
        Err(err) if is_virtual_identity_duplicate(&err, &plan) => {
            return duplicate_virtual_identity_table_report(table, current, original, &plan)
                .map(|table| candidate_table_to_audit(table, ReconciliationMode::Apply, false));
        }
        Err(err) => return Err(err),
    };
    let original_rows = match source_hashes(original, &plan, "original") {
        Ok(rows) => rows,
        Err(err) if is_virtual_identity_duplicate(&err, &plan) => {
            return duplicate_virtual_identity_table_report(table, current, original, &plan)
                .map(|table| candidate_table_to_audit(table, ReconciliationMode::Apply, false));
        }
        Err(err) => return Err(err),
    };
    let current_rows = match source_hashes(current, &plan, "source") {
        Ok(rows) => rows,
        Err(err) if is_virtual_identity_duplicate(&err, &plan) => {
            return duplicate_virtual_identity_table_report(table, current, original, &plan)
                .map(|table| candidate_table_to_audit(table, ReconciliationMode::Apply, false));
        }
        Err(err) => return Err(err),
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(|source| CandidateError::TargetRead {
            target: plan.target.clone(),
            source: Box::new(source),
        })?;
    lock_target_table(&mut tx, &plan).await?;
    let locked_target_rows = target_hashes_in_tx(&mut tx, &plan).await?;
    let locked_candidates = row_candidate_reports(
        &plan,
        table.classification,
        &original_rows,
        &current_rows,
        &locked_target_rows,
    );

    let locked_candidate_table = TableCandidateReport {
        source_db: table.source_db.clone(),
        ledger_file: table.ledger_file.clone(),
        source_table: table.source_table.clone(),
        target: Some(plan.target.clone()),
        classification: table.classification,
        scope_filter: reconciliation_scope_filter(&plan).map(str::to_string),
        source_rows: current_rows.row_count,
        original_rows: original_rows.row_count,
        target_rows_read: locked_target_rows.row_count,
        counters: candidate_counters(&locked_candidates),
        candidates: locked_candidates,
        skipped_reason: None,
    };

    let has_locked_blocking_decision = locked_candidate_table.candidates.iter().any(|candidate| {
        matches!(
            candidate.decision,
            MergeDecision::Conflict | MergeDecision::Manual
        )
    });
    if abort_table_on_conflict && has_locked_blocking_decision {
        drop(tx);
        return Ok(candidate_table_to_audit(
            locked_candidate_table,
            ReconciliationMode::Apply,
            true,
        ));
    }

    let mut upsert_rows = Vec::new();
    let mut insert_rows = Vec::new();
    for candidate in &locked_candidate_table.candidates {
        if !matches!(
            candidate.decision,
            MergeDecision::InsertAllowed | MergeDecision::UpdateAllowed
        ) {
            continue;
        }
        let row = current_target_rows
            .get(&candidate.target_pk)
            .cloned()
            .ok_or_else(|| CandidateError::MissingTargetPrimaryKeyValue {
                target: plan.target.clone(),
                column: candidate.target_pk.clone(),
            })?;
        if plan.primary_key.is_empty() {
            insert_rows.push(row);
        } else {
            upsert_rows.push(row);
        }
    }

    if !upsert_rows.is_empty() {
        writer
            .upsert_rows_in_tx(&mut tx, &plan, &upsert_rows)
            .await
            .map_err(|source| CandidateError::TargetWrite {
                target: plan.target.clone(),
                source: Box::new(source),
            })?;
    }
    if !insert_rows.is_empty() {
        writer
            .insert_rows_in_tx(&mut tx, &plan, &insert_rows)
            .await
            .map_err(|source| CandidateError::TargetWrite {
                target: plan.target.clone(),
                source: Box::new(source),
            })?;
    }
    tx.commit()
        .await
        .map_err(|source| CandidateError::TargetRead {
            target: plan.target.clone(),
            source: Box::new(source),
        })?;

    Ok(candidate_table_to_audit(
        locked_candidate_table,
        ReconciliationMode::Apply,
        false,
    ))
}

pub fn classify_candidate(
    original_hash: Option<&str>,
    source_hash: &str,
    target_hash: Option<&str>,
) -> Option<CandidateClassification> {
    let delta = match original_hash {
        None => SourceDelta::New,
        Some(original) if original != source_hash => SourceDelta::Changed,
        Some(_) => return None,
    };

    let decision = match (delta, original_hash, target_hash) {
        (SourceDelta::New, _, None) => MergeDecision::InsertAllowed,
        (_, _, Some(target)) if target == source_hash => MergeDecision::Noop,
        (SourceDelta::Changed, Some(original), Some(target)) if target == original => {
            MergeDecision::UpdateAllowed
        }
        _ => MergeDecision::Conflict,
    };

    Some(CandidateClassification { delta, decision })
}

pub fn canonical_target_row_hash(
    row: &TargetRow,
    columns: &[ColumnPlan],
) -> Result<String, CandidateError> {
    let mut columns = columns
        .iter()
        .map(|column| {
            let value = row.values.get(&column.target_column).ok_or_else(|| {
                CandidateError::MissingTargetPrimaryKeyValue {
                    target: "<canonical-row>".to_string(),
                    column: column.target_column.clone(),
                }
            })?;
            Ok(CanonicalColumn {
                column: column.target_column.as_str(),
                value: target_value_to_json(&column.target_column, value)?,
            })
        })
        .collect::<Result<Vec<_>, CandidateError>>()?;
    columns.sort_by_key(|column| column.column);

    let bytes = serde_json::to_vec(&columns)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

async fn reconcile_table(
    table: &crate::reconciliation_manifest::TableManifest,
    table_ledger: &TableLedger,
    current: &SourceSqlite,
    original: &SourceSqlite,
    pool: &PgPool,
) -> Result<TableCandidateReport, CandidateError> {
    let plan = build_table_plan(
        current,
        &table.source_db,
        &table.source_table,
        table_ledger,
        pool,
    )
    .await
    .map_err(|source| CandidateError::Engine {
        source_db: table.source_db.clone(),
        source_table: table.source_table.clone(),
        source: Box::new(source),
    })?;

    if effective_primary_key(&plan).is_empty() {
        let source_rows = count_rows(current, &plan, "source")?;
        let original_rows = count_rows(original, &plan, "original")?;
        return Ok(TableCandidateReport {
            source_db: table.source_db.clone(),
            ledger_file: table.ledger_file.clone(),
            source_table: table.source_table.clone(),
            target: Some(plan.target.clone()),
            classification: table.classification,
            scope_filter: reconciliation_scope_filter(&plan).map(str::to_string),
            source_rows,
            original_rows,
            target_rows_read: 0,
            counters: CandidateCounters {
                manual: source_rows,
                ..CandidateCounters::default()
            },
            candidates: Vec::new(),
            skipped_reason: Some("target_primary_key_missing_manual_policy_required".to_string()),
        });
    }

    let original_rows = match source_hashes(original, &plan, "original") {
        Ok(rows) => rows,
        Err(err) if is_virtual_identity_duplicate(&err, &plan) => {
            return duplicate_virtual_identity_table_report(table, current, original, &plan);
        }
        Err(err) => return Err(err),
    };
    let current_rows = match source_hashes(current, &plan, "source") {
        Ok(rows) => rows,
        Err(err) if is_virtual_identity_duplicate(&err, &plan) => {
            return duplicate_virtual_identity_table_report(table, current, original, &plan);
        }
        Err(err) => return Err(err),
    };
    let candidate_count = current_rows
        .rows
        .iter()
        .filter(|(pk, source_row)| {
            original_rows
                .rows
                .get(*pk)
                .is_none_or(|original_row| original_row.hash != source_row.hash)
        })
        .count();
    let target_rows = if candidate_count == 0 {
        TargetHashSet::default()
    } else {
        match target_hashes(pool, &plan).await {
            Ok(rows) => rows,
            Err(err) if is_virtual_identity_duplicate(&err, &plan) => {
                return duplicate_virtual_identity_table_report(table, current, original, &plan);
            }
            Err(err) => return Err(err),
        }
    };
    let candidates = row_candidate_reports(
        &plan,
        table.classification,
        &original_rows,
        &current_rows,
        &target_rows,
    );
    let counters = candidate_counters(&candidates);

    Ok(TableCandidateReport {
        source_db: table.source_db.clone(),
        ledger_file: table.ledger_file.clone(),
        source_table: table.source_table.clone(),
        target: Some(plan.target.clone()),
        classification: table.classification,
        scope_filter: reconciliation_scope_filter(&plan).map(str::to_string),
        source_rows: current_rows.row_count,
        original_rows: original_rows.row_count,
        target_rows_read: target_rows.row_count,
        counters,
        candidates,
        skipped_reason: None,
    })
}

fn duplicate_virtual_identity_table_report(
    table: &crate::reconciliation_manifest::TableManifest,
    current: &SourceSqlite,
    original: &SourceSqlite,
    plan: &TablePlan,
) -> Result<TableCandidateReport, CandidateError> {
    let source_rows = count_rows(current, plan, "source")?;
    let original_rows = count_rows(original, plan, "original")?;

    Ok(TableCandidateReport {
        source_db: table.source_db.clone(),
        ledger_file: table.ledger_file.clone(),
        source_table: table.source_table.clone(),
        target: Some(plan.target.clone()),
        classification: table.classification,
        scope_filter: reconciliation_scope_filter(plan).map(str::to_string),
        source_rows,
        original_rows,
        target_rows_read: 0,
        counters: CandidateCounters {
            manual: source_rows,
            ..CandidateCounters::default()
        },
        candidates: Vec::new(),
        skipped_reason: Some("duplicate_virtual_identity_manual_policy_required".to_string()),
    })
}

fn is_virtual_identity_duplicate(error: &CandidateError, plan: &TablePlan) -> bool {
    matches!(error, CandidateError::DuplicatePrimaryKey { .. })
        && plan.target == "steam.steam_rank_history"
}

fn source_hashes(
    source: &SourceSqlite,
    plan: &TablePlan,
    scope: &'static str,
) -> Result<RowHashSet, CandidateError> {
    let rows = read_scoped_source_rows(source, plan, scope)?;
    let mut hashes = BTreeMap::new();
    for row in &rows {
        let target_row = transform_source_row(&plan.source_db, &plan.source_table, plan, row)
            .map_err(|source| CandidateError::Engine {
                source_db: plan.source_db.clone(),
                source_table: plan.source_table.clone(),
                source: Box::new(source),
            })?;
        let pk = target_primary_key(&target_row, plan)?;
        let hash = canonical_target_row_hash(&target_row, &plan.columns)?;
        let fingerprint = RowFingerprint {
            hash,
            url: row_url(&target_row),
        };
        if hashes.insert(pk, fingerprint).is_some() {
            return Err(CandidateError::DuplicatePrimaryKey {
                scope,
                source_db: plan.source_db.clone(),
                source_table: plan.source_table.clone(),
            });
        }
    }

    Ok(RowHashSet {
        row_count: rows.len() as u64,
        rows: hashes,
    })
}

async fn target_hashes(pool: &PgPool, plan: &TablePlan) -> Result<TargetHashSet, CandidateError> {
    let sql = target_select_sql(plan);
    let rows =
        sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .map_err(|source| CandidateError::TargetRead {
                target: plan.target.clone(),
                source: Box::new(source),
            })?;
    let mut rows_by_pk = BTreeMap::new();
    for row in &rows {
        let target_row = pg_target_row(row, plan)?;
        let pk = target_primary_key(&target_row, plan)?;
        let hash = canonical_target_row_hash(&target_row, &plan.columns)?;
        let fingerprint = RowFingerprint {
            hash,
            url: row_url(&target_row),
        };
        if rows_by_pk.insert(pk, fingerprint).is_some() {
            return Err(CandidateError::DuplicatePrimaryKey {
                scope: "target",
                source_db: plan.source_db.clone(),
                source_table: plan.source_table.clone(),
            });
        }
    }

    Ok(TargetHashSet::from_rows(rows.len() as u64, rows_by_pk))
}

async fn target_hashes_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    plan: &TablePlan,
) -> Result<TargetHashSet, CandidateError> {
    let sql = target_select_sql(plan);
    let rows = sqlx::query(&sql)
        .fetch_all(&mut **tx)
        .await
        .map_err(|source| CandidateError::TargetRead {
            target: plan.target.clone(),
            source: Box::new(source),
        })?;
    let mut rows_by_pk = BTreeMap::new();
    for row in &rows {
        let target_row = pg_target_row(row, plan)?;
        let pk = target_primary_key(&target_row, plan)?;
        let hash = canonical_target_row_hash(&target_row, &plan.columns)?;
        let fingerprint = RowFingerprint {
            hash,
            url: row_url(&target_row),
        };
        if rows_by_pk.insert(pk, fingerprint).is_some() {
            return Err(CandidateError::DuplicatePrimaryKey {
                scope: "target",
                source_db: plan.source_db.clone(),
                source_table: plan.source_table.clone(),
            });
        }
    }

    Ok(TargetHashSet::from_rows(rows.len() as u64, rows_by_pk))
}

fn source_target_rows(
    source: &SourceSqlite,
    plan: &TablePlan,
    scope: &'static str,
) -> Result<BTreeMap<String, TargetRow>, CandidateError> {
    let rows = read_scoped_source_rows(source, plan, scope)?;
    let mut out = BTreeMap::new();
    for row in &rows {
        let target_row = transform_source_row(&plan.source_db, &plan.source_table, plan, row)
            .map_err(|source| CandidateError::Engine {
                source_db: plan.source_db.clone(),
                source_table: plan.source_table.clone(),
                source: Box::new(source),
            })?;
        let pk = target_primary_key(&target_row, plan)?;
        if out.insert(pk, target_row).is_some() {
            return Err(CandidateError::DuplicatePrimaryKey {
                scope,
                source_db: plan.source_db.clone(),
                source_table: plan.source_table.clone(),
            });
        }
    }

    Ok(out)
}

async fn lock_target_table(
    tx: &mut Transaction<'_, Postgres>,
    plan: &TablePlan,
) -> Result<(), CandidateError> {
    let sql = format!(
        "LOCK TABLE {} IN SHARE ROW EXCLUSIVE MODE",
        quote_pg_path(&plan.target)
    );
    sqlx::query(&sql)
        .execute(&mut **tx)
        .await
        .map_err(|source| CandidateError::TargetRead {
            target: plan.target.clone(),
            source: Box::new(source),
        })?;

    Ok(())
}

fn row_candidate_reports(
    plan: &TablePlan,
    table_classification: TableClassification,
    original_rows: &RowHashSet,
    current_rows: &RowHashSet,
    target_hashes: &TargetHashSet,
) -> Vec<RowCandidateReport> {
    let mut candidates = Vec::new();
    let source = format!("{}.{}", plan.source_db, plan.source_table);
    let table_policy = table_policy(plan, table_classification);

    for (pk, source_row) in &current_rows.rows {
        let original_hash = original_rows.rows.get(pk).map(|row| row.hash.as_str());
        let target_hash = target_hashes.rows.get(pk).map(|row| row.hash.as_str());
        let Some(mut classification) =
            classify_candidate(original_hash, &source_row.hash, target_hash)
        else {
            continue;
        };

        let url_conflict = patchnotes_url_conflict(plan, pk, source_row, target_hashes);
        if url_conflict.is_some() {
            classification.decision = MergeDecision::Conflict;
        }
        classification.decision = apply_table_policy(table_policy, classification);

        candidates.push(RowCandidateReport {
            source: source.clone(),
            target_pk: pk.clone(),
            source_hash: source_row.hash.clone(),
            original_hash: original_hash.map(str::to_string),
            target_hash_before: target_hash.map(str::to_string),
            action: classification.delta,
            decision: classification.decision,
            conflict_target_pk: url_conflict
                .as_ref()
                .map(|conflict| conflict.target_pk.clone()),
            conflict_target_hash: url_conflict.map(|conflict| conflict.target_hash),
        });
    }

    candidates
}

fn candidate_counters(candidates: &[RowCandidateReport]) -> CandidateCounters {
    let mut counters = CandidateCounters::default();

    for candidate in candidates {
        match candidate.action {
            SourceDelta::New => counters.source_new += 1,
            SourceDelta::Changed => counters.source_changed += 1,
        }
        match candidate.decision {
            MergeDecision::InsertAllowed => counters.insert_allowed += 1,
            MergeDecision::UpdateAllowed => counters.update_allowed += 1,
            MergeDecision::Noop => counters.noop += 1,
            MergeDecision::Conflict => counters.conflict += 1,
            MergeDecision::Manual => counters.manual += 1,
        }
    }

    counters
}

fn candidate_report_to_audit(
    report: CandidateReport,
    mode: ReconciliationMode,
    table_aborted: bool,
) -> ReconciliationAuditReport {
    let mut totals = AuditCounters::default();
    let tables = report
        .tables
        .into_iter()
        .map(|table| {
            let audit_table = candidate_table_to_audit(table, mode, table_aborted);
            totals.add(&audit_table.counters);
            audit_table
        })
        .collect::<Vec<_>>();

    ReconciliationAuditReport {
        generated_at: report.generated_at,
        mode: match mode {
            ReconciliationMode::DryRun => "dry_run_read_only".to_string(),
            ReconciliationMode::Apply => "apply_table_transactions".to_string(),
        },
        cutoff: report.cutoff,
        original_snapshot_dir: report.original_snapshot_dir,
        source_files: report.source_files,
        totals,
        tables,
    }
}

fn candidate_table_to_audit(
    table: TableCandidateReport,
    mode: ReconciliationMode,
    table_aborted: bool,
) -> TableAuditReport {
    let skipped_reason = table.skipped_reason.clone();
    let table_manual = table.counters.manual;
    let rows = table
        .candidates
        .iter()
        .map(|candidate| row_to_audit(candidate, mode, table_aborted))
        .collect::<Vec<_>>();
    let mut counters = audit_counters(&rows);
    if skipped_reason.is_some() && rows.is_empty() {
        counters.skipped += table_manual;
    }

    TableAuditReport {
        source_db: table.source_db,
        ledger_file: table.ledger_file,
        source_table: table.source_table,
        target: table.target,
        classification: table.classification,
        scope_filter: table.scope_filter,
        source_rows: table.source_rows,
        original_rows: table.original_rows,
        target_rows_read: table.target_rows_read,
        counters,
        rows,
        skipped_reason,
    }
}

fn row_to_audit(
    candidate: &RowCandidateReport,
    mode: ReconciliationMode,
    table_aborted: bool,
) -> RowAuditReport {
    let action = audit_action(candidate.decision, mode, table_aborted);
    let reason = audit_reason(candidate, mode, table_aborted);

    RowAuditReport {
        source: candidate.source.clone(),
        target_pk: candidate.target_pk.clone(),
        source_hash: candidate.source_hash.clone(),
        target_hash_before: candidate.target_hash_before.clone(),
        action,
        reason,
        source_delta: candidate.action,
        merge_decision: candidate.decision,
        original_hash: candidate.original_hash.clone(),
        conflict_target_pk: candidate.conflict_target_pk.clone(),
        conflict_target_hash: candidate.conflict_target_hash.clone(),
    }
}

fn audit_action(
    decision: MergeDecision,
    mode: ReconciliationMode,
    table_aborted: bool,
) -> AuditAction {
    match decision {
        MergeDecision::Noop => AuditAction::Noop,
        MergeDecision::Conflict => AuditAction::Conflict,
        MergeDecision::Manual => AuditAction::Skipped,
        MergeDecision::InsertAllowed | MergeDecision::UpdateAllowed => {
            if mode == ReconciliationMode::Apply && !table_aborted {
                AuditAction::Applied
            } else {
                AuditAction::Skipped
            }
        }
    }
}

fn audit_reason(
    candidate: &RowCandidateReport,
    mode: ReconciliationMode,
    table_aborted: bool,
) -> String {
    if table_aborted
        && matches!(
            candidate.decision,
            MergeDecision::InsertAllowed | MergeDecision::UpdateAllowed
        )
    {
        return "table_conflict_abort_no_rows_applied".to_string();
    }

    match candidate.decision {
        MergeDecision::InsertAllowed => match mode {
            ReconciliationMode::DryRun => "dry_run_would_insert_target_missing".to_string(),
            ReconciliationMode::Apply => "insert_applied_target_missing".to_string(),
        },
        MergeDecision::UpdateAllowed => match mode {
            ReconciliationMode::DryRun => "dry_run_would_update_target_equals_original".to_string(),
            ReconciliationMode::Apply => "update_applied_target_equals_original".to_string(),
        },
        MergeDecision::Noop => "target_already_equals_source".to_string(),
        MergeDecision::Conflict if candidate.conflict_target_pk.is_some() => {
            "patchnotes_url_conflict_same_url_different_id".to_string()
        }
        MergeDecision::Conflict => {
            "target_changed_independently_or_append_identity_payload_conflict".to_string()
        }
        MergeDecision::Manual => "manual_policy_required_for_table".to_string(),
    }
}

fn audit_counters(rows: &[RowAuditReport]) -> AuditCounters {
    let mut counters = AuditCounters::default();
    for row in rows {
        match row.action {
            AuditAction::Applied => counters.applied += 1,
            AuditAction::Noop => counters.noop += 1,
            AuditAction::Conflict => counters.conflict += 1,
            AuditAction::Skipped => counters.skipped += 1,
        }
    }
    counters
}

fn apply_table_policy(
    policy: TablePolicy,
    classification: CandidateClassification,
) -> MergeDecision {
    match policy {
        TablePolicy::Normal => classification.decision,
        TablePolicy::ManualAll => match classification.decision {
            MergeDecision::Noop => MergeDecision::Noop,
            MergeDecision::Conflict => MergeDecision::Conflict,
            _ => MergeDecision::Manual,
        },
        TablePolicy::InsertOnly => match classification.decision {
            MergeDecision::UpdateAllowed => MergeDecision::Manual,
            other => other,
        },
        TablePolicy::AppendIdentityOnly => match classification.decision {
            MergeDecision::Noop | MergeDecision::InsertAllowed => classification.decision,
            _ if classification.delta == SourceDelta::Changed => MergeDecision::Conflict,
            other => other,
        },
    }
}

fn table_policy(plan: &TablePlan, classification: TableClassification) -> TablePolicy {
    if plan.target == "steam.steam_tasks" {
        return TablePolicy::ManualAll;
    }

    if plan.target == "steam.steam_rank_history" {
        return TablePolicy::AppendIdentityOnly;
    }

    if is_website_meta_target(&plan.target) {
        return TablePolicy::InsertOnly;
    }

    match classification {
        TableClassification::QueueState
            if plan.target == "steam.steam_tasks"
                || plan.target.ends_with(".notification_queue") =>
        {
            TablePolicy::ManualAll
        }
        _ => TablePolicy::Normal,
    }
}

fn is_website_meta_target(target: &str) -> bool {
    target == "core.meta_users"
        || target == "patchnotes.meta_patch_notes"
        || target.starts_with("tierlist.meta_")
        || target.starts_with("content.meta_")
}

fn patchnotes_url_conflict(
    plan: &TablePlan,
    source_pk: &str,
    source_row: &RowFingerprint,
    target_rows: &TargetHashSet,
) -> Option<UrlConflict> {
    if !requires_patchnotes_url_conflict_check(plan) {
        return None;
    }
    let source_url = source_row.url.as_ref()?;
    for target_pk in target_rows.url_index.get(source_url)? {
        if target_pk == source_pk {
            continue;
        }
        let target_row = target_rows.rows.get(target_pk)?;
        return Some(UrlConflict {
            target_pk: target_pk.clone(),
            target_hash: target_row.hash.clone(),
        });
    }

    None
}

fn requires_patchnotes_url_conflict_check(plan: &TablePlan) -> bool {
    matches!(
        plan.target.as_str(),
        "patchnotes.changelog_posts" | "patchnotes.deadlock_changelogs"
    )
}

fn pg_target_row(
    row: &sqlx::postgres::PgRow,
    plan: &TablePlan,
) -> Result<TargetRow, CandidateError> {
    let mut values = BTreeMap::new();

    for column in &plan.columns {
        let alias = column.target_column.as_str();
        let value = match target_kind(column.converter) {
            TargetKind::Text => TargetValue::Text(row.try_get(alias).map_err(|source| {
                CandidateError::TargetRead {
                    target: plan.target.clone(),
                    source: Box::new(source),
                }
            })?),
            TargetKind::Int8 => TargetValue::Int8(row.try_get(alias).map_err(|source| {
                CandidateError::TargetRead {
                    target: plan.target.clone(),
                    source: Box::new(source),
                }
            })?),
            TargetKind::Int4 => TargetValue::Int4(row.try_get(alias).map_err(|source| {
                CandidateError::TargetRead {
                    target: plan.target.clone(),
                    source: Box::new(source),
                }
            })?),
            TargetKind::Float8 => TargetValue::Float8(row.try_get(alias).map_err(|source| {
                CandidateError::TargetRead {
                    target: plan.target.clone(),
                    source: Box::new(source),
                }
            })?),
            TargetKind::Bool => TargetValue::Bool(row.try_get(alias).map_err(|source| {
                CandidateError::TargetRead {
                    target: plan.target.clone(),
                    source: Box::new(source),
                }
            })?),
            TargetKind::Jsonb => {
                let raw: Option<String> =
                    row.try_get(alias)
                        .map_err(|source| CandidateError::TargetRead {
                            target: plan.target.clone(),
                            source: Box::new(source),
                        })?;
                TargetValue::Jsonb(raw.map(|value| serde_json::from_str(&value)).transpose()?)
            }
            TargetKind::Timestamptz => {
                TargetValue::Timestamptz(row.try_get(alias).map_err(|source| {
                    CandidateError::TargetRead {
                        target: plan.target.clone(),
                        source: Box::new(source),
                    }
                })?)
            }
            TargetKind::Date => TargetValue::Date(row.try_get(alias).map_err(|source| {
                CandidateError::TargetRead {
                    target: plan.target.clone(),
                    source: Box::new(source),
                }
            })?),
        };
        values.insert(column.target_column.clone(), value);
    }

    Ok(TargetRow { values })
}

fn target_select_sql(plan: &TablePlan) -> String {
    let columns = plan
        .columns
        .iter()
        .map(|column| {
            let column_name = quote_pg_ident(&column.target_column);
            if target_kind(column.converter) == TargetKind::Jsonb {
                format!("{column_name}::text AS {column_name}")
            } else {
                format!("{column_name} AS {column_name}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!("SELECT {columns} FROM {}", quote_pg_path(&plan.target));
    if let Some(where_clause) = target_scope_where_clause(plan) {
        sql.push_str(" WHERE ");
        sql.push_str(where_clause);
    }
    sql
}

fn target_primary_key(row: &TargetRow, plan: &TablePlan) -> Result<String, CandidateError> {
    effective_primary_key(plan)
        .iter()
        .map(|column| {
            let value = row.values.get(column).ok_or_else(|| {
                CandidateError::MissingTargetPrimaryKeyValue {
                    target: plan.target.clone(),
                    column: column.clone(),
                }
            })?;
            Ok(format!(
                "{column}={}",
                target_value_primary_key_part(value)?
            ))
        })
        .collect::<Result<Vec<_>, CandidateError>>()
        .map(|parts| parts.join(","))
}

fn effective_primary_key(plan: &TablePlan) -> Vec<String> {
    if plan.target == "steam.steam_rank_history" {
        vec!["user_id".to_string(), "captured_at".to_string()]
    } else {
        plan.primary_key.clone()
    }
}

fn target_value_primary_key_part(value: &TargetValue) -> Result<String, CandidateError> {
    match value {
        TargetValue::Text(value) => Ok(value.clone().unwrap_or_else(|| "NULL".to_string())),
        TargetValue::Int8(value) => Ok(value.map_or_else(|| "NULL".to_string(), |v| v.to_string())),
        TargetValue::Int4(value) => Ok(value.map_or_else(|| "NULL".to_string(), |v| v.to_string())),
        TargetValue::Float8(value) => value.map_or_else(
            || Ok("NULL".to_string()),
            |value| {
                if value.is_finite() {
                    Ok(value.to_string())
                } else {
                    Err(CandidateError::NonFiniteFloat {
                        column: "<primary-key>".to_string(),
                    })
                }
            },
        ),
        TargetValue::Bool(value) => Ok(value.map_or_else(|| "NULL".to_string(), |v| v.to_string())),
        TargetValue::Jsonb(value) => value.as_ref().map_or_else(
            || Ok("NULL".to_string()),
            |value| serde_json::to_string(&canonical_json(value)).map_err(CandidateError::from),
        ),
        TargetValue::Timestamptz(value) => Ok(value.map_or_else(
            || "NULL".to_string(),
            |value| value.to_rfc3339_opts(SecondsFormat::Nanos, true),
        )),
        TargetValue::Date(value) => Ok(value.map_or_else(|| "NULL".to_string(), format_date)),
    }
}

fn target_value_to_json(column: &str, value: &TargetValue) -> Result<Value, CandidateError> {
    match value {
        TargetValue::Text(value) => Ok(option_string(value)),
        TargetValue::Int8(value) => {
            Ok(value.map_or(Value::Null, |value| Value::Number(Number::from(value))))
        }
        TargetValue::Int4(value) => {
            Ok(value.map_or(Value::Null, |value| Value::Number(Number::from(value))))
        }
        TargetValue::Float8(value) => value.map_or_else(
            || Ok(Value::Null),
            |value| {
                Number::from_f64(value).map(Value::Number).ok_or_else(|| {
                    CandidateError::NonFiniteFloat {
                        column: column.to_string(),
                    }
                })
            },
        ),
        TargetValue::Bool(value) => Ok(value.map_or(Value::Null, Value::Bool)),
        TargetValue::Jsonb(value) => Ok(value.as_ref().map_or(Value::Null, canonical_json)),
        TargetValue::Timestamptz(value) => Ok(value.map_or(Value::Null, |value| {
            Value::String(value.to_rfc3339_opts(SecondsFormat::Nanos, true))
        })),
        TargetValue::Date(value) => {
            Ok(value.map_or(Value::Null, |value| Value::String(format_date(value))))
        }
    }
}

fn option_string(value: &Option<String>) -> Value {
    value
        .as_ref()
        .map_or(Value::Null, |value| Value::String(value.clone()))
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(value) = map.get(key) {
                    out.insert(key.clone(), canonical_json(value));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn format_date(value: NaiveDate) -> String {
    value.format("%Y-%m-%d").to_string()
}

fn target_kind(converter: Converter) -> TargetKind {
    match converter {
        Converter::TextToText | Converter::IntegerToText => TargetKind::Text,
        Converter::TextToJsonb => TargetKind::Jsonb,
        Converter::TextToBigint | Converter::IntegerToInt8 => TargetKind::Int8,
        Converter::IntegerToInt4 => TargetKind::Int4,
        Converter::RealToFloat8 => TargetKind::Float8,
        Converter::NumericToBool | Converter::IntegerToBool => TargetKind::Bool,
        Converter::TextToTimestamptz
        | Converter::NumericToTimestamptz
        | Converter::IntegerToTimestamptz
        | Converter::RealToTimestamptz => TargetKind::Timestamptz,
        Converter::TextToDate => TargetKind::Date,
    }
}

fn count_rows(
    source: &SourceSqlite,
    plan: &TablePlan,
    scope: &'static str,
) -> Result<u64, CandidateError> {
    read_scoped_source_rows(source, plan, scope).map(|rows| rows.len() as u64)
}

fn read_scoped_source_rows(
    source: &SourceSqlite,
    plan: &TablePlan,
    scope: &'static str,
) -> Result<Vec<crate::source::SourceRow>, CandidateError> {
    let mut rows =
        source
            .read_rows(&plan.source_table)
            .map_err(|source| CandidateError::Source {
                source_db: plan.source_db.clone(),
                path: PathBuf::from(format!("<{scope}>")),
                source,
            })?;
    rows.retain(|row| source_row_in_reconciliation_scope(row, plan));
    Ok(rows)
}

fn source_row_in_reconciliation_scope(row: &crate::source::SourceRow, plan: &TablePlan) -> bool {
    if plan.target == "bot.kv_store" {
        return matches!(
            row.values.get("ns"),
            Some(Value::String(ns)) if ns == "patchnotes_bot"
        );
    }

    true
}

fn reconciliation_scope_filter(plan: &TablePlan) -> Option<&'static str> {
    if plan.target == "bot.kv_store" {
        Some("ns='patchnotes_bot'")
    } else {
        None
    }
}

fn target_scope_where_clause(plan: &TablePlan) -> Option<&'static str> {
    if plan.target == "bot.kv_store" {
        Some("\"ns\" = 'patchnotes_bot'")
    } else {
        None
    }
}

fn row_url(row: &TargetRow) -> Option<String> {
    match row.values.get("url") {
        Some(TargetValue::Text(Some(url))) if !url.trim().is_empty() => Some(url.clone()),
        _ => None,
    }
}

fn load_manifest(path: &Path) -> Result<ReconciliationManifest, CandidateError> {
    let contents = fs::read_to_string(path).map_err(|source| CandidateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| CandidateError::ManifestJson {
        path: path.to_path_buf(),
        source,
    })
}

fn load_manifest_ledgers(
    manifest: &ReconciliationManifest,
    ledger_dir: &Path,
) -> Result<BTreeMap<(String, String), Ledger>, CandidateError> {
    let mut seen = BTreeSet::new();
    let mut out = BTreeMap::new();

    for ledger_file in &manifest.ledger_files {
        let key = (
            ledger_file.source_db.clone(),
            ledger_file.ledger_file.clone(),
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        let path = ledger_dir
            .join(&ledger_file.source_db)
            .join(&ledger_file.ledger_file);
        let contents = fs::read_to_string(&path).map_err(|source| CandidateError::Io {
            path: path.clone(),
            source,
        })?;
        let ledger = Ledger::from_toml_str(&contents)
            .map_err(|source| CandidateError::Ledger { path, source })?;
        out.insert(key, ledger);
    }

    Ok(out)
}

fn open_source_pairs(
    manifest: &ReconciliationManifest,
    options: &CandidateOptions,
    original_snapshot_dir: &Path,
) -> Result<BTreeMap<String, SourcePair>, CandidateError> {
    let mut source_dbs = manifest
        .tables
        .iter()
        .map(|table| table.source_db.clone())
        .collect::<BTreeSet<_>>();
    source_dbs.retain(|source_db| options.current_sources.contains_key(source_db));

    let mut out = BTreeMap::new();
    for source_db in source_dbs {
        let current_path = options
            .current_sources
            .get(&source_db)
            .cloned()
            .ok_or_else(|| CandidateError::MissingCurrentSource {
                source_db: source_db.clone(),
            })?;
        let original_path = source_snapshot_path(original_snapshot_dir, &source_db);
        let current = SourceSqlite::open_read_only(&current_path).map_err(|source| {
            CandidateError::Source {
                source_db: source_db.clone(),
                path: current_path.clone(),
                source,
            }
        })?;
        let original = SourceSqlite::open_read_only(&original_path).map_err(|source| {
            CandidateError::Source {
                source_db: source_db.clone(),
                path: original_path.clone(),
                source,
            }
        })?;
        out.insert(
            source_db,
            SourcePair {
                current,
                original,
                current_path,
                original_path,
            },
        );
    }

    Ok(out)
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

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn system_time_rfc3339(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Nanos, true)
}

impl CandidateCounters {
    fn add(&mut self, other: &Self) {
        self.source_new += other.source_new;
        self.source_changed += other.source_changed;
        self.insert_allowed += other.insert_allowed;
        self.update_allowed += other.update_allowed;
        self.noop += other.noop;
        self.conflict += other.conflict;
        self.manual += other.manual;
    }
}

impl AuditCounters {
    fn add(&mut self, other: &Self) {
        self.applied += other.applied;
        self.noop += other.noop;
        self.conflict += other.conflict;
        self.skipped += other.skipped;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalColumn<'a> {
    column: &'a str,
    value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Text,
    Int8,
    Int4,
    Float8,
    Bool,
    Jsonb,
    Timestamptz,
    Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TablePolicy {
    Normal,
    ManualAll,
    InsertOnly,
    AppendIdentityOnly,
}

struct SourcePair {
    current: SourceSqlite,
    original: SourceSqlite,
    current_path: PathBuf,
    original_path: PathBuf,
}

#[derive(Debug, Default)]
struct RowHashSet {
    row_count: u64,
    rows: BTreeMap<String, RowFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RowFingerprint {
    hash: String,
    url: Option<String>,
}

#[derive(Debug, Default)]
struct TargetHashSet {
    row_count: u64,
    rows: BTreeMap<String, RowFingerprint>,
    url_index: BTreeMap<String, BTreeSet<String>>,
}

impl TargetHashSet {
    fn from_rows(row_count: u64, rows: BTreeMap<String, RowFingerprint>) -> Self {
        let mut url_index = BTreeMap::<String, BTreeSet<String>>::new();
        for (pk, row) in &rows {
            if let Some(url) = &row.url {
                url_index.entry(url.clone()).or_default().insert(pk.clone());
            }
        }

        Self {
            row_count,
            rows,
            url_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UrlConflict {
    target_pk: String,
    target_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    #[test]
    fn classifies_new_source_missing_target_as_insert() {
        let result = classify_candidate(None, "source", None).expect("candidate");

        assert_eq!(result.delta, SourceDelta::New);
        assert_eq!(result.decision, MergeDecision::InsertAllowed);
    }

    #[test]
    fn classifies_changed_source_target_still_original_as_update() {
        let result =
            classify_candidate(Some("original"), "source", Some("original")).expect("candidate");

        assert_eq!(result.delta, SourceDelta::Changed);
        assert_eq!(result.decision, MergeDecision::UpdateAllowed);
    }

    #[test]
    fn classifies_target_already_equal_to_source_as_noop() {
        let result =
            classify_candidate(Some("original"), "source", Some("source")).expect("candidate");

        assert_eq!(result.delta, SourceDelta::Changed);
        assert_eq!(result.decision, MergeDecision::Noop);
    }

    #[test]
    fn classifies_parallel_target_and_source_changes_as_conflict() {
        let result =
            classify_candidate(Some("original"), "source", Some("target")).expect("candidate");

        assert_eq!(result.delta, SourceDelta::Changed);
        assert_eq!(result.decision, MergeDecision::Conflict);
    }

    #[test]
    fn unchanged_source_is_not_a_candidate() {
        assert!(classify_candidate(Some("same"), "same", Some("target")).is_none());
    }

    #[test]
    fn row_report_includes_auditable_pk_hashes_and_decisions() {
        let plan = demo_items_plan();
        let (_original_file, original) = sqlite_source(
            r#"
            CREATE TABLE items(id INTEGER PRIMARY KEY, body TEXT NOT NULL);
            INSERT INTO items(id, body) VALUES (1, 'old');
            "#,
        );
        let (_current_file, current) = sqlite_source(
            r#"
            CREATE TABLE items(id INTEGER PRIMARY KEY, body TEXT NOT NULL);
            INSERT INTO items(id, body) VALUES (1, 'new'), (2, 'fresh');
            "#,
        );
        let original_rows = source_hashes(&original, &plan, "original").expect("original hashes");
        let current_rows = source_hashes(&current, &plan, "source").expect("current hashes");
        let target_rows = TargetHashSet::from_rows(
            1,
            BTreeMap::from([(
                "id=1".to_string(),
                original_rows.rows.get("id=1").expect("id=1").clone(),
            )]),
        );

        let candidates = row_candidate_reports(
            &plan,
            TableClassification::ClockPresent,
            &original_rows,
            &current_rows,
            &target_rows,
        );

        assert_eq!(candidates.len(), 2);
        let changed = candidate_for_pk(&candidates, "id=1");
        assert_eq!(changed.source, "test.items");
        assert_eq!(changed.source_hash, current_rows.rows["id=1"].hash);
        assert_eq!(
            changed.original_hash.as_deref(),
            Some(original_rows.rows["id=1"].hash.as_str())
        );
        assert_eq!(
            changed.target_hash_before.as_deref(),
            Some(original_rows.rows["id=1"].hash.as_str())
        );
        assert_eq!(changed.action, SourceDelta::Changed);
        assert_eq!(changed.decision, MergeDecision::UpdateAllowed);
        assert!(changed.conflict_target_pk.is_none());
        assert!(changed.conflict_target_hash.is_none());

        let inserted = candidate_for_pk(&candidates, "id=2");
        assert_eq!(inserted.source_hash, current_rows.rows["id=2"].hash);
        assert_eq!(inserted.original_hash, None);
        assert_eq!(inserted.target_hash_before, None);
        assert_eq!(inserted.action, SourceDelta::New);
        assert_eq!(inserted.decision, MergeDecision::InsertAllowed);

        let counters = candidate_counters(&candidates);
        assert_eq!(counters.source_new, 1);
        assert_eq!(counters.source_changed, 1);
        assert_eq!(counters.insert_allowed, 1);
        assert_eq!(counters.update_allowed, 1);
    }

    #[test]
    fn steam_tasks_queue_policy_forces_manual_decisions() {
        let plan = steam_tasks_plan();
        let (_original_file, original) = sqlite_source(
            r#"
            CREATE TABLE steam_tasks(id INTEGER PRIMARY KEY, type TEXT NOT NULL, status TEXT NOT NULL);
            INSERT INTO steam_tasks(id, type, status) VALUES (1, 'rank_fetch', 'PENDING');
            "#,
        );
        let (_current_file, current) = sqlite_source(
            r#"
            CREATE TABLE steam_tasks(id INTEGER PRIMARY KEY, type TEXT NOT NULL, status TEXT NOT NULL);
            INSERT INTO steam_tasks(id, type, status) VALUES (1, 'rank_fetch', 'RUNNING');
            "#,
        );
        let original_rows = source_hashes(&original, &plan, "original").expect("original hashes");
        let current_rows = source_hashes(&current, &plan, "source").expect("current hashes");
        let target_rows = TargetHashSet::from_rows(
            1,
            BTreeMap::from([(
                "id=1".to_string(),
                original_rows.rows.get("id=1").expect("id=1").clone(),
            )]),
        );

        let candidates = row_candidate_reports(
            &plan,
            TableClassification::QueueState,
            &original_rows,
            &current_rows,
            &target_rows,
        );
        let counters = candidate_counters(&candidates);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].decision, MergeDecision::Manual);
        assert_eq!(counters.source_changed, 1);
        assert_eq!(counters.manual, 1);
        assert_eq!(counters.update_allowed, 0);
        assert_eq!(counters.insert_allowed, 0);
        assert_eq!(counters.noop, 0);
    }

    #[test]
    fn patchnotes_same_url_with_different_id_forces_conflict() {
        let plan = patchnotes_plan();
        let (_original_file, original) = sqlite_source(
            r#"
            CREATE TABLE changelog_posts(id INTEGER PRIMARY KEY, url TEXT NOT NULL);
            "#,
        );
        let (_current_file, current) = sqlite_source(
            r#"
            CREATE TABLE changelog_posts(id INTEGER PRIMARY KEY, url TEXT NOT NULL);
            INSERT INTO changelog_posts(id, url)
            VALUES (7, 'https://forums.playdeadlock.com/threads/update-42');
            "#,
        );
        let original_rows = source_hashes(&original, &plan, "original").expect("original hashes");
        let current_rows = source_hashes(&current, &plan, "source").expect("current hashes");
        let target_rows = target_hash_set(
            vec![TargetRow {
                values: BTreeMap::from([
                    ("id".to_string(), TargetValue::Int8(Some(99))),
                    (
                        "url".to_string(),
                        TargetValue::Text(Some(
                            "https://forums.playdeadlock.com/threads/update-42".to_string(),
                        )),
                    ),
                ]),
            }],
            &plan,
        );

        let candidates = row_candidate_reports(
            &plan,
            TableClassification::AppendOnly,
            &original_rows,
            &current_rows,
            &target_rows,
        );
        let counters = candidate_counters(&candidates);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].target_pk, "id=7");
        assert_eq!(candidates[0].decision, MergeDecision::Conflict);
        assert_eq!(candidates[0].target_hash_before, None);
        assert_eq!(candidates[0].conflict_target_pk.as_deref(), Some("id=99"));
        assert_eq!(
            candidates[0].conflict_target_hash.as_deref(),
            Some(target_rows.rows["id=99"].hash.as_str())
        );
        assert_eq!(counters.conflict, 1);
        assert_eq!(counters.insert_allowed, 0);
    }

    #[test]
    fn kv_store_candidate_scope_only_reads_patchnotes_namespace() {
        let plan = kv_store_plan();
        let (_original_file, original) = sqlite_source(
            r#"
            CREATE TABLE kv_store(ns TEXT NOT NULL, k TEXT NOT NULL, v TEXT NOT NULL, PRIMARY KEY(ns, k));
            INSERT INTO kv_store(ns, k, v) VALUES
                ('patchnotes_bot', 'last_seen', 'old-post'),
                ('voice_cfg', 'panel', 'old-panel');
            "#,
        );
        let (_current_file, current) = sqlite_source(
            r#"
            CREATE TABLE kv_store(ns TEXT NOT NULL, k TEXT NOT NULL, v TEXT NOT NULL, PRIMARY KEY(ns, k));
            INSERT INTO kv_store(ns, k, v) VALUES
                ('patchnotes_bot', 'last_seen', 'new-post'),
                ('voice_cfg', 'panel', 'new-panel');
            "#,
        );
        let original_rows = source_hashes(&original, &plan, "original").expect("original hashes");
        let current_rows = source_hashes(&current, &plan, "source").expect("current hashes");

        assert_eq!(original_rows.row_count, 1);
        assert_eq!(current_rows.row_count, 1);
        assert!(current_rows
            .rows
            .contains_key("ns=patchnotes_bot,k=last_seen"));
        assert!(!current_rows.rows.contains_key("ns=voice_cfg,k=panel"));
        assert_eq!(
            reconciliation_scope_filter(&plan),
            Some("ns='patchnotes_bot'")
        );
        assert!(target_select_sql(&plan).contains("WHERE \"ns\" = 'patchnotes_bot'"));

        let target_rows = TargetHashSet::from_rows(
            1,
            BTreeMap::from([(
                "ns=patchnotes_bot,k=last_seen".to_string(),
                original_rows
                    .rows
                    .get("ns=patchnotes_bot,k=last_seen")
                    .expect("patchnotes key")
                    .clone(),
            )]),
        );
        let candidates = row_candidate_reports(
            &plan,
            TableClassification::NoClock,
            &original_rows,
            &current_rows,
            &target_rows,
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].target_pk, "ns=patchnotes_bot,k=last_seen");
        assert_eq!(candidates[0].decision, MergeDecision::UpdateAllowed);
    }

    #[test]
    fn website_meta_policy_allows_inserts_but_not_updates() {
        let plan = meta_users_plan();
        let (_original_file, original) = sqlite_source(
            r#"
            CREATE TABLE meta_users(id INTEGER PRIMARY KEY, username TEXT);
            INSERT INTO meta_users(id, username) VALUES (1, 'old');
            "#,
        );
        let (_current_file, current) = sqlite_source(
            r#"
            CREATE TABLE meta_users(id INTEGER PRIMARY KEY, username TEXT);
            INSERT INTO meta_users(id, username) VALUES (1, 'new'), (2, 'fresh');
            "#,
        );
        let original_rows = source_hashes(&original, &plan, "original").expect("original hashes");
        let current_rows = source_hashes(&current, &plan, "source").expect("current hashes");
        let target_rows = TargetHashSet::from_rows(
            1,
            BTreeMap::from([(
                "id=1".to_string(),
                original_rows.rows.get("id=1").expect("id=1").clone(),
            )]),
        );

        let candidates = row_candidate_reports(
            &plan,
            TableClassification::NoClock,
            &original_rows,
            &current_rows,
            &target_rows,
        );

        assert_eq!(
            candidate_for_pk(&candidates, "id=1").decision,
            MergeDecision::Manual
        );
        assert_eq!(
            candidate_for_pk(&candidates, "id=2").decision,
            MergeDecision::InsertAllowed
        );
    }

    #[test]
    fn steam_rank_history_uses_virtual_append_identity() {
        let plan = steam_rank_history_plan();
        let (_original_file, original) = sqlite_source(
            r#"
            CREATE TABLE steam_rank_history(user_id INTEGER NOT NULL, captured_at TEXT NOT NULL, rank_name TEXT);
            INSERT INTO steam_rank_history(user_id, captured_at, rank_name)
            VALUES (7, '2026-07-01T00:00:00Z', 'Old');
            "#,
        );
        let (_current_file, current) = sqlite_source(
            r#"
            CREATE TABLE steam_rank_history(user_id INTEGER NOT NULL, captured_at TEXT NOT NULL, rank_name TEXT);
            INSERT INTO steam_rank_history(user_id, captured_at, rank_name)
            VALUES
                (7, '2026-07-01T00:00:00Z', 'Changed'),
                (7, '2026-07-02T00:00:00Z', 'Fresh');
            "#,
        );
        let original_rows = source_hashes(&original, &plan, "original").expect("original hashes");
        let current_rows = source_hashes(&current, &plan, "source").expect("current hashes");
        let target_rows = TargetHashSet::from_rows(
            1,
            BTreeMap::from([(
                "user_id=7,captured_at=2026-07-01T00:00:00.000000000Z".to_string(),
                original_rows
                    .rows
                    .get("user_id=7,captured_at=2026-07-01T00:00:00.000000000Z")
                    .expect("old history row")
                    .clone(),
            )]),
        );

        let candidates = row_candidate_reports(
            &plan,
            TableClassification::AppendOnly,
            &original_rows,
            &current_rows,
            &target_rows,
        );

        assert_eq!(
            candidate_for_pk(
                &candidates,
                "user_id=7,captured_at=2026-07-01T00:00:00.000000000Z"
            )
            .decision,
            MergeDecision::Conflict
        );
        assert_eq!(
            candidate_for_pk(
                &candidates,
                "user_id=7,captured_at=2026-07-02T00:00:00.000000000Z"
            )
            .decision,
            MergeDecision::InsertAllowed
        );
    }

    #[test]
    fn canonical_hash_normalizes_json_object_key_order() {
        let left = TargetRow {
            values: BTreeMap::from([(
                "payload".to_string(),
                TargetValue::Jsonb(Some(serde_json::json!({"b": 2, "a": {"z": 1, "m": 0}}))),
            )]),
        };
        let right = TargetRow {
            values: BTreeMap::from([(
                "payload".to_string(),
                TargetValue::Jsonb(Some(serde_json::json!({"a": {"m": 0, "z": 1}, "b": 2}))),
            )]),
        };
        let columns = vec![ColumnPlan {
            source_column: "payload".to_string(),
            target_column: "payload".to_string(),
            source_affinity: "TEXT".to_string(),
            target_pg_type: "jsonb".to_string(),
            converter: Converter::TextToJsonb,
            generated: None,
        }];

        let left_hash = canonical_target_row_hash(&left, &columns).expect("left hash");
        let right_hash = canonical_target_row_hash(&right, &columns).expect("right hash");

        assert_eq!(left_hash, right_hash);
    }

    fn sqlite_source(sql: &str) -> (NamedTempFile, SourceSqlite) {
        let file = NamedTempFile::new().expect("create temp sqlite file");
        {
            let conn = Connection::open(file.path()).expect("open temp sqlite");
            conn.execute_batch(sql).expect("seed sqlite");
        }
        let source = SourceSqlite::open_read_only(file.path()).expect("open read-only source");
        (file, source)
    }

    fn demo_items_plan() -> TablePlan {
        TablePlan {
            source_db: "test".to_string(),
            source_table: "items".to_string(),
            target: "test.items".to_string(),
            target_schema: "test".to_string(),
            target_table: "items".to_string(),
            columns: vec![
                column("id", "id", "INTEGER", "int8", Converter::IntegerToInt8),
                column("body", "body", "TEXT", "text", Converter::TextToText),
            ],
            primary_key: vec!["id".to_string()],
            dropped_cols: Vec::new(),
        }
    }

    fn patchnotes_plan() -> TablePlan {
        TablePlan {
            source_db: "deadlock-sqlite3".to_string(),
            source_table: "changelog_posts".to_string(),
            target: "patchnotes.changelog_posts".to_string(),
            target_schema: "patchnotes".to_string(),
            target_table: "changelog_posts".to_string(),
            columns: vec![
                column("id", "id", "INTEGER", "int8", Converter::IntegerToInt8),
                column("url", "url", "TEXT", "text", Converter::TextToText),
            ],
            primary_key: vec!["id".to_string()],
            dropped_cols: Vec::new(),
        }
    }

    fn kv_store_plan() -> TablePlan {
        TablePlan {
            source_db: "deadlock-sqlite3".to_string(),
            source_table: "kv_store".to_string(),
            target: "bot.kv_store".to_string(),
            target_schema: "bot".to_string(),
            target_table: "kv_store".to_string(),
            columns: vec![
                column("ns", "ns", "TEXT", "text", Converter::TextToText),
                column("k", "k", "TEXT", "text", Converter::TextToText),
                column("v", "v", "TEXT", "text", Converter::TextToText),
            ],
            primary_key: vec!["ns".to_string(), "k".to_string()],
            dropped_cols: Vec::new(),
        }
    }

    fn steam_tasks_plan() -> TablePlan {
        TablePlan {
            source_db: "deadlock-sqlite3".to_string(),
            source_table: "steam_tasks".to_string(),
            target: "steam.steam_tasks".to_string(),
            target_schema: "steam".to_string(),
            target_table: "steam_tasks".to_string(),
            columns: vec![
                column("id", "id", "INTEGER", "int8", Converter::IntegerToInt8),
                column("type", "type", "TEXT", "text", Converter::TextToText),
                column("status", "status", "TEXT", "text", Converter::TextToText),
            ],
            primary_key: vec!["id".to_string()],
            dropped_cols: Vec::new(),
        }
    }

    fn meta_users_plan() -> TablePlan {
        TablePlan {
            source_db: "website".to_string(),
            source_table: "meta_users".to_string(),
            target: "core.meta_users".to_string(),
            target_schema: "core".to_string(),
            target_table: "meta_users".to_string(),
            columns: vec![
                column("id", "id", "INTEGER", "int8", Converter::IntegerToInt8),
                column(
                    "username",
                    "username",
                    "TEXT",
                    "text",
                    Converter::TextToText,
                ),
            ],
            primary_key: vec!["id".to_string()],
            dropped_cols: Vec::new(),
        }
    }

    fn steam_rank_history_plan() -> TablePlan {
        TablePlan {
            source_db: "deadlock-sqlite3".to_string(),
            source_table: "steam_rank_history".to_string(),
            target: "steam.steam_rank_history".to_string(),
            target_schema: "steam".to_string(),
            target_table: "steam_rank_history".to_string(),
            columns: vec![
                column(
                    "user_id",
                    "user_id",
                    "INTEGER",
                    "int8",
                    Converter::IntegerToInt8,
                ),
                column(
                    "captured_at",
                    "captured_at",
                    "TEXT",
                    "timestamptz",
                    Converter::TextToTimestamptz,
                ),
                column(
                    "rank_name",
                    "rank_name",
                    "TEXT",
                    "text",
                    Converter::TextToText,
                ),
            ],
            primary_key: Vec::new(),
            dropped_cols: Vec::new(),
        }
    }

    fn column(
        source_column: &str,
        target_column: &str,
        source_affinity: &str,
        target_pg_type: &str,
        converter: Converter,
    ) -> ColumnPlan {
        ColumnPlan {
            source_column: source_column.to_string(),
            target_column: target_column.to_string(),
            source_affinity: source_affinity.to_string(),
            target_pg_type: target_pg_type.to_string(),
            converter,
            generated: None,
        }
    }

    fn target_hash_set(rows: Vec<TargetRow>, plan: &TablePlan) -> TargetHashSet {
        let rows_by_pk = rows
            .into_iter()
            .map(|row| {
                let pk = target_primary_key(&row, plan).expect("target pk");
                let hash = canonical_target_row_hash(&row, &plan.columns).expect("target hash");
                let fingerprint = RowFingerprint {
                    hash,
                    url: row_url(&row),
                };
                (pk, fingerprint)
            })
            .collect::<BTreeMap<_, _>>();
        TargetHashSet::from_rows(rows_by_pk.len() as u64, rows_by_pk)
    }

    fn candidate_for_pk<'a>(
        candidates: &'a [RowCandidateReport],
        pk: &str,
    ) -> &'a RowCandidateReport {
        candidates
            .iter()
            .find(|candidate| candidate.target_pk == pk)
            .expect("candidate pk")
    }
}
