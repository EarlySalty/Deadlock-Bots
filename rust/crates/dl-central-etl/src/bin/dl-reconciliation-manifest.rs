use std::{ffi::OsString, fs, path::PathBuf, time::SystemTime};

use dl_central_etl::reconciliation_manifest::{build_reconciliation_manifest, ManifestOptions};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    ledger_dir: PathBuf,
    snapshot_root: PathBuf,
    snapshot_dir: Option<PathBuf>,
    output: Option<PathBuf>,
    pretty: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args(std::env::args_os().skip(1))?;
    let manifest = build_reconciliation_manifest(&ManifestOptions {
        ledger_dir: args.ledger_dir,
        snapshot_root: args.snapshot_root,
        snapshot_dir: args.snapshot_dir,
        generated_at: SystemTime::now(),
    })?;

    let json = if args.pretty {
        serde_json::to_string_pretty(&manifest)?
    } else {
        serde_json::to_string(&manifest)?
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
        ledger_dir: default_ledger_dir(),
        snapshot_root: default_snapshot_root(),
        snapshot_dir: None,
        output: None,
        pretty: false,
    };

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--ledger-dir" => parsed.ledger_dir = next_path(&mut args, "--ledger-dir")?,
            "--snapshot-root" => parsed.snapshot_root = next_path(&mut args, "--snapshot-root")?,
            "--snapshot-dir" => parsed.snapshot_dir = Some(next_path(&mut args, "--snapshot-dir")?),
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

fn default_ledger_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ledger")
}

fn default_snapshot_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/central-etl-snapshots")
}

fn print_help() {
    println!(
        "Usage: dl-reconciliation-manifest [--ledger-dir PATH] [--snapshot-root PATH] [--snapshot-dir PATH] [--output PATH] [--pretty]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults() {
        let args = parse_args(Vec::<OsString>::new()).expect("parse defaults");
        assert!(args.ledger_dir.ends_with("ledger"));
        assert!(args
            .snapshot_root
            .to_string_lossy()
            .contains("central-etl-snapshots"));
        assert!(args.snapshot_dir.is_none());
        assert!(args.output.is_none());
        assert!(!args.pretty);
    }

    #[test]
    fn parse_custom_args() {
        let args = parse_args([
            OsString::from("--ledger-dir"),
            OsString::from("/tmp/ledger"),
            OsString::from("--snapshot-root"),
            OsString::from("/tmp/root"),
            OsString::from("--snapshot-dir"),
            OsString::from("/tmp/root/p4-final"),
            OsString::from("--output"),
            OsString::from("/tmp/manifest.json"),
            OsString::from("--pretty"),
        ])
        .expect("parse custom args");

        assert_eq!(args.ledger_dir, PathBuf::from("/tmp/ledger"));
        assert_eq!(args.snapshot_root, PathBuf::from("/tmp/root"));
        assert_eq!(args.snapshot_dir, Some(PathBuf::from("/tmp/root/p4-final")));
        assert_eq!(args.output, Some(PathBuf::from("/tmp/manifest.json")));
        assert!(args.pretty);
    }

    #[test]
    fn parse_rejects_unknown_arg() {
        let err = parse_args([OsString::from("--bad")]).expect_err("unknown arg");
        assert!(err.contains("unbekanntes Argument"));
    }
}
