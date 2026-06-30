use anyhow::Context;

const SEED_JSON: &str = include_str!("../../../../docs/specs/seed-roster-2026-06-29.json");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dl_core::observability::init_tracing("info");

    let central_dsn = dl_central_db::dsn_from_env().context("load central DB DSN")?;
    let pool = dl_central_db::connect_pool(&central_dsn)
        .await
        .context("connect central DB")?;

    let summary = dl_squads::seed::import_seed_roster_json(&pool, SEED_JSON)
        .await
        .context("import scrim seed")?;

    println!(
        "ok players={} pool_unassigned={} teams={} matches={}",
        summary.players, summary.pool_unassigned, summary.teams, summary.matches
    );
    Ok(())
}
