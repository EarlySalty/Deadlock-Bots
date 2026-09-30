use super::*;
include!("qualified_invite_live_tests.rs");

fn at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixture timestamp")
        .with_timezone(&Utc)
}

async fn database() -> dl_central_db::TestDb {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-database.json");
    let db = match std::fs::read(path) {
        Ok(bytes) => {
            let config: serde_json::Value =
                serde_json::from_slice(&bytes).expect("valid local test-database.json");
            let options = config["database_url"]
                .as_str()
                .expect("database_url string")
                .parse::<sqlx::postgres::PgConnectOptions>()
                .expect("valid local test database options");
            dl_central_db::testing::test_pool_with_options(options).await
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            dl_central_db::testing::test_pool().await
        }
        Err(_) => panic!("cannot read local test-database.json"),
    }
    .expect("isolated database");
    sqlx::query("UPDATE activity.twitch_invite_tracking SET started_at = '2025-01-01T00:00:00Z'")
        .execute(db.pool())
        .await
        .expect("tracking epoch");
    sqlx::query(
        "INSERT INTO bot.twitch_streamer_invites
         (streamer_login, guild_id, channel_id, twitch_user_id, invite_code, invite_url)
         VALUES ('streamer', 1, 2, '42', 'ChannelCode', 'https://discord.gg/ChannelCode')",
    )
    .execute(db.pool())
    .await
    .expect("channel mapping");
    sqlx::query(
        "INSERT INTO bot.twitch_personal_invites
         (streamer_twitch_user_id, inviter_twitch_user_id, streamer_login, guild_id, channel_id, invite_code, invite_url)
         VALUES ('42', '43', 'streamer', 1, 2, 'ViewerCode', 'https://discord.gg/ViewerCode')",
    ).execute(db.pool()).await.expect("viewer mapping");
    db
}

async fn join(pool: &PgPool, id: i64, user: i64, joined_at: DateTime<Utc>, code: &str) {
    let metadata = json!({"invite_code": code, "discord_joined_at": joined_at.to_rfc3339()});
    let mut tx = pool.begin().await.expect("join transaction");
    remember_member_event(
        &mut tx,
        id,
        1,
        user,
        "join",
        Some(joined_at),
        Some(&metadata.to_string()),
    )
    .await
    .expect("membership ledger");
    sqlx::query(
        "INSERT INTO activity.member_events (id, user_id, guild_id, event_type, occurred_at, metadata)
         VALUES ($1, $2, 1, 'join', $3, $4)",
    ).bind(id).bind(user).bind(joined_at).bind(metadata)
        .execute(&mut *tx).await.expect("source join");
    tx.commit().await.expect("join commit");
    reconcile_attribution(pool, 1).await.expect("attribution");
}

async fn candidate(pool: &PgPool, user: i64) -> PendingInvite {
    pending(pool, 1)
        .await
        .expect("pending candidates")
        .into_iter()
        .find(|row| row.user_id == user)
        .expect("candidate")
}

fn proof(invite: &PendingInvite, days: i64) -> MemberProof {
    MemberProof {
        joined_at: Some(invite.joined_at),
        is_bot: false,
        checked_at: invite.joined_at + Duration::days(days),
    }
}

async fn messages(pool: &PgPool, user: u64, first_id: u64, times: &[DateTime<Utc>]) {
    for (index, time) in times.iter().enumerate() {
        record_message(pool, first_id + index as u64, 1, user, time.timestamp())
            .await
            .expect("message evidence");
    }
}

async fn voice(
    pool: &PgPool,
    id: i64,
    user: i64,
    channel: i64,
    start: DateTime<Utc>,
    seconds: i64,
) {
    sqlx::query(
        "INSERT INTO activity.voice_session_log
         (id, user_id, guild_id, channel_id, started_at, ended_at, duration_seconds, points)
         VALUES ($1, $2, 1, $3, $4, $5, $6, 0)",
    )
    .bind(id)
    .bind(user)
    .bind(channel)
    .bind(start)
    .bind(start + Duration::seconds(seconds))
    .bind(seconds)
    .execute(pool)
    .await
    .expect("voice evidence");
}

