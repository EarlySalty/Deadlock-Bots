use dl_ai::tool_chat::GatewayConfig;
use dl_ai::upstream::{GATEWAY_BINARY, GATEWAY_UNIT, HEALTH_PATH, HELPER_BINARY, SYSTEMCTL};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::{self, File, OpenOptions},
    net::SocketAddr,
    os::unix::{fs::symlink, fs::MetadataExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

macro_rules! owner_home {
    () => {
        "/home/nathanael"
    };
}

const ROOT: &str = "/opt/deadlock/bots";
const RELEASES: &str = "releases";
const CURRENT: &str = "current";
const OWNER: &str = "nathanael";
const WORKTREES: &str = concat!(owner_home!(), "/.worktrees");
const CONFIG_DIRECTORY: &str = concat!(owner_home!(), "/.config/deadlock-bots");
const UNIT_DIRECTORY: &str = concat!(owner_home!(), "/.config/systemd/user");
const SOURCE_CONFIG_DIRECTORY: &str = "config";
const SOURCE_UNIT_DIRECTORY: &str = "service/systemd";
const UNITS: [&str; 2] = [GATEWAY_UNIT, "dl-community-points-sync.service"];
const OWNER_UID: u32 = 1000;
const BOT_BINARY: &str = "dl-bot";
const LAUNCHER: &str = "dl-infisical-env";
const REBUILT: [&str; 3] = [BOT_BINARY, GATEWAY_BINARY, HELPER_BINARY];
const COMPANIONS: [&str; 2] = ["dl-web", LAUNCHER];
const PROVENANCE_STEM: &str = "BUILD-PROVENANCE";
const MANIFEST_SUFFIX: &str = ".toml";
const SERVE_MARGIN_SECS: u64 = 30;
const SERVE_PAUSE_MILLIS: u64 = 1_000;
const SERVE_PROBE_SECS: u64 = 2;
const INVALID_TARGET: &str = "Ungültiger Zielpfad";
const CANDIDATE_MISMATCH: &str = "Der vorhandene Release passt nicht zum geprüften Artefakt";

fn gateway_config_name() -> String {
    format!("{GATEWAY_BINARY}{MANIFEST_SUFFIX}")
}

fn gateway_config() -> PathBuf {
    Path::new(CONFIG_DIRECTORY).join(gateway_config_name())
}

fn provenance_file() -> String {
    format!("{PROVENANCE_STEM}{MANIFEST_SUFFIX}")
}

fn base_provenance_name(previous_sha: &str) -> String {
    format!("BASE-{PROVENANCE_STEM}-{previous_sha}{MANIFEST_SUFFIX}")
}

fn checksum_key(binary: &str) -> String {
    format!("{}_sha256", binary.replace('-', "_"))
}

fn carried_over() -> Vec<String> {
    COMPANIONS
        .iter()
        .map(|name| (*name).to_owned())
        .chain([provenance_file()])
        .collect()
}

fn run(program: &str, arguments: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(arguments)
        .env("PATH", "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "{program}: {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn text(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| "Ungültiger Pfad".into())
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn owned(path: &Path, directory: bool, uid: u32) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || metadata.uid() != uid
        || metadata.mode() & 0o022 != 0
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(format!("Unsicherer Releasepfad: {}", path.display()).into());
    }
    Ok(())
}

fn source_matches(source: &Path, sha: &str) -> Result<()> {
    let source = text(source)?;
    let git = |arguments: &[&str]| {
        let mut command = vec!["-u", OWNER, "--", "/usr/bin/git", "-C", source];
        command.extend_from_slice(arguments);
        run("/usr/sbin/runuser", &command)
    };
    if git(&["remote", "get-url", "origin"])? != "git@github.com:EarlySalty/Deadlock-Bots.git" {
        return Err("Die Quelle gehört nicht zum Discord-Bot-Repository".into());
    }
    if git(&["rev-parse", "HEAD"])? != sha || !git(&["status", "--porcelain"])?.is_empty() {
        return Err("Der Quellstand ist nicht sauber auf dem angegebenen Commit".into());
    }
    let remote = git(&["ls-remote", "origin", "refs/heads/main"])?;
    if remote.split_whitespace().next() != Some(sha) {
        return Err("Der Quellstand entspricht nicht dem aktuellen origin/main".into());
    }
    Ok(())
}

fn hash(path: &Path) -> Result<String> {
    let result = run("/usr/bin/sha256sum", &[text(path)?])?;
    result
        .split_whitespace()
        .next()
        .filter(|value| valid_hex(value, 64))
        .map(str::to_owned)
        .ok_or_else(|| "Ungültiger Dateihash".into())
}

fn as_owner(command: &[&str]) -> Result<String> {
    let runtime = format!("XDG_RUNTIME_DIR=/run/user/{OWNER_UID}");
    let bus = format!("DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{OWNER_UID}/bus");
    let mut arguments = vec![
        "-u",
        OWNER,
        "--",
        "/usr/bin/env",
        runtime.as_str(),
        bus.as_str(),
    ];
    arguments.extend_from_slice(command);
    run("/usr/sbin/runuser", &arguments)
}

fn restart() -> Result<()> {
    every(vec![
        as_owner(&["/usr/local/bin/bot-restart", BOT_BINARY]).map(drop),
        as_owner(&[SYSTEMCTL, "--user", "restart", GATEWAY_UNIT]).map(drop),
    ])
}

fn every(steps: Vec<Result<()>>) -> Result<()> {
    let failures: Vec<String> = steps
        .into_iter()
        .filter_map(|step| step.err())
        .map(|error| error.to_string())
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; ").into())
    }
}

fn with_rollback(error: Box<dyn Error>, rollback: Result<()>) -> Box<dyn Error> {
    match rollback {
        Ok(()) => error,
        Err(incomplete) => format!("{error}; Rücknahme unvollständig: {incomplete}").into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Proof {
    address: SocketAddr,
    window: Duration,
}

fn proof(config: &Path) -> Result<Proof> {
    let config = GatewayConfig::load(config)?;
    Ok(Proof {
        address: config.listen,
        window: config.alarm.start_budget() + Duration::from_secs(SERVE_MARGIN_SECS),
    })
}

fn previous_proof(config: &Path, failed: &Proof) -> Result<Proof> {
    proof(config).or_else(|_| {
        Ok(Proof {
            address: GatewayConfig::listen_address(config)?,
            window: failed.window,
        })
    })
}

fn answers(url: &str) -> Result<()> {
    let limit = SERVE_PROBE_SECS.to_string();
    let probe = [
        "--silent",
        "--show-error",
        "--fail",
        "--max-time",
        limit.as_str(),
        url,
    ];
    run("/usr/bin/curl", &probe).map(drop)
}

fn serves(proof: &Proof, pause: Duration) -> Result<()> {
    let url = format!("http://{}{HEALTH_PATH}", proof.address);
    let deadline = Instant::now() + proof.window;
    loop {
        match answers(&url) {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(format!(
                    "Das KI-Gateway antwortet {} s nach dem Neustart nicht auf {url}: {error}",
                    proof.window.as_secs()
                )
                .into())
            }
            Err(_) => std::thread::sleep(pause),
        }
    }
}

fn restart_and_prove(proof: &Proof) -> Result<()> {
    restart()?;
    serves(proof, Duration::from_millis(SERVE_PAUSE_MILLIS))
}

fn restart_and_prove_previous(config: &Path, failed: &Proof) -> Result<()> {
    restart()?;
    serves(
        &previous_proof(config, failed)?,
        Duration::from_millis(SERVE_PAUSE_MILLIS),
    )
}

fn backup_of(target: &Path) -> Result<PathBuf> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(INVALID_TARGET)?;
    let directory = target.parent().ok_or(INVALID_TARGET)?;
    Ok(directory.join(format!(".{name}.previous")))
}

fn stage_owner_file(origin: &Path, target: &Path, mode: &str) -> Result<PathBuf> {
    if !fs::symlink_metadata(origin)?.is_file() {
        return Err(format!("Im Quellstand fehlt {}", origin.display()).into());
    }
    let directory = target.parent().ok_or(INVALID_TARGET)?;
    owned(directory, true, OWNER_UID)?;
    if fs::symlink_metadata(target).is_ok() {
        owned(target, false, OWNER_UID)?;
    }
    let staged = backup_of(target)?.with_extension("install");
    run(
        "/usr/bin/install",
        &[
            "-o",
            OWNER,
            "-g",
            OWNER,
            "-m",
            mode,
            text(origin)?,
            text(&staged)?,
        ],
    )?;
    Ok(staged)
}

fn replace_owner_file(staged: &Path, target: &Path) -> Result<bool> {
    let backup = backup_of(target)?;
    if fs::symlink_metadata(&backup).is_ok() {
        owned(&backup, false, OWNER_UID)?;
        fs::remove_file(&backup)?;
    }
    let existed = fs::symlink_metadata(target).is_ok();
    if existed {
        fs::hard_link(target, &backup)?;
    }
    fs::rename(staged, target)?;
    Ok(existed)
}

fn restore_owner_file(existed: bool, target: &Path) -> Result<()> {
    if existed {
        fs::rename(backup_of(target)?, target)?;
    } else {
        fs::remove_file(target)?;
    }
    Ok(())
}

fn discard_backup(target: &Path) -> Result<()> {
    match fs::remove_file(backup_of(target)?) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn reload_units() -> Result<()> {
    as_owner(&[SYSTEMCTL, "--user", "daemon-reload"]).map(drop)
}

fn restore_units(installed: &[(PathBuf, bool)]) -> Result<()> {
    every(
        installed
            .iter()
            .map(|(target, existed)| restore_owner_file(*existed, target))
            .collect(),
    )
}

fn replace_units(source: &Path, directory: &Path) -> Result<Vec<(PathBuf, bool)>> {
    let mut installed = Vec::new();
    for name in UNITS {
        let target = directory.join(name);
        let origin = source.join(SOURCE_UNIT_DIRECTORY).join(name);
        let replaced = stage_owner_file(&origin, &target, "0644")
            .and_then(|staged| replace_owner_file(&staged, &target));
        match replaced {
            Ok(existed) => installed.push((target, existed)),
            Err(error) => return Err(with_rollback(error, restore_units(&installed))),
        }
    }
    Ok(installed)
}

fn install_units(source: &Path) -> Result<Vec<(PathBuf, bool)>> {
    let installed = replace_units(source, Path::new(UNIT_DIRECTORY))?;
    if let Err(error) = reload_units() {
        let rollback = every(vec![restore_units(&installed), reload_units()]);
        return Err(with_rollback(error, rollback));
    }
    Ok(installed)
}

fn install_gateway_config(source: &Path, gateway: &Path, target: &Path) -> Result<bool> {
    let origin = source
        .join(SOURCE_CONFIG_DIRECTORY)
        .join(gateway_config_name());
    let staged = stage_owner_file(&origin, target, "0600")?;
    if let Err(error) = as_owner(&[text(gateway)?, "--check-config", "--config", text(&staged)?]) {
        let cleanup = fs::remove_file(&staged).map_err(Into::into);
        return Err(with_rollback(error, cleanup));
    }
    replace_owner_file(&staged, target)
}

struct HostFiles {
    units: Vec<(PathBuf, bool)>,
    config_existed: bool,
}

impl HostFiles {
    fn install(source: &Path, gateway: &Path) -> Result<Self> {
        let units = install_units(source)?;
        match install_gateway_config(source, gateway, &gateway_config()) {
            Ok(config_existed) => Ok(Self {
                units,
                config_existed,
            }),
            Err(error) => {
                let rollback = every(vec![restore_units(&units), reload_units()]);
                Err(with_rollback(error, rollback))
            }
        }
    }

    fn restore(&self) -> Result<()> {
        every(vec![
            restore_owner_file(self.config_existed, &gateway_config()),
            restore_units(&self.units),
            reload_units(),
        ])
    }

    fn keep(&self) -> Result<()> {
        discard_backup(&gateway_config())?;
        self.units
            .iter()
            .try_for_each(|(target, _)| discard_backup(target))
    }
}

fn provenance(sha: &str, checksums: [&str; 3], previous_sha: &str, base_name: &str) -> String {
    let rebuilt = REBUILT.map(|name| format!("\"{name}\"")).join(", ");
    let units = UNITS
        .map(|unit| unit.trim_end_matches(".service"))
        .join(" und ");
    let sums: String = REBUILT
        .iter()
        .zip(checksums)
        .map(|(name, checksum)| format!("{} = \"{checksum}\"\n", checksum_key(name)))
        .collect();
    format!(
        "source_sha = \"{sha}\"\nrebuilt_binaries = [{rebuilt}]\n{sums}retained_from_release = \"{previous_sha}\"\nretained_provenance = \"{base_name}\"\nrelease_scope = \"Discord-Bot, KI-Gateway und dessen Ausfallmelder aus demselben Quellstand; übrige Begleitbinaries unverändert übernommen; Gateway-Konfiguration und die Units {units} aus dem Repository; dieser Release wendet keine Migrationen an, Schemaänderungen laufen getrennt über dl-central-migrate\"\n"
    )
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(pattern: &str) -> Result<Self> {
        Ok(Self(PathBuf::from(run(
            "/usr/bin/mktemp",
            &["-d", pattern],
        )?)))
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Temporärer Releasepfad bleibt erhalten: {error}");
            }
        }
    }
}

