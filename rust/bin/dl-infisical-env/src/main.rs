use clap::Parser;
use std::{ffi::OsString, os::unix::process::CommandExt};

#[repr(C)]
struct CapabilityHeader {
    version: u32,
    pid: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CapabilityData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

fn clear_capability_sets() -> std::io::Result<()> {
    let header = CapabilityHeader {
        version: 0x20080522,
        pid: 0,
    };
    let data = [CapabilityData {
        effective: 0,
        permitted: 0,
        inheritable: 0,
    }; 2];
    // SAFETY: Linux capability ABI v3 takes this fixed header and two u32 triples;
    // pointers remain valid for the syscall and no process other than self is selected.
    if unsafe { nix::libc::syscall(nix::libc::SYS_capset, &header, data.as_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn signal_child(
    child: &mut std::process::Child,
    signal: nix::sys::signal::Signal,
) -> std::io::Result<()> {
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    // The owned, unreaped Child prevents PID reuse while signalling this PID.
    match kill(Pid::from_raw(child.id() as i32), signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(std::io::Error::from_raw_os_error(error as i32)),
    }
}

async fn abort_child(child: &mut std::process::Child) -> anyhow::Result<()> {
    use anyhow::Context;
    signal_child(child, nix::sys::signal::Signal::SIGKILL)
        .context("Eigener Dienst konnte nicht abgebrochen werden.")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if child
            .try_wait()
            .context("Eigener Dienstabschluss ist nicht prüfbar.")?
            .is_some()
        {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("Eigener Dienstabschluss überschreitet die Abbruchgrenze.");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

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
    /// Preserve the application's no-new-privileges boundary after bootstrap.
    #[arg(long, requires = "token_pipe")]
    child_no_new_privileges: bool,
    /// Root-controlled service groups; other consumers keep an empty group list.
    #[arg(long, requires = "token_pipe")]
    supplementary_group: Vec<u32>,
    /// Restricted bootstrap capabilities must not remain in the app's bounding set.
    #[arg(long, requires_all = ["token_pipe", "child_no_new_privileges"])]
    child_clear_capability_bounding_set: bool,
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
    if cli.supplementary_group.len() > 16 || cli.supplementary_group.contains(&0) {
        bail!("Ergänzende Dienstgruppen sind ungültig.");
    }
    let mut group_ids = cli.supplementary_group;
    group_ids.sort_unstable();
    group_ids.dedup();
    let supplementary_groups: Vec<Gid> = group_ids.into_iter().map(Gid::from_raw).collect();
    let last_capability = if cli.child_clear_capability_bounding_set {
        let last = std::fs::read_to_string("/proc/sys/kernel/cap_last_cap")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|value| *value <= 128)
            .context("Bootstrap-Fähigkeitsgrenze ist nicht prüfbar.")?;
        Some(last)
    } else {
        None
    };
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
    let child_no_new_privileges = cli.child_no_new_privileges;
    let mut command = Command::new(program);
    command.args(cli.command.iter().skip(1)).env_clear();
    // SAFETY: only async-signal-safe fd/credential syscalls execute after fork;
    // all allocations and parsing occur before installing this callback.
    unsafe {
        command.pre_exec(move || {
            dup2(read_fd, 3)?;
            fcntl(3, FcntlArg::F_SETFD(FdFlag::empty()))?;
            setgroups(&supplementary_groups)?;
            setgid(Gid::from_raw(gid))?;
            if let Some(last) = last_capability {
                for capability in 0..=last {
                    if nix::libc::prctl(
                        nix::libc::PR_CAPBSET_DROP,
                        capability as nix::libc::c_ulong,
                        0,
                        0,
                        0,
                    ) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                if nix::libc::prctl(
                    nix::libc::PR_CAP_AMBIENT,
                    nix::libc::PR_CAP_AMBIENT_CLEAR_ALL,
                    0,
                    0,
                    0,
                ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            setuid(Uid::from_raw(uid))?;
            if last_capability.is_some() {
                clear_capability_sets()?;
            }
            if child_no_new_privileges {
                nix::sys::prctl::set_no_new_privs()?;
            }
            Ok(())
        });
    }
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("Dienststoppsignal ist nicht verfügbar.")?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("Dienstabbruchsignal ist nicht verfügbar.")?;
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
    let mut pipe_complete = false;
    let mut stop_deadline = None;
    loop {
        if !pipe_complete {
            match received.try_recv() {
                Ok(true) => pipe_complete = true,
                Ok(false) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    abort_child(&mut child).await?;
                    bail!("Dienst hat die private Secret-Pipe nicht angenommen.");
                }
                Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {}
                Err(_) => {
                    abort_child(&mut child).await?;
                    bail!("Dienst hat die Secret-Pipe nicht rechtzeitig angenommen.");
                }
            }
        }
        if let Some(status) = child
            .try_wait()
            .context("Dienstabschluss ist nicht prüfbar.")?
        {
            if pipe_complete && (status.success() || stop_deadline.is_some()) {
                return Ok(());
            }
            if !status.success() && stop_deadline.is_none() {
                bail!("Dienst wurde ohne erfolgreichen Abschluss beendet.");
            }
        }
        if stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            abort_child(&mut child).await?;
            bail!("Dienst hat die normale Stoppgrenze überschritten.");
        }
        tokio::select! {
            _ = terminate.recv(), if stop_deadline.is_none() => {
                signal_child(&mut child, nix::sys::signal::Signal::SIGTERM)
                    .context("Dienststoppsignal konnte nicht weitergegeben werden.")?;
                stop_deadline = Some(Instant::now() + Duration::from_secs(10));
            }
            _ = interrupt.recv(), if stop_deadline.is_none() => {
                signal_child(&mut child, nix::sys::signal::Signal::SIGINT)
                    .context("Dienstabbruchsignal konnte nicht weitergegeben werden.")?;
                stop_deadline = Some(Instant::now() + Duration::from_secs(10));
            }
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn child_privilege_boundary_requires_private_bootstrap() {
        assert!(
            Cli::try_parse_from(["launcher", "--child-no-new-privileges", "--", "/bin/true"])
                .is_err()
        );
        let cli = Cli::try_parse_from([
            "launcher",
            "--token-pipe",
            "--child-no-new-privileges",
            "--",
            "/bin/true",
        ])
        .expect("Private bootstrap accepts the child privilege boundary");
        assert!(cli.child_no_new_privileges);
    }

    #[test]
    fn groups_and_capability_clear_require_private_bootstrap() {
        assert!(Cli::try_parse_from([
            "launcher",
            "--supplementary-group",
            "985",
            "--",
            "/bin/true"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "launcher",
            "--token-pipe",
            "--child-clear-capability-bounding-set",
            "--",
            "/bin/true"
        ])
        .is_err());
        let cli = Cli::try_parse_from([
            "launcher",
            "--token-pipe",
            "--child-no-new-privileges",
            "--child-clear-capability-bounding-set",
            "--supplementary-group",
            "985",
            "--",
            "/bin/true",
        ])
        .expect("Restricted private bootstrap fixture");
        assert_eq!(cli.supplementary_group, [985]);
        assert!(cli.child_clear_capability_bounding_set);
        let ordinary = Cli::try_parse_from(["launcher", "--token-pipe", "--", "/bin/true"])
            .expect("Ordinary private bootstrap fixture");
        assert!(ordinary.supplementary_group.is_empty());
        assert!(!ordinary.child_clear_capability_bounding_set);
    }

    #[tokio::test]
    async fn failed_pipe_child_abort_is_bounded_and_reaped() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("Owned child fixture");
        let started = std::time::Instant::now();
        super::abort_child(&mut child)
            .await
            .expect("Owned child abort and reap");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(child
            .try_wait()
            .expect("Owned child reap fixture")
            .is_some());
    }
}
