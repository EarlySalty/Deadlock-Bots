use anyhow::Context;

const SEED_JSON: &str = include_str!("../../../../docs/specs/seed-roster-2026-06-29.json");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dl_core::observability::init_tracing("info");

    let cfg = dl_core::Config::from_env().context("load config")?;
    let db = dl_db::Db::open(&cfg.db_path)
        .with_context(|| format!("open shared DB: {}", cfg.db_path.display()))?;
    db.bootstrap_schema().await.context("bootstrap schema")?;

    let summary = dl_squads::seed::import_seed_roster_json(&db, SEED_JSON)
        .await
        .context("import scrim seed")?;

    println!(
        "ok players={} pool_unassigned={} teams={} matches={}",
        summary.players, summary.pool_unassigned, summary.teams, summary.matches
    );
    Ok(())
}
