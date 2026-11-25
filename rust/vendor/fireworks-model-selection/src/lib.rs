//! Lokaler, geheimerfreier Auswahlvertrag. Dieser Reader ruft keine API auf.
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub const STATE_PATH: &str = "/var/lib/deadlock/llm-model-selection.json";
pub const TRUSTED_UID: u32 = 1000;
pub const MAX_BYTES: u64 = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub schema_version: u32,
    pub provider: String,
    pub model: String,
    pub release: Option<DateTime<Utc>>,
    pub probed_at: DateTime<Utc>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionError {
    Unavailable,
    UnsafeFile,
    InvalidState,
}
impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Zentrale Modellauswahl fehlt",
            Self::UnsafeFile => "Zentrale Modellauswahl ist nicht geschützt",
            Self::InvalidState => "Zentrale Modellauswahl ist ungültig",
        })
    }
}
impl std::error::Error for SelectionError {}

/// Ausschließlich stabile numerische DeepSeek-Flash-Versionen des offiziellen Owners.
pub fn version(model: &str) -> Option<Vec<u32>> {
    let rest = model.strip_prefix("accounts/fireworks/models/deepseek-v")?;
    let (number, suffix) = rest.split_once("-flash")?;
    if number.is_empty() || number.len() > 24 {
        return None;
    }
    let mut parts = number
        .split(['p', '.'])
        .map(|p| {
            if p.is_empty()
                || !p.bytes().all(|b| b.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
            {
                None
            } else {
                p.parse::<u32>().ok()
            }
        })
        .collect::<Option<Vec<_>>>()?;
    if parts.len() > 4 || parts.as_slice() < [4, 1].as_slice() {
        return None;
    }
    if !suffix.is_empty() {
        let date = suffix.strip_prefix('-')?;
        if !date.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let valid = match date.len() {
            4 => NaiveDate::parse_from_str(&format!("2000{date}"), "%Y%m%d").is_ok(),
            8 => NaiveDate::parse_from_str(date, "%Y%m%d").is_ok(),
            _ => false,
        };
        if !valid {
            return None;
        }
    }
    while parts.len() > 1 && parts.last() == Some(&0) {
        parts.pop();
    }
    Some(parts)
}

impl Selection {
    pub fn validate(&self) -> Result<(), SelectionError> {
        let now = Utc::now() + chrono::Duration::minutes(5);
        if self.schema_version != 1
            || self.provider != "fireworks"
            || version(&self.model).is_none()
            || self.probed_at > self.checked_at
            || self.checked_at > now
            || self.probed_at.timestamp() <= 0
            || self.release.is_some_and(|d| d > now)
        {
            return Err(SelectionError::InvalidState);
        }
        Ok(())
    }
}

fn protected_file(path: &Path, trusted_uid: u32) -> Result<File, SelectionError> {
    if !path.is_absolute() {
        return Err(SelectionError::UnsafeFile);
    }
    for parent in path.ancestors().skip(1) {
        let meta = std::fs::symlink_metadata(parent).map_err(|_| SelectionError::Unavailable)?;
        if !meta.is_dir() || ![0, trusted_uid].contains(&meta.uid()) || meta.mode() & 0o022 != 0 {
            return Err(SelectionError::UnsafeFile);
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| SelectionError::Unavailable)?;
    let meta = file.metadata().map_err(|_| SelectionError::Unavailable)?;
    if !meta.is_file()
        || ![0, trusted_uid].contains(&meta.uid())
        || meta.mode() & 0o022 != 0
        || meta.nlink() != 1
        || meta.len() > MAX_BYTES
    {
        return Err(SelectionError::UnsafeFile);
    }
    Ok(file)
}

pub fn read_selection(path: &Path, trusted_uid: u32) -> Result<Selection, SelectionError> {
    let mut bytes = Vec::new();
    protected_file(path, trusted_uid)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SelectionError::Unavailable)?;
    if bytes.len() > MAX_BYTES as usize {
        return Err(SelectionError::InvalidState);
    }
    let selection: Selection =
        serde_json::from_slice(&bytes).map_err(|_| SelectionError::InvalidState)?;
    selection.validate()?;
    Ok(selection)
}

pub fn selected_model() -> Result<String, SelectionError> {
    read_selection(Path::new(STATE_PATH), TRUSTED_UID).map(|s| s.model)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_versions_and_strict_family() {
        let id = |n: &str| format!("accounts/fireworks/models/deepseek-{n}");
        assert!(version(&id("v4p10-flash")) > version(&id("v4p9-flash")));
        assert!(version(&id("v10-flash")) > version(&id("v4p10-flash")));
        for bad in [
            "v4-pro",
            "v4p1-flash-preview",
            "v4p1-flash-vision",
            "v4p1-flash-distilled",
            "v4p1-flash-0230",
            "v04-flash",
        ] {
            assert!(version(&id(bad)).is_none(), "{bad}");
        }
        assert!(version("accounts/private/models/deepseek-v4p1-flash").is_none());
    }
    #[test]
    fn future_wrong_provider_and_unverified_time_rejected() {
        let now = Utc::now();
        let mut s = Selection {
            schema_version: 1,
            provider: "fireworks".into(),
            model: "accounts/fireworks/models/deepseek-v4p1-flash".into(),
            release: None,
            probed_at: now,
            checked_at: now,
        };
        assert!(s.validate().is_ok());
        s.provider = "other".into();
        assert_eq!(s.validate(), Err(SelectionError::InvalidState));
        s.provider = "fireworks".into();
        s.checked_at = now + chrono::Duration::days(1);
        assert_eq!(s.validate(), Err(SelectionError::InvalidState));
    }
    #[test]
    fn filesystem_fail_closed_and_reload() {
        use std::os::unix::{
            ffi::OsStrExt,
            fs::{symlink, PermissionsExt},
        };
        let cwd = std::env::current_dir().unwrap();
        let base = cwd
            .ancestors()
            .find(|p| {
                p.ancestors().all(|ancestor| {
                    std::fs::symlink_metadata(ancestor).is_ok_and(|m| {
                        m.is_dir() && [0, TRUSTED_UID].contains(&m.uid()) && m.mode() & 0o022 == 0
                    })
                })
            })
            .expect("Fixture benötigt geschützten Ancestor");
        let dir = base.join(format!(".reader-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("selection.json");
        let now = Utc::now();
        let mut state = Selection {
            schema_version: 1,
            provider: "fireworks".into(),
            model: "accounts/fireworks/models/deepseek-v4p1-flash".into(),
            release: None,
            probed_at: now,
            checked_at: now,
        };
        let store = |s: &Selection| {
            let next = dir.join("next.json");
            std::fs::write(&next, serde_json::to_vec(s).unwrap()).unwrap();
            std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::rename(next, &path).unwrap();
        };
        store(&state);
        assert!(read_selection(&path, TRUSTED_UID).is_ok());
        state.model = "accounts/fireworks/models/deepseek-v4p2-flash".into();
        store(&state);
        assert_eq!(
            read_selection(&path, TRUSTED_UID).unwrap().model,
            state.model
        );
        let alias = dir.join("alias");
        symlink(&path, &alias).unwrap();
        assert!(read_selection(&alias, TRUSTED_UID).is_err());
        std::fs::write(&path, b"{").unwrap();
        assert!(read_selection(&path, TRUSTED_UID).is_err());
        std::fs::write(&path, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(read_selection(&path, TRUSTED_UID).is_err());
        std::fs::remove_file(&path).unwrap();
        let raw = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: CString ist NUL-terminiert und lebt während des Aufrufs;
        // der Pfad gehört ausschließlich dem privaten Testverzeichnis.
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        assert!(read_selection(&path, TRUSTED_UID).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
