use clap::Parser;
use std::{ffi::OsString, os::unix::process::CommandExt};

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    token_pipe: bool,
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    #[arg(long)]
    uid: Option<u32>,
    #[arg(long)]
    gid: Option<u32>,
    /// Stable existing backup key, delivered only through the private pipe.
    #[arg(long)]
    pipe_secret: Option<String>,
    #[arg(long, value_enum, default_value = "all")]
    profile: dl_infisical_env::Profile,
    #[arg(last = true, required = true)]
    command: Vec<OsString>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.token_pipe {
        return token_pipe(cli).await;
    }
    let config = dl_infisical_env::config_from_env()?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let base = dl_infisical_env::validated_base_url(dl_infisical_env::INFISICAL_BASE_URL)?;
    let secrets = dl_infisical_env::fetch_secrets_with_retry(&client, &base, &config).await?;
    let error = dl_infisical_env::command_with_secrets(cli.profile, &secrets, &cli.command)?.exec();
    Err(error.into())
}

async fn token_pipe(cli: Cli) -> anyhow::Result<()> {
    use anyhow::{bail, Context};
    use nix::fcntl::{fcntl, FcntlArg, FdFlag, OFlag};
    use nix::unistd::{dup2, pipe2, setgid, setgroups, setuid, Gid, Uid};
    use std::{
        collections::BTreeMap,
        io::{Read, Write},
        os::fd::AsRawFd,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::Path,
        process::Command,
        time::{Duration, Instant},
    };
    use zeroize::Zeroizing;
    if !nix::unistd::geteuid().is_root() {
        bail!("Secret-Pipe benötigt den vorhandenen privilegierten Launcher.");
    }
    let uid = cli
        .uid
        .filter(|v| *v > 0)
        .context("Unprivilegierte Dienst-UID fehlt.")?;
    let gid = cli
        .gid
        .filter(|v| *v > 0)
        .context("Unprivilegierte Dienst-GID fehlt.")?;
    let config = cli
        .config
        .context("Normale Infisical-Konfiguration fehlt.")?;
    if !config.is_absolute() {
        bail!("Launcher-Konfiguration muss absolut sein.");
    }
    for parent in config.parent().into_iter().flat_map(Path::ancestors) {
        let metadata = std::fs::symlink_metadata(parent).map_err(|_| {
            anyhow::anyhow!("Launcher-Konfigurationsverzeichnis ist nicht prüfbar.")
        })?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            bail!("Launcher-Konfigurationsverzeichnis muss root gehören und geschützt sein.");
        }
    }
    let mut config_file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(&config)
        .map_err(|_| anyhow::anyhow!("Infisical-Konfiguration fehlt."))?;
    let metadata = config_file
        .metadata()
        .context("Infisical-Konfiguration fehlt.")?;
    if !config.is_absolute()
        || !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
    {
        bail!(
            "Launcher-Konfiguration muss root gehören und gegen fremde Änderungen geschützt sein."
        );
    }
    let program = cli.command.first().context("Dienstprogramm fehlt.")?;
    if !Path::new(program).is_absolute() {
        bail!("Dienstprogramm muss einen absoluten Pfad haben.");
    }
    let mut config_bytes = Vec::new();
    std::io::Read::by_ref(&mut config_file)
        .take(65537)
        .read_to_end(&mut config_bytes)?;
    if config_bytes.len() > 65536 {
        bail!("Launcher-Konfiguration ist zu groß.");
    }
    let values = dl_token_secrets::values_from_config(&config_bytes).await?;
    let view: BTreeMap<&str, &str> = values
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let body = if let Some(name) = cli.pipe_secret {
        if name != "DB_MASTER_KEY_V1" {
            bail!("Backup-Pipe darf nur den bestehenden Datenbankschlüssel verwenden.");
        }
        let value = view
            .get(name.as_str())
            .context("Bestehender Backupschlüssel fehlt in Infisical.")?;
        Zeroizing::new(value.as_bytes().to_vec())
    } else {
        Zeroizing::new(
            serde_json::to_vec(&view)
                .map_err(|_| anyhow::anyhow!("Secret-Pipe konnte nicht vorbereitet werden."))?,
        )
    };
    if body.len() > 2 * 1024 * 1024 {
        bail!("Secret-Pipe ist zu groß.");
    }
    drop(view);
    drop(values);
    let (read, write) = pipe2(OFlag::O_CLOEXEC)?;
    let read_fd = read.as_raw_fd();
    let mut command = Command::new(program);
    command.args(cli.command.iter().skip(1)).env_clear();
    // SAFETY: only async-signal-safe fd/credential syscalls execute after fork;
    // all allocations and parsing occur before installing this callback.
    unsafe {
        command.pre_exec(move || {
            dup2(read_fd, 3)?;
            fcntl(3, FcntlArg::F_SETFD(FdFlag::empty()))?;
            setgroups(&[])?;
            setgid(Gid::from_raw(gid))?;
            setuid(Uid::from_raw(uid))?;
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| {
        anyhow::anyhow!("Dienst konnte nicht aus dem Secret-Launcher gestartet werden.")
    })?;
    drop(read);
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut writer = std::fs::File::from(write);
        let result = writer.write_all(&body).is_ok();
        drop(writer);
        let _ = sent.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match received.try_recv() {
            Ok(true) => break,
            Ok(false) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("Dienst hat die private Secret-Pipe nicht angenommen.");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("Dienst hat die Secret-Pipe nicht rechtzeitig angenommen.");
            }
        }
    }
    let status = child.wait().context("Dienstabschluss ist nicht prüfbar.")?;
    if !status.success() {
        bail!("Dienst wurde ohne erfolgreichen Abschluss beendet.");
    }
    Ok(())
}
