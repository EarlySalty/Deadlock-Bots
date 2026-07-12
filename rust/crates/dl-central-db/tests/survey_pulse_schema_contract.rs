use std::path::Path;

fn migration() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/2026071302_survey_pulse.sql"),
    )
    .expect("read survey pulse migration")
}

#[test]
fn migration_defines_survey_tables_and_sql_summary() {
    let sql = migration();

    for contract in [
        "CREATE TABLE bot.survey_waves",
        "config_snapshot JSONB NOT NULL",
        "CREATE TABLE bot.survey_responses",
        "satisfaction SMALLINT",
        "events TEXT[]",
        "freitext TEXT",
        "PRIMARY KEY (wave_id, user_id)",
        "CREATE VIEW bot.survey_wave_summary",
        "AVG(responses.satisfaction)",
        "jsonb_object_agg",
    ] {
        assert!(
            sql.contains(contract),
            "missing survey contract: {contract}"
        );
    }
}
