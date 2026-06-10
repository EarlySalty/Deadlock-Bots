//! dl-bot — der künftige Discord-Prozess.
//!
//! Phase-2-Stand: Master-Broker (:8770) und Changelog-Empfänger (:8899)
//! sind voll implementiert (REST-Aktionen brauchen kein Gateway, nur den
//! Bot-Token). Das Gateway selbst ist user-gated (DL_BOT_GATEWAY=1) —
//! bis zum koordinierten Cutover hält der Python-Bot die Discord-Session,
//! deshalb sind die Standard-Ports hier erst nach Freigabe zu übernehmen.

use std::sync::Arc;

use anyhow::Context;
use dl_webcore::WebConfig;

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dl_core::observability::init_tracing("info");

    let cfg = dl_core::Config::from_env().context("Konfiguration laden")?;
    let _web_cfg = WebConfig::from_env();
    let db = dl_db::Db::open(&cfg.db_path)
        .with_context(|| format!("gemeinsame DB öffnen: {}", cfg.db_path.display()))?;
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
    tracing::info!(db = %cfg.db_path.display(), tabellen = tables, "DB-Vertrag ok");

    // Discord-Adapter (REST sofort, Cache erst mit Gateway)
    let Some(discord_token) = env("DISCORD_TOKEN") else {
        anyhow::bail!("DISCORD_TOKEN fehlt — dl-bot kann ohne Bot-Token nichts ausrichten");
    };
    let adapter = dl_discord::DiscordAdapter::new(&discord_token);
    let dispatcher = Arc::new(dl_discord::Dispatcher::new());

    // Interaction-Routing: Steam-Bridge + Twitch-Live-Bridge
    let mut router = dl_discord::InteractionRouter::new();
    let steam_client = dl_bridges::steam::SteamBotClient::from_env(|k| std::env::var(k).ok());
    dl_bridges::steam::register(&mut router, steam_client.clone());
    let twitch_registry = dl_bridges::twitch::TrackingRegistry::new();
    let twitch_client = dl_bridges::twitch::TwitchApiClient::from_env(|k| std::env::var(k).ok());
    let matcher = match &twitch_client {
        Some(twitch_client) => {
            dl_bridges::twitch::register(
                &mut router,
                twitch_client.clone(),
                twitch_registry.clone(),
            );
            // Streamer-Link-Matcher (Review-Buttons immer registrieren —
            // offene Vorschläge überleben Neustarts über den State-File)
            let matcher_config =
                dl_bridges::matcher::MatcherConfig::from_env(|k| std::env::var(k).ok());
            let glue = Arc::new(dl_bridges::glue::AdapterGlue {
                adapter: adapter.clone(),
                notify_channel_id: matcher_config.notify_channel_id,
            });
            let matcher = dl_bridges::matcher::Matcher::new(
                matcher_config,
                twitch_client.clone(),
                glue.clone(),
                glue,
                Arc::new(dl_bridges::matcher::NoAi),
            );
            dl_bridges::matcher::register(&mut router, matcher.clone());
            Some(matcher)
        }
        None => {
            tracing::warn!(
                "TWITCH_INTERNAL_API_TOKEN fehlt — Twitch-Live-Bridge + Matcher inaktiv"
            );
            None
        }
    };
    let router = Arc::new(router);

    // Listener: member_remove → Steam-Bot, !steam_*-Admin-Kommandos
    let _member_listener =
        dl_bridges::steam::spawn_member_remove_listener(&dispatcher, steam_client.clone());
    let _admin_listener =
        dl_bridges::steam::spawn_admin_command_listener(&dispatcher, steam_client, adapter.clone());

    // Master-Broker :8770 — Token-Kette wie das Original
    let broker_token = env("MASTER_BROKER_TOKEN")
        .or_else(|| env("MAIN_BOT_INTERNAL_TOKEN"))
        .or_else(|| env("TWITCH_INTERNAL_API_TOKEN"))
        .context("Broker-Token fehlt (MASTER_BROKER_TOKEN/MAIN_BOT_INTERNAL_TOKEN/TWITCH_INTERNAL_API_TOKEN)")?;
    let broker =
        dl_broker::BrokerState::new(adapter.clone(), broker_token, |key| std::env::var(key).ok())
            .map_err(|e| anyhow::anyhow!(e))?;
    let broker_host = env("MASTER_BROKER_HOST").unwrap_or_else(|| "127.0.0.1".to_string());
    let broker_addr = format!("{broker_host}:{}", cfg.ports.master_broker);
    let broker_listener = tokio::net::TcpListener::bind(&broker_addr)
        .await
        .with_context(|| format!("Broker-Port binden: {broker_addr}"))?;
    tracing::info!(addr = %broker_addr, "Master-Broker gebunden");
    let broker_server = axum::serve(
        broker_listener,
        dl_broker::router(broker).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );

    // Changelog-Empfänger :8899
    let changelog = dl_changelog::ChangelogState::new(adapter.clone(), env("CHANGELOG_API_TOKEN"));
    let changelog_addr = format!("127.0.0.1:{}", cfg.ports.changelog_api);
    let changelog_listener = tokio::net::TcpListener::bind(&changelog_addr)
        .await
        .with_context(|| format!("Changelog-Port binden: {changelog_addr}"))?;
    tracing::info!(addr = %changelog_addr, "Changelog-Empfänger gebunden");
    let changelog_server = axum::serve(
        changelog_listener,
        dl_changelog::router(changelog)
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );

    // Gateway: user-gated — Python hält die Session bis zum Cutover
    let gateway_enabled = env("DL_BOT_GATEWAY").as_deref() == Some("1");
    let gateway_task = if gateway_enabled {
        // Aktive Twitch-Live-Ankündigungen rehydrieren (Klick-Routing)
        if let Some(twitch_client) = &twitch_client {
            dl_bridges::twitch::spawn_restore(twitch_client.clone(), twitch_registry.clone());
        }
        // Streamer-Link-Matcher: 6h-Scan + Admin-Kommandos (brauchen Gateway-Cache)
        if let Some(matcher) = &matcher {
            dl_bridges::matcher::spawn_scan_loop(matcher.clone());
            dl_bridges::matcher::spawn_command_listener(&dispatcher, matcher.clone());
        }
        // Voice-Session-Tracker (4a): Subscriber + Wartungs-Loops
        let voice_tracker = dl_voice::tracker::VoiceTracker::new(
            db.clone(),
            Arc::new(dl_voice::glue::CacheSnapshot {
                adapter: adapter.clone(),
            }),
        );
        dl_voice::tracker::spawn(voice_tracker, &dispatcher);
        // Slash-Commands syncen (optional, wie Pythons COMMAND_SYNC_ON_START)
        if env("DL_BOT_COMMAND_SYNC").as_deref() == Some("1") {
            let guild_id = env("DL_BOT_COMMAND_GUILD_ID").and_then(|v| v.parse::<u64>().ok());
            match dl_discord::dispatch::sync_commands(&adapter.http, &router, guild_id).await {
                Ok(count) => tracing::info!(count, ?guild_id, "Slash-Commands synchronisiert"),
                Err(err) => tracing::error!(%err, "Slash-Command-Sync fehlgeschlagen"),
            }
        }
        let mut client = dl_discord::gateway::build_client(
            &discord_token,
            adapter.clone(),
            dispatcher.clone(),
            router.clone(),
        )
        .await
        .context("Gateway-Client bauen")?;
        tracing::warn!(
            "Gateway AKTIV — sicherstellen, dass der Python-Bot die Events abgegeben hat"
        );
        Some(tokio::spawn(async move { client.start().await }))
    } else {
        tracing::info!("Gateway inaktiv (DL_BOT_GATEWAY != 1) — nur REST/Broker/Changelog");
        None
    };

    tracing::info!("dl-bot läuft — beenden mit Ctrl+C");
    tokio::select! {
        result = broker_server => result.context("Broker-Server")?,
        result = changelog_server => result.context("Changelog-Server")?,
        _ = tokio::signal::ctrl_c() => tracing::info!("dl-bot beendet"),
    }
    if let Some(task) = gateway_task {
        task.abort();
    }
    Ok(())
}
