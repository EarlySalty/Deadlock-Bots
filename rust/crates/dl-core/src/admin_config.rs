//! Registry-basierter Editor ausschließlich für nicht geheime Bot-TOMLs.
//! Browser wählen IDs, niemals Dateipfade, Prüfprogramme oder systemd-Units.
use crate::operating_config::digest;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

pub const MAX_BYTES: usize = 256 * 1024;
const IDS: &[&str] = &[
    "discord",
    "steam",
    "patchnotes",
    "brain",
    "turniere",
    "twitch",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Diese Bot-Konfiguration ist noch nicht für das Dashboard eingerichtet.")]
    Unavailable,
    #[error("Unbekannte Bot-Konfiguration.")]
    NotFound,
    #[error("Die Datei wurde inzwischen geändert. Dein Entwurf bleibt erhalten. Bitte den aktuellen Stand neu laden.")]
    Conflict,
    #[error("Die Konfiguration wird gerade bearbeitet oder aktiviert. Bitte erneut versuchen.")]
    Busy,
    #[error("Die Konfiguration darf höchstens 256 KiB groß sein.")]
    TooLarge,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("Die Konfiguration konnte nicht sicher gelesen oder gespeichert werden.")]
    Io,
    #[error("Die Datei wurde ersetzt, ihre dauerhafte Speicherung konnte aber nicht bestätigt werden. Bitte neu laden.")]
    Durability,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dashboard {
    Discord,
    Twitch,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Validator {
    Discord,
    Command,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    pub apply_binary: PathBuf,
    pub bots: Vec<Target>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub title: String,
    pub dashboard: Dashboard,
    pub path: PathBuf,
    pub validator: Validator,
    #[serde(default)]
    pub program: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    pub units: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub note: String,
    /// Struktur- und Sicherheitsfelder bleiben bei optionalen Fremdschemata gesperrt.
    #[serde(default)]
    pub protected: Vec<String>,
}

#[derive(Serialize)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub enabled: bool,
    pub available: bool,
    pub note: String,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub id: String,
    pub title: String,
    pub revision: String,
    pub toml: String,
    pub history: Vec<String>,
    pub protected: Vec<String>,
}

#[derive(Serialize)]
pub struct Preview {
    pub revision: String,
    pub changed: Vec<String>,
}

