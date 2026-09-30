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
    pub recent_invocations: Vec<String>,
    pub unreported: u64,
    pub confirmed: Vec<u64>,
    /// Versandversuche ohne bestätigte Message-ID. Keine erfundenen Sends;
    /// diese Reservierungen verhindern Replay nach verlorener Bestätigung.
    pub uncertain_attempts: Vec<u64>,
    pub sequence: u64,
    pub pending: Option<Pending>,
    /// Einmalige Übergangsruhe: Legacy hat keine vollständige Wochenhistorie.
    pub legacy_not_before: u64,
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
        if !self
            .recent_invocations
            .iter()
            .any(|known| known == invocation)
        {
            if self.recent_invocations.len() >= 256 {
                self.recent_invocations.remove(0);
            }
            self.recent_invocations.push(invocation.to_owned());
            self.unreported = self.unreported.saturating_add(1);
        }
    }

    pub fn eligible(&self, now: u64) -> bool {
        if now < self.legacy_not_before {
            return false;
        }
        let week = self
            .confirmed
            .iter()
            .chain(self.uncertain_attempts.iter())
            .filter(|time| now.saturating_sub(**time) < WEEK)
            .count();
        week < 2
            && !self
                .confirmed
                .iter()
                .chain(self.uncertain_attempts.iter())
                .any(|time| now.saturating_sub(*time) < DAY)
    }

    pub fn confirm(&mut self, now: u64) {
        if let Some(pending) = self.pending.take() {
            // Die aktuelle Reservierung wird durch genau eine bestätigte
            // Nachricht ersetzt, niemals in beiden Budgets doppelt gezählt.
            self.uncertain_attempts.pop();
            self.unreported = self.unreported.saturating_sub(pending.included);
            self.sequence = self.sequence.saturating_add(1);
            self.confirmed
                .retain(|time| now.saturating_sub(*time) < WEEK);
            self.confirmed.push(now);
        }
    }

    pub fn reserve_attempt(&mut self, now: u64) {
        self.uncertain_attempts
            .retain(|time| now.saturating_sub(*time) < WEEK);
        self.uncertain_attempts.push(now);
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
        crate::protected_path(directory, true)?;
        let metadata =
            fs::symlink_metadata(directory).context("Zustandsverzeichnis ist nicht verfügbar.")?;
        // SAFETY: geteuid hat keine Zeigerargumente und verändert keinen Zustand.
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

    pub fn load_or_import(&self, legacy_directory: Option<&Path>, unit: &str) -> Result<State> {
        match fs::symlink_metadata(&self.path) {
            Ok(_) => return self.load(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("Meldezustand ist nicht verfügbar."),
        }
        let Some(directory) = legacy_directory else {
            return Ok(State::default());
        };
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join(format!("{unit}.state")))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(State::default())
            }
            Err(_) => bail!("Bestehender Meldezustand ist nicht sicher lesbar."),
        };
        let metadata = file.metadata()?;
        // SAFETY: geteuid hat keine Zeigerargumente und verändert keinen Zustand.
        if !metadata.is_file()
            || (metadata.mode() & 0o022 != 0 && !private_legacy_directory(directory))
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.len() > 256
        {
            bail!("Bestehender Meldezustand ist nicht als begrenzte eigene Datei bestätigt.");
        }
        use std::io::Read;
        let mut text = String::new();
        file.take(257)
            .read_to_string(&mut text)
            .context("Bestehender Meldezustand ist ungültig.")?;
        let values: Vec<u64> = text
            .split_whitespace()
            .map(str::parse)
            .collect::<std::result::Result<_, _>>()
            .context("Bestehender Meldezustand enthält ungültige Zähler.")?;
        if values.len() != 4 || text.len() > 256 || values[0] > values[2] || values[1] > values[2] {
            bail!("Bestehender Meldezustand hat ein ungültiges Format.");
        }
        // LAST_NOTIFY FIRST_FAIL LAST_FAIL SUPPRESSED. Keine erfundenen Sends:
        // fehlende Wochenhistorie führt nur zu einer einmaligen Übergangsruhe.
        let mut state = State {
            unreported: values[3],
            ..State::default()
        };
        if values[0] > 0 {
            state.confirmed.push(values[0]);
            state.legacy_not_before = values[0]
                .checked_add(WEEK)
                .context("Bestehende Meldezeit ist ungültig.")?;
        }
        self.save(&state)?;
        Ok(state)
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

