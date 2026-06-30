use dl_central_db::{connect_pool, upsert_user};

fn test_dsn() -> String {
    std::env::var("CENTRAL_TEST_DSN")
        .expect("CENTRAL_TEST_DSN must be set for ignored DB tests; do not silently skip")
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via test script/CI"]
async fn core_users_join_to_verified_steam_links_by_discord_id() {
    let dsn = test_dsn();
    let pool = connect_pool(&dsn).await.expect("connect test database");

    let linked_discord_id = 42_i64;
    let unlinked_discord_id = 43_i64;
    let orphan_discord_id = 44_i64;
    let steam_id64 = 7_656_119_801_234_567_i64;
    let orphan_steam_id64 = 7_656_119_801_234_568_i64;
    let steam_id = steam_id64.to_string();
    let orphan_steam_id = orphan_steam_id64.to_string();

    sqlx::query!(
        "DELETE FROM core.steam_links WHERE discord_id IN ($1, $2, $3)",
        linked_discord_id,
        unlinked_discord_id,
        orphan_discord_id,
    )
    .execute(&pool)
    .await
    .expect("clean test steam links");

    sqlx::query!(
        "DELETE FROM core.users WHERE discord_id IN ($1, $2, $3)",
        linked_discord_id,
        unlinked_discord_id,
        orphan_discord_id,
    )
    .execute(&pool)
    .await
    .expect("clean test users");

    let orphan_insert = sqlx::query(
        r#"
        INSERT INTO core.steam_links (discord_id, steam_id, steam_id64, verified)
        VALUES ($1, $2, $3, true)
        "#,
    )
    .bind(orphan_discord_id)
    .bind(&orphan_steam_id)
    .bind(orphan_steam_id64)
    .execute(&pool)
    .await;

    assert!(
        orphan_insert.is_err(),
        "steam_link without matching core.users row must fail"
    );
    match orphan_insert.expect_err("orphan insert error") {
        sqlx::Error::Database(db_error) => {
            assert_eq!(
                db_error.code().as_deref(),
                Some("23503"),
                "orphan insert must fail with a foreign-key violation"
            );
        }
        err => panic!("orphan insert must fail with a database FK error, got {err:?}"),
    }

    upsert_user(&pool, linked_discord_id, Some("alice"), Some("Alice"), None)
        .await
        .expect("insert linked user");

    sqlx::query(
        r#"
        INSERT INTO core.steam_links (discord_id, steam_id, steam_id64, verified)
        VALUES ($1, $2, $3, true)
        "#,
    )
    .bind(linked_discord_id)
    .bind(&steam_id)
    .bind(steam_id64)
    .execute(&pool)
    .await
    .expect("insert steam link");

    let joined: (Option<String>, String, Option<i64>, bool) = sqlx::query_as(
        r#"
        SELECT u.username, s.steam_id, s.steam_id64, s.verified
        FROM core.users u
        JOIN core.steam_links s ON s.discord_id = u.discord_id
        WHERE u.discord_id = $1
        "#,
    )
    .bind(linked_discord_id)
    .fetch_one(&pool)
    .await
    .expect("joined core user and steam link");

    assert_eq!(joined.0.as_deref(), Some("alice"));
    assert_eq!(joined.1, steam_id);
    assert_eq!(joined.2, Some(steam_id64));
    assert!(joined.3);

    upsert_user(&pool, unlinked_discord_id, Some("bob"), Some("Bob"), None)
        .await
        .expect("insert unlinked user");

    let missing_join: Option<(Option<String>, String, Option<i64>, bool)> = sqlx::query_as(
        r#"
        SELECT u.username, s.steam_id, s.steam_id64, s.verified
        FROM core.users u
        JOIN core.steam_links s ON s.discord_id = u.discord_id
        WHERE u.discord_id = $1
        "#,
    )
    .bind(unlinked_discord_id)
    .fetch_optional(&pool)
    .await
    .expect("join for unlinked user");

    assert!(
        missing_join.is_none(),
        "user without steam_link must not appear in the inner join"
    );

    sqlx::query!(
        "DELETE FROM core.users WHERE discord_id = $1",
        linked_discord_id,
    )
    .execute(&pool)
    .await
    .expect("delete linked user");

    let remaining_links = sqlx::query_scalar!(
        r#"
        SELECT count(*)::BIGINT AS "count!"
        FROM core.steam_links
        WHERE discord_id = $1
        "#,
        linked_discord_id,
    )
    .fetch_one(&pool)
    .await
    .expect("count links after deleting linked user");

    assert_eq!(
        remaining_links, 0,
        "deleting core.users must cascade to core.steam_links"
    );

    sqlx::query!(
        "DELETE FROM core.steam_links WHERE discord_id IN ($1, $2, $3)",
        linked_discord_id,
        unlinked_discord_id,
        orphan_discord_id,
    )
    .execute(&pool)
    .await
    .expect("delete test steam links");

    sqlx::query!(
        "DELETE FROM core.users WHERE discord_id IN ($1, $2, $3)",
        linked_discord_id,
        unlinked_discord_id,
        orphan_discord_id,
    )
    .execute(&pool)
    .await
    .expect("delete test users");
}
