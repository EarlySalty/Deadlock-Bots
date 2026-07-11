#![cfg(feature = "testing")]

use std::path::Path;

use dl_central_db::testing::test_pool;
use sqlx::PgPool;

const DECISION_PREFIX: &str = "identity_backfill:";

fn migration_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("migrations")
        .join("2026071125_steam_bot_event_log_identity_backfill.sql")
}

#[allow(clippy::too_many_arguments)]
async fn insert_event(
    pool: &PgPool,
    decision: &str,
    event_type: &str,
    discord_id: Option<i64>,
    steam_id: Option<i64>,
    reason: Option<&str>,
    detail: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO steam.bot_event_log(event_type, decision, discord_id, steam_id, reason, detail)
         VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
    )
    .bind(event_type)
    .bind(format!("{DECISION_PREFIX}{decision}"))
    .bind(discord_id)
    .bind(steam_id)
    .bind(reason)
    .bind(detail)
    .execute(pool)
    .await
    .expect("insert bot event log row");
}

async fn seed_user(pool: &PgPool, discord_id: i64) {
    sqlx::query("INSERT INTO core.users(discord_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(discord_id)
        .execute(pool)
        .await
        .expect("seed core user");
}

async fn seed_privacy(pool: &PgPool, discord_id: i64, opted_out: bool, deleted: bool) {
    sqlx::query(
        "INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason)
         VALUES ($1, $2, CASE WHEN $3 THEN now() ELSE NULL END, 'test')",
    )
    .bind(discord_id)
    .bind(opted_out)
    .bind(deleted)
    .execute(pool)
    .await
    .expect("seed privacy tombstone");
}

async fn seed_link(pool: &PgPool, discord_id: i64, steam_id: &str, steam_id64: Option<i64>) {
    if discord_id != 0 {
        seed_user(pool, discord_id).await;
    }

    sqlx::query(
        "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
         VALUES ($1, $2, $3)",
    )
    .bind(discord_id)
    .bind(steam_id)
    .bind(steam_id64)
    .execute(pool)
    .await
    .expect("seed steam link");
}

#[derive(Debug, PartialEq, Eq)]
struct EventSnapshot {
    discord_id: Option<i64>,
    steam_id: Option<i64>,
    reason: Option<String>,
    detail_is_null: bool,
    has_trigger: bool,
    trigger_type: Option<String>,
    trigger_text: Option<String>,
    keep: Option<String>,
    other: Option<String>,
}

async fn event(pool: &PgPool, decision: &str) -> EventSnapshot {
    let row = sqlx::query_as::<
        _,
        (
            Option<i64>,
            Option<i64>,
            Option<String>,
            bool,
            bool,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT discord_id,
                steam_id,
                reason,
                detail IS NULL,
                COALESCE(detail ? 'trigger', false),
                jsonb_typeof(detail -> 'trigger'),
                detail ->> 'trigger',
                detail ->> 'keep',
                detail ->> 'other'
           FROM steam.bot_event_log
          WHERE decision = $1",
    )
    .bind(format!("{DECISION_PREFIX}{decision}"))
    .fetch_one(pool)
    .await
    .expect("fetch event snapshot");

    EventSnapshot {
        discord_id: row.0,
        steam_id: row.1,
        reason: row.2,
        detail_is_null: row.3,
        has_trigger: row.4,
        trigger_type: row.5,
        trigger_text: row.6,
        keep: row.7,
        other: row.8,
    }
}

async fn state_digest(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT string_agg(
                    decision || '|' ||
                    COALESCE(discord_id::text, '<NULL>') || '|' ||
                    COALESCE(steam_id::text, '<NULL>') || '|' ||
                    COALESCE(reason, '<NULL>') || '|' ||
                    COALESCE(detail::text, '<NULL>'),
                    E'\n'
                    ORDER BY decision
                )
           FROM steam.bot_event_log
          WHERE decision LIKE $1",
    )
    .bind(format!("{DECISION_PREFIX}%"))
    .fetch_one(pool)
    .await
    .expect("event log digest")
    .unwrap_or_default()
}

