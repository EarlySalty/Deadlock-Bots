use std::{collections::BTreeMap, ffi::OsString, fs, net::IpAddr, path::PathBuf, time::SystemTime};

use dl_central_etl::{
    reconciliation_manifest::{build_reconciliation_manifest, ManifestOptions},
    run_reconciliation, CandidateOptions, ReconciliationMode, ReconciliationOptions,
    DEADLOCK_SQLITE3_SOURCE, WEBSITE_SOURCE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Manifest(ManifestArgs),
    DryRun(ReconcileArgs),
    Apply(ReconcileArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestArgs {
    ledger_dir: PathBuf,
    snapshot_root: PathBuf,
    snapshot_dir: Option<PathBuf>,
    output: Option<PathBuf>,
    pretty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReconcileArgs {
    manifest_path: PathBuf,
    ledger_dir: PathBuf,
    original_snapshot_dir: Option<PathBuf>,
    deadlock_sqlite3: PathBuf,
    website: PathBuf,
    output: Option<PathBuf>,
    pretty: bool,
    abort_table_on_conflict: bool,
    allow_central_prod_port: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconcileCommandKind {
    DryRun,
    Apply,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match parse_args(std::env::args_os().skip(1))? {
        Command::Manifest(args) => {
            let manifest = build_reconciliation_manifest(&ManifestOptions {
                ledger_dir: args.ledger_dir,
                snapshot_root: args.snapshot_root,
                snapshot_dir: args.snapshot_dir,
                generated_at: SystemTime::now(),
            })?;
            write_json(&manifest, args.output, args.pretty)?;
        }
        Command::DryRun(args) => {
            let dsn = dl_central_db::dsn_from_env()?;
            let pool = dl_central_db::connect_pool(&dsn).await?;
            drop(dsn);
            let report = run_reconciliation(
                &ReconciliationOptions {
                    candidate_options: candidate_options(&args),
                    mode: ReconciliationMode::DryRun,
                    abort_table_on_conflict: args.abort_table_on_conflict,
                },
                &pool,
            )
            .await?;
            write_json(&report, args.output, args.pretty)?;
        }
        Command::Apply(args) => {
            let dsn = dl_central_db::dsn_from_env()?;
            refuse_central_prod_apply(&dsn, args.allow_central_prod_port)?;
            let pool = dl_central_db::connect_pool(&dsn).await?;
            drop(dsn);
            let report = run_reconciliation(
                &ReconciliationOptions {
                    candidate_options: candidate_options(&args),
                    mode: ReconciliationMode::Apply,
                    abort_table_on_conflict: args.abort_table_on_conflict,
                },
                &pool,
            )
            .await?;
            write_json(&report, args.output, args.pretty)?;
        }
    }

    Ok(())
}

fn candidate_options(args: &ReconcileArgs) -> CandidateOptions {
    CandidateOptions {
        manifest_path: args.manifest_path.clone(),
        ledger_dir: args.ledger_dir.clone(),
        original_snapshot_dir: args.original_snapshot_dir.clone(),
        current_sources: BTreeMap::from([
            (
                "deadlock-sqlite3".to_string(),
                args.deadlock_sqlite3.clone(),
            ),
            ("website".to_string(), args.website.clone()),
        ]),
        generated_at: SystemTime::now(),
    }
}

fn write_json<T: serde::Serialize>(
    value: &T,
    output: Option<PathBuf>,
    pretty: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = if pretty {
        serde_json::to_string_pretty(value)?
    } else {
        serde_json::to_string(value)?
    };

    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{json}\n"))?;
    } else {
        println!("{json}");
    }

    Ok(())
}

fn refuse_central_prod_apply(dsn: &str, allow: bool) -> Result<(), String> {
    let endpoint = parse_postgres_endpoint(dsn)?;
    if endpoint.must_refuse_central_prod_port() && !allow {
        return Err(
            "apply verweigert auf zentralem Produktions-Port; fuer Wartungsfenster explizit --allow-central-prod-port setzen"
                .to_string(),
        );
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PostgresEndpoint {
    host: Option<String>,
    port: u16,
}

impl PostgresEndpoint {
    fn must_refuse_central_prod_port(&self) -> bool {
        if self.port != 5434 {
            return false;
        }

        match self.host.as_deref().map(classify_host) {
            Some(HostClassification::NonLoopbackIp) => false,
            Some(
                HostClassification::Localhost
                | HostClassification::AmbiguousName
                | HostClassification::Socket,
            )
            | None => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostClassification {
    Localhost,
    NonLoopbackIp,
    AmbiguousName,
    Socket,
}

fn parse_postgres_endpoint(dsn: &str) -> Result<PostgresEndpoint, String> {
    let url = parse_postgres_url(dsn)?;
    if !matches!(url.scheme(), "postgres" | "postgresql") {
        return Err("apply verweigert: DEADLOCK_CENTRAL_DSN ist keine Postgres-URL".to_string());
    }

    let mut host = url.host_str().map(ToOwned::to_owned);
    let mut port = url.port();

    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "host" => host = Some(value.into_owned()),
            "hostaddr" => {
                value.parse::<IpAddr>().map_err(|_| {
                    "apply verweigert: DEADLOCK_CENTRAL_DSN hostaddr ist ungueltig".to_string()
                })?;
                host = Some(value.into_owned());
            }
            "port" => {
                port = Some(value.parse::<u16>().map_err(|_| {
                    "apply verweigert: DEADLOCK_CENTRAL_DSN port ist ungueltig".to_string()
                })?);
            }
            _ => {}
        }
    }

    let port = match port {
        Some(port) => port,
        None => std::env::var("PGPORT")
            .ok()
            .map(|value| {
                value.parse::<u16>().map_err(|_| {
                    "apply verweigert: PGPORT aus der Umgebung ist ungueltig".to_string()
                })
            })
            .transpose()?
            .unwrap_or(5432),
    };

    if host.is_none() {
        host = std::env::var("PGHOSTADDR")
            .ok()
            .or_else(|| std::env::var("PGHOST").ok());
    }

    Ok(PostgresEndpoint { host, port })
}

fn parse_postgres_url(dsn: &str) -> Result<url::Url, String> {
    url::Url::parse(dsn)
        .or_else(|_| {
            normalize_empty_host_userinfo_url(dsn)
                .map(|normalized| url::Url::parse(&normalized))
                .transpose()
                .and_then(|parsed| parsed.ok_or(url::ParseError::EmptyHost))
        })
        .map_err(|_| {
            "apply verweigert: DEADLOCK_CENTRAL_DSN konnte nicht eindeutig geparst werden"
                .to_string()
        })
}

fn normalize_empty_host_userinfo_url(dsn: &str) -> Option<String> {
    let scheme_end = dsn
        .strip_prefix("postgres://")
        .map(|_| "postgres://".len())
        .or_else(|| {
            dsn.strip_prefix("postgresql://")
                .map(|_| "postgresql://".len())
        })?;
    let after_scheme = &dsn[scheme_end..];
    let authority_end = after_scheme.find(['?', '#']).unwrap_or(after_scheme.len());
    let authority_and_path = &after_scheme[..authority_end];
    let at_slash = authority_and_path.find("@/")?;

    if at_slash == 0 {
        return None;
    }

    Some(format!(
        "{}{}",
        &dsn[..scheme_end],
        &after_scheme[at_slash + 1..]
    ))
}

fn classify_host(host: &str) -> HostClassification {
    if host.starts_with('/') {
        return HostClassification::Socket;
    }

    let normalized = host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();

    if normalized == "localhost" {
        return HostClassification::Localhost;
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return if is_loopback_ip(ip) {
            HostClassification::Localhost
        } else {
            HostClassification::NonLoopbackIp
        };
    }

    HostClassification::AmbiguousName
}

fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => {
            ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback())
        }
    }
}

fn parse_args<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        print_help();
        return Err("fehlender Subcommand".to_string());
    };

    match command.to_string_lossy().as_ref() {
        "manifest" => parse_manifest_args(args).map(Command::Manifest),
        "dry-run" => parse_reconcile_args(args, ReconcileCommandKind::DryRun).map(Command::DryRun),
        "apply" => parse_reconcile_args(args, ReconcileCommandKind::Apply).map(Command::Apply),
        "--help" | "-h" => {
            print_help();
            std::process::exit(0);
        }
        other => Err(format!("unbekannter Subcommand: {other}")),
    }
}

fn parse_manifest_args<I>(args: I) -> Result<ManifestArgs, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut parsed = ManifestArgs {
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
            other => return Err(format!("unbekanntes manifest-Argument: {other}")),
        }
    }

    Ok(parsed)
}

