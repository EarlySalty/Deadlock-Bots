#![cfg(feature = "testing")]

use dl_central_db::testing::test_pool;

const MIGRATION: &str = include_str!("../migrations/2026071112_pate_journey_metadata_scrub.sql");

#[tokio::test]
async fn migration_entfernt_pate_id_auch_nach_spaeterem_journey_event() {
    let db = test_pool().await.expect("create migrated test pool");
    sqlx::query(
        r#"INSERT INTO activity.journey_user_state(
               user_id, guild_id, last_event_at, last_event_type, metadata
           ) VALUES
               (42, 1, now(), 'first_message',
                '{"pate_id":"77","channel_id":"900","later_channel":"901","keep":"yes"}'::jsonb),
               (43, 1, now(), 'concierge_reply',
                '{"pate_id":"77","channel_id":"901","keep":"yes"}'::jsonb)"#,
    )
    .execute(db.pool())
    .await
    .expect("insert legacy state");
    sqlx::query(
        r#"INSERT INTO activity.journey_events(
               user_id, guild_id, event_type, event_source, metadata
           ) VALUES (
               43, 1, 'steckbrief_posted', 'migration_test',
               '{"channel_id":"901"}'::jsonb
           )"#,
    )
    .execute(db.pool())
    .await
    .expect("insert proven later channel");

    sqlx::raw_sql(MIGRATION)
        .execute(db.pool())
        .await
        .expect("reapply idempotent scrub");
    sqlx::raw_sql(MIGRATION)
        .execute(db.pool())
        .await
        .expect("second idempotent scrub");

    let metadata = sqlx::query_as::<_, (bool, Option<String>, Option<String>, Option<String>)>(
        "SELECT metadata ? 'pate_id', metadata ->> 'channel_id',
                metadata ->> 'later_channel', metadata ->> 'keep'
           FROM activity.journey_user_state
          WHERE user_id = 42 AND guild_id = 1",
    )
    .fetch_one(db.pool())
    .await
    .expect("scrubbed metadata");
    assert_eq!(
        metadata,
        (
            false,
            None,
            Some("901".to_string()),
            Some("yes".to_string())
        )
    );

    let later_channel = sqlx::query_as::<_, (bool, Option<String>, Option<String>)>(
        "SELECT metadata ? 'pate_id', metadata ->> 'channel_id', metadata ->> 'keep'
           FROM activity.journey_user_state
          WHERE user_id = 43 AND guild_id = 1",
    )
    .fetch_one(db.pool())
    .await
    .expect("proven later channel");
    assert_eq!(
        later_channel,
        (false, Some("901".to_string()), Some("yes".to_string()))
    );
}
