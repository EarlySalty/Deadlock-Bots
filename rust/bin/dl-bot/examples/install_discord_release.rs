use dl_ai::upstream::{GATEWAY_UNIT, HELPER_BINARY};
use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    net::SocketAddr,
    os::unix::{fs::symlink, fs::MetadataExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const ROOT: &str = "/opt/deadlock/bots";
const GATEWAY_CONFIG: &str = "/home/nathanael/.config/deadlock-bots/dl-ai-gateway.toml";
const UNIT_DIRECTORY: &str = "/home/nathanael/.config/systemd/user";
const UNITS: [&str; 2] = [GATEWAY_UNIT, "dl-community-points-sync.service"];
const OWNER_UID: u32 = 1000;
const REBUILT: [&str; 3] = ["dl-bot", "dl-ai-gateway", HELPER_BINARY];
const SERVE_ATTEMPTS: u32 = 60;
const SERVE_PAUSE_MILLIS: u64 = 1_000;
const SERVE_PROBE_SECS: &str = "2";

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
        let mut command = vec!["-u", "nathanael", "--", "/usr/bin/git", "-C", source];
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
    let mut arguments = vec![
        "-u",
        "nathanael",
        "--",
        "/usr/bin/env",
        "XDG_RUNTIME_DIR=/run/user/1000",
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
    ];
    arguments.extend_from_slice(command);
    run("/usr/sbin/runuser", &arguments)
}

fn restart() -> Result<()> {
    as_owner(&["/usr/local/bin/bot-restart", "dl-bot"])?;
    as_owner(&["/usr/bin/systemctl", "--user", "restart", GATEWAY_UNIT])?;
    Ok(())
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

#[derive(serde::Deserialize)]
struct Listen {
    listen: SocketAddr,
}

fn listen_address(config: &str) -> Result<SocketAddr> {
    Ok(toml::from_str::<Listen>(config)?.listen)
}

fn serves(address: SocketAddr, attempts: u32, pause: Duration) -> Result<()> {
    let url = format!("http://{address}/healthz");
    let mut last = String::new();
    for attempt in 0..attempts {
        if attempt > 0 {
            std::thread::sleep(pause);
        }
        let probe = [
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            SERVE_PROBE_SECS,
            url.as_str(),
        ];
        match run("/usr/bin/curl", &probe) {
            Ok(_) => return Ok(()),
            Err(error) => last = error.to_string(),
        }
    }
    Err(format!("Das KI-Gateway antwortet nach dem Neustart nicht auf {url}: {last}").into())
}

fn restart_and_prove() -> Result<()> {
    restart()?;
    let address = listen_address(&fs::read_to_string(GATEWAY_CONFIG)?)?;
    serves(
        address,
        SERVE_ATTEMPTS,
        Duration::from_millis(SERVE_PAUSE_MILLIS),
    )
}

fn backup_of(target: &Path) -> Result<PathBuf> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Ungültiger Zielpfad")?;
    let directory = target.parent().ok_or("Ungültiger Zielpfad")?;
    Ok(directory.join(format!(".{name}.previous")))
}

fn stage_owner_file(origin: &Path, target: &Path, mode: &str) -> Result<PathBuf> {
    if !fs::symlink_metadata(origin)?.is_file() {
        return Err(format!("Im Quellstand fehlt {}", origin.display()).into());
    }
    let directory = target.parent().ok_or("Ungültiger Zielpfad")?;
    owned(directory, true, OWNER_UID)?;
    if fs::symlink_metadata(target).is_ok() {
        owned(target, false, OWNER_UID)?;
    }
    let staged = backup_of(target)?.with_extension("install");
    run(
        "/usr/bin/install",
        &[
            "-o",
            "nathanael",
            "-g",
            "nathanael",
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
    as_owner(&["/usr/bin/systemctl", "--user", "daemon-reload"]).map(drop)
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
        let origin = source.join("service/systemd").join(name);
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
    let staged = stage_owner_file(&source.join("config/dl-ai-gateway.toml"), target, "0600")?;
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
        match install_gateway_config(source, gateway, Path::new(GATEWAY_CONFIG)) {
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
            restore_owner_file(self.config_existed, Path::new(GATEWAY_CONFIG)),
            restore_units(&self.units),
            reload_units(),
        ])
    }

    fn keep(&self) -> Result<()> {
        discard_backup(Path::new(GATEWAY_CONFIG))?;
        self.units
            .iter()
            .try_for_each(|(target, _)| discard_backup(target))
    }
}

