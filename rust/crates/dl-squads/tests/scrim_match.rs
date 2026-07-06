use serde_json::{json, Value};

use dl_squads::scrim_match::{
    build_apply_convars_payload, build_create_custom_lobby_payload, build_invite_player_payload,
    build_leave_payload, build_match_result_payload, build_ready_payload,
    build_set_spectator_payload, build_start_match_payload, GC_CREATE_CUSTOM_LOBBY,
    GC_GET_MATCH_RESULT, GC_LOBBY_APPLY_CONVARS, GC_LOBBY_INVITE_PLAYER, GC_LOBBY_LEAVE,
    GC_LOBBY_READY, GC_LOBBY_SET_SPECTATOR, GC_LOBBY_START_MATCH,
};

#[test]
fn gc_task_payloads_match_steam_handler_contracts() {
    assert_eq!(GC_CREATE_CUSTOM_LOBBY, "GC_CREATE_CUSTOM_LOBBY");
    assert_eq!(GC_LOBBY_INVITE_PLAYER, "GC_LOBBY_INVITE_PLAYER");
    assert_eq!(GC_LOBBY_SET_SPECTATOR, "GC_LOBBY_SET_SPECTATOR");
    assert_eq!(GC_LOBBY_APPLY_CONVARS, "GC_LOBBY_APPLY_CONVARS");
    assert_eq!(GC_LOBBY_READY, "GC_LOBBY_READY");
    assert_eq!(GC_LOBBY_START_MATCH, "GC_LOBBY_START_MATCH");
    assert_eq!(GC_LOBBY_LEAVE, "GC_LOBBY_LEAVE");
    assert_eq!(GC_GET_MATCH_RESULT, "GC_GET_MATCH_RESULT");

    let create = build_create_custom_lobby_payload();
    assert_eq!(
        sorted_keys(&create),
        ["convars", "game_mode", "is_private", "region_mode"]
    );
    assert_eq!(create["game_mode"], json!(1));
    assert_eq!(create["region_mode"], json!(1));
    assert_eq!(create["is_private"], json!(true));
    assert_eq!(create["convars"], json!({}));

    let invite = build_invite_player_payload("4242", 76_561_197_960_265_729);
    assert_eq!(sorted_keys(&invite), ["party_id", "steam_id"]);
    assert_eq!(invite["party_id"], json!("4242"));
    assert_eq!(invite["steam_id"], json!("76561197960265729"));
    assert!(invite.get("steam_id64").is_none());
    assert!(invite.get("account_id").is_none());

    for payload in [
        build_set_spectator_payload("4242"),
        build_ready_payload("4242"),
        build_start_match_payload("4242"),
        build_leave_payload("4242"),
    ] {
        assert_eq!(sorted_keys(&payload), ["party_id"]);
        assert_eq!(payload["party_id"], json!("4242"));
    }

    let convars = build_apply_convars_payload("4242");
    assert_eq!(sorted_keys(&convars), ["convars", "party_id"]);
    assert_eq!(convars["party_id"], json!("4242"));
    assert_eq!(
        convars["convars"],
        json!({ "citadel_allow_duplicate_heroes": 0 })
    );

    let result = build_match_result_payload(Some(987_654_321), Some("4242"));
    assert_eq!(sorted_keys(&result), ["match_id", "party_id"]);
    assert_eq!(result["match_id"], json!(987_654_321));
    assert_eq!(result["party_id"], json!("4242"));

    let result_by_party = build_match_result_payload(None, Some(" 4242 "));
    assert_eq!(sorted_keys(&result_by_party), ["party_id"]);
    assert_eq!(result_by_party["party_id"], json!("4242"));
}

#[cfg(feature = "testing")]
#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn resolve_team_steam_ids_splittet_linked_und_unlinked(
) -> Result<(), Box<dyn std::error::Error>> {
    use dl_central_db::{testing::test_pool, upsert_user};
    use dl_squads::model::SeedPlayer;
    use dl_squads::scrim_match::resolve_team_steam_ids;
    use dl_squads::store::{add_team_member, create_team, upsert_participant_by_discord};
    use std::collections::BTreeMap;

    let db = test_pool().await?;
    let pool = db.pool();
    upsert_user(pool, 101, Some("a"), None, None).await?;
    upsert_user(pool, 102, Some("b"), None, None).await?;

    let p1 = upsert_participant_by_discord(pool, 101, "A").await?;
    let p2 = upsert_participant_by_discord(pool, 102, "B").await?;
    let p3 = dl_squads::store::upsert_participant_by_name(
        pool,
        &SeedPlayer {
            name: "C".to_string(),
            rank: None,
            roles: None,
            availability: BTreeMap::new(),
            coach: None,
            note: None,
        },
    )
    .await?;
    let team_id = create_team(pool, "Team", None, None, None).await?;
    add_team_member(pool, team_id, p1, None, false, false).await?;
    add_team_member(pool, team_id, p2, None, false, false).await?;
    add_team_member(pool, team_id, p3, None, false, false).await?;

    insert_link(pool, 101, 76_561_197_960_265_729, false).await?;
    insert_link(pool, 102, 76_561_197_960_265_730, false).await?;
    insert_link(pool, 102, 76_561_197_960_265_731, true).await?;

    let (linked, unlinked) = resolve_team_steam_ids(pool, team_id).await?;

    assert_eq!(
        linked,
        vec![(p1, 76_561_197_960_265_729), (p2, 76_561_197_960_265_731),]
    );
    assert_eq!(unlinked, vec![p3]);
    Ok(())
}

#[cfg(feature = "testing")]
async fn insert_link(
    pool: &sqlx::PgPool,
    discord_id: i64,
    steam_id64: i64,
    primary_account: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO core.steam_links(
            discord_id, steam_id, steam_id64, verified, primary_account,
            is_steam_friend, linked_at, updated_at
        )
        VALUES($1, $2, $3, TRUE, $4, TRUE, now(), now())
        "#,
    )
    .bind(discord_id)
    .bind(steam_id64.to_string())
    .bind(steam_id64)
    .bind(primary_account)
    .execute(pool)
    .await?;
    Ok(())
}

fn sorted_keys(value: &Value) -> Vec<&str> {
    let mut keys = value
        .as_object()
        .map(|object| object.keys().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort_unstable();
    keys
}
