//! Einmaliger privater Infisical-Snapshot für die Token-DB-Verbraucher.
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, OnceLock},
};
use zeroize::Zeroizing;

static SNAPSHOT: OnceLock<BTreeMap<String, Zeroizing<String>>> = OnceLock::new();
static START: Mutex<()> = Mutex::new(());

/// Metadatenquelle liegt neben der explizit gewählten normalen Betriebsconfig.
/// Keine ENV-Pfadauswahl und kein Dateitoken-Fallback im gestarteten Verbraucher.
pub fn load(config_source: &Path) -> Result<(), &'static str> {
    let _guard = START
        .lock()
        .map_err(|_| "Private Infisical-Startgrenze ist nicht verfügbar.")?;
    if installed() {
        return Err("Private Infisical-Momentaufnahme wurde bereits geladen.");
    }
    let source = config_source.with_file_name("infisical.json");
    let values = dl_token_secrets::private_values(&source)
        .map_err(|_| "Private Infisical-Momentaufnahme konnte nicht geladen werden.")?;
    SNAPSHOT
        .set(values.into_iter().collect())
        .map_err(|_| "Private Infisical-Momentaufnahme wurde bereits geladen.")
}

pub fn installed() -> bool {
    SNAPSHOT.get().is_some()
}

pub fn value(name: &str) -> Option<&'static str> {
    SNAPSHOT.get()?.get(name).map(|value| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };

    #[test]
    fn real_fifo_snapshot_is_single_read_and_missing_values_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("infisical.json"),
            br#"{"secret_values_fd":3,"project_id":"fixture","environment":"fixture","secret_path":"/","socket_path":"/nonexistent","database_secret":"DEADLOCK_CENTRAL_DSN"}"#).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "token_snapshot::tests::child_fifo_contract",
            ])
            .current_dir(directory.path())
            .stdin(Stdio::piped());
        // SAFETY: Command owns the piped stdin FD0 until exec; dup2 is
        // async-signal-safe and creates the designated inherited private FD3.
        unsafe {
            command.pre_exec(|| {
                if libc::dup2(0, 3) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                br#"{"DEADLOCK_CENTRAL_DSN":"synthetic-dsn","DISCORD_TOKEN":"synthetic-token"}"#,
            )
            .unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    #[ignore = "isolierter Kindprozess mit echter privater FIFO"]
    fn child_fifo_contract() {
        let config = std::env::current_dir().unwrap().join("bot.toml");
        load(&config).unwrap();
        assert_eq!(value("DEADLOCK_CENTRAL_DSN"), Some("synthetic-dsn"));
        assert_eq!(
            crate::runtime_config::secret_value("DISCORD_TOKEN").as_deref(),
            Some("synthetic-token")
        );
        assert!(crate::runtime_config::secret_value("MASTER_BROKER_TOKEN").is_none());
        assert!(crate::runtime_config::secret_value("RUST_LOG").is_none());
        assert!(load(&config).is_err());
    }
}
