#![cfg(feature = "testing")]
//! Community-Punkte (Paket C) gegen die zentrale Test-DB: Upsert und Cursor,
//! Ledger-Importe, gemeinsames und Partner-Leaderboard je Zeitraum,
//! Datenschutz (nur verknuepfte Mitglieder, Privacy-Grabstein).

use chrono::{DateTime, NaiveDate, Utc};
use dl_central_db::community_points::{
    apply_streamer_page, apply_viewer_page, community_board, import_clip_contest_ledger,
    import_qualified_join_ledger, load_cursor, member_rank, record_ledger_event, streamer_board,
    LedgerEvent, LedgerRecipient, MemberPoints, Period, StreamerDailyRow, ViewerDailyRow,
    CURSOR_STREAMERS, CURSOR_VIEWERS, SOURCE_STREAMER_SUGGESTION, STREAMER_SUGGESTION_POINTS,
};
use dl_central_db::testing::test_pool;
use sqlx::PgPool;

fn dsn_available() -> bool {
    std::env::var("CENTRAL_TEST_DSN").is_ok() || std::env::var("DATABASE_URL").is_ok()
}

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("day")
}

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("timestamp")
        .with_timezone(&Utc)
}

fn viewer(
    twitch: &str,
    channel: &str,
    d: &str,
    minutes: i32,
    pw: i32,
    pc: i32,
    pd: i32,
) -> ViewerDailyRow {
    ViewerDailyRow {
        twitch_user_id: twitch.into(),
        channel_twitch_user_id: channel.into(),
        day: day(d),
        watch_minutes: minutes,
        chat_messages: pc,
        points_watch: pw,
        points_chat: pc,
        points_discovery: pd,
        source_updated_at: ts("2026-10-15T10:00:00Z"),
    }
}

fn streamer(id: &str, login: &str, discord: Option<i64>, d: &str, raids: i32) -> StreamerDailyRow {
    StreamerDailyRow {
        streamer_twitch_user_id: id.into(),
        day: day(d),
        streamer_login: login.into(),
        discord_user_id: discord,
        viewer_minutes: 1000,
        unique_viewers: 10,
        raids_to_partners: raids,
        source_updated_at: ts("2026-10-15T10:00:00Z"),
    }
}

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
}

#[tokio::test]
async fn upsert_ist_idempotent_und_cursor_transaktional() {
    if !dsn_available() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = test_pool().await.expect("test pool");
    let pool = db.pool();

    assert_eq!(
        load_cursor(pool, CURSOR_VIEWERS).await.expect("cursor"),
        None
    );
    let row = viewer("111", "456", "2026-10-14", 95, 19, 12, 10);
    let cursor = "2026-10-15T10:00:00.000001Z";
    assert_eq!(
        apply_viewer_page(pool, std::slice::from_ref(&row), Some(cursor))
            .await
            .expect("apply"),
        1
    );
    // Gleiche Seite nochmal: eine Zeile, Cursor exakt gespeichert.
    apply_viewer_page(pool, std::slice::from_ref(&row), Some(cursor))
        .await
        .expect("apply again");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM community_points.twitch_viewer_daily")
            .fetch_one(pool)
            .await
            .expect("count");
    assert_eq!(count, 1);
    assert_eq!(
        load_cursor(pool, CURSOR_VIEWERS)
            .await
            .expect("cursor")
            .as_deref(),
        Some(cursor)
    );

    // Juengerer Quellstand ueberschreibt, aelterer nicht.
    let mut newer = row.clone();
    newer.points_watch = 20;
    newer.source_updated_at = ts("2026-10-15T11:00:00Z");
    apply_viewer_page(pool, &[newer], None)
        .await
        .expect("newer");
    let mut older = row.clone();
    older.points_watch = 1;
    older.source_updated_at = ts("2026-10-15T09:00:00Z");
    apply_viewer_page(pool, &[older], None)
        .await
        .expect("older");
    let pw: i32 = sqlx::query_scalar(
        "SELECT points_watch FROM community_points.twitch_viewer_daily WHERE twitch_user_id = '111'",
    )
    .fetch_one(pool)
    .await
    .expect("pw");
    assert_eq!(pw, 20);
    // `None` loescht den gespeicherten Cursor nicht.
    assert_eq!(
        load_cursor(pool, CURSOR_VIEWERS)
            .await
            .expect("cursor")
            .as_deref(),
        Some(cursor)
    );

    // Eine ungueltige Zeile rollt die ganze Seite samt Cursor zurueck.
    let good = streamer("456", "partner_a", Some(9001), "2026-10-14", 1);
    let mut bad = streamer("457", "partner_b", None, "2026-10-14", 0);
    bad.raids_to_partners = -1;
    assert!(apply_streamer_page(pool, &[good.clone(), bad], Some("x"))
        .await
        .is_err());
    assert_eq!(
        load_cursor(pool, CURSOR_STREAMERS).await.expect("cursor"),
        None
    );
    let streamers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM community_points.twitch_streamer_daily")
            .fetch_one(pool)
            .await
            .expect("count");
    assert_eq!(streamers, 0);
    apply_streamer_page(pool, &[good], Some("y"))
        .await
        .expect("good");
    assert_eq!(
        load_cursor(pool, CURSOR_STREAMERS)
            .await
            .expect("cursor")
            .as_deref(),
        Some("y")
    );
}

