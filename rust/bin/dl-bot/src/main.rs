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
mod mcp;
mod modglue;
mod onboardglue;
mod onboardingbridgeglue;
mod scrimglue;
mod serversync;
mod vanity;

use std::{num::NonZeroU64, sync::Arc};

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

fn moderation_enforce_from_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    fn explicit_true(value: &str) -> bool {
        matches!(value.to_ascii_lowercase().as_str(), "1" | "true")
    }

    lookup("MODERATION_ENFORCE")
        .or_else(|| lookup("MOD_ENFORCE"))
        .map(|value| explicit_true(&value))
        .unwrap_or_else(|| {
            lookup("SECURITY_GUARD_ENFORCE")
                .map(|value| explicit_true(&value))
                .unwrap_or(false)
        })
}

fn env_u64_default(name: &str, default: u64) -> u64 {
    env(name)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn lfg_panel_channel_id_from_env() -> (Option<u64>, Option<String>) {
    match std::env::var("DL_LFG_PANEL_CHANNEL_ID") {
        Ok(raw) => lfg_panel_channel_id_from_value(Some(raw.as_str())),
        Err(std::env::VarError::NotPresent) => lfg_panel_channel_id_from_value(None),
        Err(err) => (
            None,
            Some(format!(
                "DL_LFG_PANEL_CHANNEL_ID konnte nicht gelesen werden: {err}"
            )),
        ),
    }
}

fn lfg_forum_channel_id_from_env() -> (Option<u64>, Option<String>) {
    match std::env::var("DL_LFG_FORUM_CHANNEL_ID") {
        Ok(raw) => lfg_forum_channel_id_from_value(Some(raw.as_str())),
        Err(std::env::VarError::NotPresent) => lfg_forum_channel_id_from_value(None),
        Err(err) => (
            None,
            Some(format!(
                "DL_LFG_FORUM_CHANNEL_ID konnte nicht gelesen werden: {err}"
            )),
        ),
    }
}

fn lfg_panel_channel_id_from_value(raw: Option<&str>) -> (Option<u64>, Option<String>) {
    lfg_channel_id_from_value("DL_LFG_PANEL_CHANNEL_ID", raw)
}

fn lfg_forum_channel_id_from_value(raw: Option<&str>) -> (Option<u64>, Option<String>) {
    lfg_channel_id_from_value("DL_LFG_FORUM_CHANNEL_ID", raw)
}

fn lfg_channel_id_from_value(name: &str, raw: Option<&str>) -> (Option<u64>, Option<String>) {
    let Some(raw) = raw else {
        return (None, Some(format!("{name} ist nicht gesetzt")));
    };
    let value = raw.trim();
    if value.is_empty() {
        return (None, Some(format!("{name} ist leer")));
    }
    match value.parse::<u64>() {
        Ok(0) => (None, Some(format!("{name} darf nicht 0 sein"))),
        Ok(id) => (
            Some(NonZeroU64::new(id).expect("checked non-zero").get()),
            None,
        ),
        Err(_) => (
            None,
            Some(format!("{name} ist keine gueltige positive Discord-ID")),
        ),
    }
}

fn lfg_cutover_active(lfg_forum_cutover_enabled: bool, lfg_forum_channel_id: Option<u64>) -> bool {
    lfg_forum_cutover_enabled && lfg_forum_channel_id.is_some()
}

fn legacy_lfg_responder_enabled(lfg_cutover_active: bool) -> bool {
    !lfg_cutover_active
}

fn env_i64_default(name: &str, default: i64) -> i64 {
    env(name)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default)
}

fn env_f64_default(name: &str, default: f64) -> f64 {
    env(name)
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value.clamp(0.0, 1.0))
        .unwrap_or(default)
}

