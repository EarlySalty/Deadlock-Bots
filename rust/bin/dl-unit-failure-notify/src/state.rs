use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const DAY: u64 = 86_400;
const WEEK: u64 = 7 * DAY;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub last_invocation: Option<String>,
    pub unreported: u64,
    pub confirmed: Vec<u64>,
    pub sequence: u64,
    pub pending: Option<Pending>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub key: String,
    pub content: String,
    pub included: u64,
}

impl State {
    pub fn observe(&mut self, invocation: &str) {
        if self.last_invocation.as_deref() != Some(invocation) {
            self.last_invocation = Some(invocation.to_owned());
            self.unreported = self.unreported.saturating_add(1);
        }
    }

    pub fn eligible(&self, now: u64) -> bool {
        let week = self
            .confirmed
            .iter()
            .filter(|time| now.saturating_sub(**time) < WEEK)
            .count();
        week < 2
            && !self
                .confirmed
                .iter()
                .any(|time| now.saturating_sub(*time) < DAY)
    }

    pub fn confirm(&mut self, now: u64) {
        if let Some(pending) = self.pending.take() {
            self.unreported = self.unreported.saturating_sub(pending.included);
            self.sequence = self.sequence.saturating_add(1);
            self.confirmed
                .retain(|time| now.saturating_sub(*time) < WEEK);
            self.confirmed.push(now);
        }
    }
}

/// Eine eigene Sperrdatei bleibt beim atomaren Austausch der Zustandsdatei offen.
/// Sie deckt auch den Versand ab; zwei OneShots können daher nicht zugleich senden.
pub struct Store {
    _lock: File,
    path: PathBuf,
}

impl Store {
    pub fn open(directory: &Path, unit_key: &str) -> Result<Self> {
        if !directory.is_absolute() {
            bail!("Das Zustandsverzeichnis muss ein absoluter Pfad sein.");
        }
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .context("Zustandsverzeichnis ist nicht verfügbar.")?;
        let metadata =
            fs::symlink_metadata(directory).context("Zustandsverzeichnis ist nicht verfügbar.")?;
        let uid = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            bail!("Zustandsverzeichnis muss dem Dienstnutzer gehören und Modus 0700 haben.");
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join(format!("{unit_key}.lock")))
            .context("Meldesperre ist nicht verfügbar.")?;
        lock.lock_exclusive()
            .context("Meldesperre konnte nicht gehalten werden.")?;
        Ok(Self {
            _lock: lock,
            path: directory.join(format!("{unit_key}.json")),
        })
    }

    pub fn load(&self) -> Result<State> {
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)
        {
            Ok(file) => {
                if file.metadata()?.len() > 32_768 {
                    bail!("Meldezustand ist zu groß.");
                }
                serde_json::from_reader(file)
                    .context("Meldezustand ist ungültig; Versand bleibt gesperrt.")
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(_) => bail!("Meldezustand ist nicht lesbar."),
        }
    }

    pub fn save(&self, state: &State) -> Result<()> {
        let temporary = self.path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)
            .context("Meldezustand kann nicht gespeichert werden.")?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(state)?)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.path)?;
        File::open(self.path.parent().context("Zustandsverzeichnis fehlt.")?)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_per_day_and_two_per_rolling_week_survive_clock_rollback() {
        let mut state = State {
            confirmed: vec![1_000],
            ..State::default()
        };
        assert!(!state.eligible(500));
        assert!(!state.eligible(1_000 + DAY - 1));
        assert!(state.eligible(1_000 + DAY));
        state.confirmed.push(1_000 + DAY);
        assert!(!state.eligible(1_000 + 2 * DAY));
        assert!(state.eligible(1_000 + WEEK));
    }
    #[test]
    fn duplicate_invocation_is_counted_once_and_failed_send_does_not_spend_budget() {
        let mut state = State::default();
        state.observe("first");
        state.observe("first");
        state.observe("second");
        assert_eq!(state.unreported, 2);
        state.pending = Some(Pending {
            key: "synthetic-key".into(),
            content: "sicher".into(),
            included: 2,
        });
        assert!(state.eligible(1_000));
        assert_eq!(state.confirmed.len(), 0);
        state.observe("third");
        state.confirm(1_000);
        assert_eq!(state.unreported, 1);
        assert!(!state.eligible(1_001));
    }
    #[test]
    fn persisted_pending_and_counts_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::open(dir.path(), "unit").unwrap();
        let mut state = State::default();
        state.observe("invocation");
        state.pending = Some(Pending {
            key: "same-key".into(),
            content: "sicher".into(),
            included: 1,
        });
        store.save(&state).unwrap();
        drop(store);
        let state = Store::open(dir.path(), "unit").unwrap().load().unwrap();
        assert_eq!(state.unreported, 1);
        assert_eq!(state.pending.unwrap().key, "same-key");
        assert!(state.confirmed.is_empty());
    }

    #[test]
    fn concurrent_sender_cannot_acquire_same_unit_lock() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::open(dir.path(), "unit").unwrap();
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.path().join("unit.lock"))
            .unwrap();
        assert!(second.try_lock_exclusive().is_err());
        drop(store);
        assert!(second.try_lock_exclusive().is_ok());
    }

    #[test]
    fn corrupt_persistence_never_resets_budget_to_send_again() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(dir.path().join("unit.json"), "broken state").unwrap();
        assert!(Store::open(dir.path(), "unit").unwrap().load().is_err());
    }
}
