#![cfg(feature = "testing")]
use dl_central_db::{bearer, test_pool};

#[tokio::test]
async fn legacy_bearers_keep_their_external_links_without_retaining_raw_values() {
    let db = test_pool().await.expect("Wegwerf-Datenbank erforderlich");
    let mut tx = db.begin().await.unwrap();
    // Ausschließlich im frischen TestDb: altes Schema für den echten SQL-Upgrade.
    sqlx::raw_sql("DROP TABLE turnier.sessions CASCADE; DROP TABLE turnier.draft_sessions CASCADE; DROP TABLE steam.steam_launch_tokens CASCADE;
        CREATE TABLE turnier.sessions(token TEXT PRIMARY KEY, marker INTEGER NOT NULL);
        CREATE TABLE turnier.draft_sessions(id BIGINT PRIMARY KEY,team1_token TEXT,team2_token TEXT);
        CREATE TABLE steam.steam_launch_tokens(token TEXT PRIMARY KEY,marker INTEGER NOT NULL);
        INSERT INTO turnier.sessions VALUES('synthetic-session',7);
        INSERT INTO turnier.draft_sessions VALUES(17,'synthetic-team-a','synthetic-team-b');
        INSERT INTO steam.steam_launch_tokens VALUES('synthetic-launch',23);")
        .execute(&mut *tx).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/20260930220000_bearer_lookup_keys.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let session: (String, i32) = sqlx::query_as("SELECT token,marker FROM turnier.sessions")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(session.0, bearer::lookup("synthetic-session"));
    assert_eq!(session.1, 7);
    assert!(bearer::matches("synthetic-session", &session.0));
    assert!(!bearer::matches(&session.0, &session.0));
    let teams: (i64, String, String) =
        sqlx::query_as("SELECT id,team1_token,team2_token FROM turnier.draft_sessions")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(teams.0, 17);
    assert!(bearer::matches("synthetic-team-a", &teams.1));
    assert!(bearer::matches("synthetic-team-b", &teams.2));
    assert!(!bearer::matches("synthetic-team-a", &teams.2));
    let launch: (String, i32) =
        sqlx::query_as("SELECT token,marker FROM steam.steam_launch_tokens")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(launch.0, bearer::lookup("synthetic-launch"));
    assert_eq!(launch.1, 23);
    sqlx::query("SAVEPOINT invalid_raw_write")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query("INSERT INTO turnier.sessions VALUES('raw-must-fail',99)")
            .execute(&mut *tx)
            .await
            .is_err()
    );
    sqlx::query("ROLLBACK TO invalid_raw_write")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
}
