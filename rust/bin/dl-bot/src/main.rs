//! dl-bot — der künftige Discord-Prozess.
//!
//! Phase-2-Stand: Master-Broker (:8770) und Changelog-Empfänger (:8899)
//! sind voll implementiert (REST-Aktionen brauchen kein Gateway, nur den
//! Bot-Token). Das Gateway selbst ist user-gated (DL_BOT_GATEWAY=1) —
//! bis zum koordinierten Cutover hält der Python-Bot die Discord-Session,
//! deshalb sind die Standard-Ports hier erst nach Freigabe zu übernehmen.

mod aiglue;
mod build_publisher;
mod journeyglue;
mod master;
mod mcp;
mod modglue;
mod onboardglue;
mod scrim_adapter;
mod scrimglue;
mod serversync;
mod turnierglue;
mod vanity;

use std::{
    collections::HashSet,
    num::NonZeroU64,
    os::unix::fs::PermissionsExt,
    sync::{atomic::AtomicBool, Arc},
};

use anyhow::Context;
use dl_core::runtime_config::lookup as operating_value;
use dl_webcore::WebConfig;

fn env(name: &str) -> Option<String> {
    operating_value(name)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn warn_if_lagebild_token_empty(token: &str) {
    if token.is_empty() {
        tracing::warn!(
            "TURNIER_INTERNAL_API_TOKEN is empty; Lagebild endpoint will reject every request with 401"
        );
    }
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

fn validate_voice_worker_token<'a>(
    main_token: &str,
    worker_token: Option<&'a str>,
) -> Result<&'a str, &'static str> {
    let worker_token = worker_token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or("DISCORD_TOKEN_RANKED fehlt oder ist leer")?;
    if worker_token == main_token {
        return Err("DISCORD_TOKEN_RANKED darf nicht der Main-Bot-Token sein");
    }
    Ok(worker_token)
}

struct ReadinessReset(Arc<AtomicBool>);

impl Drop for ReadinessReset {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
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
    lfg_panel_channel_id_from_value(operating_value("DL_LFG_PANEL_CHANNEL_ID").as_deref())
}

fn lfg_forum_channel_id_from_env() -> (Option<u64>, Option<String>) {
    lfg_forum_channel_id_from_value(operating_value("DL_LFG_FORUM_CHANNEL_ID").as_deref())
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

/// Einziger Weg vom Bot zum Sprachmodell: Anbieterwahl und Compliance-Gate aus
/// dl-ai, danach die TextGenerator-Bruecke. Jeder Ausgang wird geloggt, damit
/// ein stiller Ausfall nicht wie "Feature aus" aussieht.
fn chat_text_generator(
    use_case: dl_ai::LlmUseCase,
    json_mode: bool,
) -> Option<Arc<dyn dl_ai::TextGenerator>> {
    chat_text_generator_with(use_case, json_mode, operating_value)
}

fn chat_text_generator_with(
    use_case: dl_ai::LlmUseCase,
    json_mode: bool,
    lookup: impl Fn(&str) -> Option<String> + Copy,
) -> Option<Arc<dyn dl_ai::TextGenerator>> {
    let config = match dl_ai::LlmProviderConfig::from_env(lookup) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(
                use_case = use_case.as_str(),
                %error,
                "LLM-Anbieterkonfiguration abgelehnt — Feature laeuft ohne KI"
            );
            return None;
        }
    };
    match config.build_provider_for_env(use_case, lookup) {
        Ok(provider) => {
            tracing::info!(
                use_case = use_case.as_str(),
                json_mode,
                "LLM ueber den geprueften Anbieterweg verdrahtet"
            );
            Some(if json_mode {
                dl_ai::ChatTextGenerator::new_json(provider, use_case)
                    as Arc<dyn dl_ai::TextGenerator>
            } else {
                dl_ai::ChatTextGenerator::new(provider, use_case) as Arc<dyn dl_ai::TextGenerator>
            })
        }
        Err(error) => {
            tracing::warn!(
                use_case = use_case.as_str(),
                %error,
                "LLM-Anbieter nicht verfuegbar — Feature laeuft ohne KI"
            );
            None
        }
    }
}

/// Modellwahl aus dem geprüften Betriebssnapshot. Der Anbieter kommt aus dem Gate,
/// das bestehende Modell wandert als `GenerateRequest::model`
/// bis in die Anfrage.
fn model_from_lookup(
    lookup: impl Fn(&str) -> Option<String>,
    key: &str,
    default_model: &str,
) -> String {
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_model.to_string())
}

/// Legacy-Anbieterwahl des Streamer-Matchers.
///
/// Gesetzt und bekannt: der Wert wandert als Gate-Schluessel weiter, das
/// Compliance-Gate entscheidet darueber. Gesetzt und unbekannt (frueher der
/// `_ => NoAi`-Zweig, etwa `gemini` oder `off`): kein KI-Scoring. Nicht
/// gesetzt: es gilt der Standard aus `DL_LLM_PROVIDER_STREAMER_MATCHER`.
enum MatcherProviderChoice {
    Gate(Option<String>),
    Off(String),
}

fn matcher_provider_choice(raw: Option<String>) -> MatcherProviderChoice {
    let Some(raw) = raw
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return MatcherProviderChoice::Gate(None);
    };
    match raw.parse::<dl_ai::LlmProviderKind>() {
        Ok(kind) => MatcherProviderChoice::Gate(Some(kind.as_str().to_string())),
        Err(_) => MatcherProviderChoice::Off(raw),
    }
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

