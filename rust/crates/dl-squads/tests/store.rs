use dl_db::Db;
use dl_squads::model::SeedPlayer;
use dl_squads::store::{
    add_team_member, create_team, list_pool, upsert_participant_by_discord,
    upsert_participant_by_name,
};
use rusqlite::params;
use std::collections::BTreeMap;

async fn test_db() -> Result<(tempfile::TempDir, Db), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let db = Db::open_creating(dir.path().join("squads.sqlite3"))?;
    db.bootstrap_schema().await?;
    Ok((dir, db))
}

#[tokio::test]
async fn discord_upsert_is_idempotent_and_keeps_new_status(
) -> Result<(), Box<dyn std::error::Error>> {
    let (_dir, db) = test_db().await?;

    let first = upsert_participant_by_discord(&db, 123_456, "Erster Name").await?;
    let second = upsert_participant_by_discord(&db, 123_456, "Neuer Name").await?;

    assert_eq!(first, second);

    let pool = list_pool(&db, None).await?;
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].id, first);
    assert_eq!(pool[0].discord_id, Some(123_456));
    assert_eq!(pool[0].display_name, "Neuer Name");
    assert_eq!(pool[0].status, "new");
    assert_eq!(pool[0].source, "discord_reaction");
    Ok(())
}

#[tokio::test]
async fn discord_upsert_verknuepft_bestehenden_seed_namen() -> Result<(), Box<dyn std::error::Error>>
{
    let (_dir, db) = test_db().await?;

    let seed_id = upsert_participant_by_name(&db, &seed_player("Vicky")).await?;
    let discord_id = upsert_participant_by_discord(&db, 987, "Vicky").await?;

    assert_eq!(seed_id, discord_id);
    let pool = list_pool(&db, None).await?;
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].discord_id, Some(987));
    assert_eq!(pool[0].display_name, "Vicky");
    Ok(())
}

#[tokio::test]
async fn name_upsert_conflict_bleibt_ein_datensatz() -> Result<(), Box<dyn std::error::Error>> {
    let (_dir, db) = test_db().await?;

    let first = upsert_participant_by_name(&db, &seed_player("Eno")).await?;
    let second = upsert_participant_by_name(&db, &seed_player("Eno")).await?;

    assert_eq!(first, second);
    let pool = list_pool(&db, None).await?;
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].display_name, "Eno");
    Ok(())
}

#[tokio::test]
async fn create_team_and_add_member_are_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let (_dir, db) = test_db().await?;

    let participant_id = upsert_participant_by_discord(&db, 987, "Vicky").await?;
    let first_team_id = create_team(&db, "Team 1", Some("Leo"), Some(111), Some(222)).await?;
    let second_team_id = create_team(&db, "Team 1", Some("Leo"), Some(111), Some(222)).await?;
    add_team_member(&db, first_team_id, participant_id, Some("S"), true, false).await?;
    add_team_member(&db, first_team_id, participant_id, Some("S"), true, false).await?;

    assert_eq!(first_team_id, second_team_id);

    let member_count: i64 = db
        .read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM scrim_team_member
                  WHERE team_id = ?1 AND participant_id = ?2 AND role = 'S'
                    AND is_captain = 1 AND is_bench = 0",
                params![first_team_id, participant_id],
                |row| row.get(0),
            )
        })
        .await?;
    assert_eq!(member_count, 1);
    Ok(())
}

#[tokio::test]
async fn list_pool_filters_by_status() -> Result<(), Box<dyn std::error::Error>> {
    let (_dir, db) = test_db().await?;

    let new_id = upsert_participant_by_discord(&db, 1, "Neu").await?;
    let waitlist_id = upsert_participant_by_discord(&db, 2, "Wartend").await?;
    db.write(move |conn| {
        conn.execute(
            "UPDATE scrim_participant SET status = 'waitlist' WHERE id = ?1",
            params![waitlist_id],
        )?;
        Ok(())
    })
    .await?;

    let new_pool = list_pool(&db, Some("new")).await?;
    assert_eq!(new_pool.len(), 1);
    assert_eq!(new_pool[0].id, new_id);

    let waitlist_status: String = db
        .read(move |conn| {
            conn.query_row(
                "SELECT status FROM scrim_participant WHERE id = ?1",
                params![waitlist_id],
                |row| row.get(0),
            )
        })
        .await?;
    assert_eq!(waitlist_status, "waitlist");
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
