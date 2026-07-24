fn lagebild_migration() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/2026072407_scrim_lagebilder.sql"),
    )
    .expect("scrim lagebild migration exists")
}

#[test]
fn migration_definiert_lagebild_snapshots_evidenzen_und_korrekturen() {
    let sql = lagebild_migration();

    for needle in [
        "CREATE TABLE IF NOT EXISTS scrim.lagebild_snapshots",
        "team_id INTEGER NOT NULL REFERENCES scrim.teams(id)",
        "lagebild_text TEXT NOT NULL",
        "data_summary JSONB NOT NULL DEFAULT '{}'::jsonb",
        "CREATE TABLE IF NOT EXISTS scrim.lagebild_evidences",
        "snapshot_id BIGINT NOT NULL REFERENCES scrim.lagebild_snapshots(id) ON DELETE CASCADE",
        "url TEXT",
        "CREATE TABLE IF NOT EXISTS scrim.lagebild_corrections",
        "role TEXT NOT NULL CHECK (role IN ('user', 'assistant'))",
        "message TEXT NOT NULL",
        "lagebild_snapshots_team_generated_idx",
        "lagebild_evidences_snapshot_idx",
        "lagebild_corrections_team_created_idx",
    ] {
        assert!(sql.contains(needle), "missing {needle}");
    }
}