fn activate(current: &Path, target: &Path, switch: &Path) -> Result<()> {
    let temporary = switch.join(CURRENT);
    symlink(target, &temporary)?;
    fs::rename(&temporary, current)?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct Provenance {
    source_sha: String,
    rebuilt_binaries: Vec<String>,
    retained_from_release: String,
    retained_provenance: String,
    #[serde(flatten)]
    further: BTreeMap<String, toml::Value>,
}

impl Provenance {
    fn records(&self, expected: &[(&str, &str); 3]) -> bool {
        expected.iter().all(|(name, checksum)| {
            self.further
                .get(&checksum_key(name))
                .and_then(toml::Value::as_str)
                == Some(*checksum)
        })
    }
}

fn unchanged(current: &Path, previous: &Path, previous_link: &Path) -> Result<()> {
    if current.canonicalize()? != previous || fs::read_link(current)? != previous_link {
        return Err("Der Ausgangsrelease hat sich vor dem Wechsel verändert".into());
    }
    Ok(())
}

fn reusable_candidate(
    candidate: &Path,
    previous: &Path,
    sha: &str,
    expected: &[(&str, &str); 3],
    uid: u32,
) -> Result<()> {
    owned(candidate, true, uid)?;
    for name in REBUILT {
        owned(&candidate.join(name), false, uid)?;
    }
    for name in carried_over() {
        owned(&candidate.join(name), false, uid)?;
    }
    let manifest: Provenance =
        toml::from_str(&fs::read_to_string(candidate.join(provenance_file()))?)?;
    if manifest.source_sha != sha
        || !manifest.records(expected)
        || manifest.rebuilt_binaries != REBUILT
        || !valid_hex(&manifest.retained_from_release, 40)
        || manifest.retained_provenance != base_provenance_name(&manifest.retained_from_release)
    {
        return Err(CANDIDATE_MISMATCH.into());
    }
    for (name, checksum) in expected {
        if hash(&candidate.join(name))? != *checksum {
            return Err(CANDIDATE_MISMATCH.into());
        }
    }
    owned(&candidate.join(&manifest.retained_provenance), false, uid)?;
    if previous != candidate
        && previous.file_name().and_then(|name| name.to_str())
            != Some(manifest.retained_from_release.as_str())
    {
        return Err(
            "Der vorhandene Release wurde für einen anderen Ausgangsstand vorbereitet".into(),
        );
    }
    for name in COMPANIONS {
        owned(&previous.join(name), false, uid)?;
        if hash(&candidate.join(name))? != hash(&previous.join(name))? {
            return Err(
                "Die Begleitbinaries des vorhandenen Releases passen nicht zum Ausgangsstand"
                    .into(),
            );
        }
    }
    Ok(())
}

fn refresh_assets(source: &Path, stage: &Path) -> Result<()> {
    let assets = stage.join("assets");
    if fs::symlink_metadata(&assets).is_ok() {
        fs::remove_dir_all(&assets)?;
    }
    run(
        "/usr/bin/cp",
        &[
            "-R",
            "--no-preserve=all",
            text(&source.join("assets"))?,
            text(&assets)?,
        ],
    )?;
    run("/usr/bin/chmod", &["-R", "u=rwX,go=rX", text(&assets)?])?;
    owned(&assets, true, fs::metadata(stage)?.uid())
}

fn install(source: &Path, sha: &str, artifacts: &[(&Path, &str); 3]) -> Result<()> {
    if run("/usr/bin/id", &["-u"])? != "0" {
        return Err("Der Releasewechsel benötigt root".into());
    }
    if !valid_hex(sha, 40)
        || !artifacts
            .iter()
            .all(|(_, checksum)| valid_hex(checksum, 64))
    {
        return Err("Ungültiger Commit oder Dateihash".into());
    }
    let source = source.canonicalize()?;
    if !source.starts_with(WORKTREES) {
        return Err("Die Quelle muss ein eigener Worktree sein".into());
    }
    for (artifact, _) in artifacts {
        if !fs::symlink_metadata(artifact)?.is_file() {
            return Err("Das Artefakt muss eine reguläre Datei sein".into());
        }
    }
    let expected = [0, 1, 2].map(|index| (REBUILT[index], artifacts[index].1));
    let root = Path::new(ROOT);
    let releases = root.join(RELEASES);
    for path in root.ancestors().filter(|path| path.parent().is_some()) {
        owned(path, true, 0)?;
    }
    owned(&releases, true, 0)?;
    let lock_path = root.join(".deploy.lock");
    owned(&lock_path, false, 0)?;
    let lock: File = OpenOptions::new().read(true).write(true).open(&lock_path)?;
    let deadline = Instant::now() + Duration::from_secs(900);
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => break,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(error) => return Err(error.into()),
        }
    }
    source_matches(&source, sha)?;
    let current = root.join(CURRENT);
    let metadata = fs::symlink_metadata(&current)?;
    if !metadata.file_type().is_symlink() || metadata.uid() != 0 {
        return Err("Unsicherer aktueller Releasezeiger".into());
    }
    let previous_link = fs::read_link(&current)?;
    let previous = current.canonicalize()?;
    if previous.parent() != Some(releases.as_path())
        || !previous
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| valid_hex(name, 40))
    {
        return Err("Der aktuelle Release liegt nicht im geschützten Releaseordner".into());
    }
    owned(&previous, true, 0)?;
    owned(&previous.join(BOT_BINARY), false, 0)?;
    for name in carried_over() {
        owned(&previous.join(name), false, 0)?;
    }
    let candidate = releases.join(sha);
    let switch = TemporaryDirectory::new(&format!("{ROOT}/.switch-discord-XXXXXXXX"))?;
    if candidate.try_exists()? || fs::symlink_metadata(&candidate).is_ok() {
        reusable_candidate(&candidate, &previous, sha, &expected, 0)?;
    } else {
        let stage =
            TemporaryDirectory::new(&format!("{ROOT}/{RELEASES}/.stage-discord-{sha}-XXXXXXXX"))?;
        run(
            "/usr/bin/cp",
            &["-a", text(&previous.join("."))?, text(&stage.0)?],
        )?;
        owned(&stage.0.join(BOT_BINARY), false, 0)?;
        for name in carried_over() {
            owned(&stage.0.join(name), false, 0)?;
        }
        let previous_sha = previous
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Ungültiger Ausgangsrelease")?;
        let base_name = base_provenance_name(previous_sha);
        let base = stage.0.join(&base_name);
        if fs::symlink_metadata(&base).is_ok() {
            return Err("Der Herkunftsnachweis des Ausgangsreleases existiert bereits".into());
        }
        fs::copy(stage.0.join(provenance_file()), &base)?;
        for ((artifact, checksum), name) in artifacts.iter().zip(REBUILT) {
            run(
                "/usr/bin/install",
                &[
                    "-o",
                    "root",
                    "-g",
                    "root",
                    "-m",
                    "0755",
                    text(artifact)?,
                    text(&stage.0.join(name))?,
                ],
            )?;
            if hash(&stage.0.join(name))? != *checksum {
                return Err(
                    "Das kopierte Binary stimmt nicht mit dem geprüften Artefakt überein".into(),
                );
            }
        }
        refresh_assets(&source, &stage.0)?;
        for name in COMPANIONS {
            if hash(&previous.join(name))? != hash(&stage.0.join(name))? {
                return Err("Ein beibehaltenes Begleitbinary wurde verändert".into());
            }
        }
        fs::write(
            stage.0.join(provenance_file()),
            provenance(
                sha,
                expected.map(|(_, checksum)| checksum),
                previous_sha,
                &base_name,
            ),
        )?;
        source_matches(&source, sha)?;
        unchanged(&current, &previous, &previous_link)?;
        fs::rename(&stage.0, &candidate)?;
    }
    source_matches(&source, sha)?;
    unchanged(&current, &previous, &previous_link)?;
    let host = HostFiles::install(&source, &candidate.join(GATEWAY_BINARY))?;
    let installed = match proof(&gateway_config()) {
        Ok(installed) => installed,
        Err(error) => return Err(with_rollback(error, host.restore())),
    };
    if let Err(error) = activate(&current, &Path::new(RELEASES).join(sha), &switch.0) {
        return Err(with_rollback(error, host.restore()));
    }
    if let Err(error) = restart_and_prove(&installed) {
        let rollback = every(vec![
            host.restore(),
            activate(&current, &previous_link, &switch.0),
            restart_and_prove_previous(&gateway_config(), &installed),
        ]);
        return Err(with_rollback(error, rollback));
    }
    host.keep()?;
    println!(
        "Discord-Release aktiviert: {sha}; {}",
        checksum_report(&expected)
    );
    Ok(())
}

