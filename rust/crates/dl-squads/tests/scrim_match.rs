use serde_json::{json, Value};
use sqlx::Row;

use dl_squads::scrim_match::{
    build_apply_convars_payload, build_create_custom_lobby_payload, build_invite_player_payload,
    build_leave_payload, build_match_result_payload, build_ready_payload,
    build_set_spectator_payload, build_start_match_payload,
    fetch_scrim_match_result_by_steam_match_id, GC_CREATE_CUSTOM_LOBBY, GC_GET_MATCH_RESULT,
    GC_LOBBY_APPLY_CONVARS, GC_LOBBY_INVITE_PLAYER, GC_LOBBY_LEAVE, GC_LOBBY_READY,
    GC_LOBBY_SET_SPECTATOR, GC_LOBBY_START_MATCH,
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

#[tokio::test]
async fn geladenes_ergebnis_bleibt_gespeichert_wenn_lobby_leave_fehlschlaegt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    let pool = db.pool();
    for (id, name) in [(1, "A"), (2, "B")] {
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES($1, $2, now())")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await?;
    }
    sqlx::query(
        r#"
        INSERT INTO scrim.matches(
            id, team_a_id, team_b_id, status, lobby_state, party_id, steam_match_id,
            created_at, updated_at
        )
        VALUES(10, 1, 2, 'scheduled', 'result_fetching', 'party-10', 987654321, now(), now())
        "#,
    )
    .execute(pool)
    .await?;

    let pool_for_fetch = pool.clone();
    let fetch = tokio::spawn(async move {
        fetch_scrim_match_result_by_steam_match_id(&pool_for_fetch, 10, 987_654_321).await
    });
    let result_task = wait_for_steam_task(pool, GC_GET_MATCH_RESULT).await?;
    sqlx::query("UPDATE steam.steam_tasks SET status = 'DONE', result = $2::jsonb WHERE id = $1")
        .bind(result_task)
        .bind(json!({ "match_id": 987654321, "winning_team": 0 }))
        .execute(pool)
        .await?;

    let leave_task = wait_for_steam_task(pool, GC_LOBBY_LEAVE).await?;
    sqlx::query(
        "UPDATE steam.steam_tasks SET status = 'FAILED', error = 'leave failed' WHERE id = $1",
    )
    .bind(leave_task)
    .execute(pool)
    .await?;

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), fetch).await???;
    assert_eq!(outcome.winner_team_id, Some(1));
    let row = sqlx::query(
        "SELECT lobby_state, winner_team_id, result_json FROM scrim.matches WHERE id = 10",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(row.get::<String, _>("lobby_state"), "finished");
    assert_eq!(row.get::<Option<i32>, _>("winner_team_id"), Some(1));
    assert!(row.get::<Option<Value>, _>("result_json").is_some());
    Ok(())
}

#[tokio::test]
async fn explizite_lookup_id_bleibt_bei_result_ohne_eigene_match_id_erhalten(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    let pool = db.pool();
    for (id, name) in [(1, "A"), (2, "B")] {
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES($1, $2, now())")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await?;
    }
    sqlx::query(
        r#"
        INSERT INTO scrim.matches(
            id, team_a_id, team_b_id, status, lobby_state, steam_match_id, created_at, updated_at
        )
        VALUES(11, 1, 2, 'scheduled', 'result_fetching', 111, now(), now())
        "#,
    )
    .execute(pool)
    .await?;

    let pool_for_fetch = pool.clone();
    let fetch = tokio::spawn(async move {
        fetch_scrim_match_result_by_steam_match_id(&pool_for_fetch, 11, 222).await
    });
    let result_task = wait_for_steam_task(pool, GC_GET_MATCH_RESULT).await?;
    sqlx::query("UPDATE steam.steam_tasks SET status = 'DONE', result = $2::jsonb WHERE id = $1")
        .bind(result_task)
        .bind(json!({ "winning_team": 1 }))
        .execute(pool)
        .await?;

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), fetch).await???;
    assert_eq!(outcome.steam_match_id, Some(222));
    let primary_id: Option<i64> =
        sqlx::query_scalar("SELECT steam_match_id FROM scrim.matches WHERE id = 11")
            .fetch_one(pool)
            .await?;
    assert_eq!(primary_id, Some(111));
    Ok(())
}

async fn wait_for_steam_task(
    pool: &sqlx::PgPool,
    task_type: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    for _ in 0..100 {
        if let Some(id) = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM steam.steam_tasks WHERE type = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(task_type)
        .fetch_optional(pool)
        .await?
        {
            return Ok(id);
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    Err(format!("Steam task {task_type} was not created").into())
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
