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

use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::{json, Value};

pub const RULES_CHANNEL_ID: u64 = 1315684135175716975;
pub const VERIFIED_ROLE_ID: u64 = 1419608095533043774;
pub const ONBOARD_COMPLETE_ROLE_ID: u64 = 1304216250649415771;
pub const CONTENT_CREATOR_ROLE_ID: u64 = 1466630749255106590;
pub const AUTO_ONBOARDING_CLAIM_NS: &str = "onboarding:auto_start";
pub const STREAMER_STEP: usize = 2;
pub const TONE_STEP: usize = 7;
pub const AGE_STEP: usize = 8;
pub const ACCOUNT_STEP: usize = 9;
pub const ONBOARDING_TRANSIENT_START_HINT: &str =
    "⚠️ Discord hat gerade Serverprobleme. Bitte versuche es in ein paar Sekunden erneut.";
pub const ONBOARDING_RECHECK_UNVERIFIED_HINT: &str =
    "Du hast die **Verified**-Rolle noch nicht. Bitte stelle sicher, dass du deinen Account verknüpft hast \
und dem Steam-Bot (Freundescode 820142646) eine Freundschaftsanfrage geschickt und angenommen hast. \
(Es kann ein paar Minuten dauern)";

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

/// Regelwerk-/Onboarding-Panel-Embed (Text byte-genau aus
/// `cogs/rules_channel.py::publish_rules_panel`).
pub fn build_panel_embed() -> Value {
    json!({
        "title": "📜 Regelwerk · Deutsche Deadlock Community",
        "description":
            "### Neu hier? Klick auf **Hier starten ➤**\n\
             und wir erklären dir alles in 5 Minuten.\n\n\n\
             **Verhalten**\n\
             - Respekt gegenüber allen – keine Beleidigungen, Diskriminierung oder persönlichen Angriffe\n\
             - Keine Hassrede, kein NSFW, kein Spam, keine Fremdwerbung\n\
             - Privatsphäre respektieren – keine fremden Daten posten\n\
             - Schädliche Inhalte (Viren, IP-Grabber etc.) = sofortiger permanenter Bann\n\n\
             **Im Spielkontext erlaubt**\n\
             Situatives Trash Talking, Sarkasmus, Wortspiele – solange es nicht persönlich wird. \
             Ohne nonverbale Signale kann Ton schnell schiefgehen, also vorher abchecken ob alle damit fein sind.\n\n\
             **Universalregel:** Sei kein Arschloch 😄\n\n\
             **Moderation**\n\
             Probleme? @Moderator oder @Owner pingen. \
             Konsequenzen je nach Schwere: Verwarnung → Timeout → Ban.",
        "color": 0x00AEEF,
    })
}

/// Action-Row mit dem persistenten „Hier starten"-Button (`rp:panel:start`).
pub fn build_panel_components() -> Value {
    json!([{ "type": 1, "components": [{
        "type": 2, "style": 1, "label": "Hier starten ➜",
        "custom_id": "rp:panel:start",
    }]}])
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
    ) -> Result<u64, OnboardingThreadError>;
    async fn send_step(&self, channel_id: u64, embed: Value, components: Value);
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    async fn member_is_bot(&self, guild_id: u64, user_id: u64) -> bool;
    async fn has_verified_steam_link(&self, user_id: u64) -> bool;
    async fn claim_screening_auto_start(&self, user_id: u64) -> bool;
    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String;
    /// Einmal-Steam-Link-URL (wie onboardglue::fetch_steam_link_url).
    async fn steam_link_url(&self, user_id: u64) -> Option<String>;
    /// Tag setzen (TagService).
    async fn set_user_tag(&self, user_id: u64, key: &str, value: &str);
    /// Merkt den Onboarding-Channel, bis die Verified-Rolle vergeben wird
    /// (`onboarding_pending_verify`); für den Abschluss-Trigger.
    async fn register_pending_verify(&self, user_id: u64, channel_id: u64);
    /// Holt + entfernt den gemerkten Channel (oder `None`).
    async fn pop_pending_verify(&self, user_id: u64) -> Option<u64>;
    /// Schickt eine reine Text-Nachricht in einen Channel.
    async fn send_text(&self, channel_id: u64, content: String);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingThreadError {
    Transient(String),
    Permanent(String),
}

