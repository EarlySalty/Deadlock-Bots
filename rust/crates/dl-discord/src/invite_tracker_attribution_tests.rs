fn attribution_snapshot(uses: u64) -> InviteSnap {
    InviteSnap {
        uses,
        url: "https://discord.gg/Fixture".into(),
        inviter_id: None,
        inviter_name: String::new(),
        inviter_bot: false,
        channel_id: Some(2),
        channel_name: String::new(),
    }
}

#[test]
fn one_invite_use_can_be_attributed_but_multiple_deltas_cannot() {
    let before = HashMap::from([
        ("First".into(), attribution_snapshot(0)),
        ("Second".into(), attribution_snapshot(0)),
    ]);
    let mut after = before.clone();
    after.get_mut("First").expect("first invite").uses = 1;
    let mut metadata = Map::new();
    assert_eq!(
        classify_snapshots(&mut metadata, Some(&before), Some(&after)),
        "invite_link"
    );
    assert_eq!(metadata["invite_code"], "First");
    after.get_mut("Second").expect("second invite").uses = 1;
    assert_eq!(
        classify_snapshots(&mut metadata, Some(&before), Some(&after)),
        "unknown"
    );
    assert!(!metadata.contains_key("invite_code"));
    assert!(!metadata.contains_key("inviter_id"));
    assert_eq!(metadata["join_source_reason"], "ambiguous_invite_delta");
}

#[test]
fn a_batch_of_uses_is_not_assigned_to_one_arbitrary_join() {
    let before = HashMap::from([("First".into(), attribution_snapshot(1))]);
    let after = HashMap::from([("First".into(), attribution_snapshot(3))]);
    let mut metadata = Map::new();
    assert_eq!(
        classify_snapshots(&mut metadata, Some(&before), Some(&after)),
        "unknown"
    );
    assert!(!metadata.contains_key("invite_code"));
}

#[tokio::test]
async fn join_snapshot_locks_serialize_the_same_guild_without_blocking_other_guilds() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1/unused")
        .expect("unused pool");
    let tracker = InviteTracker::new(pool);
    let held = tracker.lock_guild(1).await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), tracker.lock_guild(1))
            .await
            .is_err()
    );
    let other = tracker.lock_guild(2).await;
    drop(other);
    drop(held);
    let _same = tracker.lock_guild(1).await;
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn observed_ambiguous_join_stays_unknown_after_a_code_disappears() {
    let db = invite_db().await;
    let tracker = InviteTracker::new(db.pool().clone());
    let before = HashMap::from([
        ("First".into(), attribution_snapshot(0)),
        ("Second".into(), attribution_snapshot(0)),
    ]);
    let after = HashMap::from([
        ("First".into(), attribution_snapshot(1)),
        ("Second".into(), attribution_snapshot(1)),
    ]);
    let mut metadata = Map::new();
    let kind = classify_snapshots(&mut metadata, Some(&before), Some(&after));
    assert_eq!(kind, "unknown");
    assert!(!should_retry_join_source(
        true,
        &kind,
        metadata.get("join_source_reason").and_then(Value::as_str)
    ));
    metadata.insert("join_source".into(), Value::String(kind.into()));
    tracker
        .persist_complete_snapshot(1, &after, chrono::Utc::now())
        .await
        .expect("consume ambiguous snapshot");
    sqlx::query(
        "INSERT INTO activity.member_events (id,user_id,guild_id,event_type,occurred_at,metadata)
        VALUES (918273645,100,1,'join',NOW(),$1)",
    )
    .bind(Value::Object(metadata.clone()))
    .execute(db.pool())
    .await
    .expect("terminal join evidence");
    let later = HashMap::from([("First".into(), attribution_snapshot(1))]);
    tracker
        .persist_complete_snapshot(1, &later, chrono::Utc::now())
        .await
        .expect("later complete snapshot");
    let stored: Value =
        sqlx::query_scalar("SELECT metadata FROM activity.member_events WHERE id=918273645")
            .fetch_one(db.pool())
            .await
            .expect("join evidence");
    assert_eq!(stored["join_source_reason"], "ambiguous_invite_delta");
    assert!(stored.get("invite_code").is_none());
    let baseline = tracker
        .load_snapshot_from_db(1)
        .await
        .expect("new baseline");
    assert_eq!(baseline["First"].uses, 1);
}
