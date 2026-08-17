//! Spielt Discord Server-Insights in `activity.insights_imports` ein.
//!
//! Reihenfolge:
//! 1. `INSIGHTS_IMPORT_DIR` — fertige CSVs
//! 2. `INSIGHTS_BRAVE_DUMP_DIR` — Highcharts-Dumps aus der Brave-Sitzung
//! 3. sonst Portal-API mit `DISCORD_INSIGHTS_USER_TOKEN` (optional, nicht empfohlen)
//!
//! Env: `DEADLOCK_CENTRAL_DSN`, `DISCORD_INSIGHTS_GUILD_ID`,
//! `INSIGHTS_ARCHIVE_DIR`.

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
    let guild_id = std::env::var("DISCORD_INSIGHTS_GUILD_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(dl_dashboard::insights_sync::DEFAULT_GUILD_ID);
    let report = if let Ok(dir) = std::env::var("INSIGHTS_IMPORT_DIR") {
        dl_dashboard::insights_sync::import_csv_dir(&pool, guild_id, dir.as_ref())
            .await
            .map_err(|err| anyhow::anyhow!(err))?
    } else if let Ok(dump_dir) = std::env::var("INSIGHTS_BRAVE_DUMP_DIR") {
        let archive = std::env::var("INSIGHTS_ARCHIVE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::path::PathBuf::from(format!(
                    "/home/nathanael/repos/Deadlock-Bots/docs/insights/discord/{}",
                    chrono::Utc::now().date_naive()
                ))
            });
        let mut dumps = Vec::new();
        for entry in std::fs::read_dir(&dump_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                dumps.push(path);
            }
        }
        dumps.sort();
        dl_dashboard::insights_sync::brave_dumps_to_csv_dir(&dumps, &archive)
            .map_err(|err| anyhow::anyhow!(err))?;
        let readme = archive.join("README.md");
        if !readme.exists() {
            std::fs::write(&readme, archive_readme(&archive))?;
        }
        dl_dashboard::insights_sync::import_csv_dir(&pool, guild_id, &archive)
            .await
            .map_err(|err| anyhow::anyhow!(err))?
    } else if std::env::var("DISCORD_INSIGHTS_USER_TOKEN").is_err() {
        println!("kein Brave-Dump und kein Token: nichts zu tun");
        return Ok(());
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