fn checksum_report(expected: &[(&str, &str); 3]) -> String {
    expected
        .map(|(name, checksum)| format!("{name} SHA256 {checksum}"))
        .join("; ")
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [source, bot, sha, bot_hash, gateway, gateway_hash, notify, notify_hash] =
        arguments.as_slice()
    else {
        return Err(
            "Aufruf: install_discord_release QUELLE ARTEFAKT SHA SHA256 GATEWAY_ARTEFAKT GATEWAY_SHA256 MELDER_ARTEFAKT MELDER_SHA256"
                .into(),
        );
    };
    install(
        Path::new(source),
        sha,
        &[
            (Path::new(bot), bot_hash.as_str()),
            (Path::new(gateway), gateway_hash.as_str()),
            (Path::new(notify), notify_hash.as_str()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn commits_and_hashes_reject_paths_and_uppercase() {
        assert!(valid_hex(&"a".repeat(40), 40));
        assert!(valid_hex(&"1".repeat(64), 64));
        for value in ["../main", "MAIN", "a", "A".repeat(40).as_str()] {
            assert!(!valid_hex(value, 40));
        }
    }

    #[test]
    fn ownership_check_rejects_symlinks_and_wrong_file_types() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("file");
        fs::write(&file, b"fixture")?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;
        let uid = fs::metadata(&file)?.uid();
        owned(&file, false, uid)?;
        assert!(owned(&file, false, uid ^ 1).is_err());
        assert!(owned(&file, true, uid).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o620))?;
        assert!(owned(&file, false, uid).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;
        let link = directory.path().join("link");
        symlink(&file, &link)?;
        assert!(owned(&link, false, uid).is_err());
        Ok(())
    }

    struct CandidateFixture {
        directory: tempfile::TempDir,
        previous: PathBuf,
        candidate: PathBuf,
        sha: String,
        checksums: [String; 3],
        uid: u32,
    }

    impl CandidateFixture {
        fn expected(&self) -> [(&str, &str); 3] {
            [0, 1, 2].map(|index| (REBUILT[index], self.checksums[index].as_str()))
        }
    }

    fn candidate_fixture() -> Result<CandidateFixture> {
        let directory = tempfile::tempdir()?;
        let previous_sha = "b".repeat(40);
        let sha = "a".repeat(40);
        let previous = directory.path().join(&previous_sha);
        let candidate = directory.path().join(&sha);
        for path in [&previous, &candidate] {
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            for name in REBUILT.into_iter().chain(COMPANIONS) {
                fs::write(path.join(name), name.as_bytes())?;
                fs::set_permissions(path.join(name), fs::Permissions::from_mode(0o600))?;
            }
        }
        let checksums = [
            hash(&candidate.join(REBUILT[0]))?,
            hash(&candidate.join(REBUILT[1]))?,
            hash(&candidate.join(REBUILT[2]))?,
        ];
        let base_name = base_provenance_name(&previous_sha);
        fs::write(candidate.join(&base_name), "source_sha = 'fixture'\n")?;
        fs::set_permissions(
            candidate.join(&base_name),
            fs::Permissions::from_mode(0o600),
        )?;
        fs::write(
            candidate.join(provenance_file()),
            provenance(
                &sha,
                [0, 1, 2].map(|index| checksums[index].as_str()),
                &previous_sha,
                &base_name,
            ),
        )?;
        fs::set_permissions(
            candidate.join(provenance_file()),
            fs::Permissions::from_mode(0o600),
        )?;
        let uid = fs::metadata(&candidate)?.uid();
        Ok(CandidateFixture {
            directory,
            previous,
            candidate,
            sha,
            checksums,
            uid,
        })
    }

    #[test]
    fn failed_candidate_is_reusable_after_rollback() -> Result<()> {
        let fixture = candidate_fixture()?;
        let current = fixture.directory.path().join(CURRENT);
        let switch = fixture.directory.path().join("switch");
        fs::create_dir(&switch)?;
        symlink(&fixture.previous, &current)?;
        activate(&current, &fixture.candidate, &switch)?;
        activate(&current, &fixture.previous, &switch)?;
        reusable_candidate(
            &fixture.candidate,
            &fixture.previous,
            &fixture.sha,
            &fixture.expected(),
            fixture.uid,
        )?;
        reusable_candidate(
            &fixture.candidate,
            &fixture.candidate,
            &fixture.sha,
            &fixture.expected(),
            fixture.uid,
        )?;
        Ok(())
    }

    #[test]
    fn changed_binaries_are_not_reusable() -> Result<()> {
        let fixture = candidate_fixture()?;
        for name in REBUILT.into_iter().chain(COMPANIONS) {
            fs::write(fixture.candidate.join(name), b"changed")?;
            assert!(
                reusable_candidate(
                    &fixture.candidate,
                    &fixture.previous,
                    &fixture.sha,
                    &fixture.expected(),
                    fixture.uid
                )
                .is_err(),
                "{name}"
            );
            fs::write(fixture.candidate.join(name), name.as_bytes())?;
            reusable_candidate(
                &fixture.candidate,
                &fixture.previous,
                &fixture.sha,
                &fixture.expected(),
                fixture.uid,
            )?;
        }
        Ok(())
    }

    #[test]
    fn stage_assets_follow_the_source_revision() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source");
        let stage = directory.path().join("stage");
        fs::create_dir_all(source.join("assets/nested"))?;
        fs::create_dir_all(stage.join("assets"))?;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o755))?;
        fs::write(source.join("assets/paten_panel.toml"), b"new")?;
        fs::write(source.join("assets/nested/emoji.png"), b"png")?;
        fs::write(stage.join("assets/paten_panel.toml"), b"old")?;
        fs::write(stage.join("assets/stale.toml"), b"stale")?;
        refresh_assets(&source, &stage)?;
        assert_eq!(fs::read(stage.join("assets/paten_panel.toml"))?, b"new");
        assert_eq!(fs::read(stage.join("assets/nested/emoji.png"))?, b"png");
        assert!(!stage.join("assets/stale.toml").exists());
        Ok(())
    }

    #[test]
    fn activation_replaces_the_pointer_atomically() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let current = directory.path().join(CURRENT);
        let switch = directory.path().join("switch");
        fs::create_dir(&switch)?;
        let old = Path::new(RELEASES).join("old");
        let new = Path::new(RELEASES).join("new");
        symlink(&old, &current)?;
        activate(&current, &new, &switch)?;
        assert_eq!(fs::read_link(&current)?, new);
        activate(&current, &old, &switch)?;
        assert_eq!(fs::read_link(&current)?, old);
        Ok(())
    }

    #[test]
    fn rollback_restores_or_removes_the_gateway_config_and_unit() -> Result<()> {
        let directory = tempfile::tempdir()?;
        for name in [gateway_config_name(), GATEWAY_UNIT.to_owned()] {
            let target = directory.path().join(&name);
            fs::write(&target, b"old")?;
            fs::write(directory.path().join("new"), b"new")?;
            assert!(replace_owner_file(&directory.path().join("new"), &target)?);
            assert_eq!(fs::read(&target)?, b"new");
            assert_eq!(
                backup_of(&target)?,
                directory.path().join(format!(".{name}.previous"))
            );
            restore_owner_file(true, &target)?;
            assert_eq!(fs::read(&target)?, b"old");
            assert!(fs::symlink_metadata(backup_of(&target)?).is_err());
            discard_backup(&target)?;
            restore_owner_file(false, &target)?;
            assert!(fs::symlink_metadata(&target).is_err());
            fs::write(directory.path().join("new"), b"new")?;
            assert!(!replace_owner_file(&directory.path().join("new"), &target)?);
            assert!(fs::symlink_metadata(backup_of(&target)?).is_err());
        }
        Ok(())
    }

    #[test]
    fn rollback_attempts_every_step_and_keeps_the_first_error() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let missing = directory.path().join("missing.service");
        let kept = directory.path().join("kept.service");
        fs::write(&kept, b"old")?;
        fs::write(directory.path().join("new"), b"new")?;
        assert!(replace_owner_file(&directory.path().join("new"), &kept)?);
        let restored = restore_units(&[(missing, true), (kept.clone(), true)]);
        assert!(restored.is_err());
        assert_eq!(fs::read(&kept)?, b"old");
        let reported = with_rollback("Ursache".into(), restored).to_string();
        assert!(reported.starts_with("Ursache; Rücknahme unvollständig: "));
        assert_eq!(
            with_rollback("Ursache".into(), Ok(())).to_string(),
            "Ursache"
        );
        let both = every(vec![Err("eins".into()), Ok(()), Err("zwei".into())]);
        assert_eq!(
            both.map_err(|error| error.to_string()),
            Err("eins; zwei".to_owned())
        );
        Ok(())
    }

    fn repository() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    #[test]
    fn the_proof_takes_address_and_window_from_the_shipped_config() -> Result<()> {
        let shipped = repository()
            .join(SOURCE_CONFIG_DIRECTORY)
            .join(gateway_config_name());
        let written: SocketAddr = fs::read_to_string(&shipped)?
            .lines()
            .find_map(|line| line.strip_prefix("listen = "))
            .ok_or("listen fehlt")?
            .trim_matches('"')
            .parse()?;
        let config = GatewayConfig::load(&shipped)?;
        let planned = proof(&shipped)?;
        assert_eq!(planned.address, written);
        assert_eq!(
            planned.window,
            config.alarm.start_budget() + Duration::from_secs(SERVE_MARGIN_SECS)
        );
        assert!(planned.window > config.alarm.start_budget());
        assert_eq!(previous_proof(&shipped, &planned)?, planned);
        Ok(())
    }

    #[test]
    fn a_rollback_to_a_config_without_alarm_section_still_finds_the_old_listener() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let older = directory.path().join(gateway_config_name());
        let shipped = fs::read_to_string(
            repository()
                .join(SOURCE_CONFIG_DIRECTORY)
                .join(gateway_config_name()),
        )?;
        let (before_alarm, after_alarm) = shipped.split_once("[alarm]").ok_or("alarm fehlt")?;
        let (_, slots) = after_alarm.split_once("[slots.").ok_or("slots fehlen")?;
        fs::write(&older, format!("{before_alarm}[slots.{slots}"))?;
        assert!(proof(&older).is_err());
        let failed = Proof {
            address: "127.0.0.1:1".parse()?,
            window: Duration::from_secs(SERVE_MARGIN_SECS),
        };
        let previous = previous_proof(&older, &failed)?;
        assert_eq!(previous.address, GatewayConfig::listen_address(&older)?);
        assert_ne!(previous.address, failed.address);
        assert_eq!(previous.window, failed.window);
        fs::write(&older, "slots = []")?;
        assert!(previous_proof(&older, &failed).is_err());
        assert!(previous_proof(&directory.path().join("absent"), &failed).is_err());
        Ok(())
    }

    #[test]
    fn the_shipped_gateway_unit_starts_the_release_binary_through_the_launcher_with_the_installed_config(
    ) -> Result<()> {
        let unit = fs::read_to_string(repository().join(SOURCE_UNIT_DIRECTORY).join(GATEWAY_UNIT))?;
        let starts: Vec<&str> = unit
            .lines()
            .filter_map(|line| line.strip_prefix("ExecStart="))
            .collect();
        let [start] = starts.as_slice() else {
            return Err("genau ein ExecStart erwartet".into());
        };
        let release = Path::new(ROOT).join(CURRENT);
        let words: Vec<&str> = start.split(' ').collect();
        let launcher = release.join(LAUNCHER);
        let gateway = release.join(GATEWAY_BINARY);
        let config = gateway_config();
        assert_eq!(words.get(2).copied(), Some(text(&launcher)?));
        assert_eq!(
            words.rchunks(3).next(),
            Some(&[text(&gateway)?, "--config", text(&config)?][..])
        );
        assert_eq!(
            words.iter().filter(|word| **word == "--").count(),
            1,
            "{start}"
        );
        let working: Vec<&str> = unit
            .lines()
            .filter_map(|line| line.strip_prefix("WorkingDirectory="))
            .collect();
        assert_eq!(working, [text(&release)?]);
        Ok(())
    }

    type Answering = std::thread::JoinHandle<std::io::Result<()>>;

    fn answered(answer: Answering) -> Result<()> {
        Ok(answer.join().map_err(|_| "Antwortfaden abgebrochen")??)
    }

    fn answering_health_only() -> Result<(SocketAddr, Answering)> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let answer = std::thread::spawn(move || -> std::io::Result<()> {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept()?;
            let mut request = [0_u8; 512];
            let read = stream.read(&mut request)?;
            let asked = String::from_utf8_lossy(&request[..read]).into_owned();
            let status = if asked.starts_with(&format!("GET {HEALTH_PATH} ")) {
                "200 OK"
            } else {
                "404 Not Found"
            };
            stream.write_all(
                format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
                    .as_bytes(),
            )
        });
        Ok((address, answer))
    }

    #[test]
    fn the_probe_fails_on_a_closed_port_and_passes_only_on_the_health_path() -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        drop(listener);
        let closed = Proof {
            address,
            window: Duration::ZERO,
        };
        let refused = serves(&closed, Duration::ZERO);
        assert!(refused.is_err_and(|error| error.to_string().contains(HEALTH_PATH)));
        let (address, answer) = answering_health_only()?;
        let open = Proof {
            address,
            window: Duration::ZERO,
        };
        serves(&open, Duration::ZERO)?;
        answered(answer)?;
        let (address, answer) = answering_health_only()?;
        assert!(answers(&format!("http://{address}/")).is_err());
        answered(answer)
    }

    #[test]
    fn written_provenance_is_what_a_reused_candidate_is_checked_against() -> Result<()> {
        let written = provenance(
            &"a".repeat(40),
            ["1", "2", "3"],
            &"b".repeat(40),
            "BASE.toml",
        );
        let manifest: Provenance = toml::from_str(&written)?;
        assert_eq!(manifest.rebuilt_binaries, REBUILT);
        let expected = [0, 1, 2].map(|index| (REBUILT[index], ["1", "2", "3"][index]));
        assert!(manifest.records(&expected));
        let swapped = [0, 1, 2].map(|index| (REBUILT[index], ["2", "1", "3"][index]));
        assert!(!manifest.records(&swapped));
        assert_eq!(manifest.retained_provenance, "BASE.toml");
        assert_eq!(
            checksum_report(&expected),
            format!(
                "{} SHA256 1; {} SHA256 2; {} SHA256 3",
                REBUILT[0], REBUILT[1], REBUILT[2]
            )
        );
        Ok(())
    }
}
