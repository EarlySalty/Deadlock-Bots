// In interface::tests eingebunden: dieselben Ports, Handler und Wegwerf-DBs.

async fn live_streamer_fixture() -> (dl_central_db::TestDb, PanelHandler, Arc<ForeignLanePort>) {
    let (db, mut handler) = panel_handler_for_test().await;
    let port = Arc::new(ForeignLanePort::default());
    port.live_streamer
        .store(true, std::sync::atomic::Ordering::SeqCst);
    port.voice_channels.lock().expect("test mutex").extend([
        (42, Some(4242)),
        (99, Some(4242)),
        (77, Some(4242)),
    ]);
    handler.engine = TempVoiceEngine::new(
        handler.engine.config.clone(),
        TempVoiceStore::new(db.pool().clone()),
        port.clone(),
    );
    handler
        .engine
        .store
        .upsert_lane(LaneRecord {
            channel_id: 4242,
            guild_id: 1,
            owner_id: 42,
            initial_owner_id: Some(42),
            base_name: "Lane 1".to_string(),
            category_id: 1289721245281292290,
            source_staging_id: None,
        })
        .await
        .expect("lane");
    handler.engine.rehydrate().await;
    port.actions.lock().expect("test mutex").clear();
    (db, handler, port)
}

fn streamer_click(custom_id: impl Into<String>, target: Option<u64>) -> BridgeInteraction {
    BridgeInteraction {
        custom_id: custom_id.into(),
        guild_id: 1,
        user_id: 99,
        values: target.map(|id| vec![id.to_string()]).unwrap_or_default(),
        ..BridgeInteraction::default()
    }
}

async fn streamer_menu(handler: &PanelHandler, action: &str) -> String {
    let reply = handler.handle(streamer_click(action, None)).await;
    reply_custom_ids(&reply)
        .into_iter()
        .find(|id| id.starts_with(MODERATION_PREFIX))
        .unwrap_or_else(|| panic!("bound menu missing: {:?}", reply.content))
}

#[test]
fn live_streamer_moderation_context_validates_action_and_ids() {
    let context = ModerationContext {
        lane: 4242,
        owner: 42,
    };
    for action in ["tv_kick_sel", "tv_ban_sel", "tv_unban_sel"] {
        let id = context.custom_id(action);
        assert_eq!(ModerationContext::parse(&id), Some((context, action)));
        assert!(id.len() <= 100);
    }
    for invalid in [
        "tv_moderation:0:42:tv_ban_sel",
        "tv_moderation:4242:no:tv_ban_sel",
        "tv_moderation:4242:42:tv_owner_claim",
        "tv_moderation:4242:42:tv_ban_sel:extra",
    ] {
        assert!(ModerationContext::parse(invalid).is_none());
    }
}

#[tokio::test]
async fn live_streamer_can_use_owner_controls_without_claiming_owner() {
    let (_db, handler, _port) = live_streamer_fixture().await;
    assert!(handler
        .owned_lane_of(&streamer_click("tv_limit_btn", None))
        .await
        .is_ok());
    let reply = handler.handle(streamer_click("tv_limit_btn", None)).await;
    assert!(reply.modal.is_some());
    assert_eq!(handler.engine.lane_owner(4242).await, Some(42));
}

#[tokio::test]
async fn live_streamer_access_stops_offline_but_owner_access_remains() {
    let (_db, handler, port) = live_streamer_fixture().await;
    port.live_streamer
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(handler
        .owned_lane_of(&streamer_click("tv_limit_btn", None))
        .await
        .is_err());
    let mut owner = streamer_click("tv_limit_btn", None);
    owner.user_id = 42;
    assert!(handler.handle(owner).await.modal.is_some());
}

#[tokio::test]
async fn live_streamer_access_requires_managed_lane_and_same_guild() {
    let (_db, handler, port) = live_streamer_fixture().await;
    for channel in [None, Some(9999)] {
        port.voice_channels
            .lock()
            .expect("test mutex")
            .insert(99, channel);
        assert!(handler
            .owned_lane_of(&streamer_click("tv_limit_btn", None))
            .await
            .is_err());
    }
    port.voice_channels
        .lock()
        .expect("test mutex")
        .insert(99, Some(4242));
    let mut other_guild = streamer_click("tv_limit_btn", None);
    other_guild.guild_id = 2;
    assert!(handler.owned_lane_of(&other_guild).await.is_err());
}

#[tokio::test]
async fn live_streamer_kicks_troll_but_not_owner() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let menu = streamer_menu(&handler, "tv_kick").await;
    let reply = handler.handle(streamer_click(menu.clone(), Some(42))).await;
    assert_eq!(reply.content.as_deref(), Some(PROTECTED_OWNER));
    assert!(port.actions.lock().expect("test mutex").is_empty());
    let reply = handler.handle(streamer_click(menu, Some(77))).await;
    assert!(reply
        .content
        .expect("valid test fixture")
        .contains("gekickt"));
    assert_eq!(*port.actions.lock().expect("test mutex"), vec!["kick:77"]);
    assert_eq!(handler.engine.lane_owner(4242).await, Some(42));
}

#[tokio::test]
async fn live_streamer_kick_menu_hides_owner_and_self() {
    let (_db, handler, _port) = live_streamer_fixture().await;
    let reply = handler.handle(streamer_click("tv_kick", None)).await;
    let options = reply.components.expect("valid test fixture")[0]["components"][0]["options"]
        .as_array()
        .expect("valid test fixture")
        .clone();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0]["value"], "77");
}