impl std::fmt::Display for OnboardingThreadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient(message) | Self::Permanent(message) => f.write_str(message),
        }
    }
}

impl OnboardingThreadError {
    fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
}

/// Abschluss-Nachricht nach erfolgreicher Verifizierung (Text wie im Original).
fn completion_message(user_id: u64) -> String {
    format!(
        "<@{user_id}> ✅ **Verifizierung erfolgreich!**\n\nNice, jetzt weißt du alles! \
         Falls doch mal Fragen sind: einfach ein Ticket aufmachen oder einen Mod fragen. \
         Viel Spaß! 🎮"
    )
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

    async fn is_already_verified_or_onboarded(&self, guild_id: u64, user_id: u64) -> bool {
        let role_ids = self.port.member_role_ids(guild_id, user_id).await;
        role_ids.contains(&VERIFIED_ROLE_ID)
            || role_ids.contains(&ONBOARD_COMPLETE_ROLE_ID)
            || self.port.has_verified_steam_link(user_id).await
    }

    /// Schritt in den Thread posten.
    async fn post_step(&self, channel_id: u64, step: usize, guild_id: u64, user_id: u64) {
        let is_streamer = self.is_streamer(guild_id, user_id).await;
        let verified = if step == ACCOUNT_STEP {
            let v = self.is_verified(guild_id, user_id).await;
            if !v {
                // Noch nicht verifiziert → Channel merken, damit die Abschluss-
                // Nachricht bei Rollen-Vergabe automatisch kommt.
                self.port.register_pending_verify(user_id, channel_id).await;
            }
            v
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

    pub async fn start_for_user(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<u64, OnboardingThreadError> {
        let name = format!(
            "Onboarding – {}",
            self.port.member_display_name(guild_id, user_id).await
        );
        let thread = self
            .port
            .create_onboarding_thread(guild_id, user_id, &name)
            .await?;
        self.post_step(thread, 0, guild_id, user_id).await;
        Ok(thread)
    }

    pub async fn start_after_screening(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<Option<u64>, OnboardingThreadError> {
        if self.port.member_is_bot(guild_id, user_id).await {
            return Ok(None);
        }
        if self
            .is_already_verified_or_onboarded(guild_id, user_id)
            .await
        {
            return Ok(None);
        }
        if !self.port.claim_screening_auto_start(user_id).await {
            return Ok(None);
        }
        self.start_for_user(guild_id, user_id).await.map(Some)
    }

    /// Postet das Regelwerk-Panel (Embed + „Hier starten"-Button) in einen
    /// Kanal — Port von `publish_rules_panel`. Das Original editiert eine feste
    /// Panel-Message in RULES_CHANNEL_ID; hier wird stattdessen frisch dorthin
    /// gepostet (kein hartkodierter Message-ID-Edit nötig).
    async fn publish_panel(&self, channel_id: u64) {
        self.port
            .send_step(channel_id, build_panel_embed(), build_panel_components())
            .await;
    }

    /// Reagiert auf einen Rollen-Zugewinn: kam die Verified-Rolle dazu und ist
    /// ein Onboarding-Channel gemerkt, kommt die Abschluss-Nachricht.
    pub async fn handle_role_gained(&self, user_id: u64, role_ids: &[u64]) {
        if !role_ids.contains(&VERIFIED_ROLE_ID) {
            return;
        }
        if let Some(channel_id) = self.port.pop_pending_verify(user_id).await {
            self.port
                .send_text(channel_id, completion_message(user_id))
                .await;
        }
    }
}

/// Subscriber: Onboarding-Verifikations-Abschluss aus `RoleEvent::Gained`.
pub fn spawn_verify_completion(
    wizard: Arc<OnboardingWizard>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_roles();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::RoleEvent::Gained {
                    user_id, role_ids, ..
                }) => {
                    wizard.handle_role_gained(user_id, &role_ids).await;
                }
                Ok(dl_discord::RoleEvent::Removed { .. }) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Subscriber: Discord-Member-Screening abgeschlossen → Onboarding auto-starten.
pub fn spawn_screening_auto_start(
    wizard: Arc<OnboardingWizard>,
    dispatcher: &dl_discord::Dispatcher,
    guild_id_filter: u64,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_members();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::MemberEvent::ScreeningCompleted { guild_id, user_id })
                    if guild_id == guild_id_filter =>
                {
                    if let Err(err) = wizard.start_after_screening(guild_id, user_id).await {
                        tracing::warn!(%err, guild_id, user_id, "Auto-Onboarding konnte nicht starten");
                    }
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
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
        // /publish_rules_panel (Admin): Panel in den festen Regelwerk-Kanal posten
        // (wie das Python-Original, das immer RULES_CHANNEL_ID bedient).
        if interaction.command == "publish_rules_panel" {
            if interaction.guild_id == 0 {
                return BridgeReply::ephemeral_text("❌ Das funktioniert nur auf dem Server.");
            }
            wizard.publish_panel(RULES_CHANNEL_ID).await;
            return BridgeReply::ephemeral_text("✅ Panel gepostet.");
        }
        if interaction.custom_id == "rp:panel:start" {
            let thread = match wizard
                .start_for_user(interaction.guild_id, interaction.user_id)
                .await
            {
                Ok(id) => id,
                Err(err) => {
                    if err.is_transient() {
                        return BridgeReply::ephemeral_text(ONBOARDING_TRANSIENT_START_HINT);
                    }
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Konnte dein Onboarding nicht starten: {err}"
                    ));
                }
            };
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
                    BridgeReply::ephemeral_text(ONBOARDING_RECHECK_UNVERIFIED_HINT)
                }
            }
            "done" => BridgeReply::ephemeral_text(
                "Nice, jetzt weißt du alles! Falls doch mal Fragen sind: einfach ein Ticket aufmachen oder einen Mod fragen. Viel Spaß! 🎮",
            ),
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

pub fn register(router: &mut InteractionRouter, wizard: Arc<OnboardingWizard>) {
    let handler = Arc::new(OnboardingHandler { wizard });
    router.on_command(
        "publish_rules_panel",
        CommandSpec {
            definition: json!({
                "name": "publish_rules_panel",
                "description": "(Admin) Regelwerk-Panel posten",
                "type": 1,
                "dm_permission": false,
                "default_member_permissions": "8", // Administrator
            }),
        },
        handler.clone(),
    );
    router.on_custom_id("rp:panel:start", handler.clone());
    router.on_prefix("ob:", handler);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestPort {
        create_results: Mutex<Vec<Result<u64, OnboardingThreadError>>>,
        created: Mutex<Vec<(u64, u64, String)>>,
        sent_steps: Mutex<Vec<(u64, Value, Value)>>,
        texts: Mutex<Vec<(u64, String)>>,
        roles: Mutex<Vec<u64>>,
        bot_users: Mutex<HashSet<u64>>,
        db_verified_users: Mutex<HashSet<u64>>,
        auto_start_claims: Mutex<HashSet<u64>>,
    }

    #[async_trait::async_trait]
    impl OnboardingPort for TestPort {
        async fn create_onboarding_thread(
            &self,
            guild_id: u64,
            user_id: u64,
            name: &str,
        ) -> Result<u64, OnboardingThreadError> {
            self.created
                .lock()
                .await
                .push((guild_id, user_id, name.to_string()));
            self.create_results.lock().await.remove(0)
        }

        async fn send_step(&self, channel_id: u64, embed: Value, components: Value) {
            self.sent_steps
                .lock()
                .await
                .push((channel_id, embed, components));
        }

        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            self.roles.lock().await.clone()
        }

        async fn member_is_bot(&self, _guild_id: u64, user_id: u64) -> bool {
            self.bot_users.lock().await.contains(&user_id)
        }

        async fn has_verified_steam_link(&self, user_id: u64) -> bool {
            self.db_verified_users.lock().await.contains(&user_id)
        }

        async fn claim_screening_auto_start(&self, user_id: u64) -> bool {
            self.auto_start_claims.lock().await.insert(user_id)
        }

        async fn member_display_name(&self, _guild_id: u64, user_id: u64) -> String {
            format!("User {user_id}")
        }

        async fn steam_link_url(&self, _user_id: u64) -> Option<String> {
            None
        }

        async fn set_user_tag(&self, _user_id: u64, _key: &str, _value: &str) {}

        async fn register_pending_verify(&self, _user_id: u64, _channel_id: u64) {}

        async fn pop_pending_verify(&self, _user_id: u64) -> Option<u64> {
            None
        }

        async fn send_text(&self, channel_id: u64, content: String) {
            self.texts.lock().await.push((channel_id, content));
        }
    }

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

    #[test]
    fn panel_embed_und_button() {
        let embed = build_panel_embed();
        assert!(embed["title"]
            .as_str()
            .expect("title")
            .contains("Regelwerk"));
        assert!(embed["description"]
            .as_str()
            .expect("desc")
            .contains("Hier starten"));
        assert_eq!(embed["color"].as_u64().expect("color"), 0x00AEEF);
        let row = build_panel_components();
        let button = &row[0]["components"][0];
        assert_eq!(button["custom_id"].as_str().expect("id"), "rp:panel:start");
        assert_eq!(button["label"].as_str().expect("label"), "Hier starten ➜");
        assert_eq!(button["style"].as_u64().expect("style"), 1);
    }

    #[tokio::test]
    async fn start_for_user_erstellt_thread_und_postet_ersten_schritt() {
        let port = Arc::new(TestPort::default());
        port.create_results.lock().await.push(Ok(777));
        let wizard = OnboardingWizard::new(port.clone());

        let thread = wizard.start_for_user(1, 42).await.expect("thread");

        assert_eq!(thread, 777);
        assert_eq!(port.created.lock().await.len(), 1);
        let sent = port.sent_steps.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, 777);
        assert_eq!(sent[0].1["title"], build_step_embed(0, false)["title"]);
    }

    #[tokio::test]
    async fn screening_subscriber_startet_onboarding_automatisch() {
        let port = Arc::new(TestPort::default());
        port.create_results.lock().await.push(Ok(778));
        let wizard = OnboardingWizard::new(port.clone());
        let dispatcher = dl_discord::Dispatcher::new();
        let task = spawn_screening_auto_start(wizard, &dispatcher, 1);

        dispatcher.publish_member(dl_discord::MemberEvent::ScreeningCompleted {
            guild_id: 1,
            user_id: 42,
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if !port.sent_steps.lock().await.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("auto start");
        task.abort();
    }

    #[tokio::test]
    async fn screening_subscriber_ueberspringt_bots() {
        let port = Arc::new(TestPort::default());
        port.bot_users.lock().await.insert(42);
        let wizard = OnboardingWizard::new(port.clone());
        let dispatcher = dl_discord::Dispatcher::new();
        let task = spawn_screening_auto_start(wizard, &dispatcher, 1);

        dispatcher.publish_member(dl_discord::MemberEvent::ScreeningCompleted {
            guild_id: 1,
            user_id: 42,
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(port.created.lock().await.is_empty());
        assert!(port.sent_steps.lock().await.is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn screening_subscriber_ueberspringt_bereits_verifizierte_user() {
        let port = Arc::new(TestPort::default());
        port.roles.lock().await.push(VERIFIED_ROLE_ID);
        let wizard = OnboardingWizard::new(port.clone());
        let dispatcher = dl_discord::Dispatcher::new();
        let task = spawn_screening_auto_start(wizard, &dispatcher, 1);

        dispatcher.publish_member(dl_discord::MemberEvent::ScreeningCompleted {
            guild_id: 1,
            user_id: 42,
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(port.created.lock().await.is_empty());
        assert!(port.sent_steps.lock().await.is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn screening_subscriber_ueberspringt_bereits_onboardete_user() {
        let port = Arc::new(TestPort::default());
        port.roles.lock().await.push(ONBOARD_COMPLETE_ROLE_ID);
        let wizard = OnboardingWizard::new(port.clone());
        let dispatcher = dl_discord::Dispatcher::new();
        let task = spawn_screening_auto_start(wizard, &dispatcher, 1);

        dispatcher.publish_member(dl_discord::MemberEvent::ScreeningCompleted {
            guild_id: 1,
            user_id: 42,
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(port.created.lock().await.is_empty());
        assert!(port.sent_steps.lock().await.is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn screening_subscriber_ueberspringt_db_verifizierte_user() {
        let port = Arc::new(TestPort::default());
        port.db_verified_users.lock().await.insert(42);
        let wizard = OnboardingWizard::new(port.clone());
        let dispatcher = dl_discord::Dispatcher::new();
        let task = spawn_screening_auto_start(wizard, &dispatcher, 1);

        dispatcher.publish_member(dl_discord::MemberEvent::ScreeningCompleted {
            guild_id: 1,
            user_id: 42,
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(port.created.lock().await.is_empty());
        assert!(port.sent_steps.lock().await.is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn screening_subscriber_startet_pro_user_nur_einmal() {
        let port = Arc::new(TestPort::default());
        port.create_results.lock().await.push(Ok(778));
        let wizard = OnboardingWizard::new(port.clone());
        let dispatcher = dl_discord::Dispatcher::new();
        let task = spawn_screening_auto_start(wizard, &dispatcher, 1);

        for _ in 0..2 {
            dispatcher.publish_member(dl_discord::MemberEvent::ScreeningCompleted {
                guild_id: 1,
                user_id: 42,
            });
        }

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if port.sent_steps.lock().await.len() == 1 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("auto start");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(port.created.lock().await.len(), 1);
        assert_eq!(port.sent_steps.lock().await.len(), 1);
        task.abort();
    }

    #[tokio::test]
    async fn transienter_startfehler_nutzt_placeholder_hinweis() {
        let port = Arc::new(TestPort::default());
        port.create_results
            .lock()
            .await
            .push(Err(OnboardingThreadError::Transient("discord 500".into())));
        let wizard = OnboardingWizard::new(port);
        let handler = OnboardingHandler { wizard };

        let reply = handler
            .handle(dl_discord::BridgeInteraction {
                custom_id: "rp:panel:start".into(),
                guild_id: 1,
                user_id: 42,
                ..dl_discord::BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some(ONBOARDING_TRANSIENT_START_HINT)
        );
        assert!(reply.ephemeral);
    }

    #[tokio::test]
    async fn recheck_unverified_nutzt_placeholder_hinweis() {
        let port = Arc::new(TestPort::default());
        let wizard = OnboardingWizard::new(port);
        let handler = OnboardingHandler { wizard };

        let reply = handler
            .handle(dl_discord::BridgeInteraction {
                custom_id: "ob:recheck:42".into(),
                guild_id: 1,
                user_id: 42,
                channel_id: 777,
                ..dl_discord::BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some(ONBOARDING_RECHECK_UNVERIFIED_HINT)
        );
        assert!(reply.ephemeral);
    }
}
