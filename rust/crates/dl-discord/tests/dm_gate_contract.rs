use std::path::{Path, PathBuf};

use dl_discord::dm_gate::{attach_optout_button, DmIntent, DM_OPTOUT_CUSTOM_ID};
use serde_json::{json, Map, Value};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_the_gate_opens_dm_channels() {
    let root = workspace_root();
    let gate = root.join("crates/dl-discord/src/dm_gate.rs");
    let mut files = Vec::new();
    rust_sources(&root.join("crates"), &mut files);
    rust_sources(&root.join("bin"), &mut files);
    let forbidden = [
        "http.create_private_channel(",
        "recipient_id",
        "create_dm_channel(",
        "direct_message(",
    ];
    let offenders: Vec<String> = files
        .iter()
        .filter(|path| **path != gate && !path.ends_with("tests/dm_gate_contract.rs"))
        .filter_map(|path| {
            let source = std::fs::read_to_string(path).ok()?;
            forbidden
                .iter()
                .find(|needle| source.contains(*needle))
                .map(|needle| format!("{} enthaelt {needle}", path.display()))
        })
        .collect();
    assert!(offenders.is_empty(), "DM ohne Gate: {offenders:?}");
}

#[test]
fn only_proactive_dms_are_gated() {
    assert!(DmIntent::Proactive("test").is_gated());
    assert!(!DmIntent::Reply.is_gated());
    assert!(!DmIntent::Moderation.is_gated());
    assert!(!DmIntent::Security.is_gated());
    assert!(!DmIntent::Cleanup.is_gated());
}

#[test]
fn optout_button_is_attached_once() {
    let mut body: Map<String, Value> = Map::new();
    body.insert("content".into(), json!("Hallo"));
    attach_optout_button(&mut body);
    attach_optout_button(&mut body);
    let rendered = body["components"].to_string();
    assert_eq!(rendered.matches(DM_OPTOUT_CUSTOM_ID).count(), 1);
}

#[cfg(feature = "testing")]
mod db {
    use dl_central_db::testing::test_pool;
    use dl_discord::dm_gate::{check_dm_allowed, DmError, DmIntent};

    fn has_dsn() -> bool {
        ["CENTRAL_TEST_DSN", "DATABASE_URL", "DEADLOCK_CENTRAL_DSN"]
            .iter()
            .any(|name| std::env::var(name).is_ok())
    }

    #[tokio::test]
    #[ignore = "requires a central Postgres DSN; run with --include-ignored"]
    async fn opted_out_user_gets_no_proactive_dm_but_exempt_paths_pass() {
        if !has_dsn() {
            eprintln!(
                "skipping: CENTRAL_TEST_DSN, DATABASE_URL or DEADLOCK_CENTRAL_DSN is required"
            );
            return;
        }
        let db = test_pool().await.expect("create migrated test pool");
        let pool = db.pool();
        let user_id = 292_362_192_219_668_480_u64;

        check_dm_allowed(Some(pool), user_id, DmIntent::Proactive("test"))
            .await
            .expect("not opted out yet");

        sqlx::query(
            "INSERT INTO bot.kv_store (ns, k, v) VALUES ('solo_lfg_never_ask', $1, 'true')",
        )
        .bind(user_id.to_string())
        .execute(pool)
        .await
        .expect("feature opt-out");

        assert!(dl_central_db::is_dm_opted_out(pool, user_id)
            .await
            .expect("read global opt-out"));
        assert!(matches!(
            check_dm_allowed(Some(pool), user_id, DmIntent::Proactive("voice_feedback")).await,
            Err(DmError::OptedOut)
        ));
        for intent in [
            DmIntent::Reply,
            DmIntent::Moderation,
            DmIntent::Security,
            DmIntent::Cleanup,
        ] {
            check_dm_allowed(Some(pool), user_id, intent)
                .await
                .expect("exempt path passes");
        }

        let other = 4_242_u64;
        dl_central_db::set_dm_opted_out(pool, other, "dm_optout_button")
            .await
            .expect("button opt-out");
        assert!(matches!(
            check_dm_allowed(Some(pool), other, DmIntent::Proactive("router_intro")).await,
            Err(DmError::OptedOut)
        ));
    }

    #[tokio::test]
    async fn missing_gate_fails_closed() {
        assert!(matches!(
            check_dm_allowed(None, 1, DmIntent::Proactive("test")).await,
            Err(DmError::GateUnavailable(_))
        ));
        check_dm_allowed(None, 1, DmIntent::Moderation)
            .await
            .expect("moderation needs no gate");
    }
}
