//! Onboarding-Buttons — Cutover-kritischer Kern aus `cogs/welcome_dm/` +
//! `cogs/onboarding.py`.
//!
//! Die Onboarding-Nachrichten mit ihren persistenten Buttons existieren
//! bereits im Regelkanal; nach dem Cutover müssen ihre custom_ids weiter
//! funktionieren. Portiert: Regelbestätigung (`wdm:q4:confirm` →
//! Onboarding-Rolle — DAS Zugangs-Gate für neue Mitglieder), Steam-Login
//! (`steam:openid`) und die DM-Assistent-Hinweise (`dma:fallback:*`,
//! Texte wortgleich). Die Schritt-Navigation des Kanal-Flows
//! (`wdm:q0/qS/...`) antwortet ehrlich mit einem Hinweis — der volle
//! Flow folgt mit dem Onboarding-Rest (Phase 7).

use std::sync::Arc;

use dl_discord::{BridgeInteraction, BridgeReply, DiscordAdapter, InteractionHandler};
use serde_json::json;
use serenity::all::{ChannelId, GuildId, RoleId, UserId};

pub const ONBOARD_COMPLETE_ROLE_ID: u64 = 1304216250649415771;
pub const MAIN_GUILD_ID: u64 = 1289721245281292288;

pub struct OnboardingHandler {
    pub adapter: Arc<DiscordAdapter>,
    pub steam: Arc<dl_bridges::steam::SteamBotClient>,
}

#[async_trait::async_trait]
impl InteractionHandler for OnboardingHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        match interaction.custom_id.as_str() {
            // Regelbestätigung → Onboarding-Rolle (Zugangs-Gate)
            "wdm:q4:confirm" => {
                let guild_id = if interaction.guild_id > 0 {
                    interaction.guild_id
                } else {
                    MAIN_GUILD_ID
                };
                match self
                    .adapter
                    .http
                    .add_member_role(
                        GuildId::new(guild_id),
                        UserId::new(interaction.user_id),
                        RoleId::new(ONBOARD_COMPLETE_ROLE_ID),
                        Some("Welcome DM: Regeln bestätigt"),
                    )
                    .await
                {
                    Ok(()) => BridgeReply::ephemeral_text("✅ Danke! Willkommen an Bord!"),
                    Err(err) => {
                        tracing::warn!(%err, user_id = interaction.user_id, "Onboarding: Rollen-Vergabe fehlgeschlagen");
                        BridgeReply::ephemeral_text(
                            "Regeln bestätigt — die Rolle konnte gerade nicht vergeben werden, das Team schaut drauf.",
                        )
                    }
                }
            }

            // Frischer Steam-Login-Link (Einmal-URL vom Steam-Bot)
            "steam:openid" => match self.steam.fetch_steam_link_url(interaction.user_id).await {
                Some(url) => BridgeReply {
                    content: Some(
                        "Melde dich kurz bei Steam an (OpenID — kein Passwort nötig):".to_string(),
                    ),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 2, "style": 5, "label": "Mit Steam anmelden",
                        "emoji": {"name": "🎮"}, "url": url,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                },
                None => BridgeReply::ephemeral_text(
                    "Der Link-Dienst ist gerade nicht verfügbar. Nutze vorerst **/account_verknüpfen**.",
                ),
            },

            // DM-Assistent-Hinweise (Texte wortgleich zum Original)
            "dma:fallback:steam" => BridgeReply::ephemeral_text(
                "Verknüpfe deinen Steam-Account mit **/steamlink** im Server.\nDas ist wichtig für Rang, Live-Status in den Voice Lanes und Spielersuche.",
            ),
            "dma:fallback:faq" => BridgeReply::ephemeral_text(
                "Nutze **/faq** oder **/serverfaq <frage>** im Server für häufig gestellte Fragen.\nDu kannst mir hier auch direkt deine Frage stellen!",
            ),
            "dma:fallback:streamer" => BridgeReply::ephemeral_text(
                "Nutze **/streamer** im Server, um den Streamer-Partner-Prozess zu starten!\nAls Partner bekommst du: Auto-Raid, Chat Guard, Analytics und mehr.",
            ),
            "dma:fallback:beta" => BridgeReply::ephemeral_text(
                "Für einen Deadlock Beta-Invite nutze **/betainvite** im Server.\nAlternativ schau im <#1428745737323155679> Channel vorbei.",
            ),

            // Schritt-Navigation: Flow-Steuerung folgt mit dem Onboarding-Rest
            _ => BridgeReply::ephemeral_text(
                "Dieser Onboarding-Schritt wird gerade umgebaut — die Regeln kannst du oben bestätigen, alles andere findest du in den Server-Kanälen.",
            ),
        }
    }
}

pub fn register(
    router: &mut dl_discord::InteractionRouter,
    adapter: Arc<DiscordAdapter>,
    steam: Arc<dl_bridges::steam::SteamBotClient>,
) {
    let handler = Arc::new(OnboardingHandler { adapter, steam });
    for custom_id in [
        "wdm:q4:confirm",
        "wdm:q0:intro_next",
        "wdm:q1:masterbot",
        "wdm:q2:servertour",
        "wdm:qS:next",
        "wdm:qS:status",
        "steam:openid",
        "steam:next",
        "dma:fallback:steam",
        "dma:fallback:faq",
        "dma:fallback:streamer",
        "dma:fallback:beta",
    ] {
        router.on_custom_id(custom_id, handler.clone());
    }
}

// ── Onboarding-Wizard-Anbindung ────────────────────────────────────────────

pub struct WizardGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub tags: Arc<dl_community::tags::TagService>,
    pub steam: Arc<dl_bridges::steam::SteamBotClient>,
}

#[async_trait::async_trait]
impl dl_community::onboarding::OnboardingPort for WizardGlue {
    async fn create_onboarding_thread(
        &self,
        _guild_id: u64,
        user_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        // privater Thread (type 12), 60-min-Auto-Archiv, invitable — wie Original
        let body = serde_json::json!({
            "name": name,
            "type": 12,
            "auto_archive_duration": 60,
            "invitable": true,
        });
        let thread = self
            .adapter
            .http
            .create_thread(
                ChannelId::new(dl_community::onboarding::RULES_CHANNEL_ID),
                body.as_object().expect("json object"),
                Some("Onboarding-Thread"),
            )
            .await
            .map_err(|e| e.to_string())?;
        let _ = self
            .adapter
            .http
            .add_thread_channel_member(thread.id, UserId::new(user_id))
            .await;
        Ok(thread.id.get())
    }

    async fn send_step(
        &self,
        channel_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), serde_json::json!([embed]));
        body.insert("components".into(), components);
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.display_name().to_string())
            })
            .unwrap_or_else(|| format!("User {user_id}"))
    }

    async fn steam_link_url(&self, user_id: u64) -> Option<String> {
        self.steam.fetch_steam_link_url(user_id).await
    }

    async fn set_user_tag(&self, user_id: u64, key: &str, value: &str) {
        if let Err(err) = self.tags.set_user_tag(user_id, key, value).await {
            tracing::warn!(%err, user_id, key, "Onboarding-Tag konnte nicht gesetzt werden");
        }
    }
}