#[tokio::test]
async fn steam_bot_event_log_identity_backfill_respects_privacy_and_is_idempotent() {
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    seed_privacy(pool, 1004, true, false).await;
    seed_privacy(pool, 2301, false, true).await;
    seed_privacy(pool, 2402, true, false).await;

    insert_event(
        pool,
        "manual_exact",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":"manual:1001","keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "subrank_exact",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":"subrank_manual:1002","keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_existing_discord",
        "rank_sync_run",
        Some(555),
        None,
        None,
        Some(r#"{"trigger":"manual:1003","keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_zero",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":"manual:0","keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_negative",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":"manual:-1","keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_overflow",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":"manual:9223372036854775808","keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_suffix",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":"manual:123x","keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_non_string",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":123,"keep":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_unrelated",
        "other_event",
        None,
        None,
        None,
        Some(r#"{"trigger":"manual:1005","other":"yes"}"#),
    )
    .await;
    insert_event(
        pool,
        "manual_tombstone",
        "rank_sync_run",
        None,
        None,
        None,
        Some(r#"{"trigger":"manual:1004"}"#),
    )
    .await;

    seed_link(pool, 2101, "owner-a-2101", Some(7_656_119_800_021_001)).await;
    insert_event(
        pool,
        "link_id64",
        "steam_sync",
        None,
        Some(7_656_119_800_021_001),
        Some("keep"),
        Some(r#"{"keep":"yes"}"#),
    )
    .await;

    seed_link(pool, 2102, "765611980002102", Some(7_656_119_800_021_099)).await;
    insert_event(
        pool,
        "link_text",
        "steam_sync",
        None,
        Some(765_611_980_002_102),
        Some("keep"),
        Some(r#"{"keep":"yes"}"#),
    )
    .await;

    seed_link(pool, 2103, "owner-a-2103", Some(7_656_119_800_021_003)).await;
    insert_event(
        pool,
        "link_existing_discord",
        "steam_sync",
        Some(999),
        Some(7_656_119_800_021_003),
        Some("keep"),
        Some(r#"{"keep":"yes"}"#),
    )
    .await;

    seed_link(pool, 0, "owner-zero", Some(7_656_119_800_021_004)).await;
    insert_event(
        pool,
        "link_zero_owner",
        "steam_sync",
        None,
        Some(7_656_119_800_021_004),
        Some("keep"),
        Some(r#"{"keep":"yes"}"#),
    )
    .await;

    seed_link(pool, 2201, "owner-a-2201", Some(7_656_119_800_022_001)).await;
    seed_link(pool, 2202, "7656119800022001", Some(7_656_119_800_022_002)).await;
    insert_event(
        pool,
        "link_ambiguous",
        "steam_sync",
        None,
        Some(7_656_119_800_022_001),
        Some("keep"),
        Some(r#"{"keep":"yes"}"#),
    )
    .await;

    seed_link(pool, 2401, "owner-a-2401", Some(7_656_119_800_024_001)).await;
    seed_link(pool, 2402, "7656119800024001", Some(7_656_119_800_024_002)).await;
    insert_event(
        pool,
        "link_ambiguous_tombstone",
        "steam_sync",
        None,
        Some(7_656_119_800_024_001),
        Some("personal"),
        Some(r#"{"keep":"personal"}"#),
    )
    .await;

    seed_link(pool, 2301, "owner-a-2301", Some(7_656_119_800_023_001)).await;
    insert_event(
        pool,
        "link_tombstone",
        "steam_sync",
        None,
        Some(7_656_119_800_023_001),
        Some("known"),
        Some(r#"{"keep":"known"}"#),
    )
    .await;

    let migration =
        std::fs::read_to_string(migration_path()).expect("read identity backfill migration");
    sqlx::raw_sql(&migration)
        .execute(pool)
        .await
        .expect("apply identity backfill migration");
    let after_first = state_digest(pool).await;
    sqlx::raw_sql(&migration)
        .execute(pool)
        .await
        .expect("reapply identity backfill migration");
    assert_eq!(
        state_digest(pool).await,
        after_first,
        "second migration run must leave event log rows unchanged"
    );

    let manual_exact = event(pool, "manual_exact").await;
    assert_eq!(manual_exact.discord_id, Some(1001));
    assert!(!manual_exact.has_trigger);
    assert_eq!(manual_exact.keep.as_deref(), Some("yes"));

    let subrank_exact = event(pool, "subrank_exact").await;
    assert_eq!(subrank_exact.discord_id, Some(1002));
    assert!(!subrank_exact.has_trigger);
    assert_eq!(subrank_exact.keep.as_deref(), Some("yes"));

    let manual_existing_discord = event(pool, "manual_existing_discord").await;
    assert_eq!(manual_existing_discord.discord_id, Some(555));
    assert_eq!(
        manual_existing_discord.trigger_text.as_deref(),
        Some("manual:1003")
    );

    for decision in [
        "manual_zero",
        "manual_negative",
        "manual_overflow",
        "manual_suffix",
    ] {
        let row = event(pool, decision).await;
        assert_eq!(row.discord_id, None, "{decision} must not infer an actor");
        assert!(row.has_trigger, "{decision} must keep the invalid trigger");
        assert_eq!(row.keep.as_deref(), Some("yes"));
    }

    let manual_non_string = event(pool, "manual_non_string").await;
    assert_eq!(manual_non_string.discord_id, None);
    assert_eq!(manual_non_string.trigger_type.as_deref(), Some("number"));
    assert_eq!(manual_non_string.keep.as_deref(), Some("yes"));

    let manual_unrelated = event(pool, "manual_unrelated").await;
    assert_eq!(manual_unrelated.discord_id, None);
    assert_eq!(
        manual_unrelated.trigger_text.as_deref(),
        Some("manual:1005")
    );
    assert_eq!(manual_unrelated.other.as_deref(), Some("yes"));

    let manual_tombstone = event(pool, "manual_tombstone").await;
    assert_eq!(manual_tombstone.discord_id, None);
    assert!(manual_tombstone.detail_is_null);

    let link_id64 = event(pool, "link_id64").await;
    assert_eq!(link_id64.discord_id, Some(2101));
    assert_eq!(link_id64.steam_id, Some(7_656_119_800_021_001));
    assert_eq!(link_id64.reason.as_deref(), Some("keep"));
    assert_eq!(link_id64.keep.as_deref(), Some("yes"));

    let link_text = event(pool, "link_text").await;
    assert_eq!(link_text.discord_id, Some(2102));
    assert_eq!(link_text.steam_id, Some(765_611_980_002_102));

    let link_existing_discord = event(pool, "link_existing_discord").await;
    assert_eq!(link_existing_discord.discord_id, Some(999));
    assert_eq!(link_existing_discord.steam_id, Some(7_656_119_800_021_003));
    assert_eq!(link_existing_discord.keep.as_deref(), Some("yes"));

    let link_zero_owner = event(pool, "link_zero_owner").await;
    assert_eq!(link_zero_owner.discord_id, None);
    assert_eq!(link_zero_owner.steam_id, Some(7_656_119_800_021_004));
    assert_eq!(link_zero_owner.keep.as_deref(), Some("yes"));

    let link_ambiguous = event(pool, "link_ambiguous").await;
    assert_eq!(link_ambiguous.discord_id, None);
    assert_eq!(link_ambiguous.steam_id, Some(7_656_119_800_022_001));
    assert_eq!(link_ambiguous.keep.as_deref(), Some("yes"));

    let link_ambiguous_tombstone = event(pool, "link_ambiguous_tombstone").await;
    assert_eq!(link_ambiguous_tombstone.discord_id, None);
    assert_eq!(link_ambiguous_tombstone.steam_id, None);
    assert_eq!(link_ambiguous_tombstone.reason, None);
    assert!(link_ambiguous_tombstone.detail_is_null);

    let link_tombstone = event(pool, "link_tombstone").await;
    assert_eq!(link_tombstone.discord_id, None);
    assert_eq!(link_tombstone.steam_id, None);
    assert_eq!(link_tombstone.reason, None);
    assert!(link_tombstone.detail_is_null);
}