async fn seed(pool: &PgPool) {
    // Verknuepfungen: 1001<->111, 1002<->222 (widersprochen), 1004<->444.
    // 333 ist nicht verknuepft.
    exec(
        pool,
        "INSERT INTO core.discord_platform_connections (discord_id, platform, platform_user_id, platform_login)
         VALUES (1001, 'twitch', '111', 'a'), (1002, 'twitch', '222', 'b'), (1004, 'twitch', '444', 'd');
         INSERT INTO core.user_privacy (user_id, opted_out) VALUES (1002, TRUE);
         INSERT INTO voice.voice_stats (user_id, total_seconds, total_points)
         VALUES (1001, 30000, 500), (1003, 20000, 300), (1002, 99999, 1000);
         INSERT INTO activity.voice_session_log (id, user_id, started_at, ended_at, duration_seconds, points)
         VALUES (1, 1001, '2026-10-14T18:00:00Z', '2026-10-14T18:20:00Z', 1200, 20),
                (2, 1003, '2026-09-30T23:00:00Z', '2026-09-30T23:30:00Z', 1800, 7),
                (3, 1003, '2026-09-30T20:00:00Z', '2026-09-30T21:00:00Z', 3600, 100),
                (4, 1002, '2026-10-14T18:00:00Z', '2026-10-14T19:00:00Z', 3600, 60);",
    )
    .await;
    apply_viewer_page(
        pool,
        &[
            viewer("111", "456", "2026-10-14", 95, 19, 12, 10),
            viewer("111", "456", "2026-09-20", 40, 8, 0, 0),
            viewer("111", "789", "2026-10-14", 31, 6, 0, 10),
            viewer("222", "456", "2026-10-14", 60, 12, 0, 0),
            viewer("333", "456", "2026-10-14", 600, 72, 30, 10),
            viewer("444", "456", "2026-10-14", 25, 4, 0, 0),
        ],
        Some("c1"),
    )
    .await
    .expect("viewers");
    apply_streamer_page(
        pool,
        &[
            streamer("456", "partner_a", Some(9001), "2026-10-14", 2),
            streamer("789", "partner_b", None, "2026-10-14", 0),
            streamer("999", "partner_c", Some(1002), "2026-10-14", 5),
        ],
        Some("c2"),
    )
    .await
    .expect("streamers");
    // Qualifizierter Beitritt ueber eine Einladung von partner_a.
    exec(
        pool,
        "INSERT INTO bot.twitch_invite_joins
             (join_id, guild_id, user_id, streamer_login, streamer_twitch_user_id,
              invite_code, joined_at, eligible)
         VALUES (77, 1, 5001, 'partner_a', '456', 'abc', '2026-09-20T12:00:00Z', TRUE),
                (78, 1, 5002, 'partner_a', '456', 'abc', '2026-10-01T12:00:00Z', TRUE);
         UPDATE bot.twitch_invite_joins
            SET status = 'qualified', qualified_at = '2026-10-05T12:00:00Z'
          WHERE join_id = 77;",
    )
    .await;
}

