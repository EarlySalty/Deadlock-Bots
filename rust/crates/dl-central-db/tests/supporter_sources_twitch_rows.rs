#![cfg(feature = "testing")]
use dl_central_db::test_pool;
use sqlx::{Postgres, Transaction};

const SUPPORTER_SOURCES: &str = include_str!("../migrations/20261111120000_supporter_sources.sql");

async fn exec(tx: &mut Transaction<'_, Postgres>, sql: &str) -> Result<u64, sqlx::Error> {
    sqlx::query(sql)
        .execute(&mut **tx)
        .await
        .map(|done| done.rows_affected())
}

async fn refused(tx: &mut Transaction<'_, Postgres>, sql: &str) -> String {
    exec(tx, "SAVEPOINT probe").await.expect("savepoint");
    let outcome = exec(tx, sql).await;
    exec(tx, "ROLLBACK TO SAVEPOINT probe")
        .await
        .expect("rollback to savepoint");
    match outcome {
        Ok(rows) => panic!("{sql} must be refused, changed {rows} rows"),
        Err(sqlx::Error::Database(error)) => error.code().map(String::from).unwrap_or_default(),
        Err(error) => panic!("{sql} failed outside the database: {error}"),
    }
}

async fn seed(tx: &mut Transaction<'_, Postgres>) {
    sqlx::raw_sql(
        "DO $$ BEGIN
             IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'twitchdash') THEN
                 CREATE ROLE twitchdash NOLOGIN;
             END IF;
         END $$;",
    )
    .execute(&mut **tx)
    .await
    .expect("dashboard role");
    sqlx::raw_sql(SUPPORTER_SOURCES)
        .execute(&mut **tx)
        .await
        .expect("supporter grants with the dashboard role present");
    exec(
        tx,
        "INSERT INTO core.supporter_sources (discord_id, source, source_ref, monthly_cents, valid_until)
         VALUES (1001, 'stripe', 'sub_paid', 500, now() + interval '30 days'),
                (1002, 'twitch_sub', 'channel_a', NULL, now() + interval '1 day'),
                (1003, 'boost', 'discord', NULL, NULL)",
    )
    .await
    .expect("rows written by the table owner");
}

#[tokio::test]
async fn dashboard_role_reaches_only_twitch_rows_without_amount() {
    let db = test_pool().await.expect("Wegwerf-Datenbank erforderlich");
    let mut tx = db.begin().await.expect("transaction");
    seed(&mut tx).await;
    exec(&mut tx, "SET LOCAL ROLE twitchdash")
        .await
        .expect("act as dashboard");

    let upsert = "INSERT INTO core.supporter_sources
             (discord_id, source, source_ref, monthly_cents, valid_until, updated_at)
         VALUES ($1, $2, $3, NULL, now() + interval '2 days', now())
         ON CONFLICT (discord_id, source, source_ref) DO UPDATE SET
             valid_until = EXCLUDED.valid_until,
             updated_at = now()";
    for (discord_id, source, source_ref) in [
        (1002_i64, "twitch_sub", "channel_a"),
        (1004, "twitch_sub", "channel_b"),
        (1005, "twitch_bot_customer", "channel_c"),
    ] {
        let rows = sqlx::query(upsert)
            .bind(discord_id)
            .bind(source)
            .bind(source_ref)
            .execute(&mut *tx)
            .await
            .expect("dashboard upserts its own rows")
            .rows_affected();
        assert_eq!(rows, 1, "{source} {source_ref}");
    }
    let refs: Vec<String> = sqlx::query_scalar(
        "SELECT source_ref FROM core.supporter_sources WHERE discord_id = $1 AND source = $2",
    )
    .bind(1002_i64)
    .bind("twitch_sub")
    .fetch_all(&mut *tx)
    .await
    .expect("dashboard reads its own refs");
    assert_eq!(refs, vec!["channel_a".to_owned()]);
    let visible: Vec<(i64, String)> =
        sqlx::query_as("SELECT discord_id, source FROM core.supporter_sources ORDER BY discord_id")
            .fetch_all(&mut *tx)
            .await
            .expect("dashboard lists rows");
    assert_eq!(
        visible,
        vec![
            (1002, "twitch_sub".to_owned()),
            (1004, "twitch_sub".to_owned()),
            (1005, "twitch_bot_customer".to_owned()),
        ]
    );
    let deleted = sqlx::query(
        "DELETE FROM core.supporter_sources WHERE discord_id = $1 AND source = $2 AND source_ref = $3",
    )
    .bind(1004_i64)
    .bind("twitch_sub")
    .bind("channel_b")
    .execute(&mut *tx)
    .await
    .expect("dashboard deletes its own row")
    .rows_affected();
    assert_eq!(deleted, 1);
    sqlx::query(
        "SELECT platform_user_id, discord_id FROM core.discord_platform_connections
          WHERE platform = 'twitch' AND verified AND platform_user_id = ANY($1)",
    )
    .bind(vec!["42".to_owned()])
    .fetch_all(&mut *tx)
    .await
    .expect("dashboard reads verified Twitch links");

    for sql in [
        "UPDATE core.supporter_sources SET valid_until = now() WHERE source = 'stripe'",
        "DELETE FROM core.supporter_sources WHERE source = 'stripe'",
        "DELETE FROM core.supporter_sources WHERE source = 'boost'",
    ] {
        assert_eq!(exec(&mut tx, sql).await.expect(sql), 0, "{sql}");
    }
    for (sql, code) in [
        (
            "INSERT INTO core.supporter_sources (discord_id, source, source_ref, monthly_cents)
             VALUES (1006, 'stripe', 'sub_forged', 500)",
            "42501",
        ),
        (
            "INSERT INTO core.supporter_sources (discord_id, source, source_ref)
             VALUES (1006, 'manual', 'role_cutover')",
            "42501",
        ),
        (
            "INSERT INTO core.supporter_sources (discord_id, source, source_ref, monthly_cents)
             VALUES (1006, 'twitch_sub', 'channel_d', 500)",
            "42501",
        ),
        (
            "UPDATE core.supporter_sources SET source = 'stripe' WHERE discord_id = 1002",
            "42501",
        ),
        (
            "UPDATE core.supporter_sources SET monthly_cents = 500 WHERE discord_id = 1002",
            "42501",
        ),
        ("SELECT monthly_cents FROM core.supporter_sources", "42501"),
        ("SELECT discord_id FROM core.active_supporters", "42501"),
    ] {
        assert_eq!(refused(&mut tx, sql).await, code, "{sql}");
    }

    exec(&mut tx, "RESET ROLE")
        .await
        .expect("back to the owner");
    let paid: (i32, bool) = sqlx::query_as(
        "SELECT monthly_cents, valid_until > now() + interval '29 days'
           FROM core.supporter_sources WHERE source = 'stripe' AND discord_id = 1001",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("paid row still present");
    assert_eq!(paid, (500, true));
}
