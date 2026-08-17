//! Holt Discord Server-Insights als CSV (Portal-API) und spielt sie in
//! `activity.insights_imports` ein.
//!
//! Env:
//! - `DISCORD_INSIGHTS_USER_TOKEN` (Pflicht) — User-Token mit Recht
//!   „Server-Einblicke ansehen“. Kein Bot-Token.
//! - `DISCORD_INSIGHTS_GUILD_ID` (Default Community-Guild).
//! - `DEADLOCK_CENTRAL_DSN` (Pflicht).
//! - `INSIGHTS_ARCHIVE_DIR` (Default `docs/insights/discord/YYYY-MM-DD`).

use dl_dashboard::insights_sync::{archive_readme, sync_insights, SyncConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let dsn = dl_central_db::dsn_from_env()?;
    let pool = dl_central_db::connect_pool(&dsn).await?;
    let report = if let Ok(dir) = std::env::var("INSIGHTS_IMPORT_DIR") {
        let guild_id = std::env::var("DISCORD_INSIGHTS_GUILD_ID")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(dl_dashboard::insights_sync::DEFAULT_GUILD_ID);
        dl_dashboard::insights_sync::import_csv_dir(&pool, guild_id, dir.as_ref())
            .await
            .map_err(|err| anyhow::anyhow!(err))?
    } else {
        let cfg = SyncConfig::from_env().map_err(|err| anyhow::anyhow!(err))?;
        let report = sync_insights(&pool, &cfg)
            .await
            .map_err(|err| anyhow::anyhow!(err))?;
        let readme = cfg.archive_dir.join("README.md");
        if !readme.exists() {
            std::fs::write(&readme, archive_readme(&cfg.archive_dir))?;
        }
        report
    };
    println!(
        "insights-sync: {} Dateien, {} Zeilen nach {}",
        report.files,
        report.rows,
        report.archive_dir.display()
    );
    for skip in &report.skipped {
        println!("übersprungen: {skip}");
    }
    if report.files == 0 {
        anyhow::bail!("keine Datei importiert");
    }
    Ok(())
}
