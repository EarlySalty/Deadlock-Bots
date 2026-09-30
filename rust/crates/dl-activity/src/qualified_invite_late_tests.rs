async fn evidence_queue_count(pool: &PgPool, user: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM activity.twitch_invite_evidence_queue WHERE guild_id=1 AND user_id=$1")
        .bind(user).fetch_one(pool).await.expect("queued evidence")
}

#[tokio::test]
async fn late_evidence_reconsiders_deadline_once_for_all_three_sources() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    let deadline = joined + Duration::days(30);
    let ready = deadline - Duration::seconds(10);
    let now = deadline + Duration::minutes(1);
    for source in 0..3_i64 {
        let user = 991_001 + source;
        join(pool, user, user, joined, "ViewerCode").await;
        if source == 0 {
            messages(
                pool,
                user as u64,
                9_910_000,
                &[joined + Duration::days(1); 4],
            )
            .await;
        }
        if source == 2 {
            let members = HashMap::from([(user as u64, 20)]);
            for minute in 0..=14 {
                crate::qualified_invite_voice::record_snapshot(
                    pool,
                    1,
                    &members,
                    &[],
                    ready - Duration::minutes(15) + Duration::minutes(minute),
                    Some(user as u64),
                    false,
                )
                .await
                .expect("initial voice snapshot before REST and expiry");
            }
        }
        let invite = candidate(pool, user).await;
        let member = MemberProof {
            joined_at: Some(joined),
            is_bot: false,
            checked_at: now,
        };
        assert_eq!(
            evaluate_one(pool, &invite, Some(&member), &[], now)
                .await
                .expect("initial deadline"),
            Some("expired")
        );
        assert!(
            !pending(pool, 1)
                .await
                .expect("no expiry sweep")
                .iter()
                .any(|row| row.user_id == user)
        );
        assert_eq!(evidence_queue_count(pool, user).await, 0);
        match source {
            0 => messages(pool, user as u64, 9_910_004, &[ready]).await,
            1 => {
                voice(
                    pool,
                    9_910_001,
                    user,
                    20,
                    ready - Duration::minutes(15),
                    900,
                )
                .await
            }
            _ => crate::qualified_invite_voice::record_snapshot(
                pool,
                1,
                &HashMap::from([(user as u64, 20)]),
                &[],
                ready,
                Some(user as u64),
                false,
            )
            .await
            .expect("voice matures after initial snapshot and deadline decision"),
        }
        assert_eq!(evidence_queue_count(pool, user).await, 1);
        let retried = candidate(pool, user).await;
        assert_eq!(
            evaluate_one(pool, &retried, Some(&member), &[], now)
                .await
                .expect("late timely evidence"),
            Some("qualified")
        );
        assert_eq!(
            evaluate_one(pool, &retried, Some(&member), &[], now)
                .await
                .expect("idempotent repeat"),
            None
        );
        assert_eq!(evidence_queue_count(pool, user).await, 0);
        let saved: DateTime<Utc> =
            sqlx::query_scalar("SELECT qualified_at FROM bot.twitch_invite_joins WHERE join_id=$1")
                .bind(user)
                .fetch_one(pool)
                .await
                .expect("original evidence timestamp");
        assert_eq!(saved, ready);
        let history: Vec<String> = sqlx::query_scalar(
            "SELECT status FROM bot.twitch_invite_transitions WHERE join_id=$1 ORDER BY id",
        )
        .bind(user)
        .fetch_all(pool)
        .await
        .expect("append-only unique transitions");
        assert_eq!(history, vec!["pending", "expired", "qualified"]);
        let page = list_page(pool, 1, at("2020-01-01T00:00:00Z"), None, None, 100)
            .await
            .expect("updated feed");
        assert!(
            page.invites
                .iter()
                .any(|row| row.join_id == user.to_string()
                    && row.status == "qualified"
                    && row.qualified_at == Some(ready))
        );
    }
}