/// Clip-Contest-Daten im echten Schema aus Paket D (Migration 2026100112):
/// Fenster 1 abgeschlossen, Fenster 2 noch offen.
async fn seed_clip_contest(pool: &PgPool) {
    exec(
        pool,
        "INSERT INTO clips.clip_windows (id, guild_id, start_at, end_at, status)
         VALUES (1, 1, '2026-10-04T22:00:00Z', '2026-10-10T21:00:00Z', 'done'),
                (2, 1, '2026-10-11T22:00:00Z', '2026-10-17T21:00:00Z', 'running');
         INSERT INTO clips.clip_submissions (id, guild_id, user_id, link, credit, permission)
         VALUES (10, 1, 1001, 'https://clips.twitch.tv/a', 'a', 'ja'),
                (12, 1, 1002, 'https://clips.twitch.tv/c', 'c', 'ja'),
                (20, 1, 1001, 'https://clips.twitch.tv/d', 'd', 'ja');
         INSERT INTO clips.clip_submissions
             (id, guild_id, user_id, link, credit, permission, source,
              streamer_twitch_user_id, streamer_login, idempotency_key)
         VALUES (11, 1, NULL, 'https://clips.twitch.tv/b', 'partner_a', 'ja', 'twitch',
                 '456', 'partner_a', 'twitch-clip-b');
         INSERT INTO clips.clip_votings
             (window_id, guild_id, channel_id, status, message_id,
              voting_start_at, voting_end_at, closed_at)
         VALUES (1, 1, 5, 'closed', 900, '2026-10-10T21:00:00Z',
                 '2026-10-11T18:00:00Z', '2026-10-11T18:01:00Z'),
                (2, 1, 5, 'open', 901, '2026-10-17T21:00:00Z',
                 '2026-10-19T21:00:00Z', NULL);
         INSERT INTO clips.clip_voting_entries (window_id, position, submission_id)
         VALUES (1, 1, 10), (1, 2, 11), (1, 3, 12), (2, 1, 20);
         INSERT INTO clips.clip_votes (window_id, voter_user_id, submission_id)
         VALUES (1, 1003, 10), (1, 1001, 11), (1, 1002, 10), (2, 1003, 20);
         INSERT INTO clips.clip_contest_results
             (window_id, place, guild_id, week_start_at, week_end_at, submission_id,
              source, user_id, streamer_twitch_user_id, streamer_login, votes, decided_at)
         VALUES (1, 1, 1, '2026-10-04T22:00:00Z', '2026-10-10T21:00:00Z', 10,
                 'discord', 1001, NULL, NULL, 2, '2026-10-10T18:00:00Z'),
                (1, 2, 1, '2026-10-04T22:00:00Z', '2026-10-10T21:00:00Z', 11,
                 'twitch', NULL, '456', 'partner_a', 1, '2026-10-10T18:00:00Z'),
                (1, 3, 1, '2026-10-04T22:00:00Z', '2026-10-10T21:00:00Z', 12,
                 'discord', 1002, NULL, NULL, 0, '2026-10-10T18:00:00Z');",
    )
    .await;
}

fn find(board: &[MemberPoints], id: i64) -> Option<&MemberPoints> {
    board.iter().find(|row| row.discord_id == id)
}

