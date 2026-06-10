//! dl-web — wird der Web-Prozess für Public-Stats :8768, Tierlist :8771,
//! Turnier :8767 und Admin-Dashboard :8766. Phase-0-Stand: Gerüst, das
//! Konfiguration und DB-Vertrag verifiziert. Es wird noch kein Port gebunden.

use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dl_core::observability::init_tracing("info");

    let cfg = dl_core::Config::from_env().context("Konfiguration laden")?;
    let db = dl_db::Db::open(&cfg.db_path)
        .with_context(|| format!("gemeinsame DB öffnen: {}", cfg.db_path.display()))?;

    // Smoke-Check gegen den DB-Vertrag: rein lesend.
    let tables: i64 = db
        .read(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get(0),
            )
        })
        .await
        .context("DB-Smoke-Check")?;

    tracing::info!(
        db = %cfg.db_path.display(),
        tabellen = tables,
        public_stats = cfg.ports.public_stats,
        tierlist = cfg.ports.tierlist_public,
        turnier = cfg.ports.turnier_public,
        dashboard = cfg.ports.dashboard,
        "dl-web-Gerüst läuft (Phase 0) — es wird noch kein Port gebunden, beenden mit Ctrl+C"
    );

    tokio::signal::ctrl_c().await.context("Signal-Handler")?;
    tracing::info!("dl-web beendet");
    Ok(())
}
