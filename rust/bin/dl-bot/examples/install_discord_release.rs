use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    os::unix::{fs::symlink, fs::MetadataExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const ROOT: &str = "/opt/deadlock/bots";

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

fn restart() -> Result<()> {
    run(
        "/usr/sbin/runuser",
        &[
            "-u",
            "nathanael",
            "--",
            "/usr/bin/env",
            "XDG_RUNTIME_DIR=/run/user/1000",
            "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
            "/usr/local/bin/bot-restart",
            "dl-bot",
        ],
    )?;
    Ok(())
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

fn install(source: &Path, artifact: &Path, sha: &str, expected_hash: &str) -> Result<()> {
    if run("/usr/bin/id", &["-u"])? != "0" {
        return Err("Der Releasewechsel benötigt root".into());
    }
    if !valid_hex(sha, 40) || !valid_hex(expected_hash, 64) {
        return Err("Ungültiger Commit oder Dateihash".into());
    }
    let source = source.canonicalize()?;
    if !source.starts_with("/home/nathanael/.worktrees") {
        return Err("Die Quelle muss ein eigener Worktree sein".into());
    }
    if !fs::symlink_metadata(artifact)?.is_file() {
        return Err("Das Artefakt muss eine reguläre Datei sein".into());
    }
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
    if candidate.try_exists()? || fs::symlink_metadata(&candidate).is_ok() {
        return Err("Der Zielrelease existiert bereits".into());
    }
    let stage = TemporaryDirectory::new(&format!("{ROOT}/releases/.stage-discord-{sha}-XXXXXXXX"))?;
    let switch = TemporaryDirectory::new(&format!("{ROOT}/.switch-discord-XXXXXXXX"))?;
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
            text(&stage.0.join("dl-bot"))?,
        ],
    )?;
    if hash(&stage.0.join("dl-bot"))? != expected_hash {
        return Err("Der kopierte Bot stimmt nicht mit dem geprüften Artefakt überein".into());
    }
    for name in ["dl-web", "dl-infisical-env"] {
        if hash(&previous.join(name))? != hash(&stage.0.join(name))? {
            return Err("Ein beibehaltenes Begleitbinary wurde verändert".into());
        }
    }
    fs::write(
        stage.0.join("BUILD-PROVENANCE.toml"),
        format!(
            "source_sha = \"{sha}\"\nrebuilt_binaries = [\"dl-bot\"]\ndl_bot_sha256 = \"{expected_hash}\"\nretained_from_release = \"{previous_sha}\"\nretained_provenance = \"{base_name}\"\nrelease_scope = \"Discord Concierge und FAQ über den Brain-Aufgabenvertrag; Begleitbinaries unverändert übernommen; keine Konfigurations- oder Migrationsänderung\"\n"
        ),
    )?;
    source_matches(&source, sha)?;
    if current.canonicalize()? != previous || fs::read_link(&current)? != previous_link {
        return Err("Der Ausgangsrelease hat sich vor dem Wechsel verändert".into());
    }
    fs::rename(&stage.0, &candidate)?;
    activate(&current, &Path::new("releases").join(sha), &switch.0)?;
    if let Err(error) = restart() {
        activate(&current, &previous_link, &switch.0)?;
        if let Err(rollback_error) = restart() {
            eprintln!("Neustart nach Rücknahme fehlgeschlagen: {rollback_error}");
        }
        return Err(error);
    }
    println!("Discord-Release aktiviert: {sha}; dl-bot SHA256 {expected_hash}");
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [source, artifact, sha, expected_hash] = arguments.as_slice() else {
        return Err("Aufruf: install_discord_release QUELLE ARTEFAKT SHA SHA256".into());
    };
    install(Path::new(source), Path::new(artifact), sha, expected_hash)
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
}