#[tokio::test]
async fn leaderboards_je_zeitraum_mit_ledger_und_datenschutz() {
    if !dsn_available() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = test_pool().await.expect("test pool");
    let pool = db.pool();
    seed(pool).await;

    // Ohne abgeschlossenes Voting gibt es nichts zu verbuchen.
    let clips = import_clip_contest_ledger(pool).await.expect("clips");
    assert!(!clips.tables_missing);
    assert_eq!((clips.places, clips.votes), (0, 0));
    seed_clip_contest(pool).await;
    let clips = import_clip_contest_ledger(pool).await.expect("clips");
    assert!(!clips.tables_missing);
    // Platz 3 gehoert einem Mitglied mit Widerspruch, Stimme von 1002 auch.
    assert_eq!((clips.places, clips.votes), (2, 2));
    let again = import_clip_contest_ledger(pool).await.expect("clips again");
    assert_eq!((again.places, again.votes), (0, 0));

    assert_eq!(import_qualified_join_ledger(pool).await.expect("joins"), 1);
    assert_eq!(import_qualified_join_ledger(pool).await.expect("joins"), 0);

    let suggestion = LedgerEvent {
        recipient: LedgerRecipient::Member(1003),
        source: SOURCE_STREAMER_SUGGESTION,
        reference: "streamer_suggestion:1".into(),
        points: STREAMER_SUGGESTION_POINTS,
        occurred_at: ts("2026-10-02T12:00:00Z"),
    };
    assert!(record_ledger_event(pool, &suggestion)
        .await
        .expect("ledger"));
    assert!(!record_ledger_event(pool, &suggestion)
        .await
        .expect("ledger again"));
    let opted_out = LedgerEvent {
        recipient: LedgerRecipient::Member(1002),
        reference: "streamer_suggestion:2".into(),
        ..suggestion.clone()
    };
    assert!(!record_ledger_event(pool, &opted_out)
        .await
        .expect("opted out"));
    assert!(record_ledger_event(
        pool,
        &LedgerEvent {
            recipient: LedgerRecipient::Member(0),
            ..suggestion.clone()
        }
    )
    .await
    .is_err());

    let today = day("2026-10-15");

    // Season Oktober.
    let season = community_board(pool, Period::Season, today)
        .await
        .expect("season");
    assert!(find(&season, 1002).is_none(), "Widerspruch erscheint nicht");
    let a = find(&season, 1001).expect("1001");
    assert_eq!(
        (
            a.voice,
            a.twitch_watch,
            a.twitch_chat,
            a.twitch_discovery,
            a.clips,
            a.suggestions
        ),
        (20, 25, 12, 20, 102, 0)
    );
    assert_eq!(a.total(), 179);
    let c = find(&season, 1003).expect("1003");
    // Sitzung endete 01:30 Berliner Zeit am 1. Oktober: zaehlt zum Oktober.
    assert_eq!(
        (c.voice, c.twitch(), c.clips, c.suggestions),
        (7, 0, 2, 150)
    );
    let d = find(&season, 1004).expect("1004");
    assert_eq!(d.total(), 4);
    let ids: Vec<i64> = season.iter().map(|r| r.discord_id).collect();
    assert_eq!(ids, vec![1001, 1003, 1004]);
    assert_eq!(member_rank(&season, 1003), Some((2, 159)));
    // Nicht verknuepfte Twitch-Zuschauer tauchen nirgends auf.
    assert!(season.iter().all(|row| row.discord_id != 333));

    // Gesamt: Voice aus voice_stats.
    let total = community_board(pool, Period::Gesamt, today)
        .await
        .expect("gesamt");
    let a = find(&total, 1001).expect("1001");
    assert_eq!((a.voice, a.twitch_watch, a.total()), (500, 33, 667));
    assert_eq!(find(&total, 1003).expect("1003").total(), 452);
    assert!(find(&total, 1002).is_none());

    // Woche ab Montag 12.10.
    let week = community_board(pool, Period::Woche, today)
        .await
        .expect("woche");
    let a = find(&week, 1001).expect("1001");
    assert_eq!((a.voice, a.twitch(), a.clips), (20, 57, 0));
    assert!(find(&week, 1003).is_none(), "keine Punkte in dieser Woche");

    // Partner-Leaderboard.
    let streamers = streamer_board(pool, Period::Season, today)
        .await
        .expect("streamer");
    let logins: Vec<&str> = streamers
        .iter()
        .map(|s| s.streamer_login.as_str())
        .collect();
    assert_eq!(
        logins,
        vec!["partner_a", "partner_b"],
        "partner_c hat widersprochen"
    );
    let a = &streamers[0];
    // 95 + 25 Minuten am selben Tag = 120 -> 4 Punkte (222 widersprochen,
    // 333 nicht verknuepft).
    assert_eq!((a.community_minutes, a.watch_points), (120, 4));
    assert_eq!((a.raids, a.raid_points()), (2, 50));
    assert_eq!(
        (a.qualified_joins, a.join_points, a.clip_points),
        (1, 50, 60)
    );
    assert_eq!(a.total(), 164);
    assert_eq!(streamers[1].total(), 1);

    let all_time = streamer_board(pool, Period::Gesamt, today)
        .await
        .expect("gesamt");
    assert_eq!(
        (all_time[0].community_minutes, all_time[0].watch_points),
        (160, 5)
    );
}
