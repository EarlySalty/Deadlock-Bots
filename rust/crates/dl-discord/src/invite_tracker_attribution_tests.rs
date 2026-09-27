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
