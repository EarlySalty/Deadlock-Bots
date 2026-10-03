//! Migrationsstart aus der bestehenden Bot-TOML und ihrem privaten FD3-Snapshot.
use anyhow::{ensure, Result};
use std::ffi::OsString;

pub fn steam_only_from_args(args: &[OsString]) -> Result<bool> {
    let mut steam_only = false;
    let mut explicit_config = false;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--steam-credentials-only" {
            ensure!(!steam_only, "Steam-Modus darf nur einmal angegeben werden.");
            steam_only = true;
        } else if args[index] == "--config" {
            explicit_config = true;
            index += 1;
            ensure!(index < args.len(), "Bot-TOML-Pfad fehlt.");
        } else if args[index].to_string_lossy().starts_with("--config=") {
            explicit_config = true;
        } else {
            anyhow::bail!("Erwartet: [--steam-credentials-only] [--config <Bot-TOML>]");
        }
        index += 1;
    }
    // Dieselbe Pfadvalidierung wie bei dl-bot und dl-web, ohne eigene Dateiquelle.
    dl_core::config::config_path_from_args(args.iter().cloned())?;
    ensure!(
        !steam_only || explicit_config,
        "Begrenzte Steam-Migration benötigt eine explizite Bot-TOML."
    );
    Ok(steam_only)
}

pub fn central_dsn_from_process() -> Result<&'static str> {
    let source = dl_core::config::process_bot_config()?.source();
    dl_core::token_snapshot::load(source).map_err(anyhow::Error::msg)?;
    dl_core::token_snapshot::value("DEADLOCK_CENTRAL_DSN")
        .ok_or_else(|| anyhow::anyhow!("Zentraler Datenbankzugang fehlt im privaten FD3-Snapshot."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn normaler_und_begrenzter_modus_nutzen_denselben_toml_pfadvertrag() {
        assert!(!steam_only_from_args(&[]).expect("Standardkonfiguration"));
        assert!(!steam_only_from_args(&args(&["--config", "/srv/bot.toml"])).expect("Bot-TOML"));
        assert!(steam_only_from_args(&args(&[
            "--steam-credentials-only",
            "--config=/srv/bot.toml"
        ]))
        .expect("Begrenzter Modus"));
        for values in [
            vec!["--steam-credentials-only"],
            vec!["--config"],
            vec!["--config", ""],
            vec!["--config", "one.toml", "--config", "two.toml"],
            vec![
                "--steam-credentials-only",
                "--steam-credentials-only",
                "--config=a.toml",
            ],
            vec!["--unknown=SYNTHETISCHER_MARKER"],
        ] {
            let error = steam_only_from_args(&args(&values)).expect_err("Ungültiger Aufruf");
            assert!(!error.to_string().contains("SYNTHETISCHER_MARKER"));
        }
    }

    #[test]
    fn normaler_migrationsstart_laedt_echte_private_fifo_neben_bot_toml() {
        let directory = tempfile::tempdir().expect("Isolierter Starttest");
        std::fs::create_dir(directory.path().join("config")).expect("Configverzeichnis");
        std::fs::write(
            directory.path().join("config/bot.toml"),
            "schema_version = 1",
        )
        .expect("Normale Bot-TOML");
        // Ausschließlich synthetische Testmetadaten für den vorhandenen FD3-Vertrag.
        std::fs::write(directory.path().join("config/infisical.json"),
            br#"{"secret_values_fd":3,"project_id":"fixture","environment":"fixture","secret_path":"/","socket_path":"/nonexistent","database_secret":"DEADLOCK_CENTRAL_DSN"}"#)
            .expect("FD3-Testmetadaten");
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exec 3<&0; exec \"$@\"", "migrations-fifo-test"])
            .arg(std::env::current_exe().expect("Testbinary"))
            .args([
                "--ignored",
                "--exact",
                "peer_config::tests::fd3_start_child",
            ])
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .spawn()
            .expect("Isolierter Kindprozess");
        child
            .stdin
            .take()
            .expect("Testpipe")
            .write_all(br#"{"DEADLOCK_CENTRAL_DSN":"synthetic-dsn"}"#)
            .expect("Synthetischer Snapshot");
        assert!(child.wait().expect("Kindprozessende").success());
    }

    #[test]
    #[ignore = "isolierter Kindprozess mit privater FIFO"]
    fn fd3_start_child() {
        assert_eq!(
            central_dsn_from_process().expect("Vorhandener TOML-/FD3-Lader"),
            "synthetic-dsn"
        );
        assert!(central_dsn_from_process().is_err());
    }
}
