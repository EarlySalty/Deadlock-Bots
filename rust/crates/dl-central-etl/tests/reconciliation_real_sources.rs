use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
    time::SystemTime,
};

use dl_central_etl::{
    reconciliation_manifest::{build_reconciliation_manifest, ManifestOptions},
    run, run_reconciliation, snapshot_db, source_snapshot_path, CandidateOptions, LedgerSet,
    ReconciliationMode, ReconciliationOptions, DEADLOCK_SQLITE3_SOURCE, WEBSITE_SOURCE,
};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tempfile::TempDir;

const LEDGER_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ledger");

#[tokio::test]
#[ignore]
async fn final_reconciliation_dry_run_real_copies_is_read_only() {
    let p4_final = p4_final_snapshot_dir();
    if !Path::new(DEADLOCK_SQLITE3_SOURCE).exists()
        || !Path::new(WEBSITE_SOURCE).exists()
        || !p4_final.exists()
    {
        println!("SKIP reconciliation real sources: Quellen oder p4-final Snapshot fehlen");
        return;
    }

    let pool = test_pool().await;
    let ledger_set = LedgerSet::from_dir(LEDGER_ROOT).expect("load complete ETL ledger set");
    let targets = mapped_targets(&ledger_set);
    clean_targets(&pool, &targets)
        .await
        .expect("clean mapped targets before baseline ETL");

    run(&ledger_set, &p4_final, &pool)
        .await
        .expect("load p4-final baseline into throwaway Postgres");
    let counts_before = target_counts(&pool, &targets).await;

    let current_dir = TempDir::new().expect("create current source copy dir");
    let current_deadlock = source_snapshot_path(current_dir.path(), "deadlock-sqlite3");
    let current_website = source_snapshot_path(current_dir.path(), "website");
    snapshot_db(Path::new(DEADLOCK_SQLITE3_SOURCE), &current_deadlock)
        .expect("copy current shared deadlock.sqlite3 read-only");
    snapshot_db(Path::new(WEBSITE_SOURCE), &current_website)
        .expect("copy current website deadlock.db read-only");

    let manifest_dir = TempDir::new().expect("create reconciliation manifest dir");
    let manifest = build_reconciliation_manifest(&ManifestOptions {
        ledger_dir: PathBuf::from(LEDGER_ROOT),
        snapshot_root: p4_final
            .parent()
            .expect("p4-final snapshot has parent")
            .to_path_buf(),
        snapshot_dir: Some(p4_final.clone()),
        generated_at: SystemTime::now(),
    })
    .expect("build T0 reconciliation manifest");
    assert_eq!(
        manifest.cutoff.source, "explicit_snapshot_dir",
        "T0 manifest uses explicit p4-final baseline"
    );
    let manifest_path = manifest_dir.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write temp manifest");

    let report = run_reconciliation(
        &ReconciliationOptions {
            candidate_options: CandidateOptions {
                manifest_path,
                ledger_dir: PathBuf::from(LEDGER_ROOT),
                original_snapshot_dir: Some(p4_final),
                current_sources: BTreeMap::from([
                    ("deadlock-sqlite3".to_string(), current_deadlock),
                    ("website".to_string(), current_website),
                ]),
                generated_at: SystemTime::now(),
            },
            mode: ReconciliationMode::DryRun,
            abort_table_on_conflict: true,
        },
        &pool,
    )
    .await
    .expect("dry-run reconciliation against real source copies");

    assert_eq!(report.mode, "dry_run_read_only");
    assert_eq!(report.totals.applied, 0);
    assert_eq!(counts_before, target_counts(&pool, &targets).await);
    assert_audit_totals_match_tables(&report);
    assert!(
        report
            .source_files
            .iter()
            .any(|source| source.source_db == "deadlock-sqlite3"),
        "shared Steam/Patchnotes SQLite source is covered"
    );
    assert!(
        report
            .source_files
            .iter()
            .any(|source| source.source_db == "website"),
        "website SQLite source is covered"
    );
}

async fn test_pool() -> PgPool {
    let dsn = env::var("CENTRAL_TEST_DSN").expect("CENTRAL_TEST_DSN is set by central_test_db.sh");
    assert!(
        !dsn.contains("127.0.0.1:5434") && !dsn.contains("localhost:5434"),
        "refuse to run reconciliation tests against the production central DB port"
    );
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&dsn)
        .await
        .expect("connect central test db")
}

fn p4_final_snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../data/central-etl-snapshots/p4-final-20260701-032848")
}

fn mapped_targets(ledger_set: &LedgerSet) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for ledger in ledger_set.sources.values() {
        for table_ledger in ledger.tables.values() {
            for status in table_ledger.columns.values() {
                let dl_central_etl::ColumnStatus::Mapped { to } = status else {
                    continue;
                };
                let Some((target, _column)) = to.rsplit_once('.') else {
                    continue;
                };
                targets.insert(target.to_string());
            }
        }
    }
    targets
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

async fn target_counts(pool: &PgPool, targets: &BTreeSet<String>) -> BTreeMap<String, i64> {
    let mut counts = BTreeMap::new();
    for target in targets {
        let sql = format!(
            "SELECT COUNT(*)::bigint AS count FROM {}",
            quote_pg_path(target)
        );
        let row = sqlx::query(&sql)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|err| panic!("count {target}: {err}"));
        counts.insert(target.clone(), row.get::<i64, _>("count"));
    }
    counts
}

fn assert_audit_totals_match_tables(report: &dl_central_etl::ReconciliationAuditReport) {
    let mut totals = dl_central_etl::AuditCounters::default();
    for table in &report.tables {
        totals.applied += table.counters.applied;
        totals.noop += table.counters.noop;
        totals.conflict += table.counters.conflict;
        totals.skipped += table.counters.skipped;
    }

    assert_eq!(report.totals, totals);
}

fn quote_pg_path(path: &str) -> String {
    path.split('.')
        .map(|identifier| format!("\"{}\"", identifier.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}
