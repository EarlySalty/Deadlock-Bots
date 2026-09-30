use dl_core::admin_config::{parse, Dashboard, Error, Registry, Target, Validator, MAX_BYTES};
use std::{fs, path::Path};

const ORIGINAL: &str = "# Betreiberkommentar bleibt erhalten\nschema_version = 1\n[concierge]\ntimeout_seconds = 100 # Zeitbudget\n[moderation]\nenforce = false\n";
fn target(path: &Path) -> Target {
    Target {
        id: "discord".into(),
        title: "Discord".into(),
        dashboard: Dashboard::Discord,
        path: path.to_owned(),
        validator: Validator::Discord,
        program: None,
        args: vec![],
        units: vec!["deadlock-bot-rust.service".into()],
        enabled: true,
        note: String::new(),
        protected: vec![],
    }
}
fn setup() -> (tempfile::TempDir, Target) {
    let directory = tempfile::tempdir().expect("Testverzeichnis");
    let path = directory.path().join("bot.toml");
    fs::write(&path, ORIGINAL).expect("Testkonfiguration");
    let target = target(&path);
    (directory, target)
}

#[test]
fn snapshot_strips_comments_and_preview_does_not_write() {
    let (_directory, target) = setup();
    let saved = target.snapshot().expect("Snapshot");
    assert!(!saved.toml.contains("Betreiberkommentar"));
    let text = saved
        .toml
        .replace("timeout_seconds = 100", "timeout_seconds = 75");
    let result = target.preview(&saved.revision, &text).expect("Vorschau");
    assert_eq!(result.changed, ["concierge.timeout_seconds"]);
    assert_eq!(fs::read_to_string(&target.path).expect("Datei"), ORIGINAL);
}

#[test]
fn save_preserves_comments_permissions_and_previous_version() {
    let (_directory, target) = setup();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target.path, fs::Permissions::from_mode(0o640)).expect("Rechte");
    }
    let before = target.snapshot().expect("Snapshot");
    let text = before
        .toml
        .replace("timeout_seconds = 100", "timeout_seconds = 75");
    let after = target.save(&before.revision, &text).expect("Speichern");
    assert_ne!(before.revision, after.revision);
    let stored = fs::read_to_string(&target.path).expect("Datei");
    assert!(stored.contains("# Betreiberkommentar bleibt erhalten"));
    assert!(stored.contains("# Zeitbudget"));
    assert!(stored.contains("timeout_seconds = 75"));
    assert!(after.history.contains(&before.revision));
    assert_eq!(
        target.history_snapshot(&before.revision).expect("Historie"),
        before.toml
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&target.path)
                .expect("Metadaten")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }
}

#[test]
fn semantic_noop_keeps_revision_and_conflicts_preserve_disk() {
    let (_directory, target) = setup();
    let first = target.snapshot().expect("Snapshot");
    let same = target
        .save(&first.revision, &first.toml)
        .expect("Unverändert");
    assert_eq!(same.revision, first.revision);
    fs::write(&target.path, ORIGINAL.replace("100", "70")).expect("Anderer Schreiber");
    assert!(matches!(
        target.save(&first.revision, &first.toml),
        Err(Error::Conflict)
    ));
    assert!(fs::read_to_string(&target.path)
        .expect("Datei")
        .contains("70"));
}

#[test]
fn invalid_unknown_secret_schema_and_protected_fields_never_write() {
    let (_directory, mut target) = setup();
    let before = target.snapshot().expect("Snapshot");
    for text in [
        "schema_version = 1\nmissing = true\n".to_owned(),
        before
            .toml
            .replace("timeout_seconds = 100", "timeout_seconds = 999999"),
        before
            .toml
            .replace("schema_version = 1", "schema_version = 2"),
        "this is not toml".to_owned(),
    ] {
        assert!(target.save(&before.revision, &text).is_err());
        assert_eq!(fs::read_to_string(&target.path).expect("Datei"), ORIGINAL);
    }
    target.protected = vec!["moderation".into()];
    assert!(target
        .save(
            &before.revision,
            &before.toml.replace("enforce = false", "enforce = true")
        )
        .is_err());
    assert_eq!(fs::read_to_string(&target.path).expect("Datei"), ORIGINAL);
    for key in [
        "password",
        "api_key",
        "private_key",
        "access_token",
        "database_dsn",
    ] {
        assert!(parse(&format!("{key} = 'synthetic-noncredential'\n")).is_err());
    }
    assert!(parse("max_tokens = 1000\n").is_ok());
    assert!(parse("url = 'http://synthetic-user@example.invalid/'\n").is_err());
    assert!(matches!(
        parse(&"x".repeat(MAX_BYTES + 1)),
        Err(Error::TooLarge)
    ));
}

#[test]
fn concurrent_writer_is_rejected_without_losing_draft() {
    let (_directory, target) = setup();
    let before = target.snapshot().expect("Snapshot");
    let source = target.safe_source().expect("Quelle");
    let _lock = target.lock(&source).expect("Sperre");
    assert!(matches!(
        target.save(&before.revision, &before.toml),
        Err(Error::Busy)
    ));
}

