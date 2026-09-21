// Eingebunden in engine::tests, dieselben DB- und Port-Fixtures wie Startup-Purge.

async fn seed_reconcile_lane(engine: &TempVoiceEngine, port: &MockPort, channel_id: u64) {
    port.names
        .lock()
        .expect("lock")
        .insert(channel_id, "Lane 1".to_string());
    port.categories
        .lock()
        .expect("lock")
        .insert(channel_id, CASUAL_CATEGORY);
    engine
        .store
        .upsert_lane(LaneRecord {
            channel_id,
            guild_id: engine.config.guild_id_hint,
            owner_id: 100,
            initial_owner_id: Some(100),
            base_name: "Lane 1".to_string(),
            category_id: CASUAL_CATEGORY,
            source_staging_id: None,
        })
        .await
        .expect("lane");
    engine.rehydrate().await;
}

#[tokio::test]
async fn reconcile_lost_leave_clears_ghost_and_deletes_after_grace() {
    let (_db, engine, port, _) = setup().await;
    seed_reconcile_lane(&engine, &port, 4242).await;
    engine
        .state
        .lock()
        .await
        .join_time
        .entry(4242)
        .or_default()
        .insert(100, Utc::now().naive_utc() - chrono::Duration::minutes(30));
    let now = tokio::time::Instant::now();
    engine.reconcile_lanes_at(now).await;
    assert!(engine.state.lock().await.join_time[&4242].is_empty());
    assert!(port.deleted.lock().expect("lock").is_empty());
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(299))
        .await;
    assert!(port.deleted.lock().expect("lock").is_empty());
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(300))
        .await;
    assert_eq!(*port.deleted.lock().expect("lock"), vec![4242]);
    assert!(engine.state.lock().await.lanes.is_empty());
    assert!(engine.state.lock().await.empty_since.is_empty());
    assert!(engine.store.all_lanes().await.expect("rows").is_empty());
}

#[tokio::test]
async fn reconcile_schuetzt_besetzte_feste_und_staging_kanaele() {
    let (_db, engine, port, staging) = setup().await;
    seed_reconcile_lane(&engine, &port, 4242).await;
    let fixed = crate::router::ROUTER_VC_ID;
    for id in [fixed, staging] {
        port.names
            .lock()
            .expect("lock")
            .insert(id, "Einstieg".to_string());
        port.categories
            .lock()
            .expect("lock")
            .insert(id, CASUAL_CATEGORY);
    }
    port.members.lock().expect("lock").insert(4242, vec![200]);
    let now = tokio::time::Instant::now();
    engine.reconcile_lanes_at(now).await;
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(3600))
        .await;
    engine.purge_empty_lanes().await;
    assert!(port.deleted.lock().expect("lock").is_empty());
    assert!(engine.state.lock().await.join_time[&4242].contains_key(&200));
    assert!(engine.state.lock().await.empty_since.is_empty());
}

#[tokio::test]
async fn reconcile_unbekannter_cache_startet_frische_leerfrist() {
    let (_db, engine, port, _) = setup().await;
    seed_reconcile_lane(&engine, &port, 4242).await;
    let now = tokio::time::Instant::now();
    engine.reconcile_lanes_at(now).await;
    port.cache_unavailable.store(true, Ordering::Relaxed);
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(360))
        .await;
    engine.cleanup_lane(4242, "TempVoice: Test").await;
    assert!(port.deleted.lock().expect("lock").is_empty());
    port.cache_unavailable.store(false, Ordering::Relaxed);
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(420))
        .await;
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(719))
        .await;
    assert!(port.deleted.lock().expect("lock").is_empty());
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(720))
        .await;
    assert_eq!(*port.deleted.lock().expect("lock"), vec![4242]);
}

