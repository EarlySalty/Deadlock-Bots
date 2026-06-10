//! Onboarding-Wizard — Port von `cogs/rules_channel.py` + `cogs/onboarding.py`
//! (StaticOnboarding).
//!
//! „Hier starten ➜" (`rp:panel:start`, Vertrag) öffnet einen privaten Thread
//! im Regelkanal und führt durch 10 Schritte (Texte byte-genau aus dem
//! Original extrahiert, eingebettet als JSON). Schritt 2 (Streamer) sehen
//! nur Content-Creator; Schritt 7/8 setzen optionale Tone-/Age-Tags über
//! das Tag-System; Schritt 9 bietet den Steam-Link an.
//!
//! Bewusste Annäherungen (dokumentiert):
//! - Rust vergibt stabile custom_ids (`ob:*`) — die Original-Views leben
//!   nur im RAM (1-h-Timeout, kein Restart-Persist), es gibt also keinen
//!   Bestands-Vertrag zu brechen; Rust-Wizard-Buttons überleben Restarts.
//! - Statt des Verifikations-Watchers (member_update) gibt es im
//!   Account-Schritt einen „Ich hab verknüpft ➜"-Button, der die Rolle
//!   live prüft.
//! - Das Streamer-Setup (step_streamer) ist ein eigener, noch nicht
//!   portierter Flow — der Schritt zeigt die Infos und verweist weiter.

use std::sync::Arc;

use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};

pub const RULES_CHANNEL_ID: u64 = 1315684135175716975;
pub const VERIFIED_ROLE_ID: u64 = 1419608095533043774;
pub const CONTENT_CREATOR_ROLE_ID: u64 = 1466630749255106590;
pub const STREAMER_STEP: usize = 2;
pub const TONE_STEP: usize = 7;
pub const AGE_STEP: usize = 8;
pub const ACCOUNT_STEP: usize = 9;

/// Schritt-Texte, byte-genau aus `cogs/onboarding.py::STEPS` extrahiert.
const STEPS_JSON: &str = include_str!("onboarding_steps.json");

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Step {
    pub title: String,
    pub description: String,
    pub color: u32,
    #[serde(default)]
    pub id: String,
}

pub fn steps() -> &'static [Step] {
    static STEPS: std::sync::OnceLock<Vec<Step>> = std::sync::OnceLock::new();
    STEPS.get_or_init(|| serde_json::from_str(STEPS_JSON).expect("onboarding_steps.json valide"))
}

/// Embed eines Schritts mit dynamischem Footer (wie `_build_embed`).
pub fn build_step_embed(step_index: usize, is_streamer: bool) -> Value {
    let step = &steps()[step_index];
    let total = if is_streamer { 10 } else { 9 };
    let mut display = step_index + 1;
    if !is_streamer && step_index > STREAMER_STEP {
        display -= 1;
    }
    json!({
        "title": step.title,
        "description": step.description,
        "color": step.color,
        "footer": { "text": format!("Deutsche Deadlock Community · Schritt {display} / {total}") },
    })
}

/// Buttons je Schritt (custom_ids tragen Schritt + Besitzer).
pub fn build_step_components(step_index: usize, user_id: u64, verified: bool) -> Value {
    let last = steps().len() - 1;
    if step_index == TONE_STEP {
        return json!([{ "type": 1, "components": [
            { "type": 2, "style": 1, "label": "Banter-OK",
              "custom_id": format!("ob:tone:banter_ok:{step_index}:{user_id}") },
            { "type": 2, "style": 2, "label": "Ragebaiter-Free",
              "custom_id": format!("ob:tone:ragebaiter_free:{step_index}:{user_id}") },
            { "type": 2, "style": 2, "label": "Überspringen",
              "custom_id": format!("ob:tone:skip:{step_index}:{user_id}") },
        ]}]);
    }
    if step_index == AGE_STEP {
        return json!([{ "type": 1, "components": [
            { "type": 2, "style": 1, "label": "25+",
              "custom_id": format!("ob:age:25+:{step_index}:{user_id}") },
            { "type": 2, "style": 2, "label": "U25",
              "custom_id": format!("ob:age:u25:{step_index}:{user_id}") },
            { "type": 2, "style": 2, "label": "Überspringen",
              "custom_id": format!("ob:age:skip:{step_index}:{user_id}") },
        ]}]);
    }
    if step_index == ACCOUNT_STEP {
        let mut row = vec![json!({
            "type": 2, "style": 3, "label": "Via Steam verknüpfen", "emoji": { "name": "🎮" },
            "custom_id": format!("ob:steam:{user_id}"),
        })];
        if verified {
            row.push(json!({
                "type": 2, "style": 1, "label": "Weiter ➜",
                "custom_id": format!("ob:done:{user_id}"),
            }));
        } else {
            row.push(json!({
                "type": 2, "style": 2, "label": "Ich hab verknüpft ➜",
                "custom_id": format!("ob:recheck:{user_id}"),
            }));
        }
        return json!([{ "type": 1, "components": row }]);
    }
    if step_index >= last {
        return json!([{ "type": 1, "components": [{
            "type": 2, "style": 3, "label": "Alles klar, viel Spaß! 🎮",
            "custom_id": format!("ob:done:{user_id}"),
        }]}]);
    }
    json!([{ "type": 1, "components": [{
        "type": 2, "style": 1, "label": "Weiter ➜",
        "custom_id": format!("ob:next:{step_index}:{user_id}"),
    }]}])
}