/// Legacydateien sind teilweise 0664 unter .local (0700). Dort schützt die
/// private eigene Grenze auch tiefere Gruppenrechte. Diese Ausnahme gilt nur
/// für den einmaligen Import, nicht für Config, Binary oder neuen Meldezustand.
fn private_legacy_directory(directory: &Path) -> bool {
    if directory.components().any(|component| {
        !matches!(
            component,
            std::path::Component::RootDir | std::path::Component::Normal(_)
        )
    }) {
        return false;
    }
    // SAFETY: geteuid hat keine Zeigerargumente und verändert keinen Zustand.
    let uid = unsafe { libc::geteuid() };
    for ancestor in directory.ancestors() {
        let Ok(metadata) = fs::symlink_metadata(ancestor) else {
            return false;
        };
        if !metadata.is_dir() || ![0, uid].contains(&metadata.uid()) {
            return false;
        }
        if metadata.uid() == uid && metadata.mode() & 0o077 == 0 {
            return crate::protected_path(ancestor, true).is_ok();
        }
    }
    false
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
        state.observe("first");
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

    #[test]
    fn legacy_cutover_imports_real_confirmed_time_and_counter_once() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let legacy = root.path().join("legacy");
        fs::create_dir(&legacy).unwrap();
        fs::write(
            legacy.join("steam-core.service.state"),
            "1000 950 1100 12\n",
        )
        .unwrap();
        let store = Store::open(&root.path().join("private"), "unit").unwrap();
        let state = store
            .load_or_import(Some(&legacy), "steam-core.service")
            .unwrap();
        assert_eq!(state.unreported, 12);
        assert_eq!(state.confirmed, vec![1000]);
        assert!(!state.eligible(1000 + WEEK - 1));
        assert!(state.eligible(1000 + WEEK));
        fs::write(
            legacy.join("steam-core.service.state"),
            "1000 950 999999 99\n",
        )
        .unwrap();
        let loaded = store
            .load_or_import(Some(&legacy), "steam-core.service")
            .unwrap();
        assert_eq!(loaded.legacy_not_before, 1000 + WEEK);
        assert_eq!(loaded.unreported, 12);
    }

    #[test]
    fn group_writable_legacy_state_requires_verified_private_ancestor() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let legacy = root.path().join("legacy");
        fs::create_dir(&legacy).unwrap();
        fs::set_permissions(&legacy, fs::Permissions::from_mode(0o775)).unwrap();
        let path = legacy.join("steam-core.service.state");
        fs::write(&path, "1000 950 1100 12").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
        let store = Store::open(&root.path().join("private"), "unit").unwrap();
        assert!(store
            .load_or_import(Some(&legacy), "steam-core.service")
            .is_ok());
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&legacy, &alias).unwrap();
        assert!(!private_legacy_directory(&alias));
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!private_legacy_directory(&legacy));
    }

    #[test]
    fn malformed_legacy_state_fails_closed_without_creating_new_state() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let legacy = root.path().join("legacy");
        fs::create_dir(&legacy).unwrap();
        fs::write(
            legacy.join("steam-core.service.state"),
            "not valid counters",
        )
        .unwrap();
        let store = Store::open(&root.path().join("private"), "unit").unwrap();
        assert!(store
            .load_or_import(Some(&legacy), "steam-core.service")
            .is_err());
        assert!(!store.path.exists());
    }

    #[test]
    fn no_confirmed_legacy_send_does_not_invent_budget_or_wait() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let legacy = root.path().join("legacy");
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join("steam-core.service.state"), "0 1000 1100 12").unwrap();
        let store = Store::open(&root.path().join("private"), "unit").unwrap();
        let state = store
            .load_or_import(Some(&legacy), "steam-core.service")
            .unwrap();
        assert!(state.confirmed.is_empty());
        assert!(state.eligible(1100));
        assert_eq!(state.unreported, 12);
    }

    #[test]
    fn lost_ack_and_broker_cache_loss_cannot_create_unbounded_replays() {
        let mut state = State::default();
        state.observe("first");
        state.pending = Some(Pending {
            key: "stable-key".into(),
            content: "sicher".into(),
            included: 1,
        });
        state.reserve_attempt(1_000);
        assert!(state.confirmed.is_empty());
        assert!(!state.eligible(1_000 + 601)); // Broker-RAMcache bereits abgelaufen.
        assert!(!state.eligible(1_000 + DAY - 1));
        assert!(state.eligible(1_000 + DAY));
        state.reserve_attempt(1_000 + DAY);
        assert!(!state.eligible(1_000 + 2 * DAY));
        assert!(state.eligible(1_000 + WEEK));
        assert_eq!(state.pending.as_ref().unwrap().key, "stable-key");
        assert_eq!(state.unreported, 1);
    }

    #[test]
    fn confirmed_message_replaces_attempt_reservation_instead_of_counting_twice() {
        let mut state = State::default();
        state.observe("first");
        state.pending = Some(Pending {
            key: "stable-key".into(),
            content: "sicher".into(),
            included: 1,
        });
        state.reserve_attempt(1_000);
        state.confirm(1_005);
        assert!(state.uncertain_attempts.is_empty());
        assert_eq!(state.confirmed, vec![1_005]);
        assert_eq!(state.unreported, 0);
        assert!(state.eligible(1_005 + DAY));
    }
}