#[test]
fn history_is_bounded_and_restoring_requires_explicit_save() {
    let (_directory, target) = setup();
    let mut current = target.snapshot().expect("Snapshot");
    let mut previous = current.revision.clone();
    for value in 50..76 {
        let mut parsed = parse(&current.toml).expect("TOML");
        parsed["concierge"]["timeout_seconds"] = toml::Value::Integer(value);
        previous = current.revision.clone();
        current = target
            .save(&previous, &toml::to_string(&parsed).expect("TOML"))
            .expect("Speichern");
    }
    let history = target
        .path
        .parent()
        .expect("Verzeichnis")
        .join(".admin-config-history/discord");
    assert!(fs::read_dir(&history).expect("Historie").count() <= 20);
    assert!(current.history.contains(&previous));
    let draft = target.history_snapshot(&previous).expect("Vorgänger");
    assert_ne!(draft, current.toml);
    assert_eq!(
        target.snapshot().expect("Snapshot").revision,
        current.revision
    );
    assert!(target.history_snapshot("../../bot.toml").is_err());
}

#[cfg(unix)]
#[test]
fn symlink_hardlink_and_git_source_are_never_editable() {
    use std::os::unix::fs::symlink;
    let (directory, target) = setup();
    let linked = directory.path().join("alias.toml");
    symlink(&target.path, &linked).expect("Symlink");
    let mut alias = target.clone();
    alias.path = linked;
    assert!(alias.snapshot().is_err());
    fs::hard_link(&target.path, directory.path().join("hard.toml")).expect("Hardlink");
    assert!(target.snapshot().is_err());
    fs::remove_file(directory.path().join("hard.toml")).expect("Hardlink entfernen");
    fs::write(directory.path().join(".git"), "test").expect("Git-Marker");
    assert!(target.snapshot().is_err());
}

#[test]
fn registry_scope_and_unit_allowlist_are_enforced() {
    let (directory, target) = setup();
    let path = directory.path().join("admin-bots.toml");
    let registry = format!("schema_version = 1\napply_binary = '/usr/bin/false'\n[[bots]]\nid = 'discord'\ntitle = 'Discord'\ndashboard = 'discord'\npath = '{}'\nvalidator = 'discord'\nunits = ['deadlock-bot-rust.service']\nenabled = true\n", target.path.display());
    fs::write(&path, &registry).expect("Registry");
    let parsed = Registry::load(&path).expect("Registry lesen");
    assert!(parsed.target("discord", Dashboard::Discord).is_ok());
    assert!(matches!(
        parsed.target("discord", Dashboard::Twitch),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        parsed.target("../../private", Dashboard::Discord),
        Err(Error::NotFound)
    ));
    for text in [
        registry.replace("deadlock-bot-rust.service", "ssh.service"),
        registry.replace("dashboard = 'discord'", "dashboard = 'twitch'"),
        registry.replace("validator = 'discord'", "validator = 'unknown'"),
        registry.replace("enabled = true", "enabled = true\nunknown = 1"),
    ] {
        fs::write(&path, text).expect("Registry");
        assert!(Registry::load(&path).is_err());
    }
}

#[test]
fn unchanged_array_tables_keep_operator_comments() {
    let (_directory, mut target) = setup();
    target.validator = Validator::Command;
    target.program = Some("/usr/bin/true".into());
    target.args = vec!["{config}".into()];
    let original = "schema_version = 1\nlimit = 5\n# Primary account\n[[accounts]]\nid = 1 # Stable ID\nlabel = 'First'\n";
    fs::write(&target.path, original).expect("Fixture");
    let before = target.snapshot().expect("Snapshot");
    target
        .save(
            &before.revision,
            &before.toml.replace("limit = 5", "limit = 6"),
        )
        .expect("Speichern");
    let result = fs::read_to_string(&target.path).expect("Datei");
    assert!(result.contains("# Primary account\n[[accounts]]\nid = 1 # Stable ID\nlabel = 'First'"));
}

#[test]
fn native_and_dashboard_writers_share_the_same_lock() {
    for (id, name) in [
        ("discord", ".bot.toml.lock"),
        ("twitch", ".bot.toml.lock"),
        ("steam", "bot.toml.lock"),
    ] {
        let (directory, mut target) = setup();
        target.id = id.into();
        let lock = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(directory.path().join(name))
            .expect("Native Sperrdatei");
        lock.try_lock().expect("Native Sperre");
        assert!(
            matches!(target.lock(&target.path), Err(Error::Busy)),
            "{id}"
        );
        drop(lock);
        assert!(target.lock(&target.path).is_ok(), "{id}");
    }
}

#[test]
fn failed_external_validator_is_safe_and_does_not_modify_source() {
    let (_directory, mut target) = setup();
    target.validator = Validator::Command;
    target.program = Some("/usr/bin/false".into());
    target.args = vec!["{config}".into()];
    let result = target.snapshot();
    assert!(matches!(result, Err(Error::Invalid(_))));
    assert_eq!(fs::read_to_string(&target.path).expect("Datei"), ORIGINAL);
    assert!(!fs::read_dir(target.path.parent().expect("Verzeichnis"))
        .expect("Dateien")
        .filter_map(Result::ok)
        .any(|file| file
            .file_name()
            .to_string_lossy()
            .starts_with(".bot-config.")));
}