pub fn valid_revision(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_bounded(path: &Path, limit: usize) -> Result<String, Error> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| Error::Io)?;
    let meta = file.metadata().map_err(|_| Error::Io)?;
    if !meta.is_file() {
        return Err(Error::Io);
    }
    if meta.len() > limit as u64 {
        return Err(Error::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    if bytes.len() > limit {
        return Err(Error::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid("Die Datei muss gültiges UTF-8 enthalten."))
}

pub fn allowed_units(id: &str) -> &'static [&'static str] {
    match id {
        "discord" => &[
            "deadlock-bot-rust.service",
            "deadlock-web-rust.service",
            "dl-knowledge.service",
        ],
        "steam" => &[
            "steam-core.service",
            "steam-core-2.service",
            "steam-bot.service",
        ],
        "patchnotes" => &["deadlock-patchnotes.service"],
        "brain" => &[
            "deadlock-brain.service",
            "deadlock-brain-api.service",
            "deadlock-brain-site.service",
        ],
        "turniere" => &["deadlock-turniere.service"],
        "twitch" => &[
            "deadlock-twitch-bot-rust.service",
            "deadlock-twitch-dashboard-rust.service",
        ],
        _ => &[],
    }
}

impl Registry {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = read_bounded(path, 64 * 1024).map_err(|_| Error::Unavailable)?;
        let mut registry: Self = toml::from_str(&text).map_err(|_| Error::Unavailable)?;
        if registry.schema_version != 1
            || registry.bots.len() > IDS.len()
            || !registry.apply_binary.is_absolute()
        {
            return Err(Error::Unavailable);
        }
        let parent = path.parent().ok_or(Error::Unavailable)?;
        let mut seen = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for target in &mut registry.bots {
            if !IDS.contains(&target.id.as_str())
                || !seen.insert(target.id.clone())
                || target.title.is_empty()
                || target.title.len() > 80
                || target.note.len() > 500
                || (target.dashboard == Dashboard::Twitch) != (target.id == "twitch")
                || target.units.is_empty()
                || target.units.len() > 3
                || target
                    .units
                    .iter()
                    .any(|unit| !allowed_units(&target.id).contains(&unit.as_str()))
                || target.units.iter().collect::<BTreeSet<_>>().len() != target.units.len()
            {
                return Err(Error::Unavailable);
            }
            if target.path.is_relative() {
                target.path = parent.join(&target.path);
            }
            if target.path.file_name().and_then(|s| s.to_str()) != Some("bot.toml")
                || !paths.insert(
                    target
                        .path
                        .canonicalize()
                        .unwrap_or_else(|_| target.path.clone()),
                )
            {
                return Err(Error::Unavailable);
            }
            match target.validator {
                Validator::Discord
                    if target.id != "discord"
                        || target.program.is_some()
                        || !target.args.is_empty() =>
                {
                    return Err(Error::Unavailable)
                }
                Validator::Command
                    if !target.program.as_ref().is_some_and(|p| p.is_absolute())
                        || target
                            .args
                            .iter()
                            .filter(|arg| arg.as_str() == "{config}")
                            .count()
                            != 1
                        || target.args.len() > 12
                        || target.args.iter().any(|arg| arg.len() > 1024) =>
                {
                    return Err(Error::Unavailable);
                }
                _ => {}
            }
        }
        Ok(registry)
    }

    pub fn target(&self, id: &str, dashboard: Dashboard) -> Result<Target, Error> {
        self.bots
            .iter()
            .find(|target| target.id == id && target.dashboard == dashboard)
            .cloned()
            .ok_or(Error::NotFound)
    }

    pub fn entries(&self, dashboard: Dashboard) -> Vec<Entry> {
        self.bots
            .iter()
            .filter(|target| target.dashboard == dashboard)
            .map(|target| Entry {
                id: target.id.clone(),
                title: target.title.clone(),
                enabled: target.enabled,
                available: target.enabled && target.safe_source().is_ok(),
                note: target.note.clone(),
            })
            .collect()
    }
}

