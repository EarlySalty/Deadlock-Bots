#[cfg(feature = "testing")]
async fn wait_for_guild_lock_waiter(pool: &PgPool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory'
                 AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))",
            ).fetch_one(pool).await.expect("lock wait evidence");
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("revoker waits for creation transaction");
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn revocation_waits_for_mapping_insert_and_snapshot_fetch_waits_for_creation() {
    for delete_event in [true, false] {
        let db = invite_db().await;
        let tracker = Arc::new(InviteTracker::new(db.pool().clone()));
        let mut creating = db.pool().begin().await.expect("creator transaction");
        InviteTracker::lock_personal_changes(&mut creating, 1)
            .await
            .expect("same creation lock");
        let revoker = tracker.clone();
        let revoked = tokio::spawn(async move {
            if delete_event {
                revoker.on_invite_delete(1, "Racing").await
            } else {
                // Production prime/on_join take this lock BEFORE observing/fetching.
                let (tx, generation) = revoker.begin_snapshot(1).await?;
                let observed_at = chrono::Utc::now();
                revoker
                    .persist_complete_snapshot_tx(tx, generation, 1, &HashMap::new(), observed_at)
                    .await
            }
        });
        wait_for_guild_lock_waiter(db.pool()).await;
        sqlx::query("INSERT INTO bot.twitch_personal_invites
            (streamer_twitch_user_id,inviter_twitch_user_id,streamer_login,guild_id,channel_id,invite_code,invite_url)
            VALUES ('42','43','fixture',1,2,'Racing','https://discord.gg/Racing')")
            .execute(&mut *creating).await.expect("mapping after external invite deletion");
        creating.commit().await.expect("creator commit");
        revoked
            .await
            .expect("revoker task")
            .expect("revoker commit");
        let marked: bool = sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM bot.twitch_personal_invites WHERE invite_code='Racing'")
            .fetch_one(db.pool()).await.expect("revocation evidence");
        assert!(marked, "both revocation paths see the committed mapping");
    }
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn snapshot_finishes_before_later_creation_and_old_epoch_cannot_restore_health() {
    let db = invite_db().await;
    let tracker = Arc::new(InviteTracker::new(db.pool().clone()));
    let (tx, generation) = tracker.begin_snapshot(1).await.expect("snapshot lock");
    let observed_at = chrono::Utc::now();
    let pool = db.pool().clone();
    let creator = tokio::spawn(async move {
        let mut creating = pool.begin().await?;
        InviteTracker::lock_personal_changes(&mut creating, 1).await?;
        sqlx::query("INSERT INTO bot.twitch_personal_invites
            (streamer_twitch_user_id,inviter_twitch_user_id,streamer_login,guild_id,channel_id,invite_code,invite_url)
            VALUES ('42','43','fixture',1,2,'Later','https://discord.gg/Later')")
            .execute(&mut *creating).await?;
        creating.commit().await
    });
    wait_for_guild_lock_waiter(db.pool()).await;
    tracker.health.invalidate_all().await;
    tracker
        .persist_complete_snapshot_tx(tx, generation, 1, &HashMap::new(), observed_at)
        .await
        .expect("old snapshot commit");
    assert!(
        !tracker.health.is_current(1).await,
        "old in-flight snapshot cannot heal a reconnect"
    );
    creator
        .await
        .expect("creator task")
        .expect("later creation");
    let active: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NULL FROM bot.twitch_personal_invites WHERE invite_code='Later'",
    )
    .fetch_one(db.pool())
    .await
    .expect("later valid mapping");
    assert!(
        active,
        "earlier snapshot cannot revoke a later valid creation"
    );
    let current = HashMap::from([("Later".to_string(), attribution_snapshot(0))]);
    tracker
        .persist_complete_snapshot(1, &current, chrono::Utc::now())
        .await
        .expect("new successful observation");
    assert!(tracker.health.is_current(1).await);
}