fn provenance(sha: &str, checksums: [&str; 3], previous_sha: &str, base_name: &str) -> String {
    let rebuilt = REBUILT.map(|name| format!("\"{name}\"")).join(", ");
    format!(
        "source_sha = \"{sha}\"\nrebuilt_binaries = [{rebuilt}]\ndl_bot_sha256 = \"{}\"\ndl_ai_gateway_sha256 = \"{}\"\ndl_unit_failure_notify_sha256 = \"{}\"\nretained_from_release = \"{previous_sha}\"\nretained_provenance = \"{base_name}\"\nrelease_scope = \"Discord-Bot, KI-Gateway und dessen Ausfallmelder aus demselben Quellstand; übrige Begleitbinaries unverändert übernommen; Gateway-Konfiguration und die Units dl-ai-gateway und dl-community-points-sync aus dem Repository; dieser Release wendet keine Migrationen an, Schemaänderungen laufen getrennt über dl-central-migrate\"\n",
        checksums[0], checksums[1], checksums[2]
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
    let temporary = switch.join("current");
    symlink(target, &temporary)?;
    fs::rename(&temporary, current)?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct Provenance {
    source_sha: String,
    rebuilt_binaries: Vec<String>,
    dl_bot_sha256: String,
    dl_ai_gateway_sha256: String,
    dl_unit_failure_notify_sha256: String,
    retained_from_release: String,
    retained_provenance: String,
}

fn reusable_candidate(
    candidate: &Path,
    previous: &Path,
    sha: &str,
    expected: &[(&str, &str); 3],
    uid: u32,
) -> Result<()> {
    owned(candidate, true, uid)?;
    let retained = ["dl-web", "dl-infisical-env", "BUILD-PROVENANCE.toml"];
    for name in REBUILT.into_iter().chain(retained) {
        owned(&candidate.join(name), false, uid)?;
    }
    let manifest: Provenance = toml::from_str(&fs::read_to_string(
        candidate.join("BUILD-PROVENANCE.toml"),
    )?)?;
    if manifest.source_sha != sha
        || manifest.dl_bot_sha256 != expected[0].1
        || manifest.dl_ai_gateway_sha256 != expected[1].1
        || manifest.dl_unit_failure_notify_sha256 != expected[2].1
        || manifest.rebuilt_binaries != REBUILT
        || !valid_hex(&manifest.retained_from_release, 40)
        || manifest.retained_provenance
            != format!(
                "BASE-BUILD-PROVENANCE-{}.toml",
                manifest.retained_from_release
            )
    {
        return Err("Der vorhandene Release passt nicht zum geprüften Artefakt".into());
    }
    for (name, checksum) in expected {
        if hash(&candidate.join(name))? != *checksum {
            return Err("Der vorhandene Release passt nicht zum geprüften Artefakt".into());
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
    for name in ["dl-web", "dl-infisical-env"] {
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
    if !source.starts_with("/home/nathanael/.worktrees") {
        return Err("Die Quelle muss ein eigener Worktree sein".into());
    }
    for (artifact, _) in artifacts {
        if !fs::symlink_metadata(artifact)?.is_file() {
            return Err("Das Artefakt muss eine reguläre Datei sein".into());
        }
    }
    let expected = [
        (REBUILT[0], artifacts[0].1),
        (REBUILT[1], artifacts[1].1),
        (REBUILT[2], artifacts[2].1),
    ];
    for path in ["/opt", "/opt/deadlock", ROOT, "/opt/deadlock/bots/releases"] {
        owned(Path::new(path), true, 0)?;
    }
    let root = Path::new(ROOT);
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
    let current = root.join("current");
    let metadata = fs::symlink_metadata(&current)?;
    if !metadata.file_type().is_symlink() || metadata.uid() != 0 {
        return Err("Unsicherer aktueller Releasezeiger".into());
    }
    let previous_link = fs::read_link(&current)?;
    let previous = current.canonicalize()?;
    if previous.parent() != Some(root.join("releases").as_path())
        || !previous
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| valid_hex(name, 40))
    {
        return Err("Der aktuelle Release liegt nicht im geschützten Releaseordner".into());
    }
    owned(&previous, true, 0)?;
    for name in [
        "dl-bot",
        "dl-web",
        "dl-infisical-env",
        "BUILD-PROVENANCE.toml",
    ] {
        owned(&previous.join(name), false, 0)?;
    }
    let candidate = root.join("releases").join(sha);
    let switch = TemporaryDirectory::new(&format!("{ROOT}/.switch-discord-XXXXXXXX"))?;
    if candidate.try_exists()? || fs::symlink_metadata(&candidate).is_ok() {
        reusable_candidate(&candidate, &previous, sha, &expected, 0)?;
    } else {
        let stage =
            TemporaryDirectory::new(&format!("{ROOT}/releases/.stage-discord-{sha}-XXXXXXXX"))?;
        run(
            "/usr/bin/cp",
            &["-a", text(&previous.join("."))?, text(&stage.0)?],
        )?;
        for name in [
            "dl-bot",
            "dl-web",
            "dl-infisical-env",
            "BUILD-PROVENANCE.toml",
        ] {
            owned(&stage.0.join(name), false, 0)?;
        }
        let previous_sha = previous
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Ungültiger Ausgangsrelease")?;
        let base_name = format!("BASE-BUILD-PROVENANCE-{previous_sha}.toml");
        let base = stage.0.join(&base_name);
        if fs::symlink_metadata(&base).is_ok() {
            return Err("Der Herkunftsnachweis des Ausgangsreleases existiert bereits".into());
        }
        fs::copy(stage.0.join("BUILD-PROVENANCE.toml"), &base)?;
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
        for name in ["dl-web", "dl-infisical-env"] {
            if hash(&previous.join(name))? != hash(&stage.0.join(name))? {
                return Err("Ein beibehaltenes Begleitbinary wurde verändert".into());
            }
        }
        fs::write(
            stage.0.join("BUILD-PROVENANCE.toml"),
            provenance(
                sha,
                [artifacts[0].1, artifacts[1].1, artifacts[2].1],
                previous_sha,
                &base_name,
            ),
        )?;
        source_matches(&source, sha)?;
        if current.canonicalize()? != previous || fs::read_link(&current)? != previous_link {
            return Err("Der Ausgangsrelease hat sich vor dem Wechsel verändert".into());
        }
        fs::rename(&stage.0, &candidate)?;
    }
    source_matches(&source, sha)?;
    if current.canonicalize()? != previous || fs::read_link(&current)? != previous_link {
        return Err("Der Ausgangsrelease hat sich vor dem Wechsel verändert".into());
    }
    let host = HostFiles::install(&source, &candidate.join(REBUILT[1]))?;
    if let Err(error) = activate(&current, &Path::new("releases").join(sha), &switch.0) {
        return Err(with_rollback(error, host.restore()));
    }
    if let Err(error) = restart_and_prove() {
        let rollback = every(vec![
            host.restore(),
            activate(&current, &previous_link, &switch.0),
            restart_and_prove(),
        ]);
        return Err(with_rollback(error, rollback));
    }
    host.keep()?;
    println!(
        "Discord-Release aktiviert: {sha}; dl-bot SHA256 {}; dl-ai-gateway SHA256 {}; dl-unit-failure-notify SHA256 {}",
        artifacts[0].1, artifacts[1].1, artifacts[2].1
    );
    Ok(())
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
            for name in REBUILT.into_iter().chain(["dl-web", "dl-infisical-env"]) {
                fs::write(path.join(name), name.as_bytes())?;
                fs::set_permissions(path.join(name), fs::Permissions::from_mode(0o600))?;
            }
        }
        let checksums = [
            hash(&candidate.join(REBUILT[0]))?,
            hash(&candidate.join(REBUILT[1]))?,
            hash(&candidate.join(REBUILT[2]))?,
        ];
        let base_name = format!("BASE-BUILD-PROVENANCE-{previous_sha}.toml");
        fs::write(candidate.join(&base_name), "source_sha = 'fixture'\n")?;
        fs::set_permissions(
            candidate.join(&base_name),
            fs::Permissions::from_mode(0o600),
        )?;
        fs::write(
            candidate.join("BUILD-PROVENANCE.toml"),
            provenance(
                &sha,
                [0, 1, 2].map(|index| checksums[index].as_str()),
                &previous_sha,
                &base_name,
            ),
        )?;
        fs::set_permissions(
            candidate.join("BUILD-PROVENANCE.toml"),
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
        let current = fixture.directory.path().join("current");
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
        fs::write(fixture.candidate.join("dl-bot"), b"changed")?;
        assert!(reusable_candidate(
            &fixture.candidate,
            &fixture.previous,
            &fixture.sha,
            &fixture.expected(),
            fixture.uid
        )
        .is_err());
        fs::write(fixture.candidate.join("dl-bot"), b"dl-bot")?;
        fs::write(fixture.candidate.join("dl-ai-gateway"), b"changed")?;
        assert!(reusable_candidate(
            &fixture.candidate,
            &fixture.previous,
            &fixture.sha,
            &fixture.expected(),
            fixture.uid
        )
        .is_err());
        fs::write(fixture.candidate.join("dl-ai-gateway"), b"dl-ai-gateway")?;
        fs::write(fixture.candidate.join("dl-unit-failure-notify"), b"changed")?;
        assert!(reusable_candidate(
            &fixture.candidate,
            &fixture.previous,
            &fixture.sha,
            &fixture.expected(),
            fixture.uid
        )
        .is_err());
        fs::write(
            fixture.candidate.join("dl-unit-failure-notify"),
            b"dl-unit-failure-notify",
        )?;
        fs::write(fixture.candidate.join("dl-web"), b"changed")?;
        assert!(reusable_candidate(
            &fixture.candidate,
            &fixture.previous,
            &fixture.sha,
            &fixture.expected(),
            fixture.uid
        )
        .is_err());
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
        let current = directory.path().join("current");
        let switch = directory.path().join("switch");
        fs::create_dir(&switch)?;
        symlink("releases/old", &current)?;
        activate(&current, Path::new("releases/new"), &switch)?;
        assert_eq!(fs::read_link(&current)?, PathBuf::from("releases/new"));
        activate(&current, Path::new("releases/old"), &switch)?;
        assert_eq!(fs::read_link(&current)?, PathBuf::from("releases/old"));
        Ok(())
    }

    #[test]
    fn rollback_restores_or_removes_the_gateway_config_and_unit() -> Result<()> {
        let directory = tempfile::tempdir()?;
        for name in ["dl-ai-gateway.toml", "dl-ai-gateway.service"] {
            let target = directory.path().join(name);
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

    #[test]
    fn the_probe_targets_the_listen_address_of_the_shipped_config() -> Result<()> {
        let shipped = include_str!("../../../../config/dl-ai-gateway.toml");
        assert!(listen_address(shipped)?.ip().is_loopback());
        assert!(listen_address("slots = []").is_err());
        Ok(())
    }

    #[test]
    fn a_gateway_that_never_binds_fails_the_release() -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        drop(listener);
        let refused = serves(address, 2, Duration::ZERO);
        assert!(refused.is_err_and(|error| error.to_string().contains("/healthz")));
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let answer = std::thread::spawn(move || -> std::io::Result<()> {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept()?;
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request)?;
            stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
        });
        serves(address, 1, Duration::ZERO)?;
        answer.join().map_err(|_| "Antwortfaden abgebrochen")??;
        Ok(())
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
        assert_eq!(
            [
                manifest.dl_bot_sha256.as_str(),
                manifest.dl_ai_gateway_sha256.as_str(),
                manifest.dl_unit_failure_notify_sha256.as_str(),
            ],
            ["1", "2", "3"]
        );
        assert_eq!(manifest.retained_provenance, "BASE.toml");
        Ok(())
    }
}
