mod journal;
mod secrets;
mod state;

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
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
    #[serde(default)]
    legacy_state_directory: Option<PathBuf>,
    infisical_socket: PathBuf,
    project_id: String,
    environment: String,
    secret_path: String,
    /// Öffentlicher Hostname zur Zuordnung der Meldung, kein DNS-Zugriff.
    host_label: String,
}

fn config(path: &Path) -> Result<Config> {
    protected_path(path, false)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| anyhow::anyhow!("Meldekonfiguration ist nicht lesbar."))?;
    if !file.metadata()?.is_file() {
        bail!("Meldekonfiguration muss eine reguläre Datei sein.");
    }
    let mut bytes = Vec::new();
    file.take(16_385)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Meldekonfiguration ist nicht lesbar."))?;
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
        || config
            .legacy_state_directory
            .as_ref()
            .is_some_and(|path| !path.is_absolute() || path == &config.state_directory)
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

/// Dieselbe Eigentümer-/Sticky-Verzeichnisregel wie der vorhandene UDS-Client,
/// hier für vertrauenswürdige Betriebsdateien und den Installationspfad.
fn protected_path(path: &Path, directory: bool) -> Result<()> {
    if !path.is_absolute() {
        bail!("Betriebspfad muss absolut sein.");
    }
    // SAFETY: geteuid hat keine Zeigerargumente und verändert keinen Zustand.
    let uid = unsafe { libc::geteuid() };
    let components: Vec<_> = path.components().collect();
    let mut current = PathBuf::new();
    let mut after_sticky = false;
    for (index, component) in components.iter().enumerate() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            bail!("Betriebspfad ist nicht geschützt.");
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| anyhow::anyhow!("Betriebspfad ist nicht verfügbar."))?;
        let last = index + 1 == components.len();
        if ![0, uid].contains(&metadata.uid())
            || metadata.file_type().is_symlink()
            || (last && !directory && !metadata.is_file())
            || ((!last || directory) && !metadata.is_dir())
            || (after_sticky && (metadata.uid() != uid || metadata.mode() & 0o077 != 0))
        {
            bail!("Betriebspfad hat keine vertrauenswürdigen Eigentümer oder Rechte.");
        }
        after_sticky = metadata.mode() & 0o022 != 0;
        if after_sticky
            && !(metadata.is_dir() && metadata.uid() == 0 && metadata.mode() & 0o1000 != 0 && !last)
        {
            bail!("Betriebspfad darf nicht gruppen- oder weltbeschreibbar sein.");
        }
    }
    Ok(())
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
    if !directory.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    protected_path(directory, true)?;
    protected_path(binary, false)?;
    protected_path(&args.config, false)?;
    if fs::symlink_metadata(binary)?.mode() & 0o111 == 0 {
        bail!("Installiertes Binary ist nicht ausführbar.");
    }
    let unit = include_str!("../../../../service/systemd/unit-failure-notify@.service")
        .replace("@BINARY@", &binary.to_string_lossy())
        .replace("@CONFIG@", &args.config.to_string_lossy());
    let temporary = directory.join(format!(
        "unit-failure-notify@.service.{}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)
        .context("Geschützte Unit-Installation konnte nicht vorbereitet werden.")?;
    file.write_all(unit.as_bytes())?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    file.sync_all()?;
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
    let mut state = store.load_or_import(config.legacy_state_directory.as_deref(), unit)?;
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
        let cause = safe_cause(journal::cause(unit, invocation).await);
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
        // Versand erhalten. Bestätigte Sendungen und mögliche ACK-Verluste werden
        // getrennt gespeichert und gemeinsam konservativ zum Budget gezählt.
        store.save(&state)?;
    }
    let credential = args
        .credential
        .as_deref()
        .context("Bestehender Credential-Pfad fehlt.")?;
    let token = secrets::broker_token(&config, credential).await?;
    // Erst nach erfolgreichem Secretladen und unmittelbar vor HTTP reservieren:
    // verlorene ACKs dürfen nach Broker-TTL/Neustart keinen Replay-Sturm erzeugen.
    let attempt_time = now()?;
    if !state.eligible(attempt_time) {
        return Ok(());
    }
    state.reserve_attempt(attempt_time);
    store.save(&state)?;
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

fn safe_cause(diagnostic: Result<&'static str>) -> &'static str {
    diagnostic.unwrap_or("Die Ursache konnte aus dem begrenzten Journal dieser Dienstinstanz nicht sicher ermittelt werden. Das lokale Journal muss geprüft werden.")
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

    #[test]
    fn diagnostic_failure_falls_back_without_exposing_error_text() {
        let text = safe_cause(Err(anyhow::anyhow!("synthetic-secret-value")));
        assert!(text.contains("nicht sicher ermittelt"));
        assert!(!text.contains("synthetic-secret"));
    }

    #[test]
    fn config_symlinks_and_writable_config_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let file = root.path().join("config.json");
        fs::write(&file, "{}").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(protected_path(&file, false).is_ok());
        let alias = root.path().join("alias.json");
        symlink(&file, &alias).unwrap();
        assert!(config(&alias).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o660)).unwrap();
        assert!(config(&file).is_err());
    }

    #[test]
    fn installer_does_not_follow_existing_temporary_symlink() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let binary = root.path().join("binary");
        fs::write(&binary, "synthetic").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let config = root.path().join("config.json");
        fs::write(&config, "{}").unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let victim = root.path().join("victim");
        fs::write(&victim, "unchanged").unwrap();
        symlink(
            &victim,
            root.path().join(format!(
                "unit-failure-notify@.service.{}.tmp",
                std::process::id()
            )),
        )
        .unwrap();
        let args = Args {
            config,
            unit: None,
            credential: None,
            install: true,
            unit_directory: Some(root.path().to_owned()),
            binary: Some(binary),
        };
        assert!(install(&args).is_err());
        assert_eq!(fs::read_to_string(victim).unwrap(), "unchanged");
    }
}