fn no_secrets(value: &toml::Value, depth: usize) -> Result<(), Error> {
    if depth > 24 {
        return Err(Error::Invalid(
            "Die TOML-Struktur ist zu tief verschachtelt.",
        ));
    }
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let name = key.to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "token"
                        | "secret"
                        | "password"
                        | "private_key"
                        | "api_key"
                        | "dsn"
                        | "credentials"
                ) || name.ends_with("_secret")
                    || name.ends_with("_password")
                    || name.ends_with("_api_key")
                    || name.ends_with("_private_key")
                    || name.ends_with("_dsn")
                    || name.ends_with("_token")
                {
                    return Err(Error::Invalid("Zugangsdaten gehören ausschließlich nach Infisical, nicht in den TOML-Editor."));
                }
                no_secrets(value, depth + 1)?;
            }
        }
        toml::Value::Array(array) => {
            for item in array {
                no_secrets(item, depth + 1)?;
            }
        }
        toml::Value::String(text) => {
            if let Ok(url) = url::Url::parse(text) {
                if !url.username().is_empty() || url.password().is_some() {
                    return Err(Error::Invalid(
                        "Adressen mit eingebetteten Zugangsdaten sind nicht erlaubt.",
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn parse(text: &str) -> Result<toml::Value, Error> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let value: toml::Value = toml::from_str(text).map_err(|_| {
        Error::Invalid(
            "Ungültiges TOML: Bitte Anführungszeichen, Werte und doppelte Schlüssel prüfen.",
        )
    })?;
    no_secrets(&value, 0)?;
    Ok(value)
}

fn select<'a>(value: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    path.split('.').try_fold(value, |node, key| node.get(key))
}

fn changes(old: &toml::Value, new: &toml::Value, path: &str, result: &mut Vec<String>) {
    if old == new {
        return;
    }
    if let (Some(a), Some(b)) = (old.as_table(), new.as_table()) {
        let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
        for key in keys {
            let child = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            match (a.get(key), b.get(key)) {
                (Some(left), Some(right)) => changes(left, right, &child, result),
                _ => result.push(child),
            }
        }
    } else {
        result.push(path.to_owned());
    }
}

/// Unveränderte Kommentare und Feldreihenfolgen bleiben auf dem Server erhalten.
fn merge_document(original: &str, candidate: &str) -> Result<String, Error> {
    fn merge(
        old: &mut toml_edit::Table,
        new: &toml_edit::Table,
        before: &toml::Value,
        after: &toml::Value,
    ) {
        let removed: Vec<_> = old
            .iter()
            .filter(|(key, _)| !new.contains_key(key))
            .map(|(key, _)| key.to_owned())
            .collect();
        for key in removed {
            old.remove(&key);
        }
        for (key, next) in new.iter() {
            if before.get(key) == after.get(key) {
                continue;
            }
            if let Some(current) = old.get_mut(key) {
                if let (Some(a), Some(b)) = (current.as_table_mut(), next.as_table()) {
                    merge(
                        a,
                        b,
                        before.get(key).unwrap_or(before),
                        after.get(key).unwrap_or(after),
                    );
                } else {
                    let mut replacement = next.clone();
                    if let (Some(previous), Some(replacement)) =
                        (current.as_value(), replacement.as_value_mut())
                    {
                        // Display-Texte enthalten Decor. Werte semantisch vergleichen, damit
                        // bloße Formatierung im Browser nicht Originalkommentare entfernt.
                        if previous.to_string().trim() == replacement.to_string().trim() {
                            continue;
                        }
                        *replacement.decor_mut() = previous.decor().clone();
                    }
                    *current = replacement;
                }
            } else {
                old.insert(key, next.clone());
            }
        }
    }
    let mut old = original
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| Error::Io)?;
    let new = candidate
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| Error::Invalid("Ungültiges TOML."))?;
    let before = parse(original)?;
    let after = parse(candidate)?;
    merge(old.as_table_mut(), new.as_table(), &before, &after);
    Ok(old.to_string())
}

struct Temporary {
    path: PathBuf,
    file: File,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn temporary(source: &Path, text: &str) -> Result<Temporary, Error> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = source.parent().ok_or(Error::Io)?;
    let path = parent.join(format!(
        ".bot-config.{}.{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut temporary = Temporary {
        file: options.open(&path).map_err(|_| Error::Io)?,
        path,
    };
    temporary
        .file
        .write_all(text.as_bytes())
        .map_err(|_| Error::Io)?;
    temporary.file.sync_all().map_err(|_| Error::Io)?;
    Ok(temporary)
}

pub fn run_quiet(program: &Path, args: &[String], timeout: Duration) -> Result<bool, Error> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Unavailable)?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|_| Error::Io)? {
            return Ok(status.success());
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Unavailable);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

impl Target {
    pub fn safe_source(&self) -> Result<PathBuf, Error> {
        if !self.enabled {
            return Err(Error::Unavailable);
        }
        let meta = fs::symlink_metadata(&self.path).map_err(|_| Error::Unavailable)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(Error::Unavailable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() != 1 {
                return Err(Error::Unavailable);
            }
        }
        let source = self.path.canonicalize().map_err(|_| Error::Unavailable)?;
        if source.ancestors().any(|path| path.join(".git").exists()) {
            return Err(Error::Unavailable);
        }
        Ok(source)
    }

