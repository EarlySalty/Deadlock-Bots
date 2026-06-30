#![cfg(feature = "testing")]

use dl_central_db::TestDb;
use dl_squads::seed::import_seed_roster_json;
use dl_squads::SquadErr;

const SEED_JSON: &str = include_str!("../../../docs/specs/seed-roster-2026-06-29.json");

async fn test_db() -> Result<TestDb, Box<dyn std::error::Error>> {
    Ok(dl_central_db::testing::test_pool().await?)
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn imports_seed_roster_and_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;

    let first = import_seed_roster_json(db.pool(), SEED_JSON).await?;
    let second = import_seed_roster_json(db.pool(), SEED_JSON).await?;

    assert_eq!(first.players, 24);
    assert_eq!(first.pool_unassigned, 5);
    assert_eq!(first.teams, 4);
    assert_eq!(first.matches, 2);
    assert_eq!(second, first);

    let participant_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.participants
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(participant_count.count, 29);

    let assigned_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.participants
         WHERE status = 'assigned'
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(assigned_count.count, 24);

    let waitlist_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.participants
         WHERE status = 'waitlist'
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(waitlist_count.count, 5);

    let team_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.teams
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(team_count.count, 4);

    let member_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.team_members
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(member_count.count, 24);

    let match_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.matches
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(match_count.count, 2);

    let vicky = sqlx::query!(
        r#"
        SELECT stm.is_captain
          FROM scrim.team_members stm
          JOIN scrim.teams st ON st.id = stm.team_id
          JOIN scrim.participants sp ON sp.id = stm.participant_id
         WHERE st.name = $1 AND sp.display_name = $2
        "#,
        "Team 1",
        "Vicky"
    )
    .fetch_one(db.pool())
    .await?;
    assert!(vicky.is_captain);

    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn trimmt_seed_namen_vor_insert_und_member_matching() -> Result<(), Box<dyn std::error::Error>>
{
    let db = test_db().await?;
    let json = r#"{
        "players": [{"name": " Vicky ", "rank": "Alchemist"}],
        "teams": [{"name": "Team 1", "members": [{"name": "Vicky"}]}]
    }"#;

    let summary = import_seed_roster_json(db.pool(), json).await?;

    assert_eq!(summary.players, 1);
    assert_eq!(summary.teams, 1);
    let row = sqlx::query!(
        r#"
        SELECT display_name
          FROM scrim.participants
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(row.display_name, "Vicky");

    let member_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.team_members
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(member_count.count, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn exakter_doppelter_seed_name_erzeugt_keine_zweite_person(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;
    let json = r#"{
        "players": [{"name": "Eno"}],
        "pool_unassigned": [{"name": "Eno"}]
    }"#;

    let summary = import_seed_roster_json(db.pool(), json).await?;

    assert_eq!(summary.players, 1);
    assert_eq!(summary.pool_unassigned, 1);
    let participant_count = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
          FROM scrim.participants
        "#
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(participant_count.count, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
async fn trim_kollisionen_im_seed_werden_abgelehnt() -> Result<(), Box<dyn std::error::Error>> {
    let db = test_db().await?;
    let json = r#"{
        "players": [{"name": "Eno"}, {"name": " Eno "}]
    }"#;

    let err = import_seed_roster_json(db.pool(), json)
        .await
        .expect_err("trim collision must fail");

    match err {
        SquadErr::DuplicateSeedParticipantName {
            trimmed,
            first,
            second,
        } => {
            assert_eq!(trimmed, "Eno");
            assert_eq!(first, "Eno");
            assert_eq!(second, " Eno ");
        }
        other => panic!("unexpected error: {other}"),
    }
    Ok(())
}