fn brain_channel_allowlist_from_value(raw: Option<&str>) -> Option<HashSet<u64>> {
    raw.and_then(modglue::parse_brain_channel_allowlist)
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
    let cfg = dl_core::Config::from_env().context("Konfiguration laden")?;
    let operating = dl_core::config::process_bot_config()?.snapshot();
    dl_core::observability::init_tracing(
        operating
            .runtime
            .start
            .log_filter
            .as_deref()
            .unwrap_or("info"),
    );
    tracing::info!(
        config_fingerprint = dl_core::config::process_bot_config()?.fingerprint(),
        "Betriebskonfiguration geladen"
    );
    let _pid_lock = master::PidLock::acquire_default().context("Single-Instance-PID-Lock")?;
    let startup_text = master::startup_text_now();

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
    // KI-Transparenz-Log: jede Modellantwort wird im Transparenz-Kanal mitlesbar.
    // Die Senke wird bei jedem chat()-Aufruf neu aufgeloest, deshalb ist es
    // ungefaehrlich, sie hier vor dem Bau der Provider zu registrieren.
    let transparency_config = dl_ai::TransparencyConfig::from_env(operating_value);
    let _transparency_log = transparency_config.enabled.then(|| {
        let log = dl_ai::TransparencyLog::spawn(
            Arc::new(aiglue::DiscordTransparencyMessenger {
                adapter: adapter.clone(),
            }),
            transparency_config.clone(),
        );
        dl_ai::set_transparency_sink(log.sink());
        log
    });

    let changelog = dl_changelog::ChangelogState::new(adapter.clone(), env("CHANGELOG_API_TOKEN"));
    let owner_id = master::owner_id_from_lookup(operating_value);
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
    // Modellwahl bleibt bei TURNIER_AI_MODEL; der Weg zum Modell laeuft jetzt
    // ueber das Compliance-Gate statt am Gate vorbei.
    let turnier_model = model_from_lookup(env, "TURNIER_AI_MODEL", dl_ai::DEFAULT_OPENAI_MODEL);
    let turnier_generator = chat_text_generator(dl_ai::LlmUseCase::TurnierVorschlag, false);
    let turnier_proposals = Arc::new(turnierglue::TurnierProposalService::from_env(
        adapter.clone(),
        turnier_generator,
        turnier_model,
        operating_value,
    ));
    turnierglue::register(&mut router, turnier_proposals.clone());
    let steam_client = dl_bridges::steam::SteamBotClient::from_env(operating_value);
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
    let scrim_runtime_gate =
        scrim_adapter::ScrimRuntimeGate::with_default_ttl(central_pool.clone());
    let scrim_relay_handler = scrim_adapter::relay_handler(central_pool.clone(), operating_value);
    tracing::info!("Scrim-Ownership wird ueber scrim.runtime_control entschieden");
    scrimglue::register(
        &mut router,
        central_pool.clone(),
        scrim_runtime_gate.clone(),
        scrim_relay_handler,
    );
    let twitch_registry = dl_bridges::twitch::TrackingRegistry::new();
    let twitch_client = dl_bridges::twitch::TwitchApiClient::from_env(operating_value);
    let matcher = match &twitch_client {
        Some(twitch_client) => {
            dl_bridges::twitch::register(
                &mut router,
                twitch_client.clone(),
                twitch_registry.clone(),
            );
            dl_bridges::twitch::register_spam_learning(&mut router, twitch_client.clone());
            dl_bridges::twitch::register_crew_ban(&mut router, twitch_client.clone(), owner_id);
            // Streamer-Link-Matcher (Review-Buttons immer registrieren —
            // offene Vorschläge überleben Neustarts über den State-File)
            let matcher_config = dl_bridges::matcher::MatcherConfig::from_env(operating_value);
            let glue = Arc::new(dl_bridges::glue::AdapterGlue {
                adapter: adapter.clone(),
                notify_channel_id: matcher_config.notify_channel_id,
            });
            // AI-Scoring: STREAMER_LINK_AI_PROVIDER waehlt weiter den Anbieter,
            // aber ueber das Compliance-Gate statt ueber eigene Clients.
            let scorer: Arc<dyn dl_bridges::matcher::AiScorer> = match matcher_provider_choice(
                operating_value("STREAMER_LINK_AI_PROVIDER"),
            ) {
                MatcherProviderChoice::Off(raw) => {
                    tracing::info!(
                        provider = %raw,
                        "Streamer-Matcher ohne KI-Scoring: STREAMER_LINK_AI_PROVIDER nennt keinen Anbieter des Gates"
                    );
                    Arc::new(dl_bridges::matcher::NoAi)
                }
                MatcherProviderChoice::Gate(override_provider) => {
                    let generator = chat_text_generator_with(
                        dl_ai::LlmUseCase::StreamerMatcher,
                        false,
                        |key| match (key, override_provider.as_deref()) {
                            ("DL_LLM_PROVIDER_STREAMER_MATCHER", Some(provider)) => {
                                operating_value(key).or_else(|| Some(provider.to_string()))
                            }
                            _ => operating_value(key),
                        },
                    );
                    match generator {
                        Some(generator) => Arc::new(dl_ai::MatcherScorer { generator }),
                        None => Arc::new(dl_bridges::matcher::NoAi),
                    }
                }
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

    let mut concierge_config = dl_community::concierge::ConciergeConfig::from_env(operating_value);
    concierge_config.ai_timeout =
        std::time::Duration::from_secs(operating.concierge.timeout_seconds);
    let concierge_memory_store = concierge_config
        .enabled
        .then(|| dl_community::concierge::ConciergeStore::new(central_pool.clone()));

    // Startinventar: nach dem Hochfahren steht im Journal, welcher Anbieter
    // welchen KI-Pfad bedient und welcher Pfad still ohne Modell weiterlaeuft.
    aiglue::log_ai_startup_inventory(
        &dl_ai::LlmProviderConfig::from_env(operating_value),
        operating_value,
        &transparency_config,
        &concierge_config,
    );

    // Steam-Link-Nudge (4c) — standardmäßig deaktiviert. Der Close-Button bleibt
    // registriert, damit bereits versandte DMs weiterhin geschlossen werden können.
    let steam_voice_nudge_enabled = env_bool_default("DL_STEAM_VOICE_NUDGE_ENABLED", false);
    let nudge = dl_voice::nudge::VoiceNudge::new(
        central_pool.clone(),
        Arc::new(modglue::VoiceNudgeGlue {
            inner: dl_voice::glue::NudgeGlue {
                adapter: adapter.clone(),
                steam: dl_bridges::steam::SteamBotClient::from_env(operating_value),
                log_channel_id: dl_voice::nudge::LOG_CHANNEL_ID,
            },
            concierge_store: concierge_memory_store.clone(),
            concierge_guild_id: concierge_config.main_guild_id,
        }),
    );
    dl_voice::nudge::register(&mut router, nudge.clone());

    let solo_watch =
        dl_voice::solo_watch::SoloWatch::new(Arc::new(dl_voice::glue::SoloWatchGlue {
            adapter: adapter.clone(),
            pool: central_pool.clone(),
            log_channel_id: dl_voice::solo_watch::LOG_CHANNEL_ID,
        }));
    dl_voice::solo_watch::register(&mut router, solo_watch.clone());

    // TempVoice-Engine (4b/4c) — Panel-Buttons brauchen den Router,
    // der Event-Subscriber startet erst mit dem Gateway
    let voice_pair_store = Arc::new(dl_voice::voice_pair_guard::VoicePairGuardStore::new(
        central_pool.clone(),
    ));
    let voice_pair_operations =
        Arc::new(dl_voice::voice_pair_guard::VoicePairOperationLock::new(()));
    let cache_snapshot = Arc::new(dl_voice::glue::CacheSnapshot {
        adapter: adapter.clone(),
        voice_pair_store: voice_pair_store.clone(),
        voice_pair_operations: voice_pair_operations.clone(),
    });
    let recording_main_manager = dl_voice::scrim_record::recording_songbird_manager();
    let recording_worker_manager = dl_voice::scrim_record::recording_songbird_manager();
    let recording_main_readiness = Arc::new(AtomicBool::new(false));
    let recording_worker_readiness = Arc::new(AtomicBool::new(false));
    let recording_backend = Arc::new(
        dl_voice::scrim_record::SongbirdRecordingBackend::new(
            recording_main_manager.clone(),
            recording_main_readiness.clone(),
            recording_worker_manager.clone(),
            recording_worker_readiness.clone(),
        )
        .map_err(anyhow::Error::msg)
        .context("Scrim-Record-Backend bauen")?,
    );
    // Nicht XDG_RUNTIME_DIR: das ist eine tmpfs von rund 1,5 GB, und eine sechsstuendige
    // Aufnahme braucht 2,1 GB. Der Zwischenspeicher gehoert deshalb auf die Platte.
    let recording_state_dir = operating
        .runtime
        .community
        .recording_state_dir
        .clone()
        .or_else(|| std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".local/state"))
        })
        .context("XDG_STATE_HOME und HOME fehlen fuer Scrim-Record")?;
    // Der Elternordner muss privat sein (0700), sonst lehnt prepare_recording_temp_dir ab.
    // Deshalb ein eigener, selbst angelegter Ordner statt des vorgefundenen State-Verzeichnisses.
    let recording_base_dir = recording_state_dir.join("deadlock-bots");
    tokio::fs::create_dir_all(&recording_base_dir)
        .await
        .context("Scrim-Record-Basisverzeichnis erstellen")?;
    tokio::fs::set_permissions(&recording_base_dir, std::fs::Permissions::from_mode(0o700))
        .await
        .context("Scrim-Record-Basisverzeichnis absichern")?;
    let recording_temp_dir = recording_base_dir.join("scrim-recordings");
    dl_voice::scrim_record::prepare_recording_temp_dir(&recording_temp_dir)
        .await
        .map_err(anyhow::Error::msg)
        .context("Scrim-Record-Temp-Verzeichnis erstellen")?;
    // Ziel der Aufnahmen ist der Google-Drive-Ordner hinter dem rclone-Remote; beides
    // ist konfigurierbar, damit ein Umzug keinen Rebuild braucht.
    let rclone_path = operating_value("SCRIM_RECORD_RCLONE_PATH")
        .unwrap_or_else(|| "/usr/local/bin/rclone".to_string());
    let archive_base = operating_value("SCRIM_RECORD_ARCHIVE_BASE")
        .unwrap_or_else(|| "gdrive:Deadlock/Scrim-Aufnahmen".to_string());
    let scrim_recorder = dl_voice::scrim_record::ScrimRecorder::new(
        cache_snapshot.clone(),
        recording_backend,
        Arc::new(dl_voice::scrim_record::FfmpegTranscoder),
        Arc::new(dl_voice::scrim_record::RcloneArchive::new(
            rclone_path,
            archive_base,
        )),
        recording_temp_dir,
    );
    dl_voice::scrim_record::register(
        &mut router,
        dl_voice::scrim_record::RecordCommandHandler::new(scrim_recorder.clone()),
    );
    let voice_pair_guard = dl_voice::voice_pair_guard::VoicePairGuard::new(
        voice_pair_store.clone(),
        cache_snapshot.clone(),
        voice_pair_operations.clone(),
    );
    let tempvoice = dl_voice::tempvoice::TempVoiceEngine::new_with_voice_pair_operations(
        dl_voice::tempvoice::TempVoiceConfig::production(),
        dl_voice::tempvoice::TempVoiceStore::new(central_pool.clone()),
        cache_snapshot.clone(),
        voice_pair_operations.clone(),
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
    let lane_router = dl_voice::router::LaneRouter::new_with_auto_move(
        central_pool.clone(),
        router_glue.clone(),
        tempvoice.clone(),
        Some(activity.clone()),
        dl_voice::router::RouterAutoMoveConfig {
            enabled: env_bool_default("DL_ROUTER_AUTO_MOVE_ENABLED", true),
            delay: std::time::Duration::from_secs(env_u64_default(
                "DL_ROUTER_AUTO_MOVE_DELAY_SECONDS",
                60,
            )),
        },
    );
    dl_voice::router::register(&mut router, lane_router.clone());

    // Lane-Pairing (Doppel-Opt-in): zwei Einzelsitzer werden gefragt, ob sie
    // zusammen zocken wollen. Kill-Switch: DL_LANE_PAIRING_ENABLED=0.
    let lane_pairing_enabled = env_bool_default("DL_LANE_PAIRING_ENABLED", true);
    let lane_pairing = dl_voice::pairing::LanePairing::new(Arc::new(dl_voice::glue::PairingGlue {
        adapter: adapter.clone(),
        pool: central_pool.clone(),
        engine: tempvoice.clone(),
    }));
    if lane_pairing_enabled {
        dl_voice::pairing::register(&mut router, lane_pairing.clone());
    }

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

    tracing::info!("Mitspieler-Umfrage deaktiviert");

    // Community-Puls: Interactions bleiben für bereits versandte DMs aktiv;
    // nur die Wellen-Planung läuft hinter dem Opt-in-Flag.
    let survey_pulse_config = dl_activity::survey_pulse::SurveyPulseConfig::from_lookup(env);
    let survey_pulse_handler =
        dl_activity::survey_pulse::SurveyPulseHandler::new(central_pool.clone());
    dl_activity::survey_pulse::register(&mut router, survey_pulse_handler);

    // Outbox-Zustellung: eigener Lebenszyklus, läuft immer und an keinem Flag.
    // Zugestellt wird ausschließlich, wofür hier ein Handler registriert ist —
    // alles andere bleibt pending und wird pro Zyklus gezählt gemeldet.
    let _outbox_dispatcher = dl_activity::outbox::spawn(
        dl_activity::outbox::OutboxDispatcher::new(central_pool.clone()).register(
            dl_activity::survey_pulse::SURVEY_PULSE_ACTION_TYPE,
            dl_activity::survey_pulse::SurveyPulseOutboxHandler::new(adapter.clone()),
        ),
    );

    let _survey_pulse = if survey_pulse_config.enabled {
        let guild_id = i64::try_from(concierge_config.main_guild_id)
            .context("SURVEY_PULSE: Guild-ID außerhalb des BIGINT-Bereichs")?;
        tracing::info!(
            interval_days = survey_pulse_config.interval_days,
            "Umfragen-Puls-Wellen aktiviert"
        );
        Some(dl_activity::survey_pulse::spawn_scheduler(
            central_pool.clone(),
            guild_id,
            survey_pulse_config,
        ))
    } else {
        tracing::info!("Umfragen-Puls-Wellen deaktiviert, Outbox-Zustellung läuft weiter");
        None
    };

    // Tag-System (6/7): Single Source of Truth, von TempVoice-Filtern genutzt
    let tag_service = dl_community::tags::TagService::new(central_pool.clone());
    // /meine-tags-Selbstverwaltung (Slash + Select/Reset-Komponenten).
    dl_community::tags_ui::register(&mut router, tag_service.clone());

    let moderation_channel_id =
        dl_moderation::moderation_channel::moderation_channel_id_from_lookup(|k| {
            operating_value(k)
        });
    let moderation_scan_channel_ids =
        dl_moderation::moderation_channel::scan_channel_ids_from_lookup(operating_value);

    // Text-Analyse und Verify-Text laufen ueber das Gate; die Bildpfade haengen
    // am VisionGenerator, den der ChatProvider (noch) nicht kann, und bleiben
    // deshalb am OpenAI-Client. Die Modell-Envs gelten unveraendert weiter.
    let moderation_text_analyze_client =
        chat_text_generator(dl_ai::LlmUseCase::ModerationText, true);
    let moderation_text_analyze_model = model_from_lookup(
        env,
        "MOD_TEXT_ANALYZE_MODEL",
        dl_ai::DEFAULT_FIREWORKS_MODEL,
    );
    let moderation_image_analyze_client =
        openai_client_with_model_from_env("MOD_IMAGE_ANALYZE_MODEL", dl_ai::DEFAULT_OPENAI_MODEL);
    let moderation_verify_vision_client =
        openai_client_with_model_from_env("MOD_VERIFY_MODEL", dl_ai::DEFAULT_OPENAI_MODEL);
    let moderation_verify_text_client =
        chat_text_generator(dl_ai::LlmUseCase::ModerationVerify, false);
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
    let moderation_enforce = operating.moderation.enforce;
    tracing::info!(
        enforce = moderation_enforce,
        "Moderation Enforcement-Modus gelesen"
    );
    let behavior_detector = dl_moderation::behavior_detector::BehaviorDetector::new(behavior_glue);

    let concierge_ai = {
        match dl_ai::LlmProviderConfig::from_env(operating_value)
            .map_err(anyhow::Error::from)
            .and_then(|cfg| {
                cfg.build_provider_for_env(dl_ai::LlmUseCase::BotPate, operating_value)
                    .map_err(anyhow::Error::from)
            }) {
            Ok(provider) => Some(provider),
            Err(err) => {
                tracing::warn!(%err, "Concierge-LLM inaktiv");
                None
            }
        }
    };
    let shared_brain_bin =
        std::path::PathBuf::from(env("BRAIN_BIN").unwrap_or_else(default_brain_bin));
    // Keep the configured source even when its binary is temporarily absent:
    // retrieval failure must become explicit coverage, never silent omission.
    let shared_game: Option<Arc<dyn dl_answer::Retriever>> =
        Some(Arc::new(modglue::BrainRetrieverGlue {
            bin: shared_brain_bin.clone(),
        }));
    let shared_answers = Arc::new(
        dl_answer::AnswerEngine::new(
            concierge_ai.clone(),
            Arc::new(dl_community::knowledge_client::CommunityRetriever {
                base_url: concierge_config.knowledge_url.clone(),
                timeout: std::time::Duration::from_secs(20),
            }),
            shared_game,
            concierge_config.ai_timeout,
        )
        .with_persona(dl_community::concierge::ANSWER_PERSONA.to_string()),
    );

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
            let raw_allowlist = operating_value("BRAIN_CHANNEL_ALLOWLIST");
            let channel_allowlist = brain_channel_allowlist_from_value(raw_allowlist.as_deref());
            if channel_allowlist.is_none() {
                tracing::warn!(
                    "Brain-Command deaktiviert: BRAIN_CHANNEL_ALLOWLIST fehlt, ist leer oder enthaelt eine ungueltige/0-Channel-ID (deny-all)"
                );
            }
            channel_allowlist.map(|channel_allowlist| {
                let cooldown_secs = env_u64_default("BRAIN_COOLDOWN_SECS", 20);
                let max_question_len = env_usize_default("BRAIN_MAX_QUESTION_LEN", 300);
                tracing::info!(
                    bin = %brain_bin_path.display(),
                    cooldown_secs,
                    max_question_len,
                    channel_allowlist = channel_allowlist.len(),
                    "Brain-Command registriert"
                );
                let config = Arc::new(dl_brain::BrainConfig {
                    max_question_len,
                    cooldown_secs,
                });
                let answerer: Arc<dyn dl_brain::AiAnswerer> =
                    Arc::new(modglue::SharedBrainAnswerer {
                        engine: shared_answers.clone(),
                    });
                Arc::new(modglue::BrainHandler {
                    adapter: adapter.clone(),
                    config,
                    cooldowns: Arc::new(dl_brain::BrainCooldowns::default()),
                    answerer,
                    channel_allowlist: Some(channel_allowlist),
                })
            })
        }
    };

    // Coaching (7): Panel postet nur noch einen Link zur Website. Die frühere
    // Discord-Anfrageaufnahme samt KI-Analyse/Rollen-/Stale-Recovery bleibt im
    // Rust-Cutover bewusst aus (Website-driven intake, #17/#18 dropped).
    // Der Website-Client wird weiter für CoachingSync (Roster/Termin-DMs)
    // geteilt. None = kein interner Token → Sync inaktiv.
    let coaching_website = dl_community::coaching::WebsiteClient::from_env(operating_value);
    let coaching_requests = dl_community::coaching_requests::CoachingRequests::new(
        central_pool.clone(),
        Arc::new(modglue::CoachingReqGlue {
            adapter: adapter.clone(),
        }),
        chat_text_generator(dl_ai::LlmUseCase::CoachingAnfrage, false),
        1289721245281292288,
        coaching_website
            .clone()
            .map(|client| client as Arc<dyn dl_community::coaching::CoachingWebsiteSyncClient>),
    );
    dl_community::coaching_requests::register(&mut router, coaching_requests.clone());
    coaching_requests.ensure_panel().await;

    // Team-Bewerbungen: öffentliches Components-V2-Panel, Modal-Aufnahme und
    // interne Beiträge in moderator-only. Der Streamer-Weg bleibt ein Website-Link.
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .context("Repository-Wurzel für Team-Bewerbungstexte fehlt")?;
    let team_applications = dl_community::team_applications::TeamApplications::new(
        central_pool.clone(),
        Arc::new(modglue::TeamApplicationGlue {
            adapter: adapter.clone(),
        }),
        our_guild_id,
        repository_root,
    );
    dl_community::team_applications::register(&mut router, team_applications.clone());
    team_applications.ensure_panel().await;
    tokio::spawn(team_applications.clone().run_maintenance_loop());

    // FAQ-Chat (6) — Panel-Buttons brauchen den Router, Subscriber gateway-gated
    let faq = dl_community::faq::FaqChat::with_answers(
        central_pool.clone(),
        Arc::new(modglue::FaqGlue {
            adapter: adapter.clone(),
        }),
        shared_answers.clone(),
    );
    dl_community::faq::register(&mut router, faq.clone());

    // Concierge-Onboarding Slice A: default AUS, T0 nur fuer Test-Allowlist.
    let concierge = dl_community::concierge::Concierge::with_answers(
        central_pool.clone(),
        Arc::new(modglue::ConciergeGlue {
            adapter: adapter.clone(),
        }),
        concierge_ai,
        concierge_config.clone(),
        shared_answers.clone(),
    );
    dl_community::concierge::register(&mut router, concierge.clone());
    // Privacy-Oberflaeche: /datenschutz + /datenschutz-optin (Loeschung/Opt-in).
    // Nach erfolgreicher Loeschung wird auch der fluechtige Concierge-Zustand entfernt.
    dl_community::privacy_ui::register(&mut router, central_pool.clone(), {
        let concierge = concierge.clone();
        Arc::new(move |user_id| concierge.clear_user_runtime(user_id))
    });

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

    // Alt-Buttons bleiben klickbar (kein Rollen-Grant). Steam-Login bleibt.
    onboardglue::register(
        &mut router,
        dl_bridges::steam::SteamBotClient::from_env(operating_value),
    );

    // Moderation (6) — Review-Buttons brauchen den Router, Scan ist gateway-gated.
    // Text-Analyze laeuft ueber Fireworks, Bild-Analyze und Verify ueber OpenAI nano.
    let moderator = match (
        moderation_text_analyze_client.clone(),
        moderation_image_analyze_client.clone(),
        moderation_verify_vision_client.clone(),
        moderation_verify_text_client.clone(),
    ) {
        (
            Some(analyze_text),
            Some((image_analyze_client, image_analyze_model)),
            Some((verify_client, verify_model)),
            Some(verify_text),
        ) => {
            let analyze_vision: Arc<dyn dl_ai::VisionGenerator> = image_analyze_client;
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
    let command_sync_config = master::CommandSyncStartupConfig::from_lookup(operating_value);
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

    let scrim_lagebild_ai = match dl_ai::LlmProviderConfig::from_env(operating_value) {
        Ok(cfg) => {
            match cfg.build_provider_for_env(dl_ai::LlmUseCase::ScrimLagebild, operating_value) {
                Ok(provider) => Some(provider),
                Err(err) => {
                    tracing::warn!(%err, "Scrim-Lagebild-AI im Bot inaktiv");
                    None
                }
            }
        }
        Err(err) => {
            tracing::warn!(%err, "Scrim-Lagebild-AI-Konfiguration im Bot ungueltig");
            None
        }
    };

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
        broker_token.clone(),
        operating_value,
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let broker_host =
        operating_value("MASTER_BROKER_HOST").unwrap_or_else(|| "127.0.0.1".to_string());
    let broker_addr = format!("{broker_host}:{}", cfg.ports.master_broker);
    let broker_listener = tokio::net::TcpListener::bind(&broker_addr)
        .await
        .with_context(|| format!("Broker-Port binden: {broker_addr}"))?;
    tracing::info!(addr = %broker_addr, "Master-Broker gebunden");
    let lagebild_token = env("TURNIER_INTERNAL_API_TOKEN").unwrap_or_default();
    warn_if_lagebild_token_empty(&lagebild_token);
    let broker_server = axum::serve(
        broker_listener,
        dl_broker::router(broker)
            .merge(turnierglue::publisher_router(
                turnier_proposals,
                broker_token,
            ))
            .merge(scrim_adapter::lagebild_router(
                scrim_adapter::LagebildApiState {
                    provider: scrim_lagebild_ai.clone(),
                    channel_history: Some(Arc::new(scrim_adapter::DiscordChannelHistory::new(
                        adapter.as_ref(),
                    ))),
                    token: lagebild_token,
                    pool: central_pool.clone(),
                },
            ))
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
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
        mcp::McpState::from_env(discord_token.clone(), operating_value)
            .context("MCP-Connector-State")?,
    );
    let mcp_addr = mcp::McpState::bind_addr(operating_value);
    let mcp_listener = tokio::net::TcpListener::bind(&mcp_addr)
        .await
        .with_context(|| format!("MCP-Connector-Port binden: {mcp_addr}"))?;
    tracing::info!(addr = %mcp_addr, "MCP-Connector gebunden");
    let mcp_server = axum::serve(mcp_listener, mcp::router(mcp_state));

    let scrim_voice_config = scrimglue::ScrimVoiceConfig {
        enabled: env_bool_default("DL_SCRIM_VISIBLE_VCS_ENABLED", false),
        category_id: NonZeroU64::new(env_u64_default("DL_SCRIM_VISIBLE_VCS_CATEGORY_ID", 0))
            .map(NonZeroU64::get),
    };
    let mut scrim_match_driver = scrimglue::spawn(
        central_pool.clone(),
        adapter.clone(),
        tempvoice.clone(),
        scrim_voice_config,
        scrim_runtime_gate,
        scrim_lagebild_ai,
    );

    // Gateway: user-gated — Python hält die Session bis zum Cutover
    let gateway_enabled = operating_value("DL_BOT_GATEWAY").as_deref() == Some("1");
    let mut gateway_task = if gateway_enabled {
        let voice_worker_token = env("DISCORD_TOKEN_RANKED");
        let voice_worker_token =
            validate_voice_worker_token(&discord_token, voice_worker_token.as_deref())
                .map(str::to_owned)
                .map_err(anyhow::Error::msg)?;
        let presence_intent_enabled = operating.runtime.start.presence_intent.unwrap_or(false);
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
        let _scrim_record_tasks =
            dl_voice::scrim_record::spawn(scrim_recorder.clone(), &dispatcher);

        // Serverweiter Voice-Pair-Guard vor den TempVoice-spezifischen Subscribern.
        dl_voice::voice_pair_guard::spawn(voice_pair_guard.clone(), &dispatcher);

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
                voice_pair_store: voice_pair_store.clone(),
                voice_pair_operations: voice_pair_operations.clone(),
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

        // Steam-Link-Nudge (4c): DM nach 30 min Voice am zweiten Tag.
        // Opt-in, damit der automatische DM-Nudge produktiv standardmäßig aus bleibt.
        if steam_voice_nudge_enabled {
            dl_voice::nudge::spawn_restore(nudge.clone());
            dl_voice::nudge::spawn(nudge.clone(), &dispatcher);
            // !nudgesend/!t30-Admin-Test: schickt die Nudge-DM an ein Ziel.
            dl_voice::nudge::spawn_command(nudge.clone(), &dispatcher, adapter.clone());
        } else {
            tracing::info!(
                "Steam-Link-Voice-Nudge deaktiviert (DL_STEAM_VOICE_NUDGE_ENABLED nicht gesetzt)"
            );
        }
        dl_voice::solo_watch::spawn(solo_watch.clone(), &dispatcher);
        if lane_pairing_enabled {
            dl_voice::pairing::spawn(lane_pairing.clone());
        } else {
            tracing::warn!("Lane-Pairing deaktiviert (DL_LANE_PAIRING_ENABLED=0)");
        }
        // Voice-Feedback: Freitext-Antworten auf Feedback-DMs.
        dl_voice::feedback::spawn_dm_responses(
            voice_feedback.clone(),
            &dispatcher,
            adapter.clone(),
        );

        // Player-Finder (5): portiert, aber per Flag deaktiviert (Redesign geplant)
        if dl_activity::player_finder::enabled(operating_value) {
            let _finder = dl_activity::player_finder::PlayerFinder::new(central_pool.clone());
            tracing::warn!(
                "PLAYER_FINDER_ENABLED=1 gesetzt — Kern portiert, Message-Flow folgt mit dem Redesign"
            );
        } else {
            tracing::info!("Player-Finder deaktiviert (PLAYER_FINDER_ENABLED nicht gesetzt)");
        }

        match dl_activity::lfg_freetext::config_from_lookup(env) {
            Ok(None) => {
                tracing::info!("LFG-Freitext deaktiviert (DL_LFG_FREITEXT_ENABLED nicht gesetzt)")
            }
            Err(error) => {
                tracing::warn!(%error, "LFG-Freitext deaktiviert: ungueltige Konfiguration")
            }
            Ok(Some(config)) => match dl_ai::LlmProviderConfig::from_env(env) {
                Err(error) => {
                    tracing::warn!(%error, "LFG-Freitext deaktiviert: LLM-Konfiguration ungueltig")
                }
                Ok(provider_config) => match provider_config
                    .build_provider_for_env(dl_ai::LlmUseCase::LfgFreitext, env)
                {
                    Err(error) => {
                        tracing::warn!(%error, "LFG-Freitext deaktiviert: LLM-Provider nicht verfuegbar")
                    }
                    Ok(provider) => {
                        let channel_id = config.channel_id;
                        let handler = dl_activity::lfg_freetext::FreetextLfg::new(
                            config,
                            central_pool.clone(),
                            provider,
                            Arc::new(modglue::LfgFreetextGlue {
                                adapter: adapter.clone(),
                            }),
                        );
                        let _lfg_freetext = dl_activity::lfg_freetext::spawn(handler, &dispatcher);
                        tracing::info!(channel_id, "LFG-Freitext aktiviert");
                    }
                },
            },
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
        let _rollback_artifact_retention =
            dl_community::privacy::spawn_rollback_artifact_retention(central_pool.clone());
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
            dl_community::voice_change_hint::enabled_from_lookup(operating_value);
        let voice_hint_classifier = if voice_hint_enabled {
            chat_text_generator(dl_ai::LlmUseCase::VoiceHint, false).map(|generator| {
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
        // Freitext-DMs beantwortet der Concierge. Der frühere KI-DM-Assistent
        // startete nur, wenn der Concierge nicht für alle offen war, und ist
        // seit DL_CONCIERGE_ENABLED=1 mit leerer Allowlist toter Code gewesen.
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
        dl_voice::status::spawn_routing(status_worker.clone(), &dispatcher);
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
                command_prefix: operating_value("COMMAND_PREFIX")
                    .unwrap_or_else(|| "!".to_string()),
                enable_presence_intent: presence_intent_enabled,
                recording_readiness: recording_main_readiness.clone(),
                recording_guild_id: dl_server_as_code::DEFAULT_GUILD_ID,
            },
            recording_main_manager.clone(),
        )
        .await
        .context("Gateway-Client bauen")?;
        // KRITISCH: serenity legt beim Build einen EIGENEN Cache an. Ohne diese
        // Kopplung läse die gesamte Glue aus einem leeren Adapter-Cache (alle
        // Voice-/Channel-/Member-Lookups None → TempVoice baut keine Lanes usw.).
        adapter.link_cache(client.cache.clone());
        let mut voice_worker_client = dl_discord::gateway::build_voice_worker_client(
            &voice_worker_token,
            recording_worker_manager.clone(),
            recording_worker_readiness.clone(),
            dl_server_as_code::DEFAULT_GUILD_ID,
        )
        .await
        .context("Scrim-Record-Voice-Worker bauen")?;
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
        // Wer beim Neustart schon im Router-VC saß, bekommt kein Join-Event
        // mehr; ohne diesen Anstoß wartet er dort ewig.
        let mut auto_move_cache_ready = dispatcher.subscribe_gateway();
        let router_after_restart = lane_router.clone();
        tokio::spawn(async move {
            wait_for_gateway_cache_ready(
                &mut auto_move_cache_ready,
                dl_voice::router::ROUTER_GUILD_ID,
                "router_auto_move",
            )
            .await;
            router_after_restart
                .prime_auto_move(dl_voice::router::ROUTER_GUILD_ID)
                .await;
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
        Some(tokio::spawn(async move {
            let _main_readiness_reset = ReadinessReset(recording_main_readiness);
            let _worker_readiness_reset = ReadinessReset(recording_worker_readiness);
            tokio::select! {
                result = client.start() => result,
                result = voice_worker_client.start() => result,
            }
        }))
    } else {
        tracing::info!("Gateway inaktiv (DL_BOT_GATEWAY != 1) — nur REST/Broker/Changelog");
        None
    };

    tracing::info!("dl-bot läuft — beenden mit Ctrl+C");
    let mut restart_requested = false;
    let mut terminate_signal =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("SIGTERM-Handler fuer kontrollierten Shutdown registrieren")?;
    let run_result: anyhow::Result<()> = tokio::select! {
        result = broker_server => result.context("Broker-Server"),
        result = changelog_server => result.context("Changelog-Server"),
        result = serversync_server => result.context("Server-Sync-Server"),
        result = mcp_server => result.context("MCP-Connector-Server"),
        result = async {
            match gateway_task.as_mut() {
                Some(task) => task.await,
                None => std::future::pending::<
                    Result<serenity::Result<()>, tokio::task::JoinError>,
                >().await,
            }
        } => {
            match result.context("Gateway-Task").and_then(|result| result.context("Gateway-Clients")) {
                Ok(()) => Err(anyhow::anyhow!("Gateway-Clients unerwartet beendet")),
                Err(err) => Err(err),
            }
        },
        result = &mut scrim_match_driver => {
            match result.context("Scrim-Match-Treiber") {
                Ok(()) => Err(anyhow::anyhow!("Scrim-Match-Treiber beendet")),
                Err(err) => Err(err),
            }
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
            Ok(())
        },
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("dl-bot beendet");
            Ok(())
        },
        _ = terminate_signal.recv() => {
            tracing::info!("dl-bot per SIGTERM beendet");
            Ok(())
        },
    };
    scrim_recorder.shutdown().await;
    scrim_match_driver.abort();
    if let Some(task) = gateway_task {
        task.abort();
    }
    run_result?;
    if restart_requested {
        // Kein process::exit: normaler Return laesst PidLock::drop laufen,
        // der Non-Zero-Code triggert systemd Restart=on-failure.
        return Ok(master::restart_exit_code());
    }
    Ok(std::process::ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_operating_snapshot_reaches_community_moderation_and_ai_constructors() {
        let config = dl_core::bot_config::BotConfig::parse(
            r#"
schema_version=1
[discord]
guild_id="1234"
[runtime.community]
concierge_enabled=true
concierge_test_users=[55,66]
concierge_free_voice=false
survey_pulse=true
survey_interval_days=30
lfg_freetext=true
lfg_freetext_channel_id=777
[runtime.moderation]
channel_id=888
scan_channel_ids=[111,222]
[runtime.ai]
transparency_enabled=false
[llm.use_cases.bot_pate]
model="accounts/fireworks/models/deepseek-v4-flash-0731"
"#,
        )
        .expect("synthetische Betriebskonfiguration");
        let lookup = |key: &str| config.runtime_value(key);
        let concierge = dl_community::concierge::ConciergeConfig::from_env(lookup);
        assert!(concierge.enabled);
        assert!(!concierge.free_voice);
        assert_eq!(concierge.main_guild_id, 1234);
        assert_eq!(concierge.test_user_allowlist.len(), 2);
        let survey = dl_activity::survey_pulse::SurveyPulseConfig::from_lookup(lookup);
        assert!(survey.enabled);
        assert_eq!(survey.interval_days, 30);
        assert!(dl_activity::lfg_freetext::config_from_lookup(lookup)
            .expect("LFG-Konfiguration")
            .is_some());
        assert_eq!(
            dl_moderation::moderation_channel::moderation_channel_id_from_lookup(lookup),
            888
        );
        assert_eq!(
            dl_moderation::moderation_channel::scan_channel_ids_from_lookup(lookup),
            vec![111, 222]
        );
        assert!(!dl_ai::TransparencyConfig::from_env(lookup).enabled);
        let ai = dl_ai::LlmProviderConfig::from_env(lookup).expect("bestehender Anbieter");
        assert_eq!(
            ai.provider_for(dl_ai::LlmUseCase::BotPate, lookup)
                .expect("Anbieter"),
            dl_ai::LlmProviderKind::Fireworks
        );
        assert_eq!(
            lookup("DL_LLM_MODEL_BOT_PATE").as_deref(),
            Some("accounts/fireworks/models/deepseek-v4-flash-0731")
        );
        assert!(lookup("DL_LLM_PROVIDER_BOT_PATE").is_none());
        let global = dl_core::bot_config::BotConfig::parse(&format!(
            "schema_version=1\n[llm]\ndefault_provider='openai'\n[llm.use_cases.bot_pate]\nmodel='{}'\n",
            dl_ai::DEFAULT_OPENAI_MODEL,
        )).expect("reiner Pin mit bestehendem globalen Override");
        let global_lookup = |key: &str| global.runtime_value(key);
        let global_ai =
            dl_ai::LlmProviderConfig::from_env(global_lookup).expect("globaler Anbieter");
        assert_eq!(
            global_ai
                .provider_for(dl_ai::LlmUseCase::BotPate, global_lookup)
                .expect("Anbieter"),
            dl_ai::LlmProviderKind::OpenAi
        );
        assert!(global_lookup("DL_LLM_PROVIDER_BOT_PATE").is_none());
    }

    use super::{
        brain_channel_allowlist_from_value, chat_text_generator_with, legacy_lfg_responder_enabled,
        lfg_cutover_active, lfg_forum_channel_id_from_value, lfg_panel_channel_id_from_value,
        matcher_provider_choice, model_from_lookup, moderation_enforce_from_lookup,
        validate_voice_worker_token, warn_if_lagebild_token_empty, MatcherProviderChoice,
    };
    use std::{
        collections::HashMap,
        io::Write,
        sync::{Arc, Mutex},
    };

    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for LogWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log buffer").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn lookup<'a>(vars: &'a HashMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + 'a {
        |key| vars.get(key).map(|value| (*value).to_string())
    }

    #[test]
    fn empty_lagebild_token_logs_that_endpoint_rejects_all_requests() {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || LogWriter(writer.clone()))
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .finish();

        tracing::subscriber::with_default(subscriber, || warn_if_lagebild_token_empty(""));

        let logs = logs.lock().expect("log buffer");
        let logs = String::from_utf8_lossy(&logs);
        assert!(logs.contains("TURNIER_INTERNAL_API_TOKEN"));
        assert!(logs.contains("endpoint will reject every request with 401"));
    }

    #[test]
    fn alle_drei_wissenseingaenge_teilen_die_gegatete_antwortinstanz() {
        let source = include_str!("main.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("Quelldatei enthält Produktionsbereich");
        assert_eq!(source.matches("dl_answer::AnswerEngine::new(").count(), 1);
        assert!(source.contains("cfg.build_provider_for_env(dl_ai::LlmUseCase::BotPate"));
        assert!(source.contains("FaqChat::with_answers("));
        assert!(source.contains("Concierge::with_answers("));
        assert!(source.contains("SharedBrainAnswerer"));
        assert_eq!(source.matches("shared_answers.clone()").count(), 3);
        assert!(!source.contains("BrainAiGlue"));
        assert!(!source.contains("chat_text_generator(dl_ai::LlmUseCase::Faq"));
    }

    /// Der zweite KI-Weg ist zu, solange keine Quelle einen LLM-Client selbst
    /// aus der Umgebung baut. Gelesen wird das Verzeichnis, nicht eine Liste —
    /// eine neue Datei kann den Waechter sonst umgehen.
    #[test]
    fn dl_bot_baut_keine_llm_clients_mehr_selbst() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let forbidden = [
            ["MiniMaxClient::", "from_env("].concat(),
            ["FireworksClient::", "from_env("].concat(),
            ["GeminiClient::", "from_env("].concat(),
            ["OpenAiClient::text_", "from_env("].concat(),
        ];
        let entries = std::fs::read_dir(&src_dir).expect("dl-bot-Quellverzeichnis");
        let mut checked = 0_usize;
        for entry in entries {
            let path = entry.expect("Verzeichniseintrag").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("Quelldatei lesbar");
            for needle in &forbidden {
                assert!(
                    !source.contains(needle.as_str()),
                    "{} baut noch einen LLM-Client direkt ({needle}) — der Weg muss über das Compliance-Gate laufen",
                    path.display()
                );
            }
            checked += 1;
        }
        assert!(checked > 1, "es müssen mehrere Quelldateien geprüft werden");

        // Ausnahme mit Grund: der Bildpfad der Moderation haengt am
        // VisionGenerator, den der ChatProvider nicht anbietet.
        let main_source = include_str!("main.rs");
        let vision_client = ["OpenAiClient::", "new("].concat();
        assert_eq!(
            main_source.matches(vision_client.as_str()).count(),
            1,
            "nur der Vision-Pfad darf noch einen OpenAI-Client direkt bauen"
        );
    }

    /// Der Kern des Transparenz-Logs lag einmal vollstaendig im Baum, ohne dass
    /// `aiglue` in einem `mod` hing: alle Modultests waren gruen, und im
    /// laufenden Bot wurde trotzdem nie eine Zeile in den Kanal geschrieben.
    /// Dieser Test prueft deshalb nicht das Verhalten der Bausteine, sondern
    /// dass der Start sie ueberhaupt anfasst.
    #[test]
    fn start_registriert_die_transparenz_senke_und_das_inventar() {
        // Die Nadeln werden zusammengesetzt: stuenden sie hier am Stueck, faende
        // der Test sich selbst im eingebetteten Quelltext und bliebe auch dann
        // gruen, wenn der Start die Verdrahtung verloren hat.
        let main_source = include_str!("main.rs");
        for (needle, warum) in [
            (
                ["mod ", "aiglue;"].concat(),
                "ohne Modul-Einbindung wird der Discord-Weg nie kompiliert",
            ),
            (
                ["set_transparency", "_sink("].concat(),
                "ohne registrierte Senke verwirft der Decorator jede Interaktion",
            ),
            (
                ["TransparencyLog::", "spawn("].concat(),
                "ohne laufenden Sender bleibt die Warteschlange stehen",
            ),
            (
                ["log_ai_startup", "_inventory("].concat(),
                "ohne Inventar faellt ein KI-Pfad ohne Anbieter still aus",
            ),
        ] {
            assert!(
                main_source.contains(needle.as_str()),
                "dl-bot verdrahtet das KI-Transparenz-Log nicht mehr: `{needle}` fehlt — {warum}"
            );
        }
    }

    #[test]
    fn gate_verweigert_minimax_fuer_nutzertexte_und_laesst_erlaubten_anbieter_durch() {
        let gesperrt = HashMap::from([
            ("DL_LLM_PROVIDER_FAQ".to_string(), "minimax".to_string()),
            ("MINIMAX_API_KEY".to_string(), "geheim".to_string()),
            ("MISTRAL_API_KEY".to_string(), "geheim".to_string()),
        ]);
        assert!(
            chat_text_generator_with(dl_ai::LlmUseCase::Faq, false, |key| gesperrt
                .get(key)
                .cloned())
            .is_none(),
            "MiniMax muss für Nutzertexte am Gate scheitern"
        );

        let erlaubt = HashMap::from([
            ("DL_LLM_PROVIDER_FAQ".to_string(), "mistral".to_string()),
            ("MISTRAL_API_KEY".to_string(), "geheim".to_string()),
        ]);
        assert!(
            chat_text_generator_with(dl_ai::LlmUseCase::Faq, false, |key| erlaubt
                .get(key)
                .cloned())
            .is_some(),
            "derselbe Anwendungsfall muss mit einem erlaubten Anbieter funktionieren"
        );
    }

    #[test]
    fn gate_erlaubt_minimax_nur_mit_dem_dev_schluessel() {
        let mut vars = HashMap::from([
            ("DL_LLM_PROVIDER_FAQ".to_string(), "minimax".to_string()),
            ("MINIMAX_API_KEY".to_string(), "geheim".to_string()),
        ]);
        assert!(
            chat_text_generator_with(dl_ai::LlmUseCase::Faq, false, |key| vars.get(key).cloned())
                .is_none()
        );

        vars.insert(
            "DL_LLM_ALLOW_MINIMAX_USER_CONTENT_DEV_ONLY".to_string(),
            "ich-weiss-was-ich-tue".to_string(),
        );
        assert!(
            chat_text_generator_with(dl_ai::LlmUseCase::Faq, false, |key| vars.get(key).cloned())
                .is_some(),
            "die Dev-Ausnahme muss weiterhin greifen"
        );
    }

    #[test]
    fn modellwahl_bleibt_an_der_bisherigen_env() {
        let vars = HashMap::from([
            (
                "TURNIER_AI_MODEL".to_string(),
                "gpt-eigenes-turniermodell".to_string(),
            ),
            ("MOD_TEXT_ANALYZE_MODEL".to_string(), "  ".to_string()),
        ]);
        let lookup = |key: &str| vars.get(key).cloned();

        assert_eq!(
            model_from_lookup(lookup, "TURNIER_AI_MODEL", dl_ai::DEFAULT_OPENAI_MODEL),
            "gpt-eigenes-turniermodell"
        );
        assert_eq!(
            model_from_lookup(
                lookup,
                "MOD_TEXT_ANALYZE_MODEL",
                dl_ai::DEFAULT_FIREWORKS_MODEL
            ),
            dl_ai::DEFAULT_FIREWORKS_MODEL,
            "leere Werte dürfen nicht als Modellname durchgehen"
        );
        assert_eq!(
            model_from_lookup(lookup, "MOD_VERIFY_MODEL", dl_ai::DEFAULT_OPENAI_MODEL),
            dl_ai::DEFAULT_OPENAI_MODEL
        );
    }

    #[test]
    fn matcher_anbieterwahl_bleibt_an_streamer_link_ai_provider() {
        assert!(matches!(
            matcher_provider_choice(Some("openai".to_string())),
            MatcherProviderChoice::Gate(Some(provider)) if provider == "openai"
        ));
        assert!(matches!(
            matcher_provider_choice(Some("MiniMax".to_string())),
            MatcherProviderChoice::Gate(Some(provider)) if provider == "minimax"
        ));
        assert!(matches!(
            matcher_provider_choice(None),
            MatcherProviderChoice::Gate(None)
        ));
        assert!(matches!(
            matcher_provider_choice(Some("   ".to_string())),
            MatcherProviderChoice::Gate(None)
        ));
        // gemini kennt das Gate nicht — frueher der `_ => NoAi`-Zweig.
        assert!(matches!(
            matcher_provider_choice(Some("gemini".to_string())),
            MatcherProviderChoice::Off(raw) if raw == "gemini"
        ));
    }

    #[test]
    fn moderation_enforce_defaults_to_shadow_when_all_vars_are_unset() {
        let vars = HashMap::new();

        assert!(!moderation_enforce_from_lookup(lookup(&vars)));
    }

    #[test]
    fn voice_worker_token_must_exist_and_differ_from_main_token() {
        assert_eq!(
            validate_voice_worker_token("main", Some("worker")),
            Ok("worker")
        );
        assert!(validate_voice_worker_token("main", None).is_err());
        assert!(validate_voice_worker_token("main", Some("")).is_err());
        assert!(validate_voice_worker_token("main", Some("main")).is_err());
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
    fn brain_startup_aktiviert_nur_nichtleere_gueltige_allowlist() {
        for raw in [None, Some(""), Some(" \n\t"), Some("nope"), Some("0")] {
            assert_eq!(brain_channel_allowlist_from_value(raw), None, "{raw:?}");
        }
        for raw in [Some("123, nope, 456"), Some("123, 0, 456")] {
            assert_eq!(brain_channel_allowlist_from_value(raw), None, "{raw:?}");
        }
        assert_eq!(
            brain_channel_allowlist_from_value(Some("123, 456")),
            Some(std::collections::HashSet::from([123, 456]))
        );
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
