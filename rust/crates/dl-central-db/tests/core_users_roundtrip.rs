use dl_central_db::{connect_pool, get_user, upsert_user};
use sqlx::PgPool;
use tokio::time::{sleep, Duration};

fn test_dsn() -> String {
    std::env::var("CENTRAL_TEST_DSN")
        .expect("CENTRAL_TEST_DSN must be set for ignored DB tests; do not silently skip")
}

async fn clear_test_user(pool: &PgPool, discord_id: i64) {
    sqlx::query!("DELETE FROM core.users WHERE discord_id = $1", discord_id)
        .execute(pool)
        .await
        .expect("delete test user");
    sqlx::query("DELETE FROM core.user_privacy WHERE user_id = $1")
        .bind(discord_id)
        .execute(pool)
        .await
        .expect("delete test privacy tombstone");
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via test script/CI"]
async fn upsert_user_round_trips_and_updates_seen_timestamps() {
    let dsn = test_dsn();
    let pool = connect_pool(&dsn).await.expect("connect test database");
    let discord_id = 9_001_001_001_i64;
    clear_test_user(&pool, discord_id).await;

    upsert_user(
        &pool,
        discord_id,
        Some("first-user"),
        Some("First User"),
        Some("avatar-1"),
    )
    .await
    .expect("insert user");

    let inserted = get_user(&pool, discord_id)
        .await
        .expect("get inserted user")
        .expect("inserted user exists");
    assert_eq!(inserted.discord_id, discord_id);
    assert_eq!(inserted.username.as_deref(), Some("first-user"));
    assert_eq!(inserted.global_name.as_deref(), Some("First User"));
    assert_eq!(inserted.avatar.as_deref(), Some("avatar-1"));
    assert!(inserted.last_seen >= inserted.first_seen);

    sleep(Duration::from_secs(1)).await;

    upsert_user(
        &pool,
        discord_id,
        Some("updated-user"),
        Some("Updated User"),
        Some("avatar-2"),
    )
    .await
    .expect("update user");

    let updated = get_user(&pool, discord_id)
        .await
        .expect("get updated user")
        .expect("updated user exists");
    assert_eq!(updated.discord_id, discord_id);
    assert_eq!(updated.username.as_deref(), Some("updated-user"));
    assert_eq!(updated.global_name.as_deref(), Some("Updated User"));
    assert_eq!(updated.avatar.as_deref(), Some("avatar-2"));
    assert_eq!(updated.first_seen, inserted.first_seen);
    assert!(updated.last_seen >= updated.first_seen);
    assert!(updated.last_seen > inserted.last_seen);

    clear_test_user(&pool, discord_id).await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via test script/CI"]
async fn upsert_user_preserves_existing_profile_fields_when_new_values_are_none() {
    let dsn = test_dsn();
    let pool = connect_pool(&dsn).await.expect("connect test database");
    let discord_id = 9_001_001_002_i64;
    clear_test_user(&pool, discord_id).await;

    upsert_user(
        &pool,
        discord_id,
        Some("stable-user"),
        Some("Stable User"),
        Some("stable-avatar"),
    )
    .await
    .expect("insert user");

    let inserted = get_user(&pool, discord_id)
        .await
        .expect("get inserted user")
        .expect("inserted user exists");

    sleep(Duration::from_secs(1)).await;

    upsert_user(&pool, discord_id, None, None, None)
        .await
        .expect("update user without profile fields");

    let updated = get_user(&pool, discord_id)
        .await
        .expect("get updated user")
        .expect("updated user exists");
    assert_eq!(updated.discord_id, discord_id);
    assert_eq!(updated.username.as_deref(), Some("stable-user"));
    assert_eq!(updated.global_name.as_deref(), Some("Stable User"));
    assert_eq!(updated.avatar.as_deref(), Some("stable-avatar"));
    assert_eq!(updated.first_seen, inserted.first_seen);
    assert!(updated.last_seen > inserted.last_seen);

    clear_test_user(&pool, discord_id).await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via test script/CI"]
async fn upsert_user_skips_user_with_deleted_at_tombstone() {
    let dsn = test_dsn();
    let pool = connect_pool(&dsn).await.expect("connect test database");
    let discord_id = 9_001_001_003_i64;
    clear_test_user(&pool, discord_id).await;

    sqlx::query(
        r#"
        INSERT INTO core.user_privacy (user_id, opted_out, deleted_at, reason)
        VALUES ($1, FALSE, now(), 'test')
        "#,
    )
    .bind(discord_id)
    .execute(&pool)
    .await
    .expect("insert deleted user tombstone");

    upsert_user(
        &pool,
        discord_id,
        Some("deleted-user"),
        Some("Deleted User"),
        Some("deleted-avatar"),
    )
    .await
    .expect("skip deleted user");

    assert_eq!(
        get_user(&pool, discord_id).await.expect("get deleted user"),
        None
    );
    clear_test_user(&pool, discord_id).await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via test script/CI"]
async fn upsert_user_skips_opted_out_user() {
    let dsn = test_dsn();
    let pool = connect_pool(&dsn).await.expect("connect test database");
    let discord_id = 9_001_001_004_i64;
    clear_test_user(&pool, discord_id).await;

    sqlx::query(
        r#"
        INSERT INTO core.user_privacy (user_id, opted_out, reason)
        VALUES ($1, TRUE, 'test')
        "#,
    )
    .bind(discord_id)
    .execute(&pool)
    .await
    .expect("insert opted-out tombstone");

    upsert_user(
        &pool,
        discord_id,
        Some("opted-out-user"),
        Some("Opted-out User"),
        Some("opted-out-avatar"),
    )
    .await
    .expect("skip opted-out user");

    assert_eq!(
        get_user(&pool, discord_id)
            .await
            .expect("get opted-out user"),
        None
    );
    clear_test_user(&pool, discord_id).await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via test script/CI"]
async fn upsert_user_does_not_update_existing_tombstoned_user() {
    let dsn = test_dsn();
    let pool = connect_pool(&dsn).await.expect("connect test database");
    let discord_id = 9_001_001_005_i64;
    clear_test_user(&pool, discord_id).await;

    upsert_user(
        &pool,
        discord_id,
        Some("original-user"),
        Some("Original User"),
        Some("original-avatar"),
    )
    .await
    .expect("insert user");
    let original = get_user(&pool, discord_id)
        .await
        .expect("get original user")
        .expect("original user exists");

    sqlx::query(
        r#"
        INSERT INTO core.user_privacy (user_id, opted_out, deleted_at, reason)
        VALUES ($1, TRUE, now(), 'test')
        "#,
    )
    .bind(discord_id)
    .execute(&pool)
    .await
    .expect("insert user tombstone");
    sleep(Duration::from_secs(1)).await;

    upsert_user(
        &pool,
        discord_id,
        Some("revived-user"),
        Some("Revived User"),
        Some("revived-avatar"),
    )
    .await
    .expect("skip tombstoned user");

    let unchanged = get_user(&pool, discord_id)
        .await
        .expect("get tombstoned user")
        .expect("existing tombstoned user remains");
    assert_eq!(unchanged, original);
    clear_test_user(&pool, discord_id).await;
}