    pub fn lock(&self, source: &Path) -> Result<File, Error> {
        let parent = source.parent().ok_or(Error::Io)?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        // Bestehende native Schreiber verwenden bei Steam keinen führenden Punkt.
        let lock_name = if self.id == "steam" {
            "bot.toml.lock"
        } else {
            ".bot.toml.lock"
        };
        let lock = options
            .open(parent.join(lock_name))
            .map_err(|_| Error::Io)?;
        if !lock.metadata().map_err(|_| Error::Io)?.is_file() {
            return Err(Error::Io);
        }
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| Error::Busy)?;
        Ok(lock)
    }

    fn validate(&self, source: &Path, text: &str) -> Result<(), Error> {
        parse(text)?;
        match self.validator {
            Validator::Discord => {
                crate::bot_config::BotConfig::parse(text)
                    .map_err(|_| Error::Invalid("Die Datei entspricht nicht dem Discord-Konfigurationsschema oder enthält ungültige Werte."))?;
            }
            Validator::Command => {
                let candidate = temporary(source, text)?;
                let args: Vec<_> = self
                    .args
                    .iter()
                    .map(|arg| {
                        if arg == "{config}" {
                            candidate.path.to_string_lossy().into_owned()
                        } else {
                            arg.clone()
                        }
                    })
                    .collect();
                let success = run_quiet(
                    self.program.as_deref().ok_or(Error::Unavailable)?,
                    &args,
                    Duration::from_secs(15),
                )?;
                if !success {
                    return Err(Error::Invalid("Die Datei wurde vom Konfigurationsprüfer dieses Bots abgewiesen. Bitte Werte, Typen und Grenzen prüfen."));
                }
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        let source = self.safe_source()?;
        let text = read_bounded(&source, MAX_BYTES)?;
        self.validate(&source, &text)?;
        self.make_snapshot(&source, &text)
    }

    fn make_snapshot(&self, source: &Path, text: &str) -> Result<Snapshot, Error> {
        let value = parse(text)?;
        Ok(Snapshot {
            id: self.id.clone(),
            title: self.title.clone(),
            revision: digest(text.as_bytes()),
            toml: toml::to_string_pretty(&value).map_err(|_| Error::Io)?,
            history: self.history(source),
            protected: self.protected.clone(),
        })
    }

    fn candidate(
        &self,
        source: &Path,
        expected: &str,
        text: &str,
    ) -> Result<(String, String, Vec<String>), Error> {
        if !valid_revision(expected) {
            return Err(Error::Invalid(
                "Der gespeicherte Stand fehlt. Bitte neu laden.",
            ));
        }
        let original = read_bounded(source, MAX_BYTES)?;
        if digest(original.as_bytes()) != expected {
            return Err(Error::Conflict);
        }
        let old = parse(&original)?;
        let new = parse(text)?;
        if old.get("schema_version") != new.get("schema_version") {
            return Err(Error::Invalid(
                "Die Schema-Version darf nicht im Dashboard geändert werden.",
            ));
        }
        for key in &self.protected {
            if select(&old, key) != select(&new, key) {
                return Err(Error::Invalid(
                    "Ein geschütztes Infrastruktur- oder Sicherheitsfeld wurde verändert.",
                ));
            }
        }
        let mut changed = Vec::new();
        changes(&old, &new, "", &mut changed);
        let merged = if changed.is_empty() {
            original.clone()
        } else {
            merge_document(&original, text)?
        };
        if parse(&merged)? != new {
            return Err(Error::Io);
        }
        self.validate(source, &merged)?;
        Ok((original, merged, changed))
    }

    pub fn preview(&self, expected: &str, text: &str) -> Result<Preview, Error> {
        let source = self.safe_source()?;
        let _lock = self.lock(&source)?;
        let (_, _, changed) = self.candidate(&source, expected, text)?;
        Ok(Preview {
            revision: expected.to_owned(),
            changed,
        })
    }

    pub fn save(&self, expected: &str, text: &str) -> Result<Snapshot, Error> {
        let source = self.safe_source()?;
        let _lock = self.lock(&source)?;
        let (original, merged, changed) = self.candidate(&source, expected, text)?;
        if changed.is_empty() {
            return self.make_snapshot(&source, &original);
        }
        self.backup(&source, &original)?;
        self.backup(&source, &merged)?;
        let temporary = temporary(&source, &merged)?;
        let meta = fs::metadata(&source).map_err(|_| Error::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{fchown, MetadataExt};
            fchown(&temporary.file, Some(meta.uid()), Some(meta.gid())).map_err(|_| Error::Io)?;
        }
        temporary
            .file
            .set_permissions(meta.permissions())
            .map_err(|_| Error::Io)?;
        temporary.file.sync_all().map_err(|_| Error::Io)?;
        if self.safe_source()? != source
            || digest(read_bounded(&source, MAX_BYTES)?.as_bytes()) != expected
        {
            return Err(Error::Conflict);
        }
        fs::rename(&temporary.path, &source).map_err(|_| Error::Io)?;
        File::open(source.parent().ok_or(Error::Io)?)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| Error::Durability)?;
        self.prune_history(&source, expected, &digest(merged.as_bytes()));
        self.make_snapshot(&source, &merged)
    }

    fn prune_history(&self, source: &Path, previous: &str, current: &str) {
        let Some(parent) = source.parent() else {
            return;
        };
        let directory = parent.join(".admin-config-history").join(&self.id);
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        let mut found: Vec<_> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = fs::symlink_metadata(entry.path()).ok()?;
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let revision = name.strip_suffix(".toml")?;
                valid_revision(revision)
                    .then(|| (meta.modified().ok(), revision.to_owned(), entry.path()))
            })
            .collect();
        found.sort_by_key(|item| std::cmp::Reverse(item.0));
        let mut keep = 18;
        for (_, revision, path) in found {
            if revision == previous || revision == current {
                continue;
            }
            if keep > 0 {
                keep -= 1;
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn history_dir(&self, source: &Path) -> Result<PathBuf, Error> {
        let parent = source.parent().ok_or(Error::Io)?;
        let root = parent.join(".admin-config-history");
        private_directory(&root)?;
        let directory = root.join(&self.id);
        private_directory(&directory)?;
        Ok(directory)
    }

    fn backup(&self, source: &Path, text: &str) -> Result<(), Error> {
        let dir = self.history_dir(source)?;
        let path = dir.join(format!("{}.toml", digest(text.as_bytes())));
        if path.exists() {
            if read_bounded(&path, MAX_BYTES)? != text {
                return Err(Error::Io);
            }
            return Ok(());
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|_| Error::Io)?;
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| Error::Io)?;
        File::open(dir)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::Io)?;
        Ok(())
    }

    fn history(&self, source: &Path) -> Vec<String> {
        // Lesen erzeugt keine Ordner. Historie enthält nur Hashes, niemals Werte.
        let Some(parent) = source.parent() else {
            return Vec::new();
        };
        let directory = parent.join(".admin-config-history").join(&self.id);
        let Ok(entries) = fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut found: Vec<_> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = fs::symlink_metadata(entry.path()).ok()?;
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let revision = name.strip_suffix(".toml")?;
                valid_revision(revision).then(|| (meta.modified().ok(), revision.to_owned()))
            })
            .collect();
        found.sort_by_key(|item| std::cmp::Reverse(item.0));
        found
            .into_iter()
            .take(20)
            .map(|(_, revision)| revision)
            .collect()
    }

    pub fn history_snapshot(&self, revision: &str) -> Result<String, Error> {
        if !valid_revision(revision) {
            return Err(Error::NotFound);
        }
        let source = self.safe_source()?;
        let path = self.history_dir(&source)?.join(format!("{revision}.toml"));
        let text = read_bounded(&path, MAX_BYTES)?;
        if digest(text.as_bytes()) != revision {
            return Err(Error::Io);
        }
        self.validate(&source, &text)?;
        toml::to_string_pretty(&parse(&text)?).map_err(|_| Error::Io)
    }
}

pub fn private_directory(path: &Path) -> Result<(), Error> {
    if let Err(error) = fs::create_dir(path) {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(Error::Io);
        }
    }
    let meta = fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Error::Io);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Io)?;
    }
    Ok(())
}
