mod journal;
mod secrets;
mod state;

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    unit: Option<String>,
    #[arg(long)]
    credential: Option<PathBuf>,
    /// Installiert ausschließlich die bestehende OnFailure-Template-Unit.
    #[arg(long)]
    install: bool,
    #[arg(long, requires = "install")]
    unit_directory: Option<PathBuf>,
    #[arg(long, requires = "install")]
    binary: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    broker_origin: String,
    channel_id: u64,
    state_directory: PathBuf,
    infisical_socket: PathBuf,
    project_id: String,
    environment: String,
    secret_path: String,
    /// Öffentlicher Hostname zur Zuordnung der Meldung, kein DNS-Zugriff.
    host_label: String,
}

fn config(path: &Path) -> Result<Config> {
    let bytes =
        fs::read(path).map_err(|_| anyhow::anyhow!("Meldekonfiguration ist nicht lesbar."))?;
    if bytes.len() > 16_384 {
        bail!("Meldekonfiguration ist zu groß.");
    }
    let mut config: Config = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Meldekonfiguration ist ungültig."))?;
    let url = reqwest::Url::parse(&config.broker_origin)
        .map_err(|_| anyhow::anyhow!("Broker-Adresse ist ungültig."))?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if url.scheme() != "http"
        || !loopback
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || config.channel_id == 0
        || !config.state_directory.is_absolute()
        || !config.infisical_socket.is_absolute()
        || config.project_id.is_empty()
        || config.environment.is_empty()
        || !config.secret_path.starts_with('/')
        || config.host_label.is_empty()
        || config.host_label.len() > 64
        || !config
            .host_label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
    {
        bail!("Meldekonfiguration enthält einen ungültigen Betriebswert.");
    }
    config.broker_origin = url.as_str().trim_end_matches('/').into();
    Ok(config)
}

fn valid_unit(unit: &str) -> bool {
    !unit.is_empty()
        && unit.len() <= 255
        && unit.ends_with(".service")
        && unit
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_@:".contains(&byte))
        && !unit.starts_with('-')
}

fn key(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Systemzeit ist ungültig.")?
        .as_secs())
}

fn install(args: &Args) -> Result<()> {
    let directory = args
        .unit_directory
        .as_ref()
        .context("Für die Installation fehlt das Unitverzeichnis.")?;
    let binary = args
        .binary
        .as_ref()
        .context("Für die Installation fehlt der Binärpfad.")?;
    for path in [directory.as_path(), binary.as_path(), args.config.as_path()] {
        if !path.is_absolute()
            || !path
                .to_string_lossy()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
        {
            bail!("Installationspfade müssen absolute einfache Dateipfade sein.");
        }
    }
    fs::create_dir_all(directory)?;
    let unit = include_str!("../../../../service/systemd/unit-failure-notify@.service")
        .replace("@BINARY@", &binary.to_string_lossy())
        .replace("@CONFIG@", &args.config.to_string_lossy());
    let temporary = directory.join("unit-failure-notify@.service.tmp");
    fs::write(&temporary, unit)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o644))?;
    fs::rename(temporary, directory.join("unit-failure-notify@.service"))?;
    println!(
        "OnFailure-Template installiert. Der User-Manager muss anschließend neu geladen werden."
    );
    Ok(())
}

async fn run(args: Args) -> Result<()> {
    let config = config(&args.config)?;
    if args.install {
        return install(&args);
    }
    let unit = args
        .unit
        .as_deref()
        .filter(|unit| valid_unit(unit))
        .context("Auslösende Dienst-Unit ist ungültig.")?;
    // Nur vom OnFailure-Ereignis gelieferte Laufzeitmetadaten, kein ENV-Config.
    // Kein Fallback auf systemctl show: dort kann schon der nächste Start stehen.
    let invocation = std::env::var("MONITOR_INVOCATION_ID")
        .context("Systemd hat keine auslösende Dienstinstanz übergeben.")?;
    let invocation = journal::validate_invocation(&invocation)?;
    let store = state::Store::open(&config.state_directory, &key(unit))?;
    let mut state = store.load()?;
    state.observe(invocation);
    store.save(&state)?;
    if state.unreported == 0 && state.pending.is_none() {
        println!("Diese Dienstinstanz wurde bereits gemeldet.");
        return Ok(());
    }
    if !state.eligible(now()?) {
        println!("Meldung entprellt; Wiederholung wurde für die nächste Meldung gezählt.");
        return Ok(());
    }
    if state.pending.is_none() {
        let cause = journal::cause(unit, invocation).await?;
        let repeats = state.unreported.saturating_sub(1);
        let attempts = if state.unreported == 1 {
            "ein fehlgeschlagener Start".to_owned()
        } else {
            format!("{} fehlgeschlagene Starts", state.unreported)
        };
        let content = format!("**Dienst ausgefallen: `{unit}`**\nHost: `{}`\n{cause}\nSeit der letzten bestätigten Meldung: {attempts}. Weitere Wiederholungen: {repeats}.\nWeitere Meldungen: höchstens einmal pro Tag und zweimal innerhalb von sieben Tagen.", config.host_label);
        state.pending = Some(state::Pending {
            key: format!(
                "unit-failure-{}",
                key(&format!("{unit}:{invocation}:{}", state.sequence))
            ),
            content,
            included: state.unreported,
        });
        // Dieselbe Idempotenz-ID und derselbe Text bleiben nach unbestätigtem
        // Versand erhalten. Ausschließlich erfolgreiche Sendungen zählen zum Budget.
        store.save(&state)?;
    }
    let credential = args
        .credential
        .as_deref()
        .context("Bestehender Credential-Pfad fehlt.")?;
    let token = secrets::broker_token(&config, credential).await?;
    secrets::send(
        &config,
        state.pending.as_ref().context("Meldung fehlt.")?,
        &token,
    )
    .await?;
    state.confirm(now()?);
    store.save(&state)?;
    println!("Ausfallmeldung wurde bestätigt und der Meldezustand gespeichert.");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run(Args::parse()).await {
        // Keine Debug-/Ursachenketten: externe Inhalte bleiben immer lokal.
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_and_invocation_cannot_inject_journal_filters_or_discord_markup() {
        assert!(valid_unit("steam-core-2.service"));
        for name in [
            "--all.service",
            "../../x.service",
            "x`@everyone.service",
            "x\n.service",
        ] {
            assert!(!valid_unit(name));
        }
        assert!(journal::validate_invocation(&"a".repeat(32)).is_ok());
        assert!(journal::validate_invocation("--all").is_err());
    }
}
