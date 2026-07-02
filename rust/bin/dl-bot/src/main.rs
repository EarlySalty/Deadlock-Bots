//! dl-bot — der künftige Discord-Prozess.
//!
//! Phase-2-Stand: Master-Broker (:8770) und Changelog-Empfänger (:8899)
//! sind voll implementiert (REST-Aktionen brauchen kein Gateway, nur den
//! Bot-Token). Das Gateway selbst ist user-gated (DL_BOT_GATEWAY=1) —
//! bis zum koordinierten Cutover hält der Python-Bot die Discord-Session,
//! deshalb sind die Standard-Ports hier erst nach Freigabe zu übernehmen.

mod build_publisher;
mod journeyglue;
mod master;
mod modglue;
mod onboardglue;
mod serversync;

use std::sync::Arc;

use anyhow::Context;
use dl_webcore::WebConfig;

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_bool_default(name: &str, default: bool) -> bool {
    env(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_u64_default(name: &str, default: u64) -> u64 {
    env(name)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize_default(name: &str, default: usize) -> usize {
    env(name)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn default_brain_bin() -> String {
    "/home/naniadm/Documents/Deadlock-Brain/rust/target/release/deadlock-brain".to_string()
}

fn default_brain_db() -> String {
    "/home/naniadm/Documents/Deadlock-Brain/data/deadlock_brain.sqlite3".to_string()
}

async fn wait_for_gateway_cache_ready(
    events: &mut tokio::sync::broadcast::Receiver<dl_discord::GatewayEvent>,
    guild_id: u64,
    label: &'static str,
) {
    loop {
        match events.recv().await {
            Ok(dl_discord::GatewayEvent::CacheReady { guild_ids }) => {
                if guild_ids.contains(&guild_id) {
                    tracing::info!(label, guild_id, "Gateway-Cache bereit");
                    return;
                }
            }
            Ok(dl_discord::GatewayEvent::Ready { .. }) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

struct BrokerChannelInfoGlue {
    adapter: Arc<dl_discord::DiscordAdapter>,
}

fn cached_channel_info(channel: &serenity::all::GuildChannel) -> dl_broker::port::ChannelInfo {
    dl_broker::port::ChannelInfo {
        channel_id: channel.id.get(),
        name: channel.name.clone(),
        parent_id: channel.parent_id.map(|id| id.get()),
        last_message_id: channel.last_message_id.map(|id| id.get()),
    }
}

#[async_trait::async_trait]
impl dl_broker::ChannelInfoPort for BrokerChannelInfoGlue {
    async fn channel_info(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
    ) -> Result<dl_broker::port::ChannelInfo, dl_broker::PortError> {
        if channel_id == 0 {
            return Err(dl_broker::PortError::ChannelNotFound);
        }
        let cache = self.adapter.cache();
        let preferred_guild_id = match guild_id {
            Some(id) => serenity::all::GuildId::new(id),
            None => cache
                .guilds()
                .first()
                .copied()
                .ok_or(dl_broker::PortError::GuildNotFound)?,
        };
        {
            let Some(guild) = cache.guild(preferred_guild_id) else {
                return Err(dl_broker::PortError::GuildNotFound);
            };
            if let Some(channel) = guild
                .channels
                .get(&serenity::all::ChannelId::new(channel_id))
            {
                return Ok(cached_channel_info(channel));
            }
        }
        for candidate_guild_id in cache.guilds() {
            let Some(guild) = cache.guild(candidate_guild_id) else {
                continue;
            };
            if let Some(channel) = guild
                .channels
                .get(&serenity::all::ChannelId::new(channel_id))
            {
                return Ok(cached_channel_info(channel));
            }
        }
        Err(dl_broker::PortError::ChannelNotFound)
    }
}

const BRD10_CHANGELOG_COMMAND_DESC: &str = "Changelog-Eintrag in Discord posten";
const BRD10_CHANGELOG_POST_DESC: &str = "Changelog-Eintrag in Discord posten";
const BRD10_CHANGELOG_TITLE_OPTION_DESC: &str = "Überschrift des Eintrags";
const BRD10_CHANGELOG_CONTENT_OPTION_DESC: &str =
    "Inhalt als Markdown (z. B. '- Feature A\n- Bug gefixt')";
const BRD10_CHANGELOG_TARGET_OPTION_DESC: &str =
    "Zielkanal: 'all' für alle Bots, 'twitch' für den Twitch-Bot";
const BRD10_CHANGELOG_TARGET_ALL_CHOICE: &str = "Alle Bots (Dev Updates)";
const BRD10_CHANGELOG_TARGET_TWITCH_CHOICE: &str = "Twitch Bot";
const BRD10_CHANGELOG_NO_PERMISSION_MSG: &str = "Keine Berechtigung.";
const BRD10_CHANGELOG_POSTED_MSG: &str = "✅ Changelog gepostet.";
const BRD10_CHANGELOG_FAILED_MSG: &str = "❌ Changelog konnte nicht gepostet werden.";

fn changelog_command_spec() -> dl_discord::CommandSpec {
    dl_discord::CommandSpec {
        definition: serde_json::json!({
            "name": "changelog",
            "description": BRD10_CHANGELOG_COMMAND_DESC,
            "options": [{
                "type": 1,
                "name": "post",
                "description": BRD10_CHANGELOG_POST_DESC,
                "options": [
                    {
                        "type": 3,
                        "name": "title",
                        "description": BRD10_CHANGELOG_TITLE_OPTION_DESC,
                        "required": true
                    },
                    {
                        "type": 3,
                        "name": "content",
                        "description": BRD10_CHANGELOG_CONTENT_OPTION_DESC,
                        "required": true
                    },
                    {
                        "type": 3,
                        "name": "target",
                        "description": BRD10_CHANGELOG_TARGET_OPTION_DESC,
                        "required": false,
                        "choices": [
                            { "name": BRD10_CHANGELOG_TARGET_ALL_CHOICE, "value": "all" },
                            { "name": BRD10_CHANGELOG_TARGET_TWITCH_CHOICE, "value": "twitch" }
                        ]
                    }
                ]
            }],
        }),
    }
}

struct ChangelogPostCommand {
    state: dl_changelog::SharedChangelog,
}

#[async_trait::async_trait]
impl dl_discord::InteractionHandler for ChangelogPostCommand {
    async fn handle(&self, interaction: dl_discord::BridgeInteraction) -> dl_discord::BridgeReply {
        if !interaction.author_can_manage_guild {
            return dl_discord::BridgeReply::ephemeral_text(BRD10_CHANGELOG_NO_PERMISSION_MSG);
        }
        let get = |key: &str| {
            interaction
                .options
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let title = get("title");
        let content = get("content");
        let target = {
            let value = get("target");
            if value.is_empty() {
                "all".to_string()
            } else {
                value
            }
        };
        match dl_changelog::publish_changelog(&self.state, &title, &content, &target).await {
            Ok(channel_id) => {
                tracing::info!(channel_id, "Changelog per Slash-Command gepostet");
                dl_discord::BridgeReply::ephemeral_text(BRD10_CHANGELOG_POSTED_MSG)
            }
            Err(err) => {
                tracing::warn!(%err, "Changelog Slash-Command fehlgeschlagen");
                dl_discord::BridgeReply::ephemeral_text(BRD10_CHANGELOG_FAILED_MSG)
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<std::process::ExitCode> {
    dl_core::observability::init_tracing("info");
    let _pid_lock = master::PidLock::acquire_default().context("Single-Instance-PID-Lock")?;
    let startup_text = master::startup_text_now();

    let cfg = dl_core::Config::from_env().context("Konfiguration laden")?;
    let _web_cfg = WebConfig::from_env();
    let central_dsn = dl_central_db::dsn_from_env().context("zentrale DB-DSN laden")?;
    let central_pool = dl_central_db::connect_pool(&central_dsn)
        .await
        .context("zentrale DB verbinden")?;
    tracing::info!("Zentrale DB verbunden");

    // Discord-Adapter (REST sofort, Cache erst mit Gateway)
    let Some(discord_token) = env("DISCORD_TOKEN") else {
        anyhow::bail!("DISCORD_TOKEN fehlt — dl-bot kann ohne Bot-Token nichts ausrichten");
    };
    let adapter = dl_discord::DiscordAdapter::new(&discord_token);
    // Application-ID auf dem REST-Http-Client setzen — sonst scheitern
    // Interaction-Followups nach einem Defer (POST /webhooks/{app_id}/{token})
    // mit „Application id was expected but missing", und der User sieht nichts.
    if let Err(err) = adapter.init_application_id().await {
        tracing::error!(%err, "Application-ID nicht setzbar — Interaction-Followups schlagen fehl");
    }
    let changelog = dl_changelog::ChangelogState::new(adapter.clone(), env("CHANGELOG_API_TOKEN"));
    let owner_id = master::owner_id_from_lookup(env);
    if owner_id.is_none() {
        tracing::warn!("OWNER_ID fehlt — Owner-Commands bleiben gesperrt");
    }
    let serversync_service: serversync::SharedServerSync = serversync::ServerSyncService::new(
        central_pool.clone(),
        adapter.clone(),
        discord_token.clone(),
        serversync::GUILD_ID,
    );
    let dispatcher = Arc::new(dl_discord::Dispatcher::new());
    let reaction_roles = dl_community::reaction_roles::ReactionRoleService::new(
        central_pool.clone(),
        Arc::new(modglue::ReactionRoleGlue {
            adapter: adapter.clone(),
        }),
    );

    // Interaction-Routing: Steam-Bridge + Twitch-Live-Bridge
    let mut router = dl_discord::InteractionRouter::new();
    let steam_client = dl_bridges::steam::SteamBotClient::from_env(|k| std::env::var(k).ok());
    dl_bridges::steam::register_with_db(&mut router, steam_client.clone(), central_pool.clone());
    router.on_command(
        "changelog post",
        changelog_command_spec(),
        Arc::new(ChangelogPostCommand {
            state: changelog.clone(),
        }),
    );
    serversync::register_commands(&mut router, serversync_service.clone(), owner_id);
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
            // AI-Scoring: Provider wie Python via STREAMER_LINK_AI_PROVIDER.
            let scorer: Arc<dyn dl_bridges::matcher::AiScorer> =
                match matcher_config.ai_provider.as_str() {
                    "minimax" => match dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok()) {
                        Some(generator) => Arc::new(dl_ai::MatcherScorer { generator }),
                        None => Arc::new(dl_bridges::matcher::NoAi),
                    },
                    "openai" => match dl_ai::OpenAiClient::text_from_env(|k| std::env::var(k).ok())
                    {
                        Some(generator) => Arc::new(dl_ai::MatcherScorer { generator }),
                        None => Arc::new(dl_bridges::matcher::NoAi),
                    },
                    "gemini" => match dl_ai::GeminiClient::from_env(|k| std::env::var(k).ok()) {
                        Some(generator) => Arc::new(dl_ai::MatcherScorer { generator }),
                        None => Arc::new(dl_bridges::matcher::NoAi),
                    },
                    _ => Arc::new(dl_bridges::matcher::NoAi),
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
    let streamer_intents = dl_bridges::streamer_intent::StreamerIntents::new(central_pool.clone());
    if let Err(err) = streamer_intents.ensure_schema().await {
        tracing::warn!(%err, "streamer_link_intents-Schema konnte nicht angelegt werden");
    }
    dl_bridges::streamer_intent::register(&mut router, streamer_intents.clone());

    // Steam-Link-Nudge (4c) — Close-Button braucht den Router, Spawn ist gateway-gated
    let nudge = dl_voice::nudge::VoiceNudge::new(
        central_pool.clone(),
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
        dl_voice::tempvoice::TempVoiceStore::new(central_pool.clone()),
        cache_snapshot.clone(),
    );
    dl_voice::tempvoice::interface::register(&mut router, tempvoice.clone());
    let tempvoice_interface = dl_voice::tempvoice::interface::TempVoiceInterface::new(
        tempvoice.clone(),
        cache_snapshot.clone(),
    );

    // Aktivitäts-Analyzer (5) — auch Co-Spieler-Quelle für den Router
    let activity = dl_activity::analyzer::ActivityAnalyzer::new(
        central_pool.clone(),
        Arc::new(dl_activity::glue::CacheVoiceGroups {
            adapter: adapter.clone(),
        }),
    );

    // Lane-Router (4c-Rest) — Panel-Buttons brauchen den Interaction-Router
    let router_glue = Arc::new(dl_voice::glue::RouterGlue {
        adapter: adapter.clone(),
    });
    let lane_router = dl_voice::router::LaneRouter::new(
        central_pool.clone(),
        router_glue.clone(),
        tempvoice.clone(),
        Some(activity.clone()),
    );
    dl_voice::router::register(&mut router, lane_router.clone());
    let router_interface =
        dl_voice::router::RouterInterface::new(central_pool.clone(), router_glue);

    // Voice-Feedback-DMs (4a-Rest) — Button/Modal brauchen den Router
    let voice_feedback = dl_voice::feedback::VoiceFeedback::new(
        central_pool.clone(),
        Arc::new(dl_voice::glue::FeedbackGlue {
            adapter: adapter.clone(),
        }),
    );
    dl_voice::feedback::register(&mut router, voice_feedback.clone());

    // Tag-System (6/7): Single Source of Truth, von TempVoice-Filtern genutzt
    let tag_service = dl_community::tags::TagService::new(central_pool.clone());
    // /meine-tags-Selbstverwaltung (Slash + Select/Reset-Komponenten).
    dl_community::tags_ui::register(&mut router, tag_service.clone());

    // SecurityGuard (6): sg:*-Mod-Buttons am Router, Scan gateway-gated
    router.on_prefix(
        "sg:",
        Arc::new(modglue::GuardReviewHandler {
            adapter: adapter.clone(),
        }),
    );
    let guard_client = dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok());
    let openai_vision_client = dl_ai::OpenAiClient::from_env(|k| std::env::var(k).ok());
    let our_guild_id = env("OUR_GUILD_ID")
        .or_else(|| env("MAIN_GUILD_ID"))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(onboardglue::MAIN_GUILD_ID);
    let fallback_invites = env("INVITE_ALLOWLIST_FALLBACK")
        .map(|raw| modglue::parse_invite_allowlist_fallback(&raw))
        .unwrap_or_default();
    let guard_glue = Arc::new(modglue::GuardGlue::new(
        adapter.clone(),
        our_guild_id,
        fallback_invites,
    ));
    guard_glue.refresh_invite_allowlist().await;
    let escalation_contact_handle = env("ESCALATION_CONTACT_HANDLE")
        .unwrap_or_else(|| dl_moderation::guard::DEFAULT_ESCALATION_CONTACT_HANDLE.to_string());
    let security_guard_enforce = env_bool_default("SECURITY_GUARD_ENFORCE", true);
    tracing::info!(
        enforce = security_guard_enforce,
        "SecurityGuard Enforcement-Modus gelesen (SECURITY_GUARD_ENFORCE)"
    );
    let security_guard = dl_moderation::guard::SecurityGuard::new_with_config(
        central_pool.clone(),
        guard_client
            .clone()
            .map(|c| c as Arc<dyn dl_ai::TextGenerator>),
        openai_vision_client
            .clone()
            .map(|c| c as Arc<dyn dl_ai::VisionGenerator>),
        guard_glue,
        dl_moderation::guard::SecurityGuardConfig {
            escalation_contact_handle,
            enforce: security_guard_enforce,
        },
    );

    // Onboarding-Wizard (7): rp:panel:start + Thread-Schritte
    let wizard =
        dl_community::onboarding::OnboardingWizard::new(Arc::new(onboardglue::WizardGlue {
            adapter: adapter.clone(),
            tags: tag_service.clone(),
            steam: steam_client.clone(),
            pool: central_pool.clone(),
        }));
    // Verifikations-Abschluss: RoleEvent::Gained(Verified) → Abschluss-Nachricht.
    dl_community::onboarding::spawn_verify_completion(wizard.clone(), &dispatcher);
    dl_community::onboarding::spawn_screening_auto_start(
        wizard.clone(),
        &dispatcher,
        onboardglue::MAIN_GUILD_ID,
    );
    dl_community::onboarding::register(&mut router, wizard);

    // AI-Onboarding (H6): legacy `aiob:*` buttons, modal submit, MiniMax tour.
    // This complements the static wizard; it does not replace or restart the
    // disabled old Welcome-DM step flow.
    let ai_onboarding_tokens = env("DEADLOCK_ONBOARD_TOKENS")
        .and_then(|raw| raw.parse::<u32>().ok())
        .filter(|tokens| *tokens > 0)
        .unwrap_or(dl_community::ai_onboarding::AI_ONBOARDING_DEFAULT_MAX_OUTPUT_TOKENS);
    let ai_onboarding = dl_community::ai_onboarding::AiOnboarding::new(
        central_pool.clone(),
        Arc::new(onboardglue::AiOnboardingGlue {
            adapter: adapter.clone(),
        }),
        dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok())
            .map(|client| client as Arc<dyn dl_ai::TextGenerator>),
        dl_community::ai_onboarding::AiOnboardingConfig::new(
            onboardglue::MAIN_GUILD_ID,
            onboardglue::ONBOARD_COMPLETE_ROLE_ID,
        )
        .with_max_output_tokens(ai_onboarding_tokens),
    );
    match ai_onboarding.restore_persistent_views().await {
        Ok(report) => tracing::info!(
            scanned = report.scanned,
            restored = report.restored,
            removed_invalid = report.removed_invalid,
            "AI onboarding views restored"
        ),
        Err(err) => tracing::warn!(%err, "AI onboarding views could not be loaded"),
    }
    dl_community::ai_onboarding::register(&mut router, ai_onboarding);

    // Privacy-Oberflaeche: /datenschutz + /datenschutz-optin (Loeschung/Opt-in).
    dl_community::privacy_ui::register(&mut router, central_pool.clone());

    // Brain-RAG Prefix-Command: echter Textcommand ueber MessageEvent-Subscriber
    // (InteractionRouter::on_prefix ist custom_id-Routing fuer Komponenten).
    let brain_handler = {
        let brain_bin = env("BRAIN_BIN").unwrap_or_else(default_brain_bin);
        let brain_bin_path = std::path::PathBuf::from(&brain_bin);
        let enabled = env_bool_default("BRAIN_CMD_ENABLED", brain_bin_path.is_file());
        if !enabled {
            tracing::info!("Brain-Command deaktiviert (BRAIN_CMD_ENABLED)");
            None
        } else if !brain_bin_path.is_file() {
            tracing::warn!(
                bin = %brain_bin_path.display(),
                "Brain-Command nicht registriert: BRAIN_BIN existiert nicht"
            );
            None
        } else {
            let client = dl_ai::MiniMaxClient::from_env(env);
            if client.is_none() {
                tracing::warn!(
                    "Brain-Command registriert ohne MiniMax-Client; Antworten liefern Backend-Fehler"
                );
            }
            let cooldown_secs = env_u64_default("BRAIN_COOLDOWN_SECS", 20);
            let max_question_len = env_usize_default("BRAIN_MAX_QUESTION_LEN", 300);
            let brain_db = env("BRAIN_DB").unwrap_or_else(default_brain_db);
            let brain_db_path = std::path::PathBuf::from(&brain_db);
            let channel_allowlist = env("BRAIN_CHANNEL_ALLOWLIST")
                .and_then(|raw| modglue::parse_brain_channel_allowlist(&raw));
            tracing::info!(
                bin = %brain_bin_path.display(),
                db = %brain_db_path.display(),
                cooldown_secs,
                max_question_len,
                channel_allowlist = channel_allowlist.as_ref().map(|ids| ids.len()).unwrap_or(0),
                "Brain-Command registriert"
            );
            Some(Arc::new(modglue::BrainHandler {
                adapter: adapter.clone(),
                config: dl_brain::BrainConfig {
                    max_question_len,
                    cooldown_secs,
                },
                cooldowns: Arc::new(dl_brain::BrainCooldowns::default()),
                retriever: Arc::new(modglue::BrainRetrieverGlue {
                    bin: brain_bin_path,
                    db_path: Some(brain_db_path),
                }),
                answerer: Arc::new(modglue::BrainAiGlue { client }),
                channel_allowlist,
            }))
        }
    };

    // Coaching (7): Panel postet nur noch einen Link zur Website. Die frühere
    // Discord-Anfrageaufnahme samt KI-Analyse/Rollen-/Stale-Recovery bleibt im
    // Rust-Cutover bewusst aus (Website-driven intake, #17/#18 dropped).
    // Der Website-Client wird weiter für CoachingSync (Roster/Termin-DMs)
    // geteilt. None = kein interner Token → Sync inaktiv.
    let coaching_website =
        dl_community::coaching::WebsiteClient::from_env(|k| std::env::var(k).ok());
    let coaching_requests = dl_community::coaching_requests::CoachingRequests::new(
        central_pool.clone(),
        Arc::new(modglue::CoachingReqGlue {
            adapter: adapter.clone(),
        }),
        dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok())
            .map(|client| client as Arc<dyn dl_ai::TextGenerator>),
        1289721245281292288,
        coaching_website
            .clone()
            .map(|client| client as Arc<dyn dl_community::coaching::CoachingWebsiteSyncClient>),
    );
    dl_community::coaching_requests::register(&mut router, coaching_requests.clone());
    coaching_requests.ensure_panel().await;

    // FAQ-Chat (6) — Panel-Buttons brauchen den Router, Subscriber gateway-gated
    let faq_docs_path = std::env::var("FAQ_DOCS_PATH").unwrap_or_else(|_| "docs".to_string());
    let faq_ai = dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok());
    let faq_text_ai = faq_ai
        .clone()
        .map(|client| client as Arc<dyn dl_ai::TextGenerator>);
    let faq_tool_ai = faq_ai.map(|client| client as Arc<dyn dl_ai::ToolTextGenerator>);
    let faq = dl_community::faq::FaqChat::new_with_ticket_support(
        central_pool.clone(),
        Arc::new(modglue::FaqGlue {
            adapter: adapter.clone(),
        }),
        faq_text_ai,
        faq_tool_ai,
        Some(dl_community::faq::TicketDiagnostics::new(
            twitch_client.clone(),
        )),
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
        pool: central_pool.clone(),
    });
    dl_community::feedback_hub::register(&mut router, feedback_hub.clone());

    // Clip-Einsendungen (6) — Button/Modal brauchen den Router, Loops gateway-gated
    let clips = dl_community::clips::ClipSubmission::new(
        central_pool.clone(),
        Arc::new(modglue::ClipGlue {
            adapter: adapter.clone(),
        }),
    );
    dl_community::clips::register(&mut router, clips.clone());

    // Leave-Survey (6) — Select/Modal brauchen den Router, Trigger ist gateway-gated
    let leave_survey = dl_community::leave_survey::LeaveSurvey::new(
        central_pool.clone(),
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
    dl_community::retention::register(&mut router, central_pool.clone(), retention_port.clone());

    // Onboarding-Buttons (7): Regelbestätigung + Steam-Login + DM-Hinweise
    onboardglue::register(
        &mut router,
        adapter.clone(),
        dl_bridges::steam::SteamBotClient::from_env(|k| std::env::var(k).ok()),
    );

    // AI-Moderator (6) — Review-Buttons brauchen den Router, Scan ist gateway-gated
    let moderator = dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok()).map(|client| {
        let vision = openai_vision_client
            .clone()
            .map(|c| c as Arc<dyn dl_ai::VisionGenerator>);
        let moderator = dl_moderation::AiModerator::new(
            central_pool.clone(),
            client as Arc<dyn dl_ai::TextGenerator>,
            vision,
            Arc::new(modglue::ModGlue {
                adapter: adapter.clone(),
                tags: tag_service.clone(),
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
    let command_sync_config = master::CommandSyncStartupConfig::from_lookup(env);
    let command_sync = Arc::new(master::DiscordCommandSync::new(
        adapter.clone(),
        router.clone(),
        command_sync_config.guild_id,
    ));
    let (master_action_tx, mut master_action_rx) =
        tokio::sync::mpsc::unbounded_channel::<master::MasterAction>();

    // Listener: member_remove → Steam-Bot, !steam_*-Admin-Kommandos
    let _member_listener =
        dl_bridges::steam::spawn_member_remove_listener(&dispatcher, steam_client.clone());
    let _admin_listener = dl_bridges::steam::spawn_admin_command_listener(
        &dispatcher,
        steam_client.clone(),
        adapter.clone(),
    );

    // Master-Broker :8770 — Token-Kette wie das Original
    let broker_token = env("MASTER_BROKER_TOKEN")
        .or_else(|| env("MAIN_BOT_INTERNAL_TOKEN"))
        .or_else(|| env("TWITCH_INTERNAL_API_TOKEN"))
        .context("Broker-Token fehlt (MASTER_BROKER_TOKEN/MAIN_BOT_INTERNAL_TOKEN/TWITCH_INTERNAL_API_TOKEN)")?;
    let broker = dl_broker::BrokerState::new_with_channel_info(
        adapter.clone(),
        Arc::new(BrokerChannelInfoGlue {
            adapter: adapter.clone(),
        }),
        broker_token,
        |key| std::env::var(key).ok(),
    )
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

    // Server-Sync-Orchestrator :8901 — loopback-only plus X-Internal-Token.
    let serversync_token = env("SERVERSYNC_INTERNAL_TOKEN");
    if serversync_token.is_none() {
        tracing::warn!("SERVERSYNC_INTERNAL_TOKEN fehlt — Server-Sync-HTTP-Routen liefern 403");
    }
    let serversync_addr = format!("127.0.0.1:{}", serversync::PORT);
    let serversync_listener = tokio::net::TcpListener::bind(&serversync_addr)
        .await
        .with_context(|| format!("Server-Sync-Port binden: {serversync_addr}"))?;
    tracing::info!(addr = %serversync_addr, "Server-Sync-API gebunden");
    let serversync_server = axum::serve(
        serversync_listener,
        serversync::router(serversync_service.clone(), serversync_token)
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
        master::spawn_control(
            &dispatcher,
            adapter.clone(),
            command_sync.clone(),
            Arc::new(master::DiscordStatusPort::new(
                adapter.clone(),
                router.clone(),
                startup_text.clone(),
            )),
            owner_id,
            master_action_tx.clone(),
        );
        if let Some(brain_handler) = &brain_handler {
            modglue::spawn_brain_command(brain_handler.clone(), &dispatcher);
        }

        // Rename-Queue (Port rename_manager): zentrale, rate-limit-bewusste
        // Channel-Umbenennung. init() VOR den Voice-Subscribern, damit deren
        // Rename-Wuensche eingereiht statt direkt ausgefuehrt werden; EIN Worker
        // drainiert FIFO mit >=360s Abstand pro Channel.
        dl_voice::rename_queue::init(central_pool.clone());
        dl_voice::rename_queue::spawn_worker(
            central_pool.clone(),
            Arc::new(dl_voice::glue::RenameExecGlue {
                adapter: adapter.clone(),
            }),
        );

        // Voice-Session-Tracker (4a): Subscriber + Wartungs-Loops
        let voice_tracker =
            dl_voice::tracker::VoiceTracker::new(central_pool.clone(), cache_snapshot.clone());
        voice_tracker.set_feedback(voice_feedback.clone()).await;
        // Voice-Statistik-Befehle (!vstats, !vleaderboard/!vlb/!voicetop):
        // teilen sich den Tracker (Live-Session-Zuschlag) + Cache (Namen,
        // Rollen, Guild-Name). Bewusst ohne Admin-Gate (jeder darf abfragen).
        let voice_stats = dl_voice::stats::VoiceStatsCommands::new(
            central_pool.clone(),
            voice_tracker.clone(),
            cache_snapshot.clone(),
            Some(voice_feedback.clone()),
        );
        dl_voice::stats::spawn_command(voice_stats, &dispatcher, adapter.clone());
        dl_voice::tracker::spawn(voice_tracker, &dispatcher);

        // TempVoice-Engine (4b): Join-to-create + Owner-Lifecycle
        dl_voice::tempvoice::interface::spawn_command(
            tempvoice_interface.clone(),
            &dispatcher,
            adapter.clone(),
        );
        dl_voice::tempvoice::engine::spawn(tempvoice.clone(), &dispatcher);
        // Tag-Filter: Dienst anbinden + Ragebaiter-Sofort-Durchsetzung
        tag_service.rehydrate().await;
        tempvoice.set_tag_service(tag_service.clone()).await;
        dl_community::tags::spawn_cleanup(tag_service.clone());
        dl_voice::tempvoice::engine::spawn_tag_listener(tempvoice.clone(), tag_service.clone());

        // Rank-Voice-Manager (4c): Anker + Rang-Rechte auf Comp-Lanes
        let rank_manager = dl_voice::rank::RankVoiceManager::new(
            central_pool.clone(),
            Arc::new(dl_voice::glue::RankGlue {
                adapter: adapter.clone(),
            }),
            Arc::new({
                let tempvoice = tempvoice.clone();
                move |channel_id| tempvoice.initial_owner_blocking(channel_id)
            }),
        );
        // !rrang-Admin-Gruppe (toggle/anker/vcstatus/debug/aktualisieren/…):
        // Prefix-Listener über denselben Manager (gemeinsamer Anker-State).
        let rank_commands = dl_voice::rank::RankCommands::new(rank_manager.clone());
        dl_voice::rank::spawn(rank_manager, &dispatcher);
        dl_voice::rank::spawn_command(rank_commands, &dispatcher, adapter.clone());

        // Steam-Link-Nudge (4c): DM nach 30 min Voice am zweiten Tag
        dl_voice::nudge::spawn_restore(nudge.clone());
        dl_voice::nudge::spawn(nudge.clone(), &dispatcher);
        // !nudgesend/!t30-Admin-Test: schickt die Nudge-DM an ein Ziel.
        dl_voice::nudge::spawn_command(nudge.clone(), &dispatcher, adapter.clone());
        // Voice-Feedback: Freitext-Antworten auf Feedback-DMs.
        dl_voice::feedback::spawn_dm_responses(
            voice_feedback.clone(),
            &dispatcher,
            adapter.clone(),
        );

        // SecurityGuard (6): Message-Subscriber (Takeover/Burst/Keyword)
        if let Err(err) = security_guard.ensure_schema().await {
            tracing::warn!(%err, "SecurityGuard: Schema-Anlage fehlgeschlagen");
        }
        dl_moderation::guard::spawn(security_guard.clone(), &dispatcher);

        // Player-Finder (5): portiert, aber per Flag deaktiviert (Redesign geplant)
        if dl_activity::player_finder::enabled(|k| std::env::var(k).ok()) {
            let _finder = dl_activity::player_finder::PlayerFinder::new(central_pool.clone());
            tracing::warn!(
                "PLAYER_FINDER_ENABLED=1 gesetzt — Kern portiert, Message-Flow folgt mit dem Redesign"
            );
        } else {
            tracing::info!("Player-Finder deaktiviert (PLAYER_FINDER_ENABLED nicht gesetzt)");
        }

        // Website-Invites (5): permanente Codes je Unterseite sicherstellen
        let website_invites = dl_community::invites::WebsiteInvites {
            store: dl_community::invites::InviteStore {
                pool: central_pool.clone(),
            },
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

        // Coaching-Plattform-Brücke (7): Rollen-Sync 10min + Notifications 60s
        // + Roster-Resync bei Coach-Rollen-Änderung (Debounce). Teilt sich den
        // oben gebauten Website-Client mit CoachingRequests.
        match coaching_website.clone() {
            Some(client) => {
                let sync = Arc::new(dl_community::coaching::CoachingSync {
                    client,
                    port: Arc::new(modglue::CoachingGlue {
                        adapter: adapter.clone(),
                        guild_id: 1289721245281292288,
                    }),
                    request_sink: Some(coaching_requests.clone()),
                });
                dl_community::coaching::spawn(sync, &dispatcher);
            }
            None => tracing::info!("Coaching-Sync inaktiv (kein interner Token)"),
        }

        // AI-Moderator (6): Scan-Kanal-Subscriber
        // Der automatische Text-Scan ist per Default AUS (AI_MODERATOR_ENABLE),
        // bis der überarbeitete, GPT-verifizierte Moderations-Guard live ist.
        // Das Schema wird weiter angelegt und die aimod:-Review-Buttons bleiben
        // registriert, damit bestehende Fälle abgearbeitet werden können.
        // SecurityGuard (Bild-/Takeover-Schutz) ist davon unberührt.
        if let Some(moderator) = &moderator {
            if let Err(err) = moderator.store.ensure_schema().await {
                tracing::warn!(%err, "Moderation: Schema-Anlage fehlgeschlagen");
            }
            if env_bool_default("AI_MODERATOR_ENABLE", false) {
                dl_moderation::spawn(moderator.clone(), &dispatcher);
            } else {
                tracing::info!("AI-Moderator-Scan deaktiviert (AI_MODERATOR_ENABLE nicht gesetzt)");
            }
        } else {
            tracing::info!("AI-Moderator inaktiv (kein MiniMax-Key)");
        }

        // Aktivitäts-Analyzer (5): Loops starten (Instanz oben gebaut)
        dl_activity::analyzer::spawn(activity.clone());
        dl_activity::analyzer::spawn_member_events(central_pool.clone(), &dispatcher);
        // Der Task wartet intern auf READY + Cache-Guilds und retryt leere
        // Member-Snapshots, statt nach einem fixen Startup-Fenster aufzugeben.
        dl_activity::analyzer::spawn_member_backfill(
            central_pool.clone(),
            Arc::new(modglue::ActivityBackfillGlue {
                adapter: adapter.clone(),
            }),
        );
        dl_activity::analyzer::spawn_message_activity(central_pool.clone(), &dispatcher);
        let _journey_ingestion =
            dl_activity::journey::spawn_ingestion(central_pool.clone(), &dispatcher);
        let _journey_retention = dl_activity::journey::spawn_retention(central_pool.clone());
        let _journey_role_events =
            journeyglue::spawn_role_events(central_pool.clone(), &dispatcher);
        let _journey_tag_events = journeyglue::spawn_tag_events(
            central_pool.clone(),
            tag_service.clone(),
            onboardglue::MAIN_GUILD_ID,
        );
        // Text-Gamification (5): Konversations-Punkte → text_stats (speist das
        // öffentliche Text-Leaderboard) + 60-s-Flush-Loop.
        let text_sessions = Arc::new(dl_activity::text_stats::TextSessions::new(
            central_pool.clone(),
        ));
        if let Err(err) = text_sessions.ensure_schema().await {
            tracing::warn!(%err, "text_stats-Schema konnte nicht angelegt werden");
        }
        dl_activity::text_stats::spawn_text_stats(text_sessions, &dispatcher);
        // Aktivitäts-/Text-Statistik-Befehle als Prefix-Listener:
        // !useranalysis/!ua/!analyze, !myactivity, !tleaderboard/!tlb/!texttop,
        // !messagestats/!msgstats, !serverstats (nur Letzteres admin-gegated).
        let activity_stats = dl_activity::stats_cmd::ActivityStatsCommands::new(
            central_pool.clone(),
            Arc::new(dl_activity::glue::StatsNames {
                adapter: adapter.clone(),
            }),
        );
        dl_activity::stats_cmd::spawn_command(activity_stats, &dispatcher, adapter.clone());
        // Build-Publisher (#23): DB-only Queue-Steuerlogik fuer BUILD_PUBLISH.
        // Gateway-gated, damit dormant Rust-Starts nicht parallel zum Python-Cog
        // dieselbe steam_tasks-Queue befuellen.
        let _build_publisher_tasks = build_publisher::spawn(central_pool.clone());
        // Retention-Tracking (Daten-Layer): Voice-Join → user_retention_tracking
        // + 30-min avg_weekly_sessions-Sync (Quelle der Leave-Survey-Einstufung)
        // + stündlicher Miss-You-Check (Embed-DM an inaktive Stamm-User).
        dl_community::retention::spawn(
            dl_community::retention::RetentionTracker::new(central_pool.clone()),
            retention_port.clone(),
            &dispatcher,
        );
        dl_community::leave_survey::spawn(leave_survey.clone(), &dispatcher);
        dl_community::clips::spawn(clips.clone());
        dl_community::faq::spawn(faq.clone(), &dispatcher);
        // KI-DM-Assistent: beantwortet Freitext-DMs an den Bot (MiniMax + Fallback).
        let dm_assistant = dl_community::dm_assistant::DmAssistant::new(
            dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok())
                .map(|client| client as Arc<dyn dl_ai::TextGenerator>),
            Arc::new(modglue::DmGlue {
                adapter: adapter.clone(),
            }),
        );
        dl_community::dm_assistant::spawn_dm_assistant(dm_assistant, &dispatcher);
        // Coaching-Survey: Poll + Voice-Ende-Listener. Der Discord-Intake bleibt
        // website-driven (#17/#18), aber abgeschlossene Sessions muessen wie in
        // Python Reward-Rolle + Feedback-DM bekommen.
        let _coaching_request_tasks =
            dl_community::coaching_requests::spawn(coaching_requests.clone(), &dispatcher);
        // !fhub-Panel-Listener (Admin postet/editiert das Feedback-Panel)
        dl_community::feedback_hub::spawn(feedback_hub.clone(), &dispatcher);

        // LFG-Lobby-Finder (5): Antworten im Suche-Kanal
        let lfg_responder = dl_activity::lfg::LfgResponder::new(
            central_pool.clone(),
            Arc::new(modglue::LfgGlue {
                adapter: adapter.clone(),
            }),
        );
        dl_activity::lfg::spawn_responder(lfg_responder, &dispatcher);

        // Lane-Router (4c-Rest): Join auf den Router-VC einsortieren
        dl_voice::router::spawn(lane_router.clone(), &dispatcher);

        // Adaptive Spezial-Lanes: Anfänger-Routing + Duo + Sortierung
        let adaptive = dl_voice::adaptive::AdaptiveLanes::new(cache_snapshot.clone());
        adaptive.set_tempvoice(tempvoice.clone()).await;
        tempvoice.set_adaptive(adaptive.clone()).await;
        dl_voice::adaptive::spawn(adaptive, &dispatcher);

        // Voice-Status-Worker (4c): LiveMatch-Suffixe an Lane-Namen
        let status_worker = dl_voice::status::VoiceStatusWorker::new(
            central_pool.clone(),
            Arc::new(dl_voice::glue::StatusGlue {
                adapter: adapter.clone(),
                tempvoice: Some(tempvoice.clone()),
            }),
        );
        dl_voice::status::spawn(status_worker.clone());
        let status_commands = dl_voice::status::StatusCommands::new(status_worker.clone());
        dl_voice::status::spawn_command(status_commands, &dispatcher, adapter.clone());
        // Slash-Commands syncen: Python-Default ist on + guild-scope.
        if command_sync_config.enabled {
            let summary = command_sync.sync_scope(command_sync_config.scope).await;
            match summary.status.as_str() {
                "synced" => tracing::info!(
                    scope = summary.scope.as_str(),
                    global = summary.global_count,
                    guilds = summary.guild_counts.len(),
                    "Slash-Commands synchronisiert"
                ),
                "partial" => tracing::warn!(
                    scope = summary.scope.as_str(),
                    global = summary.global_count,
                    guilds = summary.guild_counts.len(),
                    errors = summary.errors.len(),
                    "Slash-Command-Sync teilweise fehlgeschlagen"
                ),
                _ => tracing::error!(
                    scope = summary.scope.as_str(),
                    errors = summary.errors.len(),
                    "Slash-Command-Sync fehlgeschlagen"
                ),
            }
        } else {
            tracing::info!("Startup app-command sync disabled via DL_BOT_COMMAND_SYNC.");
        }
        let mut client = dl_discord::gateway::build_client(
            &discord_token,
            adapter.clone(),
            dispatcher.clone(),
            router.clone(),
            dl_discord::gateway::GatewayClientOptions {
                reaction_roles: Some(Arc::new(modglue::ReactionRoleGatewayGlue {
                    service: reaction_roles.clone(),
                })),
                pool: central_pool.clone(),
                feature_module_count: master::FEATURE_MODULES.len(),
                command_prefix: env("COMMAND_PREFIX").unwrap_or_else(|| "!".to_string()),
            },
        )
        .await
        .context("Gateway-Client bauen")?;
        // KRITISCH: serenity legt beim Build einen EIGENEN Cache an. Ohne diese
        // Kopplung läse die gesamte Glue aus einem leeren Adapter-Cache (alle
        // Voice-/Channel-/Member-Lookups None → TempVoice baut keine Lanes usw.).
        adapter.link_cache(client.cache.clone());
        let mut panel_cache_ready = dispatcher.subscribe_gateway();
        let tempvoice_interface_ready = tempvoice_interface.clone();
        let router_interface_ready = router_interface.clone();
        tokio::spawn(async move {
            wait_for_gateway_cache_ready(
                &mut panel_cache_ready,
                1289721245281292288,
                "voice_panels",
            )
            .await;
            tempvoice_interface_ready.refresh_all_interfaces().await;
            router_interface_ready.ensure_panel().await;
        });
        dl_bridges::steam::spawn_panel_restore(
            steam_client.clone(),
            central_pool.clone(),
            adapter.clone(),
        );
        let reaction_backfill = reaction_roles.clone();
        tokio::spawn(async move {
            if let Err(err) = reaction_backfill.run_pending_backfills().await {
                tracing::warn!(%err, "Reaction-Role-Backfill fehlgeschlagen");
            }
        });
        tracing::warn!(
            "Gateway AKTIV — sicherstellen, dass der Python-Bot die Events abgegeben hat"
        );
        Some(tokio::spawn(async move { client.start().await }))
    } else {
        tracing::info!("Gateway inaktiv (DL_BOT_GATEWAY != 1) — nur REST/Broker/Changelog");
        None
    };

    tracing::info!("dl-bot läuft — beenden mit Ctrl+C");
    let mut restart_requested = false;
    tokio::select! {
        result = broker_server => result.context("Broker-Server")?,
        result = changelog_server => result.context("Changelog-Server")?,
        result = serversync_server => result.context("Server-Sync-Server")?,
        action = master_action_rx.recv() => {
            match action {
                Some(master::MasterAction::Restart) => {
                    restart_requested = true;
                    tracing::info!("Restart angefordert — Prozess beendet sich fuer systemd");
                }
                None => {
                    tracing::debug!("Master-Control-Kanal geschlossen");
                }
            }
        },
        _ = tokio::signal::ctrl_c() => tracing::info!("dl-bot beendet"),
    }
    if let Some(task) = gateway_task {
        task.abort();
    }
    if restart_requested {
        // Kein process::exit: normaler Return laesst PidLock::drop laufen,
        // der Non-Zero-Code triggert systemd Restart=on-failure.
        return Ok(master::restart_exit_code());
    }
    Ok(std::process::ExitCode::SUCCESS)
}
