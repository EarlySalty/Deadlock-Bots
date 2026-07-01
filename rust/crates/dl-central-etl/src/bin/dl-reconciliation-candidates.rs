use std::{collections::BTreeMap, ffi::OsString, fs, path::PathBuf, time::SystemTime};

use dl_central_etl::{
    build_candidate_report, CandidateOptions, DEADLOCK_SQLITE3_SOURCE, WEBSITE_SOURCE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    manifest_path: PathBuf,
    ledger_dir: PathBuf,
    original_snapshot_dir: Option<PathBuf>,
    deadlock_sqlite3: PathBuf,
    website: PathBuf,
    output: Option<PathBuf>,
    pretty: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args(std::env::args_os().skip(1))?;
    let dsn = dl_central_db::dsn_from_env()?;
    let pool = dl_central_db::connect_pool(&dsn).await?;
    drop(dsn);

    let report = build_candidate_report(
        &CandidateOptions {
            manifest_path: args.manifest_path,
            ledger_dir: args.ledger_dir,
            original_snapshot_dir: args.original_snapshot_dir,
            current_sources: BTreeMap::from([
                ("deadlock-sqlite3".to_string(), args.deadlock_sqlite3),
                ("website".to_string(), args.website),
            ]),
            generated_at: SystemTime::now(),
        },
        &pool,
    )
    .await?;

    let json = if args.pretty {
        serde_json::to_string_pretty(&report)?
    } else {
        serde_json::to_string(&report)?
    };

    if let Some(path) = args.output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{json}\n"))?;
    } else {
        println!("{json}");
    }

    Ok(())
}

fn parse_args<I>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut parsed = Args {
        manifest_path: default_manifest_path(),
        ledger_dir: default_ledger_dir(),
        original_snapshot_dir: None,
        deadlock_sqlite3: PathBuf::from(DEADLOCK_SQLITE3_SOURCE),
        website: PathBuf::from(WEBSITE_SOURCE),
        output: None,
        pretty: false,
    };

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--manifest" => parsed.manifest_path = next_path(&mut args, "--manifest")?,
            "--ledger-dir" => parsed.ledger_dir = next_path(&mut args, "--ledger-dir")?,
            "--original-snapshot-dir" => {
                parsed.original_snapshot_dir =
                    Some(next_path(&mut args, "--original-snapshot-dir")?);
            }
            "--deadlock-sqlite3" => {
                parsed.deadlock_sqlite3 = next_path(&mut args, "--deadlock-sqlite3")?;
            }
            "--website" => parsed.website = next_path(&mut args, "--website")?,
            "--output" => parsed.output = Some(next_path(&mut args, "--output")?),
            "--pretty" => parsed.pretty = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unbekanntes Argument: {other}")),
        }
    }

    Ok(parsed)
}

fn next_path<I>(args: &mut I, flag: &str) -> Result<PathBuf, String>
where
    I: Iterator<Item = OsString>,
{
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} braucht einen Pfad"))
}

fn default_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/_work/sp1/reconciliation/2026-07-01-t0-baseline-manifest.json")
}

fn default_ledger_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ledger")
}

fn print_help() {
    println!(
        "Usage: dl-reconciliation-candidates [--manifest PATH] [--ledger-dir PATH] [--original-snapshot-dir PATH] [--deadlock-sqlite3 PATH] [--website PATH] [--output PATH] [--pretty]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults() {
        let args = parse_args(Vec::<OsString>::new()).expect("parse defaults");

        assert!(args
            .manifest_path
            .to_string_lossy()
            .contains("2026-07-01-t0"));
        assert!(args.ledger_dir.ends_with("ledger"));
        assert_eq!(
            args.deadlock_sqlite3,
            PathBuf::from(DEADLOCK_SQLITE3_SOURCE)
        );
        assert_eq!(args.website, PathBuf::from(WEBSITE_SOURCE));
        assert!(args.original_snapshot_dir.is_none());
        assert!(args.output.is_none());
        assert!(!args.pretty);
    }

    #[test]
    fn parse_custom_args() {
        let args = parse_args([
            OsString::from("--manifest"),
            OsString::from("/tmp/manifest.json"),
            OsString::from("--ledger-dir"),
            OsString::from("/tmp/ledger"),
            OsString::from("--original-snapshot-dir"),
            OsString::from("/tmp/original"),
            OsString::from("--deadlock-sqlite3"),
            OsString::from("/tmp/deadlock.sqlite3"),
            OsString::from("--website"),
            OsString::from("/tmp/website.db"),
            OsString::from("--output"),
            OsString::from("/tmp/report.json"),
            OsString::from("--pretty"),
        ])
        .expect("parse custom args");

        assert_eq!(args.manifest_path, PathBuf::from("/tmp/manifest.json"));
        assert_eq!(args.ledger_dir, PathBuf::from("/tmp/ledger"));
        assert_eq!(
            args.original_snapshot_dir,
            Some(PathBuf::from("/tmp/original"))
        );
        assert_eq!(
            args.deadlock_sqlite3,
            PathBuf::from("/tmp/deadlock.sqlite3")
        );
        assert_eq!(args.website, PathBuf::from("/tmp/website.db"));
        assert_eq!(args.output, Some(PathBuf::from("/tmp/report.json")));
        assert!(args.pretty);
    }

    #[test]
    fn parse_rejects_unknown_arg() {
        let err = parse_args([OsString::from("--bad")]).expect_err("unknown arg");

        assert!(err.contains("unbekanntes Argument"));
    }
}
