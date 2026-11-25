use dl_core::{bot_config::BotConfigStore, operating_config::EditError};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};
fn setup() -> (tempfile::TempDir, BotConfigStore) {
    let directory = tempfile::tempdir().expect("Tempdir");
    let path = directory.path().join("bot.toml");
    fs::write(&path, "# Betreiberkommentar\nschema_version=1\n[llm.fireworks]\nmodel='accounts/fireworks/models/deepseek-v4-flash-0731' # Pin\n[runtime.community]\nconcierge_proactive=false\n").expect("Fixture");
    let store = BotConfigStore::open(path).expect("Store");
    (directory, store)
}
fn changes(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).expect("Änderungen")
}
#[test]
fn settings_reach_real_runtime_lookup_and_survive_restart_without_rounding() {
    let (_directory, store) = setup();
    let before = store.read_versioned().expect("Stand");
    let saved = store
        .save_changes_if_revision(
            &before.revision,
            &changes(json!({
                "llm.use_cases.bot_pate.max_output_tokens":"2048",
                "llm.use_cases.bot_pate.temperature":0.4,
                "llm.use_cases.bot_pate.reasoning_effort":"none",
                "llm.use_cases.bot_pate.request_timeout_seconds":"45",
                "runtime.community.concierge_proactive":true,
                "runtime.community.concierge_pate_channel_id":"1547199955133927464",
                "runtime.ai.brain_channels":["1547199955133927464","1547199955133927465"],
                "tempvoice.empty_lane_grace_seconds":"180"
            })),
        )
        .expect("Speichern");
    assert_ne!(before.revision, saved.revision);
    assert_eq!(
        store
            .snapshot()
            .expect("alter Prozess")
            .runtime_value("DL_CONCIERGE_PROACTIVE")
            .as_deref(),
        Some("0")
    );
    let restarted = BotConfigStore::open(store.path())
        .expect("Neustart")
        .snapshot()
        .expect("Snapshot");
    for (key, value) in [
        ("DL_LLM_MAX_OUTPUT_TOKENS_BOT_PATE", "2048"),
        ("DL_LLM_TEMPERATURE_BOT_PATE", "0.4"),
        ("DL_LLM_REASONING_EFFORT_BOT_PATE", "none"),
        ("DL_LLM_REQUEST_TIMEOUT_SECONDS_BOT_PATE", "45"),
        ("DL_CONCIERGE_PATE_CHANNEL_ID", "1547199955133927464"),
        (
            "BRAIN_CHANNEL_ALLOWLIST",
            "1547199955133927464,1547199955133927465",
        ),
    ] {
        assert_eq!(
            restarted.runtime_value(key).as_deref(),
            Some(value),
            "{key}"
        );
    }
    assert_eq!(restarted.tempvoice.empty_lane_grace_seconds, 180);
    let text = fs::read_to_string(store.path()).expect("Datei");
    assert!(text.contains("# Betreiberkommentar"));
    assert!(text.contains("# Pin"));
}
#[test]
fn whole_numbers_save_and_reload_for_float_settings() {
    let (_directory, store) = setup();
    let before = store.read_versioned().expect("Stand");
    store
        .save_changes_if_revision(
            &before.revision,
            &changes(json!({
                "llm.use_cases.bot_pate.temperature": 1,
                "runtime.bridges.twitch_timeout_seconds": 1,
                "runtime.moderation.analyze_flag_threshold": 1
            })),
        )
        .expect("Ganzzahl-Eingaben für f64 speichern");
    let text = fs::read_to_string(store.path()).expect("gespeicherte Datei");
    let parsed: toml::Value = toml::from_str(&text).expect("gültiges TOML");
    assert_eq!(
        parsed["llm"]["use_cases"]["bot_pate"]["temperature"].as_float(),
        Some(1.0)
    );
    assert_eq!(
        parsed["runtime"]["bridges"]["twitch_timeout_seconds"].as_float(),
        Some(1.0)
    );
    assert_eq!(
        parsed["runtime"]["moderation"]["analyze_flag_threshold"].as_float(),
        Some(1.0)
    );
    BotConfigStore::open(store.path()).expect("gültig nach Neustart");
}
#[test]
fn invalid_protected_or_expensive_pins_do_not_partially_write() {
    let (_directory, store) = setup();
    let before = store.read_versioned().expect("Stand");
    let bytes = fs::read(store.path()).expect("Datei");
    for invalid in [
        json!({"runtime.start.owner_id":"1547199955133927464"}),
        json!({"runtime.ai.fireworks_base_url":"https://untrusted.invalid"}),
        json!({"llm.fireworks.model":"accounts/fireworks/models/deepseek-v4-pro"}),
        json!({"llm.use_cases.bot_pate.max_output_tokens":"0"}),
        json!({"llm.use_cases.bot_pate.temperature":3}),
        json!({"llm.use_cases.bot_pate.reasoning_effort":"unexpected"}),
        json!({"runtime.community.concierge_pate_channel_id":1547199955133927464u64}),
        json!({"unknown.option":"not-accepted"}),
    ] {
        let mut change = changes(invalid);
        change.insert("moderation.enforce".into(), json!(true));
        assert!(store
            .save_changes_if_revision(&before.revision, &change)
            .is_err());
        assert_eq!(fs::read(store.path()).expect("Datei"), bytes);
    }
}
#[test]
fn null_removes_an_override_but_false_is_explicit_and_stale_updates_conflict() {
    let (_directory, store) = setup();
    let before = store.read_versioned().expect("Stand");
    let saved = store
        .save_changes_if_revision(
            &before.revision,
            &changes(json!({"runtime.community.concierge_proactive":null})),
        )
        .expect("Standard");
    assert_eq!(saved.config.runtime_value("DL_CONCIERGE_PROACTIVE"), None);
    assert!(matches!(
        store.save_changes_if_revision(
            &before.revision,
            &changes(json!({"runtime.community.concierge_proactive":false}))
        ),
        Err(EditError::Conflict)
    ));
    let saved = store
        .save_changes_if_revision(
            &saved.revision,
            &changes(json!({"runtime.community.concierge_proactive":false})),
        )
        .expect("Aus");
    assert_eq!(
        saved
            .config
            .runtime_value("DL_CONCIERGE_PROACTIVE")
            .as_deref(),
        Some("0")
    );
}

#[test]
fn automatic_model_bindings_reject_manual_writes_without_changing_the_config() {
    let (_directory, store) = setup();
    let before = store.read_versioned().expect("Stand");
    let bytes = fs::read(store.path()).expect("Datei");
    for path in [
        "llm.fireworks.model",
        "llm.default_provider",
        "llm.use_cases.bot_pate.model",
        "llm.use_cases.moderation_verify.provider",
        "runtime.ai.moderation_image_model",
    ] {
        let error = store
            .save_changes_if_revision(
                &before.revision,
                &BTreeMap::from([(
                    path.into(),
                    json!("accounts/fireworks/models/deepseek-v4p1-flash"),
                )]),
            )
            .err()
            .expect("automatische Auswahl ist nicht editierbar");
        assert!(error.to_string().contains("automatisch"));
        assert_eq!(fs::read(store.path()).expect("Datei"), bytes);
    }
}
