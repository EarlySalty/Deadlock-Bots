#[tokio::test]
async fn live_voice_proof_is_persisted_without_a_completed_points_session() {
    let db = database().await;
    let pool = db.pool();
    let joined = at("2026-03-15T12:00:00Z");
    join(pool, 17, 115, joined, "ViewerCode").await;
    let members = std::collections::HashMap::from([(115_u64, 20_u64)]);
    let start = joined + Duration::days(1);
    for minute in 0..=15 {
        crate::qualified_invite_voice::record_snapshot(
            pool,
            1,
            &members,
            &[98, 99],
            start + Duration::minutes(minute),
            None,
            false,
        )
        .await
        .expect("live voice snapshot");
    }
    let logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activity.voice_session_log")
        .fetch_one(pool)
        .await
        .expect("no points session required");
    assert_eq!(logs, 0);
    crate::qualified_invite_voice::reset_live_clocks(pool, 1)
        .await
        .expect("restart clears only current clock");
    let invite = candidate(pool, 115).await;
    let member = proof(&invite, 14);
    assert_eq!(
        evaluate_one(pool, &invite, Some(&member), &[98, 99], member.checked_at)
            .await
            .expect("retained live proof"),
        Some("qualified")
    );
    let qualified_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT qualified_at FROM bot.twitch_invite_joins WHERE join_id = 17")
            .fetch_one(pool)
            .await
            .expect("qualification timestamp");
    assert_eq!(qualified_at - joined, Duration::hours(336));
}

#[tokio::test]
async fn source_ids_are_not_reused_after_activity_history_cleanup() {
    let db = database().await;
    let pool = db.pool();
    join(pool, 50, 120, at("2026-01-01T12:00:00Z"), "ViewerCode").await;
    sqlx::query("DELETE FROM activity.member_events").execute(pool).await.expect("raw history cleanup");
    let mut tx = pool.begin().await.expect("source allocation");
    crate::db::lock_member_events(&mut tx).await.expect("source lock");
    let next = crate::db::next_member_event_id_in_tx(&mut tx).await.expect("retained source watermark");
    assert_eq!(next, 51);
    tx.rollback().await.expect("fixture rollback");
}
