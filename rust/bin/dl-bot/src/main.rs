//! dl-bot — wird der Discord-Gateway-Prozess (plus Master-Broker :8770 und
//! Changelog-Empfänger :8899). Phase-0-Stand: Gerüst, das Konfiguration und
//! DB-Vertrag verifiziert. Kein Go-Live, keine Discord-Verbindung.

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
        broker_port = cfg.ports.master_broker,
        "dl-bot-Gerüst läuft (Phase 0) — noch kein Go-Live, beenden mit Ctrl+C"
    );

    tokio::signal::ctrl_c().await.context("Signal-Handler")?;
    tracing::info!("dl-bot beendet");
    Ok(())
}
