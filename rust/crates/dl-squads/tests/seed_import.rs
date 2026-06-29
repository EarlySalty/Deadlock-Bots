use dl_db::Db;
use dl_squads::seed::import_seed_roster_json;
use dl_squads::SquadErr;
use rusqlite::params;

const SEED_JSON: &str = include_str!("../../../docs/specs/seed-roster-2026-06-29.json");

async fn test_db() -> Result<(tempfile::TempDir, Db), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let db = Db::open_creating(dir.path().join("seed.sqlite3"))?;
    db.bootstrap_schema().await?;
    Ok((dir, db))
}

async fn scalar_count(db: &Db, sql: &'static str) -> Result<i64, Box<dyn std::error::Error>> {
    let count = db
        .read(move |conn| conn.query_row(sql, [], |row| row.get(0)))
        .await?;
    Ok(count)
}

#[tokio::test]
async fn imports_seed_roster_and_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let (_dir, db) = test_db().await?;

    let first = import_seed_roster_json(&db, SEED_JSON).await?;
    let second = import_seed_roster_json(&db, SEED_JSON).await?;

    assert_eq!(first.players, 24);
    assert_eq!(first.pool_unassigned, 5);
    assert_eq!(first.teams, 4);
    assert_eq!(first.matches, 2);
    assert_eq!(second, first);

    assert_eq!(
        scalar_count(&db, "SELECT COUNT(*) FROM scrim_participant").await?,
        29
    );
    assert_eq!(
        scalar_count(
            &db,
            "SELECT COUNT(*) FROM scrim_participant WHERE status = 'assigned'"
        )
        .await?,
        24
    );
    assert_eq!(
        scalar_count(
            &db,
            "SELECT COUNT(*) FROM scrim_participant WHERE status = 'waitlist'"
        )
        .await?,
        5
    );
    assert_eq!(
        scalar_count(&db, "SELECT COUNT(*) FROM scrim_team").await?,
        4
    );
    assert_eq!(
        scalar_count(&db, "SELECT COUNT(*) FROM scrim_team_member").await?,
        24
    );
    assert_eq!(
        scalar_count(&db, "SELECT COUNT(*) FROM scrim_match").await?,
        2
    );

    let vicky_is_captain: i64 = db
        .read(|conn| {
            conn.query_row(
                "SELECT stm.is_captain
                   FROM scrim_team_member stm
                   JOIN scrim_team st ON st.id = stm.team_id
                   JOIN scrim_participant sp ON sp.id = stm.participant_id
                  WHERE st.name = ?1 AND sp.display_name = ?2",
                params!["Team 1", "Vicky"],
                |row| row.get(0),
            )
        })
        .await?;
    assert_eq!(vicky_is_captain, 1);

    Ok(())
}

#[tokio::test]
async fn trimmt_seed_namen_vor_insert_und_member_matching() -> Result<(), Box<dyn std::error::Error>>
{
    let (_dir, db) = test_db().await?;
    let json = r#"{
        "players": [{"name": " Vicky ", "rank": "Alchemist"}],
        "teams": [{"name": "Team 1", "members": [{"name": "Vicky"}]}]
    }"#;

    let summary = import_seed_roster_json(&db, json).await?;

    assert_eq!(summary.players, 1);
    assert_eq!(summary.teams, 1);
    let display_name: String = db
        .read(|conn| {
            conn.query_row("SELECT display_name FROM scrim_participant", [], |row| {
                row.get(0)
            })
        })
        .await?;
    assert_eq!(display_name, "Vicky");
    assert_eq!(
        scalar_count(&db, "SELECT COUNT(*) FROM scrim_team_member").await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn exakter_doppelter_seed_name_erzeugt_keine_zweite_person(
) -> Result<(), Box<dyn std::error::Error>> {
    let (_dir, db) = test_db().await?;
    let json = r#"{
        "players": [{"name": "Eno"}],
        "pool_unassigned": [{"name": "Eno"}]
    }"#;

    let summary = import_seed_roster_json(&db, json).await?;

    assert_eq!(summary.players, 1);
    assert_eq!(summary.pool_unassigned, 1);
    assert_eq!(
        scalar_count(&db, "SELECT COUNT(*) FROM scrim_participant").await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn trim_kollisionen_im_seed_werden_abgelehnt() -> Result<(), Box<dyn std::error::Error>> {
    let (_dir, db) = test_db().await?;
    let json = r#"{
        "players": [{"name": "Eno"}, {"name": " Eno "}]
    }"#;

    let err = import_seed_roster_json(&db, json)
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