#[tokio::test]
async fn timely_voice_marker_survives_a_stale_or_missing_rest_membership_proof() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    let ready = joined + Duration::days(30);
    let now = ready + Duration::minutes(1);
    join(pool, 991_100, 991_100, joined, "ViewerCode").await;
    let invite = candidate(pool, 991_100).await;
    let stale = MemberProof {
        joined_at: Some(joined),
        is_bot: false,
        checked_at: ready - Duration::seconds(1),
    };
    assert_eq!(
        evaluate_one(pool, &invite, Some(&stale), &[], now)
            .await
            .expect("deadline before persistence"),
        Some("expired")
    );
    // Real live-clock writer, observed continuously through the exact deadline.
    for minute in 0..=15 {
        crate::qualified_invite_voice::record_snapshot(
            pool,
            1,
            &HashMap::from([(991_100, 20)]),
            &[],
            ready - Duration::minutes(15) + Duration::minutes(minute),
            Some(991_100),
            false,
        )
        .await
        .expect("delayed but continuously observed live clock");
    }
    for proof in [Some(&stale), None] {
        assert_eq!(
            evaluate_one(pool, &invite, proof, &[], now)
                .await
                .expect("wait for current membership proof"),
            None
        );
        assert_eq!(evidence_queue_count(pool, 991_100).await, 1);
        assert_eq!(candidate(pool, 991_100).await.join_id, invite.join_id);
    }
    let current = MemberProof {
        checked_at: now,
        ..stale
    };
    assert_eq!(
        evaluate_one(pool, &invite, Some(&current), &[], now)
            .await
            .expect("fresh REST proof"),
        Some("qualified")
    );
    assert_eq!(evidence_queue_count(pool, 991_100).await, 0);
}

#[tokio::test]
async fn late_evidence_never_reopens_ineligible_private_prior_or_out_of_window_joins() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    let deadline = joined + Duration::days(30);
    let now = deadline + Duration::days(1);
    for case in 0..4_i64 {
        let user = 991_200 + case;
        join(pool, user, user, joined, "ViewerCode").await;
        let invite = candidate(pool, user).await;
        let member = MemberProof {
            joined_at: Some(joined),
            is_bot: false,
            checked_at: now,
        };
        assert_eq!(
            evaluate_one(pool, &invite, Some(&member), &[], now)
                .await
                .expect("deadline"),
            Some("expired")
        );
        match case {
            0 => {
                sqlx::query(
                    "UPDATE activity.twitch_invite_members SET prior_member=TRUE WHERE user_id=$1",
                )
                .bind(user)
                .execute(pool)
                .await
                .expect("prior exclusion");
            }
            1 => {
                sqlx::query("INSERT INTO core.user_privacy(user_id,opted_out) VALUES($1,TRUE) ON CONFLICT(user_id) DO UPDATE SET opted_out=TRUE").bind(user).execute(pool).await.expect("privacy exclusion");
            }
            2 => {
                sqlx::query("UPDATE activity.twitch_invite_members SET left_at=$2,current_joined_at=NULL WHERE user_id=$1").bind(user).bind(joined+Duration::days(13)).execute(pool).await.expect("retention exclusion");
            }
            _ => {}
        }
        let started = if case == 3 {
            deadline - Duration::minutes(15) + Duration::microseconds(1)
        } else {
            joined + Duration::days(2)
        };
        voice(pool, 9_912_000 + case, user, 20, started, 900).await;
        assert_eq!(evidence_queue_count(pool, user).await, 0);
        assert_eq!(
            evaluate_one(pool, &invite, Some(&member), &[], now)
                .await
                .expect("no false reopening"),
            None
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM bot.twitch_invite_joins WHERE join_id=$1")
                .bind(user)
                .fetch_one(pool)
                .await
                .expect("still expired");
        assert_eq!(status, "expired");
    }
}

