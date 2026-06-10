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
use serenity::all::{GuildId, RoleId, UserId};

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
