use dl_core::{bot_config::BotConfig, operating_config::fingerprint};

#[test]
fn typed_runtime_values_reach_existing_constructor_keys() {
    let config = BotConfig::parse(
        r#"
schema_version=1
[discord]
guild_id="1234"
[features]
gateway=true
[services]
master_broker_port=17770
dashboard_port=17766
[runtime.start]
command_scope="both"
presence_intent=false
broker_channels=[111,222]
[runtime.bridges]
matcher_enabled=false
twitch_timeout_seconds=3.5
[runtime.dashboard]
discord_client_id="3456"
session_ttl_seconds=3600
[runtime.web]
cors_origins=["https://example.invalid"]
"#,
    )
    .expect("typisierte Testkonfiguration");
    for (key, value) in [
        ("MAIN_GUILD_ID", "1234"),
        ("OUR_GUILD_ID", "1234"),
        ("DL_BOT_GATEWAY", "1"),
        ("DL_ENABLE_PRESENCE_INTENT", "0"),
        ("COMMAND_SYNC_START_SCOPE", "both"),
        ("MASTER_BROKER_CHANNEL_ALLOWLIST_IDS", "111,222"),
        ("MASTER_BROKER_BASE_URL", "http://127.0.0.1:17770"),
        ("DASHBOARD_INTERNAL_API_BASE", "http://127.0.0.1:17766"),
        ("STREAMER_LINK_ENABLED", "0"),
        ("TWITCH_INTERNAL_API_TIMEOUT_SEC", "3.5"),
        ("DISCORD_OAUTH_CLIENT_ID", "3456"),
        ("MASTER_DASHBOARD_SESSION_TTL_SEC", "3600"),
    ] {
        assert_eq!(config.runtime_value(key).as_deref(), Some(value), "{key}");
    }
    assert!(config.runtime_value("TWITCH_INTERNAL_API_TOKEN").is_none());
    assert!(config.runtime_value("UNDECLARED_VALUE").is_none());
}

#[test]
fn explicit_paths_resolve_from_config_and_not_the_process_directory() {
    let directory = tempfile::tempdir().expect("Testverzeichnis");
    let path = directory.path().join("bot.toml");
    std::fs::write(
        &path,
        "schema_version=1\n[runtime.dashboard]\ndata_dir='state'\n",
    )
    .expect("Testdatei");
    let config = BotConfig::load(&path).expect("Config");
    assert_eq!(
        config.runtime.dashboard.data_dir.as_deref(),
        Some(directory.path().join("state").as_path())
    );
    assert_eq!(
        fingerprint(&config).expect("Fingerprint"),
        fingerprint(&BotConfig::load(&path).expect("Config")).expect("Fingerprint")
    );
}

#[test]
fn dangerous_invalid_or_secret_runtime_fields_fail_before_clients() {
    for section in [
        "[runtime.start]\nmcp_host='0.0.0.0'",
        "[runtime.start]\nbroker_idempotency_ttl_seconds=nan",
        "[runtime.start]\ncommand_scope='invalid'",
        "[runtime.bridges]\ntwitch_timeout_seconds=inf",
        "[runtime.bridges]\nturnier_api_url='http://user:secret@127.0.0.1/'",
        "[runtime.dashboard]\nsession_ttl_seconds=0",
        "[runtime.dashboard]\nmaster_broker_token='secret'",
        "[runtime.web]\ntierlist_refresh='true'",
    ] {
        assert!(
            BotConfig::parse(&format!("schema_version=1\n{section}\n")).is_err(),
            "{section}"
        );
    }
}
