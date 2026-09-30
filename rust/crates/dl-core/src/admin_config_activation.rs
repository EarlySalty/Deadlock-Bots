//! Beaufsichtigte Aktivierung registrierter TOML-Dateien, ohne Shell.
//! Ein erfolgreicher Dateischreibvorgang ist ausdrücklich kein Laufzeitnachweis.
use crate::admin_config::{private_directory, valid_revision, Dashboard, Error, Registry, Target};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Service {
    pub unit: String,
    pub state: String,
    pub pid: u32,
    #[serde(default)]
    pub invocation_id: String,
    pub config_connected: bool,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Applied {
    pub revision: String,
    pub at: u64,
    pub services: Vec<Service>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Operation {
    pub revision: String,
    pub status: String,
    pub at: u64,
    pub message: String,
}
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Record {
    pub applied: Option<Applied>,
    pub operation: Option<Operation>,
}
#[derive(Serialize)]
pub struct Status {
    pub services: Vec<Service>,
    pub saved_revision_active: bool,
    pub last_applied_revision: Option<String>,
    pub operation: Option<Operation>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn capture(program: &str, args: &[String]) -> Result<String, Error> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Unavailable)?;
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|_| Error::Io)? {
            Some(status) => {
                if !status.success() {
                    return Err(Error::Unavailable);
                }
                let mut bytes = Vec::new();
                if let Some(stdout) = child.stdout.take() {
                    stdout
                        .take(8193)
                        .read_to_end(&mut bytes)
                        .map_err(|_| Error::Io)?;
                }
                if bytes.len() > 8192 {
                    return Err(Error::Unavailable);
                }
                return String::from_utf8(bytes).map_err(|_| Error::Unavailable);
            }
            None if start.elapsed() > Duration::from_secs(8) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Unavailable);
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

/// Nur der explizite Config-Schalter wird untersucht; keine Prozessumgebung,
/// keine sonstigen Argumente oder Zugangsdaten gelangen in eine Antwort.
fn uses_config(pid: u32, expected: &Path) -> bool {
    if pid == 0 {
        return false;
    }
    let Ok(file) = File::open(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(128 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 128 * 1024 {
        return false;
    }
    let args: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    let mut candidates = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if *arg == b"--config" {
            if let Some(next) = args
                .get(index + 1)
                .and_then(|value| std::str::from_utf8(value).ok())
            {
                candidates.push(next);
            }
        } else if let Ok(arg) = std::str::from_utf8(arg) {
            if let Some(path) = arg.strip_prefix("--config=") {
                candidates.push(path);
            }
        }
    }
    if candidates.len() != 1 {
        return false;
    }
    let selected = Path::new(candidates[0]);
    if !selected.is_absolute() {
        return false;
    }
    selected.canonicalize().is_ok_and(|path| path == expected)
}

pub fn observe(target: &Target) -> Result<Vec<Service>, Error> {
    let expected = target.safe_source()?;
    let mut args = Vec::new();
    if target.dashboard == Dashboard::Discord {
        args.push("--user".to_owned());
    }
    args.extend([
        "show".into(),
        "--no-pager".into(),
        "--property=Id,ActiveState,MainPID,InvocationID".into(),
    ]);
    args.extend(target.units.clone());
    let output = capture("/usr/bin/systemctl", &args)?;
    let mut result = Vec::new();
    for section in output.split("\n\n") {
        let values: std::collections::BTreeMap<_, _> = section
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        let Some(unit) = values
            .get("Id")
            .filter(|id| target.units.iter().any(|unit| unit == **id))
        else {
            continue;
        };
        let pid = values
            .get("MainPID")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let state = values
            .get("ActiveState")
            .copied()
            .unwrap_or("unknown")
            .to_owned();
        let invocation_id = values
            .get("InvocationID")
            .copied()
            .filter(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .unwrap_or("")
            .to_owned();
        result.push(Service {
            unit: (*unit).to_owned(),
            state,
            pid,
            invocation_id,
            config_connected: uses_config(pid, &expected),
        });
    }
    if result.len() != target.units.len() {
        return Err(Error::Unavailable);
    }
    Ok(result)
}

fn record_path(registry: &Path, id: &str) -> Result<std::path::PathBuf, Error> {
    let parent = registry.parent().ok_or(Error::Io)?;
    Ok(parent
        .join(".admin-bot-config-state")
        .join(format!("{id}.json")))
}
fn read_record(registry: &Path, id: &str) -> Record {
    let Ok(path) = record_path(registry, id) else {
        return Record::default();
    };
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let Ok(file) = options.open(path) else {
        return Record::default();
    };
    if !file
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() <= 16384)
    {
        return Record::default();
    }
    let mut bytes = Vec::new();
    if file.take(16385).read_to_end(&mut bytes).is_err() || bytes.len() > 16384 {
        return Record::default();
    }
    serde_json::from_slice(&bytes).unwrap_or_default()
}
fn write_record(registry: &Path, id: &str, record: &Record) -> Result<(), Error> {
    let path = record_path(registry, id)?;
    let parent = path.parent().ok_or(Error::Io)?;
    private_directory(parent)?;
    let temp = parent.join(format!(".{id}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|_| Error::Io)?;
        file.write_all(&serde_json::to_vec(record).map_err(|_| Error::Io)?)
            .and_then(|_| file.sync_all())
            .map_err(|_| Error::Io)?;
        fs::rename(&temp, &path).map_err(|_| Error::Io)?;
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| Error::Durability)
    })();
    let _ = fs::remove_file(temp);
    result
}

pub fn status(registry: &Path, target: &Target, revision: &str) -> Result<Status, Error> {
    let services = observe(target)?;
    let record = read_record(registry, &target.id);
    let active = record.applied.as_ref().is_some_and(|applied| {
        applied.revision == revision
            && services.iter().all(|service| {
                service.state == "active"
                    && service.config_connected
                    && service.pid > 0
                    && !service.invocation_id.is_empty()
                    && applied.services.iter().any(|old| {
                        old.unit == service.unit
                            && old.pid == service.pid
                            && old.invocation_id == service.invocation_id
                    })
            })
    });
    Ok(Status {
        services,
        saved_revision_active: active,
        last_applied_revision: record.applied.map(|applied| applied.revision),
        operation: record.operation,
    })
}

pub fn queue(registry_path: &Path, target: &Target, revision: &str) -> Result<Operation, Error> {
    if !valid_revision(revision) {
        return Err(Error::Invalid(
            "Der gespeicherte Stand fehlt. Bitte neu laden.",
        ));
    }
    let source = target.safe_source()?;
    let _lock = target.lock(&source)?;
    let snapshot = target.snapshot()?;
    if snapshot.revision != revision {
        return Err(Error::Conflict);
    }
    let services = observe(target)?;
    if services
        .iter()
        .any(|service| service.pid > 0 && !service.config_connected)
    {
        return Err(Error::Invalid("Mindestens ein laufender Dienst verwendet diese TOML noch nicht. Der TOML-Rollout muss zuerst abgeschlossen werden."));
    }
    let mut record = read_record(registry_path, &target.id);
    if record.operation.as_ref().is_some_and(|op| {
        matches!(op.status.as_str(), "queued" | "running") && now().saturating_sub(op.at) < 180
    }) {
        return Err(Error::Busy);
    }
    let registry = Registry::load(registry_path)?;
    if !registry.apply_binary.is_file() {
        return Err(Error::Unavailable);
    }
    let operation = Operation {
        revision: revision.to_owned(),
        status: "queued".into(),
        at: now(),
        message: "Aktivierung vorgemerkt. Die betroffenen Dienste werden neu gestartet.".into(),
    };
    record.operation = Some(operation.clone());
    write_record(registry_path, &target.id, &record)?;
    let args = vec![
        "--user".into(),
        "--quiet".into(),
        "--collect".into(),
        format!("--unit=deadlock-config-apply-{}", target.id),
        "--on-active=2s".into(),
        "--timer-property=AccuracySec=1s".into(),
        registry.apply_binary.to_string_lossy().into_owned(),
        "--registry".into(),
        registry_path.to_string_lossy().into_owned(),
        "--bot".into(),
        target.id.clone(),
        "--revision".into(),
        revision.to_owned(),
    ];
    if !crate::admin_config::run_quiet(
        Path::new("/usr/bin/systemd-run"),
        &args,
        Duration::from_secs(10),
    )
    .unwrap_or(false)
    {
        record.operation = Some(Operation {
            status: "failed".into(),
            message: "Die Aktivierung konnte nicht gestartet werden. Die Datei bleibt gespeichert."
                .into(),
            ..operation
        });
        write_record(registry_path, &target.id, &record)?;
        return Err(Error::Unavailable);
    }
    Ok(operation)
}

fn control(target: &Target, action: &str) -> Result<(), Error> {
    let mut args = Vec::new();
    let program = if target.dashboard == Dashboard::Twitch {
        args.extend(["-n".into(), "/usr/bin/systemctl".into()]);
        "/usr/bin/sudo"
    } else {
        args.push("--user".into());
        "/usr/bin/systemctl"
    };
    args.push(action.into());
    args.extend(target.units.clone());
    if crate::admin_config::run_quiet(Path::new(program), &args, Duration::from_secs(45))? {
        Ok(())
    } else {
        Err(Error::Unavailable)
    }
}

/// Ausschließlich vom beaufsichtigten, fest installierten Hilfsprogramm aufgerufen.
pub fn apply(registry_path: &Path, id: &str, revision: &str) -> Result<(), Error> {
    if !valid_revision(revision) {
        return Err(Error::Invalid("Ungültige Revision."));
    }
    let registry = Registry::load(registry_path)?;
    let dashboard = if id == "twitch" {
        Dashboard::Twitch
    } else {
        Dashboard::Discord
    };
    let target = registry.target(id, dashboard)?;
    let source = target.safe_source()?;
    let _lock = target.lock(&source)?;
    let mut record = read_record(registry_path, id);
    let result = (|| {
        if target.snapshot()?.revision != revision {
            return Err(Error::Conflict);
        }
        let before = observe(&target)?;
        if before
            .iter()
            .any(|service| service.pid > 0 && !service.config_connected)
        {
            return Err(Error::Invalid(
                "Der laufende Dienst ist noch nicht an diese TOML angeschlossen.",
            ));
        }
        record.operation = Some(Operation {
            revision: revision.to_owned(),
            status: "running".into(),
            at: now(),
            message: "Die Dienste werden neu gestartet und geprüft.".into(),
        });
        write_record(registry_path, id, &record)?;
        // Beim Wechsel der Katalogwartung dürfen nie zwei Steam-Cores überlappen.
        if id == "steam" {
            control(&target, "stop")?;
            control(&target, "start")?;
        } else {
            control(&target, "restart")?;
        }
        let started = Instant::now();
        let mut confirmations = 0;
        loop {
            let services = observe(&target)?;
            if services.iter().all(|service| {
                service.state == "active"
                    && service.pid > 0
                    && service.config_connected
                    && !service.invocation_id.is_empty()
                    && before.iter().all(|old| {
                        old.unit != service.unit
                            || (old.pid != service.pid
                                && old.invocation_id != service.invocation_id)
                    })
            }) {
                confirmations += 1;
                if confirmations >= 3 {
                    if target.snapshot()?.revision != revision {
                        return Err(Error::Conflict);
                    }
                    record.applied = Some(Applied {
                        revision: revision.to_owned(),
                        at: now(),
                        services,
                    });
                    return Ok(());
                }
            } else {
                confirmations = 0;
            }
            if started.elapsed() > Duration::from_secs(45) {
                return Err(Error::Unavailable);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    })();
    record.operation = Some(Operation {
        revision: revision.to_owned(),
        status: if result.is_ok() { "applied" } else { "failed" }.into(),
        at: now(),
        message: if result.is_ok() {
            "Gespeicherte Konfiguration nach Neustart übernommen.".into()
        } else {
            "Aktivierung nicht bestätigt. Datei und Dienststatus prüfen; eine frühere Version kann als Entwurf geladen werden.".into()
        },
    });
    write_record(registry_path, id, &record)?;
    result
}
