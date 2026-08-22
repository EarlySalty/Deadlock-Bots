//! Übrig gebliebene Onboarding-Buttons.
//!
//! Die alten custom_ids bleiben registriert, damit Discord nicht
//! "Interaction failed" zeigt. Sie vergeben keine Rolle mehr.
//! Steam-Login bleibt, das ist Account-Verknüpfung und kein Einstiegs-Flow.

use std::sync::Arc;

use dl_community::onboarding::LEGACY_ONBOARDING_RETIRED_HINT;
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler};
use serde_json::json;

pub const MAIN_GUILD_ID: u64 = dl_community::onboarding::MAIN_GUILD_ID;

pub struct OnboardingHandler {
    pub steam: Arc<dl_bridges::steam::SteamBotClient>,
}

#[async_trait::async_trait]
impl InteractionHandler for OnboardingHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        match interaction.custom_id.as_str() {
            "steam:openid" => match self.steam.fetch_steam_link_url(interaction.user_id).await {
                Some(url) => BridgeReply {
                    content: Some(
                        "Melde dich kurz bei Steam an (OpenID, kein Passwort nötig):".to_string(),
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
            _ => BridgeReply::ephemeral_text(LEGACY_ONBOARDING_RETIRED_HINT),
        }
    }
}

pub fn register(
    router: &mut dl_discord::InteractionRouter,
    steam: Arc<dl_bridges::steam::SteamBotClient>,
) {
    let handler = Arc::new(OnboardingHandler { steam });
    router.on_custom_id("steam:openid", handler.clone());
    router.on_custom_id("steam:next", handler.clone());
    router.on_custom_id("rp:panel:start", handler.clone());
    router.on_prefix("wdm:", handler.clone());
    router.on_prefix("aiob:", handler.clone());
    router.on_prefix("ob:", handler.clone());
    router.on_prefix("dma:", handler);
}
