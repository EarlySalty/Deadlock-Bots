#![cfg(feature = "testing")]

use dl_central_db::TestDb;
use dl_squads::model::SeedPlayer;
use dl_squads::store::{
    add_team_member, create_team, list_pool, upsert_participant_by_discord,
    upsert_participant_by_name, SquadErr,
};
use std::collections::BTreeMap;

async fn test_db() -> Result<TestDb, Box<dyn std::error::Error>> {
    Ok(dl_central_db::testing::test_pool().await?)
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn discord_upsert_is_idempotent_and_keeps_new_status(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;

    let first = upsert_participant_by_discord(db.pool(), 123_456, "Erster Name").await?;
    let second = upsert_participant_by_discord(db.pool(), 123_456, "Neuer Name").await?;

    assert_eq!(first, second);

    let pool = list_pool(db.pool(), None).await?;
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].id, first);
    assert_eq!(pool[0].discord_id, Some(123_456));
    assert_eq!(pool[0].display_name, "Neuer Name");
    assert_eq!(pool[0].status, "new");
    assert_eq!(pool[0].source, "discord_reaction");
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn discord_upsert_rejects_display_name_split_conflict(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;

    let discord_row_id = upsert_participant_by_discord(db.pool(), 555_001, "A").await?;
    let name_row_id = upsert_participant_by_name(db.pool(), &seed_player("B")).await?;

    let err = upsert_participant_by_discord(db.pool(), 555_001, "B")
        .await
        .expect_err("split conflict must fail like the old display_name UNIQUE constraint");
    match err {
        SquadErr::ParticipantDisplayNameTaken {
            display_name,
            existing_id,
        } => {
            assert_eq!(display_name, "B");
            assert_eq!(existing_id, name_row_id);
        }
        other => panic!("unexpected error: {other}"),
    }

    let rows = sqlx::query!(
        r#"
        SELECT id, discord_id, display_name
          FROM scrim.participants
         ORDER BY id ASC
        "#
    )
    .fetch_all(db.pool())
    .await?;
    assert_eq!(rows.len(), 2);

    let discord_row = rows
        .iter()
        .find(|row| i64::from(row.id) == discord_row_id)
        .expect("discord row remains present");
    assert_eq!(discord_row.discord_id, Some(555_001));
    assert_eq!(discord_row.display_name, "A");

    let name_row = rows
        .iter()
        .find(|row| i64::from(row.id) == name_row_id)
        .expect("name row remains present");
    assert_eq!(name_row.discord_id, None);
    assert_eq!(name_row.display_name, "B");

    let b_count = rows.iter().filter(|row| row.display_name == "B").count();
    assert_eq!(b_count, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn discord_upsert_verknuepft_bestehenden_seed_namen() -> Result<(), Box<dyn std::error::Error>>
{
    let db = test_db().await?;

    let seed_id = upsert_participant_by_name(db.pool(), &seed_player("Vicky")).await?;
    let discord_id = upsert_participant_by_discord(db.pool(), 987, "Vicky").await?;

    assert_eq!(seed_id, discord_id);
    let pool = list_pool(db.pool(), None).await?;
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].discord_id, Some(987));
    assert_eq!(pool[0].display_name, "Vicky");
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn name_upsert_conflict_bleibt_ein_datensatz() -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;

    let first = upsert_participant_by_name(db.pool(), &seed_player("Eno")).await?;
    let second = upsert_participant_by_name(db.pool(), &seed_player("Eno")).await?;

    assert_eq!(first, second);
    let pool = list_pool(db.pool(), None).await?;
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].display_name, "Eno");
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn create_team_and_add_member_are_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;

    let participant_id = upsert_participant_by_discord(db.pool(), 987, "Vicky").await?;
    let first_team_id = create_team(db.pool(), "Team 1", Some("Leo"), Some(111), Some(222)).await?;
    let second_team_id =
        create_team(db.pool(), "Team 1", Some("Leo"), Some(111), Some(222)).await?;
    add_team_member(
        db.pool(),
        first_team_id,
        participant_id,
        Some("S"),
        true,
        false,
    )
    .await?;
    add_team_member(
        db.pool(),
        first_team_id,
        participant_id,
        Some("S"),
        true,
        false,
    )
    .await?;

    assert_eq!(first_team_id, second_team_id);

    let first_team_id = i32::try_from(first_team_id)?;
    let participant_id = i32::try_from(participant_id)?;
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.team_members
         WHERE team_id = $1
           AND participant_id = $2
           AND role = 'S'
           AND is_captain = TRUE
           AND is_bench = FALSE
        "#,
        first_team_id,
        participant_id
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(row.count, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn list_pool_filters_by_status() -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;

    let new_id = upsert_participant_by_discord(db.pool(), 1, "Neu").await?;
    let waitlist_id = upsert_participant_by_discord(db.pool(), 2, "Wartend").await?;
    let waitlist_id_i32 = i32::try_from(waitlist_id)?;
    sqlx::query!(
        r#"
        UPDATE scrim.participants
           SET status = 'waitlist'
         WHERE id = $1
        "#,
        waitlist_id_i32
    )
    .execute(db.pool())
    .await?;

    let new_pool = list_pool(db.pool(), Some("new")).await?;
    assert_eq!(new_pool.len(), 1);
    assert_eq!(new_pool[0].id, new_id);

    let row = sqlx::query!(
        r#"
        SELECT status
          FROM scrim.participants
         WHERE id = $1
        "#,
        waitlist_id_i32
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(row.status, "waitlist");
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn add_team_member_rejects_missing_team_without_fake_row(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;
    let participant_id = upsert_participant_by_discord(db.pool(), 123, "Vicky").await?;

    let err = add_team_member(db.pool(), 404, participant_id, Some("S"), false, false)
        .await
        .expect_err("missing team must fail");

    assert!(matches!(err, SquadErr::MissingTeamId(404)));
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.team_members
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(row.count, 0);
    Ok(())
}

fn seed_player(name: &str) -> SeedPlayer {
    SeedPlayer {
        name: name.to_string(),
        rank: None,
        roles: None,
        availability: BTreeMap::new(),
        coach: None,
        note: None,
    }
}