#[tokio::test]
async fn evidence_writer_waits_for_expiry_and_leaves_a_fresh_marker_after_commit() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-01-01T12:00:00Z");
    let now = joined + Duration::days(31);
    join(pool, 991_300, 991_300, joined, "ViewerCode").await;
    messages(pool, 991_300, 9_913_000, &[joined + Duration::days(1); 4]).await;
    // Hold precisely the evaluator's join UPDATE lock while the real message
    // writer has inserted its event and waits in the evidence SHARE trigger.
    let mut expiring = pool.begin().await.expect("expiry transaction");
    lock_changes(&mut expiring, 1)
        .await
        .expect("evaluator guild lock");
    sqlx::query("SELECT join_id FROM bot.twitch_invite_joins WHERE join_id=991300 FOR UPDATE")
        .execute(&mut *expiring)
        .await
        .expect("evaluator join lock");
    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        record_message(
            &writer_pool,
            9_913_004,
            1,
            991_300,
            joined + Duration::days(29),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let waits:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='transactionid' AND NOT granted AND pid IN (SELECT pid FROM pg_stat_activity WHERE datname=current_database()))")
                .fetch_one(pool).await.expect("real blocked evidence writer");
            if waits {break;}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("writer waits for expiry");
    sqlx::query("UPDATE bot.twitch_invite_joins SET status='expired',reason='deadline' WHERE join_id=991300").execute(&mut *expiring).await.expect("deadline decision");
    sqlx::query(
        "DELETE FROM activity.twitch_invite_evidence_queue WHERE guild_id=1 AND user_id=991300",
    )
    .execute(&mut *expiring)
    .await
    .expect("consume old marker");
    expiring.commit().await.expect("expiry commit");
    writer
        .await
        .expect("writer task")
        .expect("evidence commits after expiry");
    assert_eq!(evidence_queue_count(pool, 991_300).await, 1);
    let invite = candidate(pool, 991_300).await;
    let member = MemberProof {
        joined_at: Some(joined),
        is_bot: false,
        checked_at: now,
    };
    assert_eq!(
        evaluate_one(pool, &invite, Some(&member), &[], now)
            .await
            .expect("consume new committed evidence"),
        Some("qualified")
    );
}

#[tokio::test]
async fn ineligible_rejoin_cannot_consume_original_join_late_evidence() {
    let db = database().await;
    let pool = db.pool();
    let user = 991_400;
    let joined = at("2026-01-01T12:00:00Z");
    let deadline = joined + Duration::days(30);
    join(pool, 991_400, user, joined, "ViewerCode").await;
    let original = candidate(pool, user).await;
    let member = MemberProof {
        joined_at: Some(joined),
        is_bot: false,
        checked_at: deadline,
    };
    assert_eq!(
        evaluate_one(pool, &original, Some(&member), &[], deadline)
            .await
            .expect("original expires"),
        Some("expired")
    );
    let left = deadline + Duration::days(1);
    let mut tx = pool.begin().await.expect("later departure");
    remember_member_event(&mut tx, 991_401, 1, user, "leave", Some(left), None)
        .await
        .expect("first departure");
    tx.commit().await.expect("departure commit");
    join(pool, 991_402, user, left + Duration::days(1), "ViewerCode").await;
    let rejoin = candidate(pool, user).await;
    assert!(!rejoin.eligible);
    assert_eq!(rejoin.join_id, 991_402);
    voice(
        pool,
        9_914_000,
        user,
        20,
        deadline - Duration::minutes(16),
        900,
    )
    .await;
    assert_eq!(evidence_queue_count(pool, user).await, 1);
    let now = deadline + Duration::days(3);
    assert_eq!(
        evaluate_one(pool, &rejoin, None, &[], now)
            .await
            .expect("captured rejoin candidate"),
        Some("expired")
    );
    assert_eq!(
        evidence_queue_count(pool, user).await,
        1,
        "unrelated rejoin leaves original marker alone"
    );
    assert_eq!(candidate(pool, user).await.join_id, original.join_id);
    assert_eq!(
        evaluate_one(pool, &original, None, &[], now)
            .await
            .expect("original historical qualification"),
        Some("qualified")
    );
    assert_eq!(evidence_queue_count(pool, user).await, 0);
}