#[tokio::test]
async fn live_streamer_cannot_kick_target_in_another_call_or_self() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let menu = streamer_menu(&handler, "tv_kick").await;
    port.voice_channels
        .lock()
        .expect("test mutex")
        .insert(77, Some(7777));
    let reply = handler.handle(streamer_click(menu.clone(), Some(77))).await;
    assert_eq!(reply.content.as_deref(), Some(TARGET_NOT_IN_LANE));
    handler.handle(streamer_click(menu, Some(99))).await;
    assert!(port.actions.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn live_streamer_cannot_ban_owner_even_with_forged_selection() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let menu = streamer_menu(&handler, "tv_ban").await;
    let reply = handler.handle(streamer_click(menu, Some(42))).await;
    assert_eq!(reply.content.as_deref(), Some(PROTECTED_OWNER));
    assert!(handler
        .engine
        .store
        .list_bans(42)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(handler
        .engine
        .store
        .list_bans(99)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(port.actions.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn live_streamer_ban_and_unban_use_existing_owner_list() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let menu = streamer_menu(&handler, "tv_ban").await;
    handler.handle(streamer_click(menu, Some(77))).await;
    assert_eq!(
        handler
            .engine
            .store
            .list_bans(42)
            .await
            .expect("valid test fixture"),
        vec![77]
    );
    assert!(handler
        .engine
        .store
        .list_bans(99)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert_eq!(*port.actions.lock().expect("test mutex"), vec!["kick:77"]);
    let menu = streamer_menu(&handler, "tv_unban").await;
    handler.handle(streamer_click(menu, Some(77))).await;
    assert!(handler
        .engine
        .store
        .list_bans(42)
        .await
        .expect("valid test fixture")
        .is_empty());
}

#[tokio::test]
async fn live_streamer_ban_never_disconnects_another_call() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let menu = streamer_menu(&handler, "tv_ban").await;
    port.voice_channels
        .lock()
        .expect("test mutex")
        .insert(77, Some(7777));
    handler.handle(streamer_click(menu, Some(77))).await;
    assert_eq!(
        handler
            .engine
            .store
            .list_bans(42)
            .await
            .expect("valid test fixture"),
        vec![77]
    );
    assert!(port.actions.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn live_streamer_old_menus_recheck_access_without_personal_ban_fallback() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let kick = streamer_menu(&handler, "tv_kick").await;
    let ban = streamer_menu(&handler, "tv_ban").await;
    port.live_streamer
        .store(false, std::sync::atomic::Ordering::SeqCst);
    for menu in [kick, ban] {
        let reply = handler.handle(streamer_click(menu, Some(77))).await;
        assert_eq!(reply.content.as_deref(), Some(NOT_OWNER));
    }
    assert!(handler
        .engine
        .store
        .list_bans(42)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(handler
        .engine
        .store
        .list_bans(99)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(port.actions.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn live_streamer_old_unban_menu_rechecks_access() {
    let (_db, handler, port) = live_streamer_fixture().await;
    handler
        .engine
        .store
        .add_ban(42, 77)
        .await
        .expect("valid test fixture");
    let menu = streamer_menu(&handler, "tv_unban").await;
    port.live_streamer
        .store(false, std::sync::atomic::Ordering::SeqCst);
    handler.handle(streamer_click(menu, Some(77))).await;
    assert_eq!(
        handler
            .engine
            .store
            .list_bans(42)
            .await
            .expect("valid test fixture"),
        vec![77]
    );
}

#[tokio::test]
async fn live_streamer_old_menus_reject_voice_leave_and_lane_change() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let menu = streamer_menu(&handler, "tv_ban").await;
    for channel in [None, Some(9999)] {
        port.voice_channels
            .lock()
            .expect("test mutex")
            .insert(99, channel);
        handler.handle(streamer_click(menu.clone(), Some(77))).await;
    }
    assert!(handler
        .engine
        .store
        .list_bans(42)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(handler
        .engine
        .store
        .list_bans(99)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(port.actions.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn live_streamer_old_menu_rejects_owner_change() {
    let (_db, handler, port) = live_streamer_fixture().await;
    let menu = streamer_menu(&handler, "tv_ban").await;
    handler
        .engine
        .store
        .set_owner(4242, 77)
        .await
        .expect("valid test fixture");
    handler.engine.rehydrate().await;
    handler.handle(streamer_click(menu, Some(77))).await;
    assert!(handler
        .engine
        .store
        .list_bans(42)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(handler
        .engine
        .store
        .list_bans(77)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert!(port.actions.lock().expect("test mutex").is_empty());
}

#[tokio::test]
async fn live_streamer_unbound_personal_menu_cannot_modify_owner_list() {
    let (_db, handler, port) = live_streamer_fixture().await;
    handler.handle(streamer_click("tv_ban_sel", Some(77))).await;
    assert!(handler
        .engine
        .store
        .list_bans(42)
        .await
        .expect("valid test fixture")
        .is_empty());
    assert_eq!(
        handler
            .engine
            .store
            .list_bans(99)
            .await
            .expect("valid test fixture"),
        vec![77]
    );
    assert!(port.actions.lock().expect("test mutex").is_empty());
}
