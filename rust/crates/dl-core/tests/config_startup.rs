use std::{ffi::OsStr, path::Path, process::Command};

fn checker(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dl-config-check"));
    command.current_dir(directory);
    command
}

fn write_config(path: &Path, master_port: u16) {
    std::fs::write(
        path,
        format!(
            "schema_version = 1\n\
             [services]\n\
             master_broker_port = {master_port}\n\
             changelog_port = 9005\n"
        ),
    )
    .expect("write fixture");
}

#[test]
fn startup_uses_file_instead_of_old_environment_values() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("bot.toml");
    write_config(&path, 9003);
    let output = checker(directory.path())
        .arg("--config")
        .arg(&path)
        .envs([
            ("MASTER_BROKER_PORT", "1"),
            ("CHANGELOG_API_PORT", "1"),
            ("DASHBOARD_PORT", "not-a-port"),
            ("PUBLIC_STATS_PORT", "not-a-port"),
            ("TIERLIST_PUBLIC_PORT", "not-a-port"),
            ("DEADLOCK_DB_PATH", "ENV_SOURCE_MUST_NOT_APPEAR"),
            ("DEADLOCK_DB_DIR", "ENV_SOURCE_MUST_NOT_APPEAR"),
        ])
        .output()
        .expect("run checker");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("master_broker_port=9003"), "{stdout}");
    assert!(stdout.contains("changelog_port=9005"), "{stdout}");
    assert!(!stdout.contains("ENV_SOURCE_MUST_NOT_APPEAR"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("ENV_SOURCE_MUST_NOT_APPEAR"));
}

#[test]
fn missing_file_cannot_be_replaced_by_environment() {
    let directory = tempfile::tempdir().expect("tempdir");
    let output = checker(directory.path())
        .env("MASTER_BROKER_PORT", "8770")
        .output()
        .expect("run checker");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nicht gelesen"));
}

#[test]
fn default_repository_relative_path_is_used() {
    let directory = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(directory.path().join("config")).expect("config dir");
    write_config(&directory.path().join("config/bot.toml"), 9003);
    let output = checker(directory.path()).output().expect("run checker");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("master_broker_port=9003"));
}

#[test]
fn next_process_observes_an_edited_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("bot.toml");
    write_config(&path, 9003);
    let first = checker(directory.path())
        .arg("--config")
        .arg(&path)
        .output()
        .expect("first startup");
    assert!(first.status.success());
    assert!(String::from_utf8_lossy(&first.stdout).contains("master_broker_port=9003"));

    write_config(&path, 9004);
    let second = checker(directory.path())
        .arg("--config")
        .arg(&path)
        .output()
        .expect("second startup");
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("master_broker_port=9004"));
}

#[test]
fn malformed_file_is_rejected_without_echoing_content() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("bot.toml");
    std::fs::write(
        &path,
        "schema_version = 1\n[services]\nmaster_broker_port = 'SYNTHETIC_MARKER'",
    )
    .expect("write invalid fixture");
    let output = checker(directory.path())
        .arg("--config")
        .arg(&path)
        .output()
        .expect("run checker");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SYNTHETIC_MARKER"));
}

#[test]
fn unknown_arguments_cannot_silently_select_defaults() {
    let directory = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(directory.path().join("config")).expect("config dir");
    write_config(&directory.path().join("config/bot.toml"), 9003);
    for arguments in [
        vec!["--conf", "bot.toml"],
        vec!["--config"],
        vec!["--config="],
        vec!["--config", "one", "--config", "two"],
    ] {
        let output = checker(directory.path())
            .args(arguments)
            .output()
            .expect("run checker");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn equals_form_selects_the_requested_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    write_config(&directory.path().join("other.toml"), 9003);
    let output = checker(directory.path())
        .arg("--config=other.toml")
        .output()
        .expect("run checker");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("master_broker_port=9003"));
}

#[test]
fn validation_does_not_rewrite_the_operator_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("bot.toml");
    write_config(&path, 9003);
    let before = std::fs::read(&path).expect("read before");
    let output = checker(directory.path())
        .arg("--config")
        .arg(&path)
        .output()
        .expect("run checker");
    assert!(output.status.success());
    assert_eq!(std::fs::read(&path).expect("read after"), before);
    let files = std::fs::read_dir(directory.path())
        .expect("read dir")
        .map(|entry| entry.expect("entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].as_os_str(), OsStr::new("bot.toml"));
}
