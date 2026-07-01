use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use dl_central_etl::{reconcile_total, run, snapshot_known_sources, LedgerSet};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    ledger_dir: PathBuf,
    snapshot_dir: PathBuf,
    reconcile: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args(std::env::args_os().skip(1))?;
    if args.snapshot_dir == args.ledger_dir {
        bail!("snapshot-dir darf nicht ledger-dir sein");
    }

    eprintln!(
        "dl-central-sync: snapshot-dir={}",
        args.snapshot_dir.display()
    );
    let snapshots = snapshot_known_sources(&args.snapshot_dir)
        .context("SQLite-Quellen als konsistente Snapshots sichern")?;
    for (source, path) in snapshots {
        eprintln!("dl-central-sync: snapshot {source} -> {}", path.display());
    }

    let ledger_set = LedgerSet::from_dir(&args.ledger_dir).context("ETL-Ledger laden")?;
    let dsn = dl_central_db::dsn_from_env().context("DEADLOCK_CENTRAL_DSN laden")?;
    let pool = dl_central_db::connect_pool(&dsn)
        .await
        .context("zentrale DB verbinden")?;

    let report = run(&ledger_set, &args.snapshot_dir, &pool)
        .await
        .context("ETL in zentrale DB ausfuehren")?;
    if args.reconcile {
        reconcile_total(&pool, &report.tables)
            .await
            .context("Zieltabellen gegen Quellsummen reconciliieren")?;
    }

    eprintln!(
        "dl-central-sync: ok tables={} source_rows={} loaded_rows={} orphans={}",
        report.tables.len(),
        report.total_source_rows,
        report.total_loaded,
        report.orphan_reports.len()
    );
    for (target, (source_rows, loaded_rows)) in aggregate_targets(&report) {
        eprintln!(
            "dl-central-sync: target {target} source_rows={source_rows} loaded_rows={loaded_rows}"
        );
    }
    if !report.orphan_reports.is_empty() {
        let mut by_table = BTreeMap::<String, u64>::new();
        for orphan in &report.orphan_reports {
            *by_table.entry(orphan.table.clone()).or_default() += orphan.count;
        }
        for (table, count) in by_table {
            eprintln!("dl-central-sync: orphan_report {table} count={count}");
        }
    }

    Ok(())
}

fn aggregate_targets(report: &dl_central_etl::EtlReport) -> BTreeMap<String, (u64, u64)> {
    let mut targets = BTreeMap::<String, (u64, u64)>::new();
    for table in &report.tables {
        let entry = targets.entry(table.target.clone()).or_default();
        entry.0 += table.source_rows;
        entry.1 += table.loaded_rows;
    }
    targets
}

fn parse_args<I>(args: I) -> anyhow::Result<Args>
where
    I: IntoIterator<Item = OsString>,
{
    let mut parsed = Args {
        ledger_dir: default_ledger_dir(),
        snapshot_dir: default_snapshot_dir(),
        reconcile: true,
    };

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--ledger-dir" => {
                parsed.ledger_dir = next_path(&mut args, "--ledger-dir")?;
            }
            "--snapshot-dir" => {
                parsed.snapshot_dir = next_path(&mut args, "--snapshot-dir")?;
            }
            "--no-reconcile" => parsed.reconcile = false,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => bail!("unbekanntes Argument: {other}"),
        }
    }

    Ok(parsed)
}

fn next_path<I>(args: &mut I, flag: &str) -> anyhow::Result<PathBuf>
where
    I: Iterator<Item = OsString>,
{
    args.next()
        .map(PathBuf::from)
        .with_context(|| format!("{flag} braucht einen Pfad"))
}

fn default_ledger_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/dl-central-etl/ledger")
}

fn default_snapshot_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/central-etl-snapshots")
        .join(format!("sync-{ts}"))
}

fn print_help() {
    println!("Usage: dl-central-sync [--ledger-dir PATH] [--snapshot-dir PATH] [--no-reconcile]");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_enable_reconcile() {
        let args = parse_args(Vec::<OsString>::new()).expect("parse defaults");
        assert!(args.reconcile);
        assert!(args.ledger_dir.ends_with("crates/dl-central-etl/ledger"));
        assert!(args
            .snapshot_dir
            .to_string_lossy()
            .contains("central-etl-snapshots"));
    }

    #[test]
    fn parse_custom_paths_and_no_reconcile() {
        let args = parse_args([
            OsString::from("--ledger-dir"),
            OsString::from("/tmp/ledger"),
            OsString::from("--snapshot-dir"),
            OsString::from("/tmp/snap"),
            OsString::from("--no-reconcile"),
        ])
        .expect("parse custom args");
        assert_eq!(args.ledger_dir, PathBuf::from("/tmp/ledger"));
        assert_eq!(args.snapshot_dir, PathBuf::from("/tmp/snap"));
        assert!(!args.reconcile);
    }

    #[test]
    fn parse_rejects_unknown_arg() {
        let err = parse_args([OsString::from("--bad")]).expect_err("unknown arg fails");
        assert!(err.to_string().contains("unbekanntes Argument"));
    }
}
