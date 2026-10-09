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

#[derive(serde::Deserialize)]
struct Provenance {
    source_sha: String,
    rebuilt_binaries: Vec<String>,
    dl_bot_sha256: String,
    retained_from_release: String,
    retained_provenance: String,
}

fn reusable_candidate(
    candidate: &Path,
    previous: &Path,
    sha: &str,
    expected_hash: &str,
    uid: u32,
) -> Result<()> {
    owned(candidate, true, uid)?;
    for name in [
        "dl-bot",
        "dl-web",
        "dl-infisical-env",
        "BUILD-PROVENANCE.toml",
    ] {
        owned(&candidate.join(name), false, uid)?;
    }
    let manifest: Provenance = toml::from_str(&fs::read_to_string(
        candidate.join("BUILD-PROVENANCE.toml"),
    )?)?;
    if manifest.source_sha != sha
        || manifest.dl_bot_sha256 != expected_hash
        || manifest.rebuilt_binaries.len() != 1
        || manifest.rebuilt_binaries[0] != "dl-bot"
        || !valid_hex(&manifest.retained_from_release, 40)
        || manifest.retained_provenance
            != format!(
                "BASE-BUILD-PROVENANCE-{}.toml",
                manifest.retained_from_release
            )
        || hash(&candidate.join("dl-bot"))? != expected_hash
    {
        return Err("Der vorhandene Release passt nicht zum geprüften Artefakt".into());
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
    let switch = TemporaryDirectory::new(&format!("{ROOT}/.switch-discord-XXXXXXXX"))?;
    if candidate.try_exists()? || fs::symlink_metadata(&candidate).is_ok() {
        reusable_candidate(&candidate, &previous, sha, expected_hash, 0)?;
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
    }
    source_matches(&source, sha)?;
    if current.canonicalize()? != previous || fs::read_link(&current)? != previous_link {
        return Err("Der Ausgangsrelease hat sich vor dem Wechsel verändert".into());
    }
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

    struct CandidateFixture {
        directory: tempfile::TempDir,
        previous: PathBuf,
        candidate: PathBuf,
        sha: String,
        checksum: String,
        uid: u32,
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
            for name in ["dl-bot", "dl-web", "dl-infisical-env"] {
                fs::write(path.join(name), name.as_bytes())?;
                fs::set_permissions(path.join(name), fs::Permissions::from_mode(0o600))?;
            }
        }
        let checksum = hash(&candidate.join("dl-bot"))?;
        let base_name = format!("BASE-BUILD-PROVENANCE-{previous_sha}.toml");
        fs::write(candidate.join(&base_name), "source_sha = 'fixture'\n")?;
        fs::set_permissions(
            candidate.join(&base_name),
            fs::Permissions::from_mode(0o600),
        )?;
        fs::write(
            candidate.join("BUILD-PROVENANCE.toml"),
            format!("source_sha = '{sha}'\nrebuilt_binaries = ['dl-bot']\ndl_bot_sha256 = '{checksum}'\nretained_from_release = '{previous_sha}'\nretained_provenance = '{base_name}'\n"),
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
            checksum,
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
            &fixture.checksum,
            fixture.uid,
        )?;
        reusable_candidate(
            &fixture.candidate,
            &fixture.candidate,
            &fixture.sha,
            &fixture.checksum,
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
            &fixture.checksum,
            fixture.uid
        )
        .is_err());
        fs::write(fixture.candidate.join("dl-bot"), b"dl-bot")?;
        fs::write(fixture.candidate.join("dl-web"), b"changed")?;
        assert!(reusable_candidate(
            &fixture.candidate,
            &fixture.previous,
            &fixture.sha,
            &fixture.checksum,
            fixture.uid
        )
        .is_err());
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