fn env_usize_default(name: &str, default: usize) -> usize {
    env(name)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn openai_client_with_model_from_env(
    model_env: &str,
    default_model: &str,
) -> Option<(Arc<dl_ai::OpenAiClient>, String)> {
    let api_key = env("OPENAI_API_KEY").or_else(|| env("DEADLOCK_OPENAI_KEY"))?;
    let base_url =
        env("OPENAI_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let model = env(model_env).unwrap_or_else(|| default_model.to_string());
    Some((
        dl_ai::OpenAiClient::new(base_url, api_key, model.clone()),
        model,
    ))
}

fn default_brain_bin() -> String {
    "/home/naniadm/Documents/Deadlock-Brain/rust/target/release/deadlock-brain".to_string()
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
    let _audit_log_poller = dl_discord::audit_log::spawn(
        central_pool.clone(),
        adapter.clone(),
        dl_server_as_code::DEFAULT_GUILD_ID,
    );
    let spam_learning_store = dl_changelog::SpamLearningStore::new();
    let changelog = dl_changelog::ChangelogState::new_with_spam_learning(
        adapter.clone(),
        env("CHANGELOG_API_TOKEN"),
        Some(spam_learning_store.clone()),
    );
    let owner_id = master::owner_id_from_lookup(env);
    if owner_id.is_none() {
        tracing::warn!("OWNER_ID fehlt — Owner-Commands bleiben gesperrt");
    }
    let serversync_concrete = serversync::ServerSyncService::new(
        central_pool.clone(),
        adapter.clone(),
        discord_token.clone(),
        serversync::GUILD_ID,
    );
    let serversync_service: serversync::SharedServerSync = serversync_concrete.clone();
    let dispatcher = Arc::new(dl_discord::Dispatcher::new());
    let reaction_roles = dl_community::reaction_roles::ReactionRoleService::new(
        central_pool.clone(),
        Arc::new(modglue::ReactionRoleGlue {
            adapter: adapter.clone(),
        }),
    );
    let scrim_signup = dl_community::scrim_signup::ScrimSignup::new(central_pool.clone());

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
    serversync::register_regelwerk_components(&mut router);
    serversync::register_faq_components(&mut router);
    dl_community::scrim_signup::register(&mut router, scrim_signup);
    let twitch_registry = dl_bridges::twitch::TrackingRegistry::new();
    let twitch_client = dl_bridges::twitch::TwitchApiClient::from_env(|k| std::env::var(k).ok());
    let matcher = match &twitch_client {
        Some(twitch_client) => {
            dl_bridges::twitch::register(
                &mut router,
                twitch_client.clone(),
                twitch_registry.clone(),
            );
            dl_bridges::twitch::register_spam_learning(
                &mut router,
                twitch_client.clone(),
                spam_learning_store.clone(),
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

    let concierge_config =
        dl_community::concierge::ConciergeConfig::from_env(|k| std::env::var(k).ok());
    let concierge_memory_store = concierge_config
        .enabled
        .then(|| dl_community::concierge::ConciergeStore::new(central_pool.clone()));

    // Steam-Link-Nudge (4c) — Close-Button braucht den Router, Spawn ist gateway-gated
    let nudge = dl_voice::nudge::VoiceNudge::new(
        central_pool.clone(),
        Arc::new(modglue::VoiceNudgeGlue {
            inner: dl_voice::glue::NudgeGlue {
                adapter: adapter.clone(),
                steam: dl_bridges::steam::SteamBotClient::from_env(|k| std::env::var(k).ok()),
                log_channel_id: dl_voice::nudge::LOG_CHANNEL_ID,
            },
            concierge_store: concierge_memory_store.clone(),
            concierge_guild_id: concierge_config.main_guild_id,
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
    let lfg_forum_cutover_enabled = env_bool_default("DL_LFG_FORUM_CUTOVER", false);
    let (lfg_panel_channel_id, lfg_panel_channel_reason) = lfg_panel_channel_id_from_env();
    let (lfg_forum_channel_id, lfg_forum_channel_reason) = lfg_forum_channel_id_from_env();
    let lfg_cutover_active = lfg_cutover_active(lfg_forum_cutover_enabled, lfg_forum_channel_id);
    let tempvoice_interface = dl_voice::tempvoice::interface::TempVoiceInterface::new(
        tempvoice.clone(),
        cache_snapshot.clone(),
        lfg_cutover_active,
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
        dl_voice::router::RouterInterface::new(central_pool.clone(), router_glue.clone());
    router_interface.ensure_panel().await;
    serversync_concrete
        .set_router_interface(router_interface.clone())
        .await;
    if lfg_forum_cutover_enabled && !lfg_cutover_active {
        let reason = lfg_forum_channel_reason
            .as_deref()
            .unwrap_or("DL_LFG_FORUM_CHANNEL_ID fehlt oder ist ungueltig");
        tracing::warn!(%reason, "LFG-Cutover deaktiviert: ungueltige Forum-Kanal-Konfiguration");
    }
    if lfg_cutover_active && lfg_panel_channel_id.is_none() {
        let reason = lfg_panel_channel_reason
            .as_deref()
            .unwrap_or("DL_LFG_PANEL_CHANNEL_ID fehlt oder ist ungueltig");
        tracing::warn!(%reason, "LFG-Panel deaktiviert: ungueltige Panel-Kanal-Konfiguration");
    }
    let lfg_panel_interface = dl_voice::lfg_panel::LfgPanelInterface::new_with_split_channel_config(
        central_pool.clone(),
        router_glue.clone(),
        lfg_panel_channel_id,
        lfg_panel_channel_reason,
        lfg_forum_channel_id,
        lfg_forum_channel_reason,
        lfg_cutover_active,
    );
    lfg_panel_interface
        .set_lane_spawner(dl_voice::lfg_panel::RouterLfgLaneSpawner::new(
            lane_router.clone(),
        ))
        .await;
    if lfg_cutover_active {
        tempvoice.set_lfg_panel(lfg_panel_interface.clone()).await;
    }
    serversync_concrete
        .set_lfg_panel_interface(lfg_panel_interface.clone())
        .await;
    dl_voice::tempvoice::interface::register(
        &mut router,
        tempvoice.clone(),
        lfg_cutover_active.then_some(lfg_panel_interface.clone()),
    );
    dl_voice::lfg_panel::register(&mut router, lfg_panel_interface.clone());

    // Voice-Feedback-DMs (4a-Rest) — Button/Modal brauchen den Router
    let voice_feedback = dl_voice::feedback::VoiceFeedback::new(
        central_pool.clone(),
        Arc::new(modglue::VoiceFeedbackGlue {
            inner: dl_voice::glue::FeedbackGlue {
                adapter: adapter.clone(),
            },
            concierge_store: concierge_memory_store.clone(),
            concierge_guild_id: concierge_config.main_guild_id,
        }),
    );
    dl_voice::feedback::register(&mut router, voice_feedback.clone());

    // Tag-System (6/7): Single Source of Truth, von TempVoice-Filtern genutzt
    let tag_service = dl_community::tags::TagService::new(central_pool.clone());
    // /meine-tags-Selbstverwaltung (Slash + Select/Reset-Komponenten).
    dl_community::tags_ui::register(&mut router, tag_service.clone());

    let moderation_channel_id =
        dl_moderation::moderation_channel::moderation_channel_id_from_lookup(|k| {
            std::env::var(k).ok()
        });
    let moderation_scan_channel_ids =
        dl_moderation::moderation_channel::scan_channel_ids_from_lookup(|k| std::env::var(k).ok());

    let moderation_text_analyze_client =
        dl_ai::FireworksClient::from_env(|k| std::env::var(k).ok());
    let moderation_text_analyze_model =
        env("MOD_TEXT_ANALYZE_MODEL").unwrap_or_else(|| dl_ai::DEFAULT_FIREWORKS_MODEL.to_string());
    let moderation_image_analyze_client =
        openai_client_with_model_from_env("MOD_IMAGE_ANALYZE_MODEL", dl_ai::DEFAULT_OPENAI_MODEL);
    let moderation_verify_client =
        openai_client_with_model_from_env("MOD_VERIFY_MODEL", dl_ai::DEFAULT_OPENAI_MODEL);
    let our_guild_id = env("OUR_GUILD_ID")
        .or_else(|| env("MAIN_GUILD_ID"))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(onboardglue::MAIN_GUILD_ID);
    let fallback_invites = env("INVITE_ALLOWLIST_FALLBACK")
        .map(|raw| modglue::parse_invite_allowlist_fallback(&raw))
        .unwrap_or_default();
    let behavior_glue = Arc::new(modglue::BehaviorDetectorGlue::new(
        adapter.clone(),
        our_guild_id,
        fallback_invites,
    ));
    behavior_glue.refresh_invite_allowlist().await;
    let moderation_enforce = moderation_enforce_from_lookup(env);
    tracing::info!(
        enforce = moderation_enforce,
        "Moderation Enforcement-Modus gelesen"
    );
    let behavior_detector = dl_moderation::behavior_detector::BehaviorDetector::new(behavior_glue);

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
            let channel_allowlist = env("BRAIN_CHANNEL_ALLOWLIST")
                .and_then(|raw| modglue::parse_brain_channel_allowlist(&raw));
            tracing::info!(
                bin = %brain_bin_path.display(),
                cooldown_secs,
                max_question_len,
                channel_allowlist = channel_allowlist.as_ref().map(|ids| ids.len()).unwrap_or(0),
                "Brain-Command registriert"
            );
            let config = Arc::new(dl_brain::BrainConfig {
                max_question_len,
                cooldown_secs,
            });
            let retriever: Arc<dyn dl_brain::BrainRetriever> =
                Arc::new(modglue::BrainRetrieverGlue {
                    bin: brain_bin_path,
                });
            let answerer: Arc<dyn dl_brain::AiAnswerer> = Arc::new(modglue::BrainAiGlue { client });
            Some(Arc::new(modglue::BrainHandler {
                adapter: adapter.clone(),
                config,
                cooldowns: Arc::new(dl_brain::BrainCooldowns::default()),
                retriever,
                answerer,
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
    let faq = dl_community::faq::FaqChat::new(
        central_pool.clone(),
        Arc::new(modglue::FaqGlue {
            adapter: adapter.clone(),
        }),
    );
    dl_community::faq::register(&mut router, faq.clone());

    // Concierge-Onboarding Slice A: default AUS, T0 nur fuer Test-Allowlist.
    let concierge_ai = if concierge_config.enabled {
        match dl_ai::LlmProviderConfig::from_env(|k| std::env::var(k).ok())
            .map_err(anyhow::Error::from)
            .and_then(|cfg| {
                cfg.build_provider_for_env(dl_ai::LlmUseCase::BotPate, |k| std::env::var(k).ok())
                    .map_err(anyhow::Error::from)
            }) {
            Ok(provider) => Some(provider),
            Err(err) => {
                tracing::warn!(%err, "Concierge-LLM inaktiv");
                None
            }
        }
    } else {
        None
    };
    let concierge = dl_community::concierge::Concierge::new(
        central_pool.clone(),
        Arc::new(modglue::ConciergeGlue {
            adapter: adapter.clone(),
            brain: brain_handler
                .as_ref()
                .map(|handler| modglue::ConciergeBrain {
                    config: handler.config.clone(),
                    cooldowns: Arc::new(dl_brain::BrainCooldowns::default()),
                    retriever: handler.retriever.clone(),
                    answerer: handler.answerer.clone(),
                }),
        }),
        concierge_ai,
        concierge_config.clone(),
    );
    dl_community::concierge::register(&mut router, concierge.clone());

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

    // Moderation (6) — Review-Buttons brauchen den Router, Scan ist gateway-gated.
    // Text-Analyze laeuft ueber Fireworks, Bild-Analyze und Verify ueber OpenAI nano.
    let moderator = match (
        moderation_text_analyze_client.clone(),
        moderation_image_analyze_client.clone(),
        moderation_verify_client.clone(),
    ) {
        (
            Some(text_analyze_client),
            Some((image_analyze_client, image_analyze_model)),
            Some((verify_client, verify_model)),
        ) => {
            let analyze_text: Arc<dyn dl_ai::TextGenerator> = text_analyze_client;
            let analyze_vision: Arc<dyn dl_ai::VisionGenerator> = image_analyze_client;
            let verify_text: Arc<dyn dl_ai::TextGenerator> = verify_client.clone();
            let verify_vision: Arc<dyn dl_ai::VisionGenerator> = verify_client;
            let pipeline = dl_moderation::content_analyzer::ContentModerationPipeline::new(
                dl_moderation::content_analyzer::ContentAnalyzer::new(
                    analyze_text,
                    Some(analyze_vision),
                    dl_moderation::content_analyzer::ContentAnalyzerConfig {
                        text_model: moderation_text_analyze_model.clone(),
                        image_model: image_analyze_model,
                    },
                ),
                dl_moderation::content_verifier::ContentVerifier::new(
                    verify_text,
                    Some(verify_vision),
                    dl_moderation::content_verifier::ContentVerifierConfig {
                        model: verify_model,
                    },
                ),
                env_f64_default("MOD_ANALYZE_FLAG_THRESHOLD", 0.5),
            );
            let policy = dl_moderation::action_policy::ActionPolicy::new(
                dl_moderation::action_policy::ActionPolicyConfig {
                    auto_execute_verified_confidence: env_f64_default(
                        "MOD_AUTO_VERIFY_THRESHOLD",
                        0.85,
                    ),
                    proposal_verified_confidence: env_f64_default(
                        "MOD_PROPOSE_VERIFY_THRESHOLD",
                        0.60,
                    ),
                    timeout_minutes: env_i64_default("MOD_TIMEOUT_MINUTES", 1440),
                    behavior_proposal_timeout_minutes: env_i64_default(
                        "MOD_BEHAVIOR_PROPOSAL_TIMEOUT_MINUTES",
                        60,
                    ),
                },
            );
            let moderator = dl_moderation::ModerationSystem::new_with_behavior_detector(
                central_pool.clone(),
                pipeline,
                Some(behavior_detector.clone()),
                policy,
                Arc::new(modglue::ModGlue {
                    adapter: adapter.clone(),
                    tags: tag_service.clone(),
                }),
                dl_moderation::moderation_system::ModerationSystemConfig {
                    scan_channel_ids: moderation_scan_channel_ids.clone(),
                    moderation_channel_id,
                    enforce: moderation_enforce,
                },
            );
            router.on_prefix(
                "aimod:",
                Arc::new(modglue::ReviewHandler::new(moderator.clone())),
            );
            Some(moderator)
        }
        _ => None,
    };

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
    // Gleiche Token-Kette wie der Master-Broker; SERVERSYNC_INTERNAL_TOKEN nur als Override.
    let serversync_token = env("SERVERSYNC_INTERNAL_TOKEN")
        .or_else(|| env("MASTER_BROKER_TOKEN"))
        .or_else(|| env("MAIN_BOT_INTERNAL_TOKEN"))
        .or_else(|| env("TWITCH_INTERNAL_API_TOKEN"));
    if serversync_token.is_none() {
        tracing::warn!(
            "Kein interner Token (SERVERSYNC_INTERNAL_TOKEN/MASTER_BROKER_TOKEN/MAIN_BOT_INTERNAL_TOKEN/TWITCH_INTERNAL_API_TOKEN) — Server-Sync-HTTP-Routen liefern 403"
        );
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

    // MCP-Connector :8890 — loopback-only Streamable-HTTP-Endpunkt für Claude.
    // Läuft mit der Bot-Identität (DISCORD_TOKEN aus dem Prozess-Env, via
    // Infisical/systemd-creds) — kein eigener Secrets-Weg nötig.
    let mcp_state = Arc::new(
        mcp::McpState::from_env(discord_token.clone(), env).context("MCP-Connector-State")?,
    );
    let mcp_addr = mcp::McpState::bind_addr(env);
    let mcp_listener = tokio::net::TcpListener::bind(&mcp_addr)
        .await
        .with_context(|| format!("MCP-Connector-Port binden: {mcp_addr}"))?;
    tracing::info!(addr = %mcp_addr, "MCP-Connector gebunden");
    let mcp_server = axum::serve(mcp_listener, mcp::router(mcp_state));

    let scrim_announcement_channel_id =
        NonZeroU64::new(env_u64_default("DL_SCRIM_ANNOUNCEMENT_CHANNEL_ID", 0))
            .map(NonZeroU64::get);
    let mut scrim_match_driver = scrimglue::spawn(
        central_pool.clone(),
        adapter.clone(),
        scrim_announcement_channel_id,
    );

    // Gateway: user-gated — Python hält die Session bis zum Cutover
    let gateway_enabled = env("DL_BOT_GATEWAY").as_deref() == Some("1");
    let gateway_task = if gateway_enabled {
        let presence_intent_enabled = env_bool_default("DL_ENABLE_PRESENCE_INTENT", false);
        if presence_intent_enabled {
            tracing::info!("GUILD_PRESENCES-Intent aktiviert via DL_ENABLE_PRESENCE_INTENT");
        }
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

        // Moderation (6): Scan-Kanal-Subscriber
        // Der automatische Text-Scan ist per Default AUS (AI_MODERATOR_ENABLE),
        // bis der überarbeitete, GPT-verifizierte Moderations-Guard live ist.
        // Das Schema wird weiter angelegt und die aimod:-Review-Buttons bleiben
        // registriert, damit bestehende Fälle abgearbeitet werden können.
        if let Some(moderator) = &moderator {
            if let Err(err) = moderator.store.ensure_schema().await {
                tracing::warn!(%err, "Moderation: Schema-Anlage fehlgeschlagen");
            }
            if env_bool_default("AI_MODERATOR_ENABLE", false) {
                dl_moderation::moderation_system::spawn(moderator.clone(), &dispatcher);
            } else {
                tracing::info!("AI-Moderator-Scan deaktiviert (AI_MODERATOR_ENABLE nicht gesetzt)");
            }
        } else {
            tracing::info!("AI-Moderator inaktiv (kein MiniMax- oder OpenAI-Key)");
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
        let _server_sync_rollback_retention =
            dl_community::privacy::spawn_server_sync_rollback_export_retention(
                central_pool.clone(),
            );
        let _moderation_content_retention =
            dl_community::privacy::spawn_moderation_content_retention(central_pool.clone());
        let _journey_role_events =
            journeyglue::spawn_role_events(central_pool.clone(), &dispatcher);
        let _journey_native_onboarding_completed = journeyglue::spawn_native_onboarding_completed(
            central_pool.clone(),
            adapter.clone(),
            &dispatcher,
            onboardglue::MAIN_GUILD_ID,
        );
        let _onboarding_bridge = onboardingbridgeglue::spawn(
            central_pool.clone(),
            adapter.clone(),
            &dispatcher,
            onboardglue::MAIN_GUILD_ID,
        );
        let _concierge_tasks = dl_community::concierge::spawn(concierge.clone(), &dispatcher);
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
        let _invite_lounge_watcher =
            dl_community::invite_lounge::spawn(central_pool.clone(), adapter.clone(), &dispatcher);
        let voice_hint_enabled =
            dl_community::voice_change_hint::enabled_from_lookup(|k| std::env::var(k).ok());
        let voice_hint_classifier = if voice_hint_enabled {
            dl_ai::OpenAiClient::text_from_env(|k| std::env::var(k).ok()).map(|client| {
                let generator: Arc<dyn dl_ai::TextGenerator> = client;
                Arc::new(dl_community::voice_change_hint::OpenAiVoiceHintClassifier::new(generator))
                    as Arc<dyn dl_community::voice_change_hint::VoiceHintClassifier>
            })
        } else {
            None
        };
        let voice_hint_responder = Arc::new(
            dl_community::voice_change_hint::VoiceChangeHintResponder::new(
                voice_hint_enabled,
                voice_hint_classifier,
                adapter.clone(),
            ),
        );
        let _voice_change_hint_responder =
            dl_community::voice_change_hint::spawn(voice_hint_responder, &dispatcher);
        // KI-DM-Assistent bleibt nur fuer Concierge-Testmodus/disabled aktiv; Open-Modus antwortet nativ.
        if !concierge_config.open_for_all() {
            let concierge_dm_ignore = if concierge_config.enabled {
                concierge_config.test_user_allowlist.clone()
            } else {
                std::collections::HashSet::new()
            };
            let dm_assistant = dl_community::dm_assistant::DmAssistant::new_with_ignore_users(
                dl_ai::MiniMaxClient::from_env(|k| std::env::var(k).ok())
                    .map(|client| client as Arc<dyn dl_ai::TextGenerator>),
                Arc::new(modglue::DmGlue {
                    adapter: adapter.clone(),
                }),
                concierge_dm_ignore,
            );
            dl_community::dm_assistant::spawn_dm_assistant(dm_assistant, &dispatcher);
        }
        // Coaching-Survey: Poll + Voice-Ende-Listener. Der Discord-Intake bleibt
        // website-driven (#17/#18), aber abgeschlossene Sessions muessen wie in
        // Python Reward-Rolle + Feedback-DM bekommen.
        let _coaching_request_tasks =
            dl_community::coaching_requests::spawn(coaching_requests.clone(), &dispatcher);
        // !fhub-Panel-Listener (Admin postet/editiert das Feedback-Panel)
        dl_community::feedback_hub::spawn(feedback_hub.clone(), &dispatcher);

        // LFG-Lobby-Finder (5): Antworten im alten Suche-Kanal bis zum Forum-Cutover.
        if legacy_lfg_responder_enabled(lfg_cutover_active) {
            let lfg_responder = dl_activity::lfg::LfgResponder::new(
                central_pool.clone(),
                Arc::new(modglue::LfgGlue {
                    adapter: adapter.clone(),
                }),
            );
            dl_activity::lfg::spawn_responder(lfg_responder, &dispatcher);
        } else {
            tracing::info!("Alter LFG-Text-Responder wegen DL_LFG_FORUM_CUTOVER deaktiviert");
        }

        // Lane-Router (4c-Rest): Join auf den Router-VC einsortieren
        dl_voice::router::spawn(lane_router.clone(), &dispatcher);
        if lfg_cutover_active {
            dl_voice::lfg_panel::spawn(lfg_panel_interface.clone(), &dispatcher);
        }

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
                enable_presence_intent: presence_intent_enabled,
            },
        )
        .await
        .context("Gateway-Client bauen")?;
        // KRITISCH: serenity legt beim Build einen EIGENEN Cache an. Ohne diese
        // Kopplung läse die gesamte Glue aus einem leeren Adapter-Cache (alle
        // Voice-/Channel-/Member-Lookups None → TempVoice baut keine Lanes usw.).
        adapter.link_cache(client.cache.clone());
        let _vanity_snapshots = vanity::spawn_vanity_snapshots(
            central_pool.clone(),
            discord_token.clone(),
            &dispatcher,
        );
        let _member_directory_sweep = vanity::spawn_member_directory_sweep(
            central_pool.clone(),
            discord_token.clone(),
            &dispatcher,
        );
        let mut panel_cache_ready = dispatcher.subscribe_gateway();
        let tempvoice_interface_ready = tempvoice_interface.clone();
        tokio::spawn(async move {
            wait_for_gateway_cache_ready(
                &mut panel_cache_ready,
                1289721245281292288,
                "voice_panels",
            )
            .await;
            tempvoice_interface_ready.refresh_all_interfaces().await;
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
        result = mcp_server => result.context("MCP-Connector-Server")?,
        result = &mut scrim_match_driver => {
            result.context("Scrim-Match-Treiber")?;
            anyhow::bail!("Scrim-Match-Treiber beendet");
        },
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
    scrim_match_driver.abort();
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

#[cfg(test)]
mod tests {
    use super::{
        legacy_lfg_responder_enabled, lfg_cutover_active, lfg_forum_channel_id_from_value,
        lfg_panel_channel_id_from_value, moderation_enforce_from_lookup,
    };
    use std::collections::HashMap;

    fn lookup<'a>(vars: &'a HashMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + 'a {
        |key| vars.get(key).map(|value| (*value).to_string())
    }

    #[test]
    fn moderation_enforce_defaults_to_shadow_when_all_vars_are_unset() {
        let vars = HashMap::new();

        assert!(!moderation_enforce_from_lookup(lookup(&vars)));
    }

    #[test]
    fn moderation_enforce_uses_ordered_strict_true_vars() {
        let vars = HashMap::from([
            ("MODERATION_ENFORCE", "false"),
            ("MOD_ENFORCE", "true"),
            ("SECURITY_GUARD_ENFORCE", "true"),
        ]);
        assert!(!moderation_enforce_from_lookup(lookup(&vars)));

        let vars = HashMap::from([("MOD_ENFORCE", "1")]);
        assert!(moderation_enforce_from_lookup(lookup(&vars)));

        let vars = HashMap::from([("SECURITY_GUARD_ENFORCE", "true")]);
        assert!(moderation_enforce_from_lookup(lookup(&vars)));

        let vars = HashMap::from([("MODERATION_ENFORCE", "yes")]);
        assert!(!moderation_enforce_from_lookup(lookup(&vars)));
    }

    #[test]
    fn lfg_panel_channel_id_parst_nur_positive_nonzero_ids() {
        assert_eq!(
            lfg_panel_channel_id_from_value(Some("12345")),
            (Some(12345), None)
        );

        for raw in [None, Some(""), Some("0"), Some("abc")] {
            let (channel_id, reason) = lfg_panel_channel_id_from_value(raw);
            assert_eq!(channel_id, None);
            assert!(
                reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("DL_LFG_PANEL_CHANNEL_ID")),
                "reason fehlt fuer {raw:?}: {reason:?}"
            );
        }
    }

    #[test]
    fn lfg_forum_channel_id_parst_nur_positive_nonzero_ids() {
        assert_eq!(
            lfg_forum_channel_id_from_value(Some("54321")),
            (Some(54321), None)
        );

        for raw in [None, Some(""), Some("0"), Some("abc")] {
            let (channel_id, reason) = lfg_forum_channel_id_from_value(raw);
            assert_eq!(channel_id, None);
            assert!(
                reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("DL_LFG_FORUM_CHANNEL_ID")),
                "reason fehlt fuer {raw:?}: {reason:?}"
            );
        }
    }

    #[test]
    fn forum_cutover_haengt_an_forum_id_nicht_an_panel_id() {
        assert!(legacy_lfg_responder_enabled(lfg_cutover_active(
            false,
            Some(123)
        )));
        assert!(legacy_lfg_responder_enabled(lfg_cutover_active(true, None)));
        assert!(!legacy_lfg_responder_enabled(lfg_cutover_active(
            true,
            Some(123)
        )));
    }
}