#[tokio::test]
async fn messages_require_five_events_on_two_berlin_days_and_retention() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    join(pool, 1, 101, joined, "ViewerCode").await;
    let invite = candidate(pool, 101).await;
    let first = at("2026-01-02T23:30:00Z");
    let same_berlin_day = at("2026-01-03T00:30:00Z");
    messages(
        pool,
        101,
        1000,
        &[first, first, first, first, same_berlin_day],
    )
    .await;
    let retained = proof(&invite, 14);
    assert_eq!(
        evaluate_one(pool, &invite, Some(&retained), &[], retained.checked_at)
            .await
            .expect("evaluate"),
        None
    );
    record_message(pool, 1000, 1, 101, first.timestamp())
        .await
        .expect("replayed message");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activity.twitch_invite_messages")
        .fetch_one(pool)
        .await
        .expect("count");
    assert_eq!(count, 5);
    messages(pool, 101, 1005, &[at("2026-01-03T23:30:00Z")]).await;
    let early = proof(&invite, 13);
    assert_eq!(
        evaluate_one(pool, &invite, Some(&early), &[], early.checked_at)
            .await
            .expect("evaluate early"),
        None
    );
    assert_eq!(
        evaluate_one(pool, &invite, Some(&retained), &[], retained.checked_at)
            .await
            .expect("qualify"),
        Some("qualified")
    );
    assert_eq!(
        evaluate_one(pool, &invite, Some(&retained), &[], retained.checked_at)
            .await
            .expect("repeat"),
        None
    );
    let row: (String, Option<String>, DateTime<Utc>) = sqlx::query_as(
        "SELECT streamer_login, inviter_twitch_user_id, qualified_at FROM bot.twitch_invite_joins WHERE join_id = 1",
    ).fetch_one(pool).await.expect("qualified row");
    assert_eq!(
        row,
        (
            "streamer".into(),
            Some("43".into()),
            joined + Duration::days(14)
        )
    );
    let transitions: Vec<String> = sqlx::query_scalar(
        "SELECT status FROM bot.twitch_invite_transitions WHERE join_id = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("history");
    assert_eq!(transitions, ["pending", "qualified"]);
}

#[tokio::test]
async fn fewer_than_five_messages_and_pre_join_activity_never_count() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    messages(
        pool,
        102,
        2000,
        &[joined - Duration::days(1), joined - Duration::hours(1)],
    )
    .await;
    messages(pool, 102, 2010, &[joined + Duration::seconds(1)]).await;
    join(pool, 2, 102, joined, "ChannelCode").await;
    messages(
        pool,
        102,
        2020,
        &[
            joined + Duration::days(1),
            joined + Duration::days(1),
            joined + Duration::days(2),
        ],
    )
    .await;
    let invite = candidate(pool, 102).await;
    let member = proof(&invite, 15);
    assert_eq!(
        evaluate_one(pool, &invite, Some(&member), &[], member.checked_at)
            .await
            .expect("four messages"),
        None
    );
    messages(pool, 102, 2030, &[joined + Duration::days(2)]).await;
    assert_eq!(
        evaluate_one(pool, &invite, Some(&member), &[], member.checked_at)
            .await
            .expect("fifth"),
        Some("qualified")
    );
    let inviter: Option<String> = sqlx::query_scalar(
        "SELECT inviter_twitch_user_id FROM bot.twitch_invite_joins WHERE join_id = 2",
    )
    .fetch_one(pool)
    .await
    .expect("channel attribution");
    assert!(inviter.is_none());
}

#[tokio::test]
async fn voice_needs_one_continuous_session_outside_excluded_channels() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    join(pool, 3, 103, joined, "ViewerCode").await;
    let invite = candidate(pool, 103).await;
    let member = proof(&invite, 14);
    voice(pool, 1, 103, 20, joined + Duration::hours(1), 899).await;
    voice(pool, 2, 103, 20, joined + Duration::hours(2), 500).await;
    voice(pool, 3, 103, 99, joined + Duration::hours(3), 1800).await;
    voice(pool, 4, 103, 98, joined + Duration::hours(4), 1800).await;
    voice(pool, 5, 103, 20, joined - Duration::hours(1), 1800).await;
    assert_eq!(
        evaluate_one(pool, &invite, Some(&member), &[98, 99], member.checked_at)
            .await
            .expect("insufficient voice"),
        None
    );
    voice(pool, 6, 103, 20, joined + Duration::hours(5), 900).await;
    assert_eq!(
        evaluate_one(pool, &invite, Some(&member), &[98, 99], member.checked_at)
            .await
            .expect("voice threshold"),
        Some("qualified")
    );
}

#[tokio::test]
async fn deadline_uses_evidence_time_and_late_evaluation_does_not_lose_credit() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    for (id, user, offset) in [(4, 104, 0), (5, 105, 1)] {
        join(pool, id, user, joined, "ChannelCode").await;
        let invite = candidate(pool, user).await;
        voice(
            pool,
            id,
            user,
            20,
            joined + Duration::days(30) - Duration::minutes(15) + Duration::seconds(offset),
            1200,
        )
        .await;
        let member = proof(&invite, 31);
        let expected = if offset == 0 { "qualified" } else { "expired" };
        assert_eq!(
            evaluate_one(pool, &invite, Some(&member), &[], member.checked_at)
                .await
                .expect("deadline evaluation"),
            Some(expected)
        );
    }
}

