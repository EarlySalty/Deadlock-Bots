use dl_core::{
    bot_config::BotConfigStore,
    operating_config::{EditError, OperatingOptions},
};

#[test]
fn editor_preserves_comments_and_rejects_stale_bytes() {
    let temp = tempfile::tempdir().expect("isolierte Testdaten gültig");
    let path = temp.path().join("bot.toml");
    std::fs::write(
        &path,
        "schema_version=1\n# Betreiberhinweis\n[moderation]\nenforce=false # bewusst\n[runtime.dashboard]\ndata_dir='state'\n",
    )
    .expect("isolierte Testdaten gültig");
    let store = BotConfigStore::open(&path).expect("isolierte Testdaten gültig");
    let before = store.read_versioned().expect("isolierte Testdaten gültig");
    let options = OperatingOptions {
        moderation_enforce: true,
        concierge_timeout_seconds: 100,
    };
    let after = store
        .save_if_revision(&before.revision, &options)
        .expect("isolierte Testdaten gültig");
    assert_ne!(before.fingerprint, after.fingerprint);
    assert!(
        !store
            .snapshot()
            .expect("isolierte Testdaten gültig")
            .moderation
            .enforce
    );
    let text = std::fs::read_to_string(&path).expect("isolierte Testdaten gültig");
    assert!(text.contains("# Betreiberhinweis"));
    assert!(text.contains("# bewusst"));
    std::fs::write(&path, format!("{text}\n# externe Änderung\n"))
        .expect("isolierte Testdaten gültig");
    let commented = store.read_versioned().expect("isolierte Testdaten gültig");
    assert_eq!(after.fingerprint, commented.fingerprint);
    assert_ne!(after.revision, commented.revision);
    assert!(matches!(
        store.save_if_revision(&after.revision, &options),
        Err(EditError::Conflict)
    ));
}

#[test]
fn invalid_editor_values_and_git_locations_do_not_write() {
    let temp = tempfile::tempdir().expect("isolierte Testdaten gültig");
    let path = temp.path().join("bot.toml");
    let original = "schema_version=1\n";
    std::fs::write(&path, original).expect("isolierte Testdaten gültig");
    let store = BotConfigStore::open(&path).expect("isolierte Testdaten gültig");
    let revision = store
        .read_versioned()
        .expect("isolierte Testdaten gültig")
        .revision;
    let options = OperatingOptions {
        moderation_enforce: false,
        concierge_timeout_seconds: 0,
    };
    assert!(matches!(
        store.save_if_revision(&revision, &options),
        Err(EditError::Invalid(_))
    ));
    assert_eq!(
        std::fs::read_to_string(&path).expect("isolierte Testdaten gültig"),
        original
    );
    std::fs::create_dir(temp.path().join(".git")).expect("isolierte Testdaten gültig");
    assert!(matches!(
        store.save_if_revision(&revision, &options),
        Err(EditError::UnsafeLocation)
    ));
}

#[cfg(unix)]
#[test]
fn permissions_are_preserved_and_symlink_writes_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let temp = tempfile::tempdir().expect("isolierte Testdaten gültig");
    let path = temp.path().join("bot.toml");
    std::fs::write(&path, "schema_version=1\n").expect("isolierte Testdaten gültig");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))
        .expect("isolierte Testdaten gültig");
    let store = BotConfigStore::open(&path).expect("isolierte Testdaten gültig");
    let options = OperatingOptions {
        moderation_enforce: false,
        concierge_timeout_seconds: 99,
    };
    store
        .save_if_revision(
            &store
                .read_versioned()
                .expect("isolierte Testdaten gültig")
                .revision,
            &options,
        )
        .expect("isolierte Testdaten gültig");
    assert_eq!(
        std::fs::metadata(&path)
            .expect("isolierte Testdaten gültig")
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    let link = temp.path().join("link.toml");
    symlink(&path, &link).expect("isolierte Testdaten gültig");
    let linked = BotConfigStore::open(&link).expect("isolierte Testdaten gültig");
    assert!(matches!(
        linked.save_if_revision(
            &linked
                .read_versioned()
                .expect("isolierte Testdaten gültig")
                .revision,
            &options
        ),
        Err(EditError::UnsafeLocation)
    ));
}