fn parse_reconcile_args<I>(args: I, kind: ReconcileCommandKind) -> Result<ReconcileArgs, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut parsed = ReconcileArgs {
        manifest_path: default_manifest_path(),
        ledger_dir: default_ledger_dir(),
        original_snapshot_dir: None,
        deadlock_sqlite3: PathBuf::from(DEADLOCK_SQLITE3_SOURCE),
        website: PathBuf::from(WEBSITE_SOURCE),
        output: None,
        pretty: false,
        abort_table_on_conflict: true,
        allow_central_prod_port: false,
    };
    let mut deadlock_sqlite3_explicit = false;
    let mut website_explicit = false;
    let mut continue_table_on_conflict = false;

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
                deadlock_sqlite3_explicit = true;
            }
            "--website" => {
                parsed.website = next_path(&mut args, "--website")?;
                website_explicit = true;
            }
            "--output" => parsed.output = Some(next_path(&mut args, "--output")?),
            "--pretty" => parsed.pretty = true,
            "--continue-table-on-conflict" => {
                parsed.abort_table_on_conflict = false;
                continue_table_on_conflict = true;
            }
            "--allow-central-prod-port" => parsed.allow_central_prod_port = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unbekanntes Reconcile-Argument: {other}")),
        }
    }

    if kind == ReconcileCommandKind::Apply {
        if continue_table_on_conflict {
            return Err(
                "--continue-table-on-conflict darf beim apply nicht verwendet werden: Konflikt = Abbruch fuer die betroffene Domain, keine stille Fortsetzung"
                    .to_string(),
            );
        }

        let mut missing = Vec::new();
        if !deadlock_sqlite3_explicit {
            missing.push("deadlock-sqlite3");
        }
        if !website_explicit {
            missing.push("website");
        }
        if !missing.is_empty() {
            return Err(format!(
                "Snapshot-Pfad fuer {} ist beim Apply Pflicht, kein impliziter Live-Pfad-Default erlaubt",
                missing.join(" und ")
            ));
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

fn default_snapshot_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/central-etl-snapshots")
}

fn print_help() {
    println!(
        "Usage: dl-reconciliation <manifest|dry-run|apply> [--manifest PATH] [--ledger-dir PATH] [--original-snapshot-dir PATH] [--deadlock-sqlite3 PATH] [--website PATH] [--snapshot-root PATH] [--snapshot-dir PATH] [--output PATH] [--pretty] [--continue-table-on-conflict] [--allow-central-prod-port]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dry_run_defaults() {
        let args = parse_args([OsString::from("dry-run")]).expect("parse dry-run");

        let Command::DryRun(args) = args else {
            panic!("expected dry-run");
        };
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
        assert!(args.abort_table_on_conflict);
        assert!(!args.allow_central_prod_port);
    }

    #[test]
    fn parse_apply_allows_explicit_prod_port_override() {
        let args = parse_args([
            OsString::from("apply"),
            OsString::from("--manifest"),
            OsString::from("/tmp/manifest.json"),
            OsString::from("--deadlock-sqlite3"),
            OsString::from("/tmp/deadlock.sqlite3"),
            OsString::from("--website"),
            OsString::from("/tmp/website.sqlite3"),
            OsString::from("--allow-central-prod-port"),
        ])
        .expect("parse apply");

        let Command::Apply(args) = args else {
            panic!("expected apply");
        };
        assert_eq!(args.manifest_path, PathBuf::from("/tmp/manifest.json"));
        assert_eq!(
            args.deadlock_sqlite3,
            PathBuf::from("/tmp/deadlock.sqlite3")
        );
        assert_eq!(args.website, PathBuf::from("/tmp/website.sqlite3"));
        assert!(args.abort_table_on_conflict);
        assert!(args.allow_central_prod_port);
    }

    #[test]
    fn parse_apply_rejects_continue_table_on_conflict() {
        let err = parse_args([
            OsString::from("apply"),
            OsString::from("--deadlock-sqlite3"),
            OsString::from("/tmp/deadlock.sqlite3"),
            OsString::from("--website"),
            OsString::from("/tmp/website.sqlite3"),
            OsString::from("--continue-table-on-conflict"),
        ])
        .expect_err("apply must reject continue-table-on-conflict");

        assert!(err.contains("--continue-table-on-conflict"));
        assert!(err.contains("Konflikt = Abbruch"));
    }

    #[test]
    fn parse_apply_requires_explicit_snapshot_paths() {
        let err = parse_args([OsString::from("apply")])
            .expect_err("apply must require explicit snapshot paths");

        assert!(err.contains("Snapshot-Pfad fuer deadlock-sqlite3 und website"));
        assert!(err.contains("kein impliziter Live-Pfad-Default"));
    }

    #[test]
    fn apply_refuses_central_prod_port_without_override() {
        assert!(refuse_central_prod_apply("postgres://x@127.0.0.1:5434/db", false).is_err());
        assert!(refuse_central_prod_apply("postgres://x@127.0.0.1:5434/db", true).is_ok());
    }

    #[test]
    fn apply_refuses_query_host_port_bypass_without_override() {
        let dsn = "postgres://user@/db?host=localhost&port=5434";

        assert!(refuse_central_prod_apply(dsn, false).is_err());
        assert!(refuse_central_prod_apply(dsn, true).is_ok());
    }

    #[test]
    fn apply_allows_clearly_other_dsn() {
        assert!(refuse_central_prod_apply("postgres://x@127.0.0.1:15434/db", false).is_ok());
    }

    #[test]
    fn apply_refuses_unparseable_dsn_even_with_override() {
        assert!(refuse_central_prod_apply("not a postgres dsn", true).is_err());
    }
}