#[tokio::test]
async fn reconcile_reconnect_entfernt_fehlende_lane_und_schliesst_offene_session() {
    let (db, engine, port, _) = setup().await;
    seed_reconcile_lane(&engine, &port, 4242).await;
    engine
        .state
        .lock()
        .await
        .join_time
        .entry(4242)
        .or_default()
        .insert(100, Utc::now().naive_utc() - chrono::Duration::minutes(30));
    sqlx::query("INSERT INTO activity.voice_open_sessions(user_id, guild_id, channel_id, joined_at, updated_at)
        VALUES(100, $1, 4242, '2026-08-02 20:00Z', '2026-08-02 20:02Z')")
        .bind(engine.config.guild_id_hint as i64).execute(db.pool()).await.expect("session");
    port.names.lock().expect("lock").remove(&4242);
    port.cache_unavailable.store(true, Ordering::Relaxed);
    engine
        .reconcile_gateway(GatewayEvent::Ready { guild_count: 1 })
        .await;
    assert_eq!(engine.lane_owner(4242).await, Some(100));
    port.cache_unavailable.store(false, Ordering::Relaxed);
    engine
        .reconcile_gateway(GatewayEvent::CacheReady {
            guild_ids: vec![999],
        })
        .await;
    assert_eq!(engine.lane_owner(4242).await, Some(100));
    engine
        .reconcile_gateway(GatewayEvent::CacheReady {
            guild_ids: vec![engine.config.guild_id_hint],
        })
        .await;
    assert!(engine.lane_owner(4242).await.is_none());
    assert!(engine.state.lock().await.join_time.is_empty());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM activity.voice_open_sessions")
        .fetch_one(db.pool())
        .await
        .expect("open");
    assert_eq!(count, 0);
    let closed: (i64, chrono::DateTime<Utc>) = sqlx::query_as(
        "SELECT duration_seconds, occurred_at FROM activity.voice_metadata_events WHERE event_type = 'leave'",
    ).fetch_one(db.pool()).await.expect("closed");
    assert_eq!(closed.0, 120);
    assert_eq!(closed.1.to_rfc3339(), "2026-08-02T20:02:00+00:00");
}

#[tokio::test]
async fn reconcile_neuerer_join_wird_nicht_von_altem_snapshot_entfernt() {
    let (_db, engine, port, _) = setup().await;
    seed_reconcile_lane(&engine, &port, 4242).await;
    // Stellt einen Join nach Erfassung des Snapshots dar.
    engine
        .state
        .lock()
        .await
        .join_time
        .entry(4242)
        .or_default()
        .insert(200, Utc::now().naive_utc() + chrono::Duration::minutes(1));
    let now = tokio::time::Instant::now();
    engine.reconcile_lanes_at(now).await;
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(600))
        .await;
    engine.cleanup_lane(4242, "TempVoice: Test").await;
    assert!(engine.state.lock().await.join_time[&4242].contains_key(&200));
    assert!(port.deleted.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn reconcile_timeout_blockiert_keine_andere_lane_und_behaelt_retry() {
    let (_db, engine, port, _) = setup().await;
    seed_reconcile_lane(&engine, &port, 4242).await;
    seed_reconcile_lane(&engine, &port, 4243).await;
    port.hanging_delete.store(4242, Ordering::Relaxed);
    let now = tokio::time::Instant::now();
    engine.reconcile_lanes_at(now).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        engine.reconcile_lanes_at(now + std::time::Duration::from_secs(300)),
    )
    .await
    .expect("Reconcile muss trotz hängendem Discord-Delete enden");
    assert_eq!(engine.lane_owner(4242).await, Some(100));
    assert!(engine.lane_owner(4243).await.is_none());
    assert_eq!(engine.store.all_lanes().await.expect("retry").len(), 1);
    port.hanging_delete.store(0, Ordering::Relaxed);
    engine
        .reconcile_lanes_at(now + std::time::Duration::from_secs(360))
        .await;
    assert!(engine.lane_owner(4242).await.is_none());
}

#[tokio::test]

async fn reconcile_custom_lane_retry_bleibt_auch_bei_cache_miss_erhalten() {
    let (_db, engine, port, _) = setup().await;

    port.names
        .lock()
        .expect("lock")
        .insert(4249, "Custom".to_string());

    port.categories
        .lock()
        .expect("lock")
        .insert(4249, CASUAL_CATEGORY);

    port.delete_fails.store(true, Ordering::Relaxed);

    assert!(!engine.cleanup_lane(4249, "TempVoice: Test").await);

    assert!(engine.state.lock().await.cleanup_retry.contains(&4249));

    port.names.lock().expect("lock").remove(&4249);

    port.delete_fails.store(false, Ordering::Relaxed);

    engine.reconcile_lanes().await;

    assert!(engine.state.lock().await.cleanup_retry.is_empty());

    assert_eq!(*port.deleted.lock().expect("lock"), vec![4249, 4249]);
}
