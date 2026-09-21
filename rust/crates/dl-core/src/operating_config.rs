//! Begrenzter Editor für Betriebswerte; Identitäten, Pfade und Modelle bleiben
//! außerhalb des HTTP-Schreibvertrags. Laufende Prozesse behalten ihren Stand.

use crate::bot_config::{BotConfig, BotConfigError, BotConfigStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperatingOptions {
    pub moderation_enforce: bool,
    pub concierge_timeout_seconds: u64,
}

impl From<&BotConfig> for OperatingOptions {
    fn from(config: &BotConfig) -> Self {
        Self {
            moderation_enforce: config.moderation.enforce,
            concierge_timeout_seconds: config.concierge.timeout_seconds,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error(transparent)]
    Invalid(#[from] BotConfigError),
    #[error("Die Datei wurde inzwischen geändert. Der Entwurf bleibt erhalten.")]
    Conflict,
    #[error("Die Einstellungen werden gerade gespeichert. Bitte erneut versuchen.")]
    Busy,
    #[error("Die Betriebsdatei muss außerhalb des Git-Arbeitsordners liegen.")]
    UnsafeLocation,
    #[error("Die Betriebsdatei konnte nicht gespeichert werden.")]
    Io,
    #[error("Die Datei wurde ersetzt, die dauerhafte Speicherung ist aber nicht bestätigt. Bitte den Stand neu laden.")]
    Durability,
}

pub struct SavedConfig {
    pub config: BotConfig,
    pub revision: String,
    pub fingerprint: String,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn fingerprint(config: &BotConfig) -> Result<String, BotConfigError> {
    serde_json::to_vec(config)
        .map(|bytes| digest(&bytes))
        .map_err(|_| BotConfigError::Encoding)
}

pub fn read_saved(path: &Path) -> Result<SavedConfig, BotConfigError> {
    let (config, text) = BotConfig::load_document(path)?;
    Ok(SavedConfig {
        revision: digest(text.as_bytes()),
        fingerprint: fingerprint(&config)?,
        config,
    })
}

impl BotConfigStore {
    pub fn read_versioned(&self) -> Result<SavedConfig, BotConfigError> {
        read_saved(self.path())
    }

    pub fn save_if_revision(
        &self,
        expected: &str,
        options: &OperatingOptions,
    ) -> Result<SavedConfig, EditError> {
        let requested = self.path();
        let metadata = fs::symlink_metadata(requested).map_err(|_| EditError::Io)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(EditError::UnsafeLocation);
        }
        let source = requested.canonicalize().map_err(|_| EditError::Io)?;
        if source
            .ancestors()
            .any(|parent| parent.join(".git").exists())
        {
            return Err(EditError::UnsafeLocation);
        }
        let directory = source.parent().ok_or(EditError::UnsafeLocation)?;
        let mut open = OpenOptions::new();
        open.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let lock = open
            .open(directory.join(".bot.toml.lock"))
            .map_err(|_| EditError::Io)?;
        if !lock.metadata().map_err(|_| EditError::Io)?.is_file() {
            return Err(EditError::UnsafeLocation);
        }
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| match error.kind() {
            std::io::ErrorKind::WouldBlock => EditError::Busy,
            _ => EditError::Io,
        })?;
        let (mut candidate, original) = BotConfig::load_document(&source)?;
        if digest(original.as_bytes()) != expected {
            return Err(EditError::Conflict);
        }
        candidate.moderation.enforce = options.moderation_enforce;
        candidate.concierge.timeout_seconds = options.concierge_timeout_seconds;
        candidate.validate()?;
        let mut document = original
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| EditError::Io)?;
        for (section, key, mut replacement) in [
            (
                "moderation",
                "enforce",
                toml_edit::value(options.moderation_enforce),
            ),
            (
                "concierge",
                "timeout_seconds",
                toml_edit::value(options.concierge_timeout_seconds as i64),
            ),
        ] {
            if !document.contains_key(section) {
                document[section] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            if let (Some(old), Some(new)) = (
                document
                    .get(section)
                    .and_then(toml_edit::Item::as_table_like)
                    .and_then(|table| table.get(key))
                    .and_then(toml_edit::Item::as_value),
                replacement.as_value_mut(),
            ) {
                *new.decor_mut() = old.decor().clone();
            }
            document[section][key] = replacement;
        }
        let text = document.to_string();
        let mut checked = BotConfig::parse(&text)?;
        checked.runtime.resolve_paths(directory);
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let temporary = directory.join(format!(
            ".bot.toml.{}.{}.tmp",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut create = OpenOptions::new();
        create.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            create.mode(0o600);
        }
        let mut file = create.open(&temporary).map_err(|_| EditError::Io)?;
        let result = (|| {
            let metadata = fs::metadata(&source).map_err(|_| EditError::Io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::{fchown, MetadataExt};
                fchown(&file, Some(metadata.uid()), Some(metadata.gid()))
                    .map_err(|_| EditError::Io)?;
            }
            file.set_permissions(metadata.permissions())
                .map_err(|_| EditError::Io)?;
            file.write_all(text.as_bytes()).map_err(|_| EditError::Io)?;
            file.sync_all().map_err(|_| EditError::Io)?;
            if requested.canonicalize().map_err(|_| EditError::Io)? != source
                || read_saved(&source)?.revision != expected
            {
                return Err(EditError::Conflict);
            }
            fs::rename(&temporary, &source).map_err(|_| EditError::Io)?;
            File::open(directory)
                .and_then(|parent| parent.sync_all())
                .map_err(|_| EditError::Durability)?;
            Ok(SavedConfig {
                revision: digest(text.as_bytes()),
                fingerprint: fingerprint(&checked)?,
                config: checked,
            })
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
}
