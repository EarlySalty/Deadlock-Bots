//! dl-bot — der künftige Discord-Prozess.
//!
//! Phase-2-Stand: Master-Broker (:8770) und Changelog-Empfänger (:8899)
//! sind voll implementiert (REST-Aktionen brauchen kein Gateway, nur den
//! Bot-Token). Das Gateway selbst ist user-gated (DL_BOT_GATEWAY=1) —
//! bis zum koordinierten Cutover hält der Python-Bot die Discord-Session,
//! deshalb sind die Standard-Ports hier erst nach Freigabe zu übernehmen.

mod modglue;
mod onboardglue;

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
            // AI-Scoring: MiniMax wenn konfiguriert, sonst Heuristik (NoAi)
            let scorer: Arc<dyn dl_bridges::matcher::AiScorer> =
                match dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok()) {
                    Some(generator) => Arc::new(dl_ai::MatcherScorer { generator }),
                    None => Arc::new(dl_bridges::matcher::NoAi),
                };
            let matcher = dl_bridges::matcher::Matcher::new(
                matcher_config,
                twitch_client.clone(),
                glue.clone(),
                glue,
                scorer,
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

    // Streamer-Partner-Verknüpfung (vereinfacht): /streamer verweist auf die
    // Website und merkt sich die Discord-ID im 1-h-Fenster. Der Korrelations-
    // Watcher (mappt neu auftauchende Streamer auf die Absicht) folgt.
    let streamer_intents = dl_bridges::streamer_intent::StreamerIntents::new(db.clone());
    if let Err(err) = streamer_intents.ensure_schema().await {
        tracing::warn!(%err, "streamer_link_intents-Schema konnte nicht angelegt werden");
    }
    dl_bridges::streamer_intent::register(&mut router, streamer_intents.clone());

    // Steam-Link-Nudge (4c) — Close-Button braucht den Router, Spawn ist gateway-gated
    let nudge = dl_voice::nudge::VoiceNudge::new(
        db.clone(),
        Arc::new(dl_voice::glue::NudgeGlue {
            adapter: adapter.clone(),
            steam: dl_bridges::steam::SteamBotClient::from_env(|k| std::env::var(k).ok()),
            log_channel_id: dl_voice::nudge::LOG_CHANNEL_ID,
        }),
    );
    dl_voice::nudge::register(&mut router, nudge.clone());

    // TempVoice-Engine (4b/4c) — Panel-Buttons brauchen den Router,
    // der Event-Subscriber startet erst mit dem Gateway
    let cache_snapshot = Arc::new(dl_voice::glue::CacheSnapshot {
        adapter: adapter.clone(),
    });
    let tempvoice = dl_voice::tempvoice::TempVoiceEngine::new(
        dl_voice::tempvoice::TempVoiceConfig::production(),
        dl_voice::tempvoice::TempVoiceStore::new(db.clone()),
        cache_snapshot.clone(),
    );
    dl_voice::tempvoice::interface::register(&mut router, tempvoice.clone());

    // Aktivitäts-Analyzer (5) — auch Co-Spieler-Quelle für den Router
    let activity = dl_activity::analyzer::ActivityAnalyzer::new(
        db.clone(),
        Arc::new(dl_activity::glue::CacheVoiceGroups {
            adapter: adapter.clone(),
        }),
    );

    // Lane-Router (4c-Rest) — Panel-Buttons brauchen den Interaction-Router
    let lane_router = dl_voice::router::LaneRouter::new(
        db.clone(),
        Arc::new(dl_voice::glue::RouterGlue {
            adapter: adapter.clone(),
        }),
        tempvoice.clone(),
        Some(activity.clone()),
    );
    dl_voice::router::register(&mut router, lane_router.clone());

    // Voice-Feedback-DMs (4a-Rest) — Button/Modal brauchen den Router
    let voice_feedback = dl_voice::feedback::VoiceFeedback::new(
        db.clone(),
        Arc::new(dl_voice::glue::FeedbackGlue {
            adapter: adapter.clone(),
        }),
    );
    dl_voice::feedback::register(&mut router, voice_feedback.clone());

    // Tag-System (6/7): Single Source of Truth, von TempVoice-Filtern genutzt
    let tag_service = dl_community::tags::TagService::new(db.clone());
    // /meine-tags-Selbstverwaltung (Slash + Select/Reset-Komponenten).
    dl_community::tags_ui::register(&mut router, tag_service.clone());

    // SecurityGuard (6): sg:*-Mod-Buttons am Router, Scan gateway-gated
    router.on_prefix(
        "sg:",
        Arc::new(modglue::GuardReviewHandler {
            adapter: adapter.clone(),
        }),
    );
    let security_guard = dl_moderation::guard::SecurityGuard::new(
        db.clone(),
        dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok())
            .map(|c| c as Arc<dyn dl_ai::TextGenerator>),
        Arc::new(modglue::GuardGlue {
            adapter: adapter.clone(),
        }),
    );

    // Onboarding-Wizard (7): rp:panel:start + Thread-Schritte
    let wizard =
        dl_community::onboarding::OnboardingWizard::new(Arc::new(onboardglue::WizardGlue {
            adapter: adapter.clone(),
            tags: tag_service.clone(),
            steam: steam_client.clone(),
            db: db.clone(),
        }));
    // Verifikations-Abschluss: RoleEvent::Gained(Verified) → Abschluss-Nachricht.
    dl_community::onboarding::spawn_verify_completion(wizard.clone(), &dispatcher);
    dl_community::onboarding::register(&mut router, wizard);

    // Privacy-Oberflaeche: /datenschutz + /datenschutz-optin (Loeschung/Opt-in).
    dl_community::privacy_ui::register(&mut router, db.clone());

    // Turnier-User-Flow (8): Panel-Buttons + Solo/Team-Anmeldung
    struct TurnierRoleGlue {
        adapter: Arc<dl_discord::DiscordAdapter>,
    }
    #[async_trait::async_trait]
    impl dl_tournament::discord_ui::TurnierPort for TurnierRoleGlue {
        async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
            self.adapter
                .cache
                .guild(serenity::all::GuildId::new(guild_id))
                .and_then(|g| {
                    g.members
                        .get(&serenity::all::UserId::new(user_id))
                        .map(|m| m.roles.iter().map(|r| r.get()).collect())
                })
                .unwrap_or_default()
        }
    }
    let turnier_ui = Arc::new(dl_tournament::discord_ui::TurnierUi {
        store: Arc::new(dl_tournament::store::TournamentStore::new(db.clone())),
        port: Arc::new(TurnierRoleGlue {
            adapter: adapter.clone(),
        }),
    });
    dl_tournament::discord_ui::register(&mut router, turnier_ui);

    // Coaching-Anfragen (7): Panel/Claim/Release/Cancel + AI-Analyse-Loops
    let coaching_requests = dl_community::coaching_requests::CoachingRequests::new(
        db.clone(),
        Arc::new(modglue::CoachingReqGlue {
            adapter: adapter.clone(),
        }),
        dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok())
            .map(|client| client as Arc<dyn dl_ai::TextGenerator>),
        1289721245281292288,
    );
    dl_community::coaching_requests::register(&mut router, coaching_requests.clone());

    // FAQ-Chat (6) — Panel-Buttons brauchen den Router, Subscriber gateway-gated
    let faq_docs_path = std::env::var("FAQ_DOCS_PATH").unwrap_or_else(|_| "docs".to_string());
    let faq = dl_community::faq::FaqChat::new(
        db.clone(),
        Arc::new(modglue::FaqGlue {
            adapter: adapter.clone(),
        }),
        dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok())
            .map(|client| client as Arc<dyn dl_ai::TextGenerator>),
        dl_community::faq::load_docs(std::path::Path::new(&faq_docs_path)),
    );
    dl_community::faq::register(&mut router, faq.clone());

    // Anonymes Feedback (6) — Button + Modal; DM an den Empfänger.
    // !fhub-Panel-Post folgt mit der Prefix-Dispatch-Infra; persistente
    // custom_ids halten ein bereits gepostetes Panel über den Cutover hinweg.
    let feedback_hub = Arc::new(dl_community::feedback_hub::FeedbackHub {
        port: Arc::new(modglue::FeedbackGlue {
            adapter: adapter.clone(),
        }),
        db: db.clone(),
    });
    dl_community::feedback_hub::register(&mut router, feedback_hub.clone());

    // Clip-Einsendungen (6) — Button/Modal brauchen den Router, Loops gateway-gated
    let clips = dl_community::clips::ClipSubmission::new(
        db.clone(),
        Arc::new(modglue::ClipGlue {
            adapter: adapter.clone(),
        }),
    );
    dl_community::clips::register(&mut router, clips.clone());

    // Leave-Survey (6) — Select/Modal brauchen den Router, Trigger ist gateway-gated
    let leave_survey = dl_community::leave_survey::LeaveSurvey::new(
        db.clone(),
        Arc::new(modglue::SurveyGlue {
            adapter: adapter.clone(),
        }),
        "Deutsche Deadlock Community",
    );
    dl_community::leave_survey::register(&mut router, leave_survey.clone());

    // Retention-Miss-You: Feedback-Button + -Modal der „Wir-vermissen-dich"-DM.
    // Der Port versorgt sowohl die Buttons als auch den gateway-gated Loop.
    let retention_port: Arc<dyn dl_community::retention::RetentionPort> =
        Arc::new(modglue::RetentionGlue {
            adapter: adapter.clone(),
        });
    dl_community::retention::register(&mut router, db.clone(), retention_port.clone());

    // Onboarding-Buttons (7): Regelbestätigung + Steam-Login + DM-Hinweise
    onboardglue::register(
        &mut router,
        adapter.clone(),
        dl_bridges::steam::SteamBotClient::from_env(|k| std::env::var(k).ok()),
    );

    // AI-Moderator (6) — Review-Buttons brauchen den Router, Scan ist gateway-gated
    let moderator = dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok()).map(|generator| {
        let moderator = dl_moderation::AiModerator::new(
            db.clone(),
            generator,
            Arc::new(modglue::ModGlue {
                adapter: adapter.clone(),
            }),
        );
        router.on_prefix(
            "aimod:",
            Arc::new(modglue::ReviewHandler {
                moderator: moderator.clone(),
            }),
        );
        moderator
    });

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
            // Streamer-Intent-Watcher: korreliert neu auftauchende Streamer mit
            // offenen /streamer-Absichten (1-h-Fenster) und verknüpft sie.
            dl_bridges::streamer_intent::spawn_watcher(
                streamer_intents.clone(),
                matcher.client.clone(),
            );
        }
        // Rename-Queue (Port rename_manager): zentrale, rate-limit-bewusste
        // Channel-Umbenennung. init() VOR den Voice-Subscribern, damit deren
        // Rename-Wuensche eingereiht statt direkt ausgefuehrt werden; EIN Worker
        // drainiert FIFO mit >=360s Abstand pro Channel.
        dl_voice::rename_queue::init(db.clone());
        dl_voice::rename_queue::spawn_worker(
            db.clone(),
            Arc::new(dl_voice::glue::RenameExecGlue {
                adapter: adapter.clone(),
            }),
        );

        // Voice-Session-Tracker (4a): Subscriber + Wartungs-Loops
        let voice_tracker =
            dl_voice::tracker::VoiceTracker::new(db.clone(), cache_snapshot.clone());
        voice_tracker.set_feedback(voice_feedback.clone()).await;
        dl_voice::tracker::spawn(voice_tracker, &dispatcher);

        // TempVoice-Engine (4b): Join-to-create + Owner-Lifecycle
        dl_voice::tempvoice::engine::spawn(tempvoice.clone(), &dispatcher);
        // Tag-Filter: Dienst anbinden + Ragebaiter-Sofort-Durchsetzung
        tag_service.rehydrate().await;
        tempvoice.set_tag_service(tag_service.clone()).await;
        dl_community::tags::spawn_cleanup(tag_service.clone());
        dl_voice::tempvoice::engine::spawn_tag_listener(tempvoice.clone(), tag_service.clone());

        // Rank-Voice-Manager (4c): Anker + Rang-Rechte auf Comp-Lanes
        let rank_manager = dl_voice::rank::RankVoiceManager::new(
            db.clone(),
            Arc::new(dl_voice::glue::RankGlue {
                adapter: adapter.clone(),
            }),
            Arc::new({
                let tempvoice = tempvoice.clone();
                move |channel_id| tempvoice.initial_owner_blocking(channel_id)
            }),
        );
        dl_voice::rank::spawn(rank_manager, &dispatcher);

        // Steam-Link-Nudge (4c): DM nach 30 min Voice am zweiten Tag
        dl_voice::nudge::spawn(nudge.clone(), &dispatcher);

        // SecurityGuard (6): Message-Subscriber (Takeover/Burst/Keyword)
        if let Err(err) = security_guard.ensure_schema().await {
            tracing::warn!(%err, "SecurityGuard: Schema-Anlage fehlgeschlagen");
        }
        dl_moderation::guard::spawn(security_guard.clone(), &dispatcher);

        // Player-Finder (5): portiert, aber per Flag deaktiviert (Redesign geplant)
        if dl_activity::player_finder::enabled(|k| std::env::var(k).ok()) {
            let _finder = dl_activity::player_finder::PlayerFinder::new(db.clone());
            tracing::warn!(
                "PLAYER_FINDER_ENABLED=1 gesetzt — Kern portiert, Message-Flow folgt mit dem Redesign"
            );
        } else {
            tracing::info!("Player-Finder deaktiviert (PLAYER_FINDER_ENABLED nicht gesetzt)");
        }

        // Website-Invites (5): permanente Codes je Unterseite sicherstellen
        let website_invites = dl_community::invites::WebsiteInvites {
            store: dl_community::invites::InviteStore { db: db.clone() },
            port: Arc::new(modglue::InviteGlue {
                adapter: adapter.clone(),
            }),
            guild_id: 1289721245281292288,
            welcome_channel_id: dl_community::invites::DEFAULT_WELCOME_CHANNEL_ID,
        };
        tokio::spawn(async move {
            // kurz warten bis das Gateway steht (Invite-Liste braucht REST)
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            website_invites.ensure_invites().await;
        });

        // Coaching-Plattform-Brücke (7): Rollen-Sync 10min + Termin-DMs 60s
        match dl_community::coaching::WebsiteClient::from_env(|k| std::env::var(k).ok()) {
            Some(client) => {
                let sync = Arc::new(dl_community::coaching::CoachingSync {
                    client,
                    port: Arc::new(modglue::CoachingGlue {
                        adapter: adapter.clone(),
                        guild_id: 1289721245281292288,
                    }),
                });
                dl_community::coaching::spawn(sync);
            }
            None => tracing::info!("Coaching-Sync inaktiv (kein interner Token)"),
        }

        // AI-Moderator (6): Scan-Kanal-Subscriber
        if let Some(moderator) = &moderator {
            if let Err(err) = moderator.store.ensure_schema().await {
                tracing::warn!(%err, "Moderation: Schema-Anlage fehlgeschlagen");
            }
            dl_moderation::spawn(moderator.clone(), &dispatcher);
        } else {
            tracing::info!("AI-Moderator inaktiv (kein MiniMax-Key)");
        }

        // Aktivitäts-Analyzer (5): Loops starten (Instanz oben gebaut)
        dl_activity::analyzer::spawn(activity.clone());
        dl_activity::analyzer::spawn_member_events(db.clone(), &dispatcher);
        dl_activity::analyzer::spawn_message_activity(db.clone(), &dispatcher);
        // Text-Gamification (5): Konversations-Punkte → text_stats (speist das
        // öffentliche Text-Leaderboard) + 60-s-Flush-Loop.
        let text_sessions = Arc::new(dl_activity::text_stats::TextSessions::new(db.clone()));
        if let Err(err) = text_sessions.ensure_schema().await {
            tracing::warn!(%err, "text_stats-Schema konnte nicht angelegt werden");
        }
        dl_activity::text_stats::spawn_text_stats(text_sessions, &dispatcher);
        // Retention-Tracking (Daten-Layer): Voice-Join → user_retention_tracking
        // + 30-min avg_weekly_sessions-Sync (Quelle der Leave-Survey-Einstufung)
        // + stündlicher Miss-You-Check (Embed-DM an inaktive Stamm-User).
        dl_community::retention::spawn(
            dl_community::retention::RetentionTracker::new(db.clone()),
            retention_port.clone(),
            &dispatcher,
        );
        dl_community::leave_survey::spawn(leave_survey.clone(), &dispatcher);
        dl_community::clips::spawn(clips.clone());
        dl_community::faq::spawn(faq.clone(), &dispatcher);
        dl_community::coaching_requests::spawn(coaching_requests.clone(), &dispatcher);
        // !fhub-Panel-Listener (Admin postet/editiert das Feedback-Panel)
        dl_community::feedback_hub::spawn(feedback_hub.clone(), &dispatcher);

        // LFG-Lobby-Finder (5): Antworten im Suche-Kanal
        let lfg_responder = dl_activity::lfg::LfgResponder::new(
            db.clone(),
            Arc::new(modglue::LfgGlue {
                adapter: adapter.clone(),
            }),
        );
        dl_activity::lfg::spawn_responder(lfg_responder, &dispatcher);

        // Lane-Router (4c-Rest): Join auf den Router-VC einsortieren
        dl_voice::router::spawn(lane_router.clone(), &dispatcher);

        // Adaptive Spezial-Lanes: Anfänger-Routing + Duo + Sortierung
        let adaptive = dl_voice::adaptive::AdaptiveLanes::new(cache_snapshot.clone());
        tempvoice.set_adaptive(adaptive.clone()).await;
        dl_voice::adaptive::spawn(adaptive, &dispatcher);

        // Voice-Status-Worker (4c): LiveMatch-Suffixe an Lane-Namen
        let status_worker = dl_voice::status::VoiceStatusWorker::new(
            db.clone(),
            Arc::new(dl_voice::glue::StatusGlue {
                adapter: adapter.clone(),
            }),
        );
        dl_voice::status::spawn(status_worker);
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
