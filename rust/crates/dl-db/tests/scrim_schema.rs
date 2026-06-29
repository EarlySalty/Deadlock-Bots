use dl_db::Db;

#[tokio::test]
async fn bootstrap_schema_creates_scrim_tables() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let db = Db::open_creating(dir.path().join("scrim_schema.sqlite3"))?;

    db.bootstrap_schema().await?;

    let tables: Vec<String> = db
        .read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name
                   FROM sqlite_master
                  WHERE type = 'table'
                    AND name LIKE 'scrim_%'
                  ORDER BY name",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .await?;

    assert_eq!(
        tables,
        vec![
            "scrim_match".to_string(),
            "scrim_participant".to_string(),
            "scrim_team".to_string(),
            "scrim_team_member".to_string(),
        ]
    );

    Ok(())
}