/// Nächster Schritt ab `current` (wie NextStepView.next_step):
/// Streamer-Schritt wird ohne Creator-Rolle übersprungen.
pub fn next_step_index(current: usize, is_streamer: bool) -> usize {
    let mut next = current + 1;
    if next == STREAMER_STEP && !is_streamer {
        next += 1;
    }
    next.min(steps().len() - 1)
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait OnboardingPort: Send + Sync {
    /// Privaten Thread im Regelkanal erstellen + User einladen → thread_id.
    async fn create_onboarding_thread(
        &self,
        guild_id: u64,
        user_id: u64,
        name: &str,
    ) -> Result<u64, String>;
    async fn send_step(&self, channel_id: u64, embed: Value, components: Value);
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String;
    /// Einmal-Steam-Link-URL (wie onboardglue::fetch_steam_link_url).
    async fn steam_link_url(&self, user_id: u64) -> Option<String>;
    /// Tag setzen (TagService).
    async fn set_user_tag(&self, user_id: u64, key: &str, value: &str);
}

pub struct OnboardingWizard {
    pub port: Arc<dyn OnboardingPort>,
}

impl OnboardingWizard {
    pub fn new(port: Arc<dyn OnboardingPort>) -> Arc<Self> {
        Arc::new(Self { port })
    }

    async fn is_streamer(&self, guild_id: u64, user_id: u64) -> bool {
        self.port
            .member_role_ids(guild_id, user_id)
            .await
            .contains(&CONTENT_CREATOR_ROLE_ID)
    }

    async fn is_verified(&self, guild_id: u64, user_id: u64) -> bool {
        self.port
            .member_role_ids(guild_id, user_id)
            .await
            .contains(&VERIFIED_ROLE_ID)
    }

    /// Schritt in den Thread posten.
    async fn post_step(&self, channel_id: u64, step: usize, guild_id: u64, user_id: u64) {
        let is_streamer = self.is_streamer(guild_id, user_id).await;
        let verified = if step == ACCOUNT_STEP {
            self.is_verified(guild_id, user_id).await
        } else {
            false
        };
        self.port
            .send_step(
                channel_id,
                build_step_embed(step, is_streamer),
                build_step_components(step, user_id, verified),
            )
            .await;
    }
}

struct OnboardingHandler {
    wizard: Arc<OnboardingWizard>,
}

/// custom_id-Schema: ob:next:{step}:{uid} | ob:tone/age:{value}:{step}:{uid}
/// | ob:steam:{uid} | ob:recheck:{uid} | ob:done:{uid} | rp:panel:start
#[async_trait::async_trait]
impl InteractionHandler for OnboardingHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let wizard = &self.wizard;
        if interaction.custom_id == "rp:panel:start" {
            let name = format!(
                "Onboarding – {}",
                wizard
                    .port
                    .member_display_name(interaction.guild_id, interaction.user_id)
                    .await
            );
            let thread = match wizard
                .port
                .create_onboarding_thread(interaction.guild_id, interaction.user_id, &name)
                .await
            {
                Ok(id) => id,
                Err(err) => {
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Konnte dein Onboarding nicht starten: {err}"
                    ))
                }
            };
            wizard
                .post_step(thread, 0, interaction.guild_id, interaction.user_id)
                .await;
            return BridgeReply::ephemeral_text(format!(
                "✅ Dein Onboarding wartet hier auf dich: <#{thread}>"
            ));
        }

        let parts: Vec<&str> = interaction.custom_id.split(':').collect();
        let owner: u64 = parts
            .last()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or_default();
        if owner != interaction.user_id {
            return BridgeReply::ephemeral_text("Dieses Onboarding gehört jemand anderem.");
        }
        match parts.get(1).copied().unwrap_or_default() {
            "next" => {
                let current: usize = parts.get(2).and_then(|r| r.parse().ok()).unwrap_or(0);
                let is_streamer = wizard
                    .is_streamer(interaction.guild_id, interaction.user_id)
                    .await;
                let next = next_step_index(current, is_streamer);
                wizard
                    .post_step(
                        interaction.channel_id,
                        next,
                        interaction.guild_id,
                        interaction.user_id,
                    )
                    .await;
                BridgeReply::default() // Schritt steht im Thread
            }
            "tone" | "age" => {
                let key = parts[1];
                let value = parts.get(2).copied().unwrap_or("skip");
                if value != "skip" {
                    wizard.port.set_user_tag(interaction.user_id, key, value).await;
                }
                let current: usize = parts.get(3).and_then(|r| r.parse().ok()).unwrap_or(0);
                let next = next_step_index(
                    current,
                    wizard
                        .is_streamer(interaction.guild_id, interaction.user_id)
                        .await,
                );
                wizard
                    .post_step(
                        interaction.channel_id,
                        next,
                        interaction.guild_id,
                        interaction.user_id,
                    )
                    .await;
                BridgeReply::default()
            }
            "steam" => match wizard.port.steam_link_url(interaction.user_id).await {
                Some(url) => BridgeReply {
                    content: Some(
                        "Hier ist dein persönlicher Steam-Link (einmalig gültig):".to_string(),
                    ),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 2, "style": 5, "label": "Mit Steam anmelden", "url": url,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                },
                None => BridgeReply::ephemeral_text(
                    "❌ Der Steam-Link-Dienst ist gerade nicht erreichbar — versuch es gleich nochmal.",
                ),
            },
            "recheck" => {
                if wizard
                    .is_verified(interaction.guild_id, interaction.user_id)
                    .await
                {
                    wizard
                        .post_step(
                            interaction.channel_id,
                            ACCOUNT_STEP,
                            interaction.guild_id,
                            interaction.user_id,
                        )
                        .await;
                    BridgeReply::ephemeral_text("✅ Verknüpft! Du kannst jetzt abschließen.")
                } else {
                    BridgeReply::ephemeral_text(
                        "Noch nicht verknüpft — klick zuerst auf **Via Steam verknüpfen** und schließe den Steam-Login ab.",
                    )
                }
            }
            "done" => BridgeReply::ephemeral_text(
                "Nice, jetzt weißt du alles! Falls doch mal Fragen sind: einfach ein Ticket aufmachen oder einen Mod fragen. Have fun! 🎮",
            ),
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

pub fn register(router: &mut InteractionRouter, wizard: Arc<OnboardingWizard>) {
    let handler = Arc::new(OnboardingHandler { wizard });
    router.on_custom_id("rp:panel:start", handler.clone());
    router.on_prefix("ob:", handler);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_eingebettet() {
        let all = steps();
        assert_eq!(all.len(), 10);
        assert!(all[0].title.starts_with("Hey, willkommen"));
        assert_eq!(all[TONE_STEP].title, "Voice-Ton");
        assert_eq!(all[AGE_STEP].title, "Alter (optional)");
        assert!(all[ACCOUNT_STEP].title.contains("Account verknüpfen"));
    }

    #[test]
    fn footer_und_streamer_skip() {
        // Nicht-Streamer: Schritt 3 zeigt "Schritt 3 / 9" (Streamer-Step übersprungen)
        let embed = build_step_embed(3, false);
        assert!(embed["footer"]["text"]
            .as_str()
            .expect("footer")
            .contains("Schritt 3 / 9"));
        let embed = build_step_embed(3, true);
        assert!(embed["footer"]["text"]
            .as_str()
            .expect("footer")
            .contains("Schritt 4 / 10"));
        // Navigation: 1 → 3 ohne Creator-Rolle, 1 → 2 mit
        assert_eq!(next_step_index(1, false), 3);
        assert_eq!(next_step_index(1, true), 2);
        // letzter Schritt bleibt letzter
        assert_eq!(next_step_index(9, false), 9);
    }

    #[test]
    fn buttons_je_schritt() {
        let tone = build_step_components(TONE_STEP, 42, false);
        let ids: Vec<&str> = tone[0]["components"]
            .as_array()
            .expect("row")
            .iter()
            .map(|b| b["custom_id"].as_str().expect("id"))
            .collect();
        assert_eq!(
            ids,
            vec![
                "ob:tone:banter_ok:7:42",
                "ob:tone:ragebaiter_free:7:42",
                "ob:tone:skip:7:42"
            ]
        );
        let account = build_step_components(ACCOUNT_STEP, 42, false);
        let labels: Vec<&str> = account[0]["components"]
            .as_array()
            .expect("row")
            .iter()
            .map(|b| b["label"].as_str().expect("label"))
            .collect();
        assert_eq!(labels, vec!["Via Steam verknüpfen", "Ich hab verknüpft ➜"]);
        let account_verified = build_step_components(ACCOUNT_STEP, 42, true);
        assert!(account_verified[0]["components"][1]["label"]
            .as_str()
            .expect("label")
            .contains("Weiter"));
        let done = build_step_components(9, 42, true);
        // Account-Schritt IST der letzte — done-Komponenten kommen über verified
        assert!(done[0]["components"][0]["custom_id"]
            .as_str()
            .expect("id")
            .starts_with("ob:steam"));
    }
}
