#![cfg(feature = "testing")]

use dl_central_db::{kv, testing::test_pool};

#[tokio::test]
#[ignore = "requires a central Postgres DSN; run with --include-ignored"]
async fn kv_round_trips_and_deletes_values() {
    if std::env::var("CENTRAL_TEST_DSN").is_err()
        && std::env::var("DATABASE_URL").is_err()
        && std::env::var("DEADLOCK_CENTRAL_DSN").is_err()
    {
        eprintln!("skipping: CENTRAL_TEST_DSN, DATABASE_URL or DEADLOCK_CENTRAL_DSN is required");
        return;
    }

    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();
    let ns = "voice";
    let key = "kv_roundtrip";
    let claim_key = "kv_set_if_absent";

    assert!(kv::set_if_absent(pool, ns, claim_key, "first")
        .await
        .expect("insert absent value"));
    assert!(!kv::set_if_absent(pool, ns, claim_key, "second")
        .await
        .expect("preserve existing value"));
    assert_eq!(
        kv::get(pool, ns, claim_key)
            .await
            .expect("get preserved value"),
        Some("first".to_string())
    );

    assert_eq!(kv::get(pool, ns, key).await.expect("get missing key"), None);

    kv::set(pool, ns, key, "1").await.expect("insert value");
    assert_eq!(
        kv::get(pool, ns, key).await.expect("get inserted value"),
        Some("1".to_string())
    );

    kv::set(pool, ns, key, "2").await.expect("overwrite value");
    assert_eq!(
        kv::get(pool, ns, key).await.expect("get overwritten value"),
        Some("2".to_string())
    );

    kv::delete(pool, ns, key).await.expect("delete value");
    assert_eq!(kv::get(pool, ns, key).await.expect("get deleted key"), None);
}
