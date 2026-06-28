//! dl-web — Web-Prozess für die öffentlichen Dienste.
//!
//! Phase-1-Stand: Tierlist (:8771) ist vollständig portiert; Public-Stats
//! und Dashboard folgen. Es bindet nur, was implementiert ist —
//! der Go-Live passiert über die Port-ENVs (Test: abweichende Ports setzen,
//! Cutover: Python-Pendant deaktivieren und Original-Ports übernehmen).

use anyhow::Context;
use dl_webcore::{DashboardClient, WebConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dl_core::observability::init_tracing("info");

    let cfg = dl_core::Config::from_env().context("Konfiguration laden")?;
    let web_cfg = WebConfig::from_env();
    let db = dl_db::Db::open(&cfg.db_path)
        .with_context(|| format!("gemeinsame DB öffnen: {}", cfg.db_path.display()))?;

    let dashboard = DashboardClient::new(
        web_cfg.dashboard_base.clone(),
        web_cfg.relay_token.clone(),
        web_cfg.twitch_token.clone(),
    );

    // Tierlist :8771
    let tierlist = dl_tierlist::TierlistApp::new(
        db.clone(),
        dashboard.clone(),
        dl_tierlist::DEADLOCK_API_BASE,
    );
    let tierlist_addr = format!("{}:{}", web_cfg.tierlist_host, cfg.ports.tierlist_public);
    let tierlist_listener = tokio::net::TcpListener::bind(&tierlist_addr)
        .await
        .with_context(|| format!("Tierlist-Port binden: {tierlist_addr}"))?;
    tracing::info!(addr = %tierlist_addr, refresh = web_cfg.tierlist_refresh_enabled, "Tierlist gebunden");

    if web_cfg.tierlist_refresh_enabled {
        tokio::spawn(dl_tierlist::refresh_loop(tierlist.clone()));
    } else {
        tracing::warn!(
            "Tierlist-Refresh-Loop DEAKTIVIERT (DL_TIERLIST_REFRESH=0) — nur Lesen/Votes"
        );
    }

    let tierlist_server = axum::serve(
        tierlist_listener,
        dl_tierlist::router(tierlist).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );

    // Public-Stats :8768
    let stats = dl_stats::StatsApp::new(db.clone(), dashboard.clone(), &web_cfg);
    let stats_addr = format!("{}:{}", web_cfg.stats_host, cfg.ports.public_stats);
    let stats_listener = tokio::net::TcpListener::bind(&stats_addr)
        .await
        .with_context(|| format!("Public-Stats-Port binden: {stats_addr}"))?;
    tracing::info!(addr = %stats_addr, "Public-Stats gebunden");
    let stats_server = axum::serve(stats_listener, dl_stats::router(stats));

    // Master-Dashboard :8766 — Auth-Provider (Phase 9a). Stats/Tierlist
    // delegieren ihre Anmeldung hierher. Die internen Routen sind loopback-only,
    // daher mit Connect-Info binden (Peer-Adresse).
    let dashboard_cfg = dl_dashboard::DashboardConfig::from_env();
    let dashboard = dl_dashboard::DashboardApp::from_config(dashboard_cfg, db.clone());
    let dashboard_host =
        std::env::var("DASHBOARD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let dashboard_addr = format!("{dashboard_host}:{}", cfg.ports.dashboard);
    let dashboard_listener = tokio::net::TcpListener::bind(&dashboard_addr)
        .await
        .with_context(|| format!("Dashboard-Port binden: {dashboard_addr}"))?;
    tracing::info!(addr = %dashboard_addr, "Master-Dashboard gebunden");
    let dashboard_server = axum::serve(
        dashboard_listener,
        dl_dashboard::router(dashboard)
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );

    tracing::info!("dl-web läuft — beenden mit Ctrl+C");
    tokio::select! {
        result = tierlist_server => result.context("Tierlist-Server")?,
        result = stats_server => result.context("Public-Stats-Server")?,
        result = dashboard_server => result.context("Dashboard-Server")?,
        _ = tokio::signal::ctrl_c() => tracing::info!("dl-web beendet"),
    }
    Ok(())
}
