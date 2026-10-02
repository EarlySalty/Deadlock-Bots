#![cfg(feature = "testing")]

use dl_central_db::{
    discord_ids_for_twitch_ids, list_twitch_links, testing::test_pool, twitch_link_for_discord,
    upsert_twitch_connection, CentralDbError, TwitchConnection, TwitchUpsertOutcome,
};

fn dsn_available() -> bool {
    std::env::var("CENTRAL_TEST_DSN").is_ok() || std::env::var("DATABASE_URL").is_ok()
}

fn conn(id: &str, login: &str, verified: bool) -> TwitchConnection {
    TwitchConnection {
        twitch_user_id: id.to_string(),
        twitch_login: login.to_string(),
        verified,
    }
}

#[tokio::test]
async fn twitch_verknuepfung_upsert_ist_idempotent_und_lesbar() {
    if !dsn_available() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    let first = upsert_twitch_connection(pool, 1001, &conn("555", "Erster", false))
        .await
        .expect("insert");
    assert_eq!(
        first,
        TwitchUpsertOutcome::Linked {
            replaced_discord_ids: vec![]
        }
    );
    let before = twitch_link_for_discord(pool, 1001)
        .await
        .expect("read")
        .expect("row");
    assert_eq!(before.twitch_user_id, "555");
    assert_eq!(before.twitch_login, "Erster");
    assert!(!before.verified);

    // Gleicher Aufruf erneut: eine Zeile, Felder aktualisiert.
    upsert_twitch_connection(pool, 1001, &conn("555", "erster_neu", true))
        .await
        .expect("update");
    let after = twitch_link_for_discord(pool, 1001)
        .await
        .expect("read")
        .expect("row");
    assert_eq!(after.twitch_login, "erster_neu");
    assert!(after.verified);
    assert!(after.updated_at >= before.updated_at);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core.discord_platform_connections WHERE discord_id = 1001",
    )
    .fetch_one(pool)
    .await
    .expect("count");
    assert_eq!(count, 1);

    // Mitglied wechselt das Twitch-Konto: weiterhin genau eine Zeile.
    upsert_twitch_connection(pool, 1001, &conn("556", "anderes", true))
        .await
        .expect("switch");
    let switched = twitch_link_for_discord(pool, 1001)
        .await
        .expect("read")
        .expect("row");
    assert_eq!(switched.twitch_user_id, "556");

    // Steam-Verknuepfungen bleiben unberuehrt.
    let steam_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM core.steam_links")
        .fetch_one(pool)
        .await
        .expect("steam count");
    assert_eq!(steam_rows, 0);
}

#[tokio::test]
async fn twitch_konto_gehoert_hoechstens_einem_mitglied() {
    if !dsn_available() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    upsert_twitch_connection(pool, 2001, &conn("777", "geteilt", true))
        .await
        .expect("first owner");
    let outcome = upsert_twitch_connection(pool, 2002, &conn("777", "geteilt", true))
        .await
        .expect("second owner");
    assert_eq!(
        outcome,
        TwitchUpsertOutcome::Linked {
            replaced_discord_ids: vec![2001]
        }
    );
    assert!(twitch_link_for_discord(pool, 2001)
        .await
        .expect("read")
        .is_none());
    let map = discord_ids_for_twitch_ids(
        pool,
        &["777".to_string(), "999".to_string(), "kaputt".to_string()],
    )
    .await
    .expect("lookup");
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("777"), Some(&2002));
}

#[tokio::test]
async fn privacy_grabstein_verhindert_schreiben_und_lesen() {
    if !dsn_available() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    upsert_twitch_connection(pool, 3001, &conn("3001", "sichtbar", true))
        .await
        .expect("visible");
    upsert_twitch_connection(pool, 3002, &conn("3002", "spaeter_weg", true))
        .await
        .expect("later opted out");
    sqlx::query(
        "INSERT INTO core.user_privacy(user_id, opted_out) VALUES (3002, TRUE), (3003, TRUE)",
    )
    .execute(pool)
    .await
    .expect("tombstones");

    let outcome = upsert_twitch_connection(pool, 3003, &conn("3003", "nie", true))
        .await
        .expect("opted out upsert");
    assert_eq!(outcome, TwitchUpsertOutcome::PrivacyOptedOut);
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core.discord_platform_connections WHERE discord_id = 3003",
    )
    .fetch_one(pool)
    .await
    .expect("count");
    assert_eq!(stored, 0);

    let links = list_twitch_links(pool).await.expect("list");
    assert_eq!(
        links.iter().map(|l| l.discord_id).collect::<Vec<_>>(),
        vec![3001]
    );
    assert!(twitch_link_for_discord(pool, 3002)
        .await
        .expect("read")
        .is_none());
    assert!(discord_ids_for_twitch_ids(pool, &["3002".to_string()])
        .await
        .expect("lookup")
        .is_empty());
}

#[tokio::test]
async fn parallele_kontozuordnung_bleibt_eindeutig_und_beide_callbacks_gelingen() {
    if !dsn_available() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = test_pool().await.expect("Testdatenbank");
    for round in 0..8 {
        let connection = conn(&format!("8{round}77"), "geteilt", true);
        upsert_twitch_connection(db.pool(), 1, &connection)
            .await
            .expect("Vorheriger Besitzer");
        let (first, second) = tokio::join!(
            upsert_twitch_connection(db.pool(), 2, &connection),
            upsert_twitch_connection(db.pool(), 3, &connection),
        );
        first.expect("Erster paralleler Callback");
        second.expect("Zweiter paralleler Callback");
        let ids: Vec<i64> = sqlx::query_scalar("SELECT discord_id FROM core.discord_platform_connections WHERE platform = 'twitch' AND platform_user_id = $1")
            .bind(&connection.twitch_user_id).fetch_all(db.pool()).await.expect("Verknüpfungen");
        assert_eq!(ids.len(), 1);
        assert!([2, 3].contains(&ids[0]));
    }
}

#[tokio::test]
async fn ungueltige_eingaben_werden_abgelehnt() {
    if !dsn_available() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    for bad in [
        conn("0123", "x", true),
        conn("abc", "x", true),
        conn("5", " ", true),
    ] {
        assert!(matches!(
            upsert_twitch_connection(pool, 4001, &bad).await,
            Err(CentralDbError::InvalidInput(_))
        ));
    }
    let too_many = vec!["1".to_string(); 5_001];
    assert!(matches!(
        discord_ids_for_twitch_ids(pool, &too_many).await,
        Err(CentralDbError::InvalidInput(_))
    ));
    // Die Datenbank selbst lehnt nicht-numerische Twitch-IDs ab.
    let raw = sqlx::query(
        "INSERT INTO core.discord_platform_connections
             (discord_id, platform, platform_user_id, platform_login)
         VALUES (4002, 'twitch', 'abc', 'x')",
    )
    .execute(pool)
    .await;
    assert!(raw.is_err());
}