#[tokio::test]
async fn prior_members_early_leavers_bots_and_rejoins_cannot_create_a_credit() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    let mut tx = pool.begin().await.expect("prior member");
    remember_prior_member(&mut tx, 1, 106)
        .await
        .expect("prior tombstone");
    tx.commit().await.expect("commit");
    for (id, user) in [(6, 106), (7, 107), (8, 108), (9, 109)] {
        join(pool, id, user, joined, "ChannelCode").await;
        voice(pool, id, user, 20, joined + Duration::hours(1), 900).await;
        let invite = candidate(pool, user).await;
        let mut member = proof(&invite, 31);
        match user {
            107 => {
                let mut tx = pool.begin().await.expect("leave transaction");
                remember_member_event(
                    &mut tx,
                    100,
                    1,
                    user,
                    "leave",
                    Some(joined + Duration::days(13)),
                    None,
                )
                .await
                .expect("leave");
                tx.commit().await.expect("leave commit");
            }
            108 => member.is_bot = true,
            109 => member.joined_at = Some(joined + Duration::days(2)),
            _ => {}
        }
        assert_eq!(
            evaluate_one(pool, &invite, Some(&member), &[], member.checked_at)
                .await
                .expect("ineligible"),
            Some("expired")
        );
    }
    join(pool, 10, 107, joined + Duration::days(32), "ViewerCode").await;
    let rejoin = candidate(pool, 107).await;
    assert!(!rejoin.eligible);
}

#[tokio::test]
async fn replay_and_concurrent_evaluation_are_idempotent_and_history_is_immutable() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    join(pool, 11, 110, joined, "ViewerCode").await;
    join(pool, 12, 110, joined, "ViewerCode").await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bot.twitch_invite_joins WHERE user_id = 110")
            .fetch_one(pool)
            .await
            .expect("natural join count");
    assert_eq!(count, 1);
    let invite = candidate(pool, 110).await;
    voice(pool, 11, 110, 20, joined + Duration::hours(1), 900).await;
    let member = proof(&invite, 14);
    let (a, b) = tokio::join!(
        evaluate_one(pool, &invite, Some(&member), &[], member.checked_at),
        evaluate_one(pool, &invite, Some(&member), &[], member.checked_at),
    );
    let results = [a.expect("concurrent a"), b.expect("concurrent b")];
    assert_eq!(
        results
            .iter()
            .filter(|value| **value == Some("qualified"))
            .count(),
        1
    );
    for sql in [
        "DELETE FROM bot.twitch_invite_joins WHERE user_id = 110",
        "UPDATE bot.twitch_invite_joins SET status = 'expired', qualified_at = NULL WHERE user_id = 110",
        "DELETE FROM bot.twitch_invite_transitions",
        "UPDATE bot.twitch_invite_transitions SET reason = 'changed'",
        "DELETE FROM activity.twitch_invite_members WHERE user_id = 110",
    ] {
        assert!(sqlx::query(sql).execute(pool).await.is_err(), "{sql}");
    }
}

#[tokio::test]
async fn incremental_pages_find_late_transitions_without_discord_identity() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    join(pool, 13, 111, joined, "ViewerCode").await;
    join(pool, 14, 112, joined, "ChannelCode").await;
    let first = list_page(pool, 1, joined, None, None, 1)
        .await
        .expect("first page");
    assert_eq!(first.invites.len(), 1);
    assert!(first.next_since.is_none());
    let last = list_page(
        pool,
        1,
        joined,
        Some(first.until),
        first.next_cursor.as_ref(),
        1,
    )
    .await
    .expect("last page");
    assert_eq!(last.invites.len(), 1);
    assert_ne!(first.invites[0].join_id, last.invites[0].join_id);
    assert!(last.next_cursor.is_none());
    let invite = candidate(pool, 111).await;
    voice(pool, 13, 111, 20, joined + Duration::days(1), 900).await;
    let member = proof(&invite, 14);
    evaluate_one(pool, &invite, Some(&member), &[], member.checked_at)
        .await
        .expect("late transition");
    let changes = list_page(
        pool,
        1,
        last.next_since.expect("watermark"),
        None,
        None,
        100,
    )
    .await
    .expect("changed rows");
    assert!(changes
        .invites
        .iter()
        .any(|row| row.join_id == "13" && row.status == "qualified"));
    let value = serde_json::to_value(&changes.invites[0]).expect("DTO");
    let keys: std::collections::BTreeSet<_> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "join_id",
            "streamer_login",
            "inviter_twitch_user_id",
            "joined_at",
            "status",
            "qualified_at",
            "updated_at",
        ])
    );
}

#[tokio::test]
async fn ambiguous_codes_and_case_mismatches_never_attribute() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    join(pool, 15, 113, joined, "viewercode").await;
    assert!(pending(pool, 1).await.expect("case mismatch").is_empty());
    sqlx::query("UPDATE bot.twitch_streamer_invites SET invite_code = 'ViewerCode'")
        .execute(pool)
        .await
        .expect("ambiguous owners");
    join(pool, 16, 114, joined, "ViewerCode").await;
    assert!(pending(pool, 1)
        .await
        .expect("ambiguous mapping")
        .is_empty());
}
