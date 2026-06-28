//! Interaction- und Slash-Command-Routing.
//!
//! Ersetzt discord.py-Konstrukte aus dem Original:
//! - persistente Views (`bot.add_view` + custom_id-Matching) → [`InteractionRouter`]
//!   mit Exakt-/Präfix-Matchern,
//! - `@app_commands.command` → [`CommandSpec`]-Registry, aus der sowohl die
//!   Discord-Command-Definitionen (Sync) als auch der Dispatch gespeist werden.
//!
//! Handler liefern eine [`BridgeReply`] — die Gateway-Schicht übersetzt sie in
//! die eigentliche Interaction-Response (inkl. der 2-Sekunden-Defer-Schwelle,
//! die das Python-Original für langsame Antworten nutzt).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde_json::Value;

/// Eingehende Interaction in normalisierter Form.
#[derive(Debug, Clone, Default)]
pub struct BridgeInteraction {
    /// custom_id des Buttons/Selects/Modals ("" bei Slash-Commands).
    pub custom_id: String,
    /// Slash-Command-Name (qualifiziert, z. B. "steam links"); leer bei Komponenten.
    pub command: String,
    /// Slash-Command-Optionen bzw. Modal-/Select-Werte.
    pub options: HashMap<String, Value>,
    pub values: Vec<String>,
    pub interaction_id: u64,
    pub user_id: u64,
    /// Discord-Username (str(user)-Äquivalent, für Tracking-Zwecke).
    pub author_name: String,
    /// Discord-Displayname: Guild-Nick vor global_name vor username.
    pub author_display_name: String,
    /// manage_roles ODER administrator (für Mod-Guards wie den Review-Flow).
    pub author_can_manage_roles: bool,
    /// manage_guild ODER administrator (für Admin-Slash-Commands wie Changelog).
    pub author_can_manage_guild: bool,
    /// manage_channels ODER administrator (für TempVoice-Lane-Mod-Guards).
    pub author_can_manage_channels: bool,
    /// Ob Discord einen Guild-Member-Kontext mitgeliefert hat.
    pub member_present: bool,
    pub guild_id: u64,
    pub channel_id: u64,
    /// Nachricht, an der die Komponente hing (None bei Slash-Commands).
    pub message_id: Option<u64>,
}

/// Modal-Definition (z. B. Steam-Freundescode).
#[derive(Debug, Clone)]
pub struct ModalSpec {
    pub custom_id: String,
    pub title: String,
    pub fields: Vec<ModalField>,
}

#[derive(Debug, Clone)]
pub struct ModalField {
    pub custom_id: String,
    pub label: String,
    pub placeholder: String,
    pub required: bool,
    pub min_length: u16,
    pub max_length: u16,
    /// true = mehrzeiliges Textfeld (Discord style 2).
    pub paragraph: bool,
}

/// Antwort eines Handlers — deklarativ, damit Tests ohne Discord laufen.
#[derive(Clone, Default)]
pub struct BridgeReply {
    pub content: Option<String>,
    /// Roh-Embeds im Discord-API-Format.
    pub embeds: Vec<Value>,
    /// Komplette components-Struktur (Action-Rows) im Discord-API-Format.
    pub components: Option<Value>,
    pub ephemeral: bool,
    /// Statt einer Nachricht ein Modal öffnen (nur als Erst-Antwort möglich).
    pub modal: Option<ModalSpec>,
    /// Antwort in den Kanal posten statt als Interaction-Reply
    /// (publish_steam_panel-Muster: Panel öffentlich, Bestätigung ephemeral).
    pub channel_message: Option<ChannelMessage>,
    /// Datei-Anhänge (z. B. der DSGVO-Datenexport als JSON). Default leer —
    /// bestehende Handler bleiben unverändert. Der Dispatch reicht sie als
    /// `Vec<CreateAttachment>` an serenity durch (Multipart erledigt serenity).
    pub attachments: Vec<BridgeAttachment>,
    /// Nur für Komponenten-Interaktionen (Button/Select): die bestehende
    /// Nachricht in-place editieren statt eine neue zu senden (Discord-Callback
    /// `UPDATE_MESSAGE`/Typ 7). Bei Slash/Modal ignoriert. Update-Antworten
    /// sollten `ephemeral` nicht setzen (die Nachricht behält ihre Sichtbarkeit).
    pub update_message: bool,
    /// Optionaler Hook, der nach erfolgreichem Senden die erzeugte Message-ID
    /// bekommt. Domain-Code nutzt das fuer persistente View-KV, ohne dass der
    /// Discord-Dispatch Domänendetails kennen muss.
    pub response_message_hook: Option<Arc<dyn ResponseMessageHook>>,
}

impl fmt::Debug for BridgeReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeReply")
            .field("content", &self.content)
            .field("embeds", &self.embeds)
            .field("components", &self.components)
            .field("ephemeral", &self.ephemeral)
            .field("modal", &self.modal)
            .field("channel_message", &self.channel_message)
            .field("attachments", &self.attachments)
            .field("update_message", &self.update_message)
            .field(
                "response_message_hook",
                &self.response_message_hook.as_ref().map(|_| "<hook>"),
            )
            .finish()
    }
}

#[async_trait::async_trait]
pub trait ResponseMessageHook: Send + Sync {
    async fn on_response_message(&self, message_id: u64);
}

/// Ein Datei-Anhang für eine Interaction-Antwort (In-Memory-Bytes).
#[derive(Debug, Clone)]
pub struct BridgeAttachment {
    pub filename: String,
    pub data: Vec<u8>,
}

#[derive(Clone)]
pub struct ChannelMessage {
    pub target_channel_id: Option<u64>,
    pub content: Option<String>,
    pub embeds: Vec<Value>,
    pub components: Option<Value>,
    pub edit_message_id: Option<u64>,
    /// Bestätigungstext als ephemere Interaction-Antwort.
    pub confirmation: String,
    pub response_message_hook: Option<Arc<dyn ResponseMessageHook>>,
}

impl fmt::Debug for ChannelMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelMessage")
            .field("target_channel_id", &self.target_channel_id)
            .field("content", &self.content)
            .field("embeds", &self.embeds)
            .field("components", &self.components)
            .field("edit_message_id", &self.edit_message_id)
            .field("confirmation", &self.confirmation)
            .field(
                "response_message_hook",
                &self.response_message_hook.as_ref().map(|_| "<hook>"),
            )
            .finish()
    }
}

impl BridgeReply {
    pub fn ephemeral_text(text: impl Into<String>) -> Self {
        Self {
            content: Some(text.into()),
            ephemeral: true,
            ..Self::default()
        }
    }
}

#[async_trait::async_trait]
pub trait InteractionHandler: Send + Sync {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply;
}

/// Nachrichten direkt in einen Kanal senden (für Listener außerhalb von
/// Interactions, z. B. die !steam_*-Admin-Kommandos). Implementiert vom
/// DiscordAdapter; Tests nutzen Mocks.
#[async_trait::async_trait]
pub trait ChannelSender: Send + Sync {
    async fn send_to_channel(
        &self,
        channel_id: u64,
        content: Option<&str>,
        embeds: &[Value],
    ) -> Result<u64, String>;
}

enum Matcher {
    Exact(String),
    Prefix(String),
}

impl Matcher {
    fn matches(&self, custom_id: &str) -> bool {
        match self {
            Matcher::Exact(id) => custom_id == id,
            Matcher::Prefix(prefix) => custom_id.starts_with(prefix.as_str()),
        }
    }
}

/// Slash-Command-Definition — Quelle für Sync UND Dispatch.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Discord-API-Definition (name, description, options, permissions …).
    pub definition: Value,
}

#[derive(Default)]
pub struct InteractionRouter {
    component_routes: Vec<(Matcher, Arc<dyn InteractionHandler>)>,
    command_routes: HashMap<String, Arc<dyn InteractionHandler>>,
    command_specs: Vec<CommandSpec>,
}

impl InteractionRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registriert einen Handler für eine exakte custom_id.
    pub fn on_custom_id(&mut self, id: impl Into<String>, handler: Arc<dyn InteractionHandler>) {
        self.component_routes
            .push((Matcher::Exact(id.into()), handler));
    }

    /// Registriert einen Handler für ein custom_id-Präfix (z. B. "twitch-live:").
    pub fn on_prefix(&mut self, prefix: impl Into<String>, handler: Arc<dyn InteractionHandler>) {
        self.component_routes
            .push((Matcher::Prefix(prefix.into()), handler));
    }

    /// Registriert einen Slash-Command (qualifizierter Name, z. B. "steam links").
    pub fn on_command(
        &mut self,
        qualified_name: impl Into<String>,
        spec: CommandSpec,
        handler: Arc<dyn InteractionHandler>,
    ) {
        self.command_routes.insert(qualified_name.into(), handler);
        self.command_specs.push(spec);
    }

    /// Discord-Command-Definitionen für den Bulk-Sync.
    pub fn command_definitions(&self) -> Vec<Value> {
        // Subcommands ("steam links") teilen sich eine Top-Level-Definition —
        // Duplikate nach name dedupen, letzter gewinnt.
        let mut by_name: HashMap<String, Value> = HashMap::new();
        for spec in &self.command_specs {
            if let Some(name) = spec.definition.get("name").and_then(Value::as_str) {
                by_name.insert(name.to_string(), spec.definition.clone());
            }
        }
        by_name.into_values().collect()
    }

    pub fn resolve_component(&self, custom_id: &str) -> Option<Arc<dyn InteractionHandler>> {
        self.component_routes
            .iter()
            .find(|(matcher, _)| matcher.matches(custom_id))
            .map(|(_, handler)| handler.clone())
    }

    pub fn resolve_command(&self, qualified_name: &str) -> Option<Arc<dyn InteractionHandler>> {
        self.command_routes.get(qualified_name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Echo(&'static str);

    #[async_trait::async_trait]
    impl InteractionHandler for Echo {
        async fn handle(&self, _interaction: BridgeInteraction) -> BridgeReply {
            BridgeReply::ephemeral_text(self.0)
        }
    }

    #[tokio::test]
    async fn exakt_vor_praefix_nach_registrierungsreihenfolge() {
        let mut router = InteractionRouter::new();
        router.on_custom_id("betainvite:panel:start", Arc::new(Echo("exact")));
        router.on_prefix("betainvite:", Arc::new(Echo("prefix")));

        let handler = router
            .resolve_component("betainvite:panel:start")
            .expect("handler");
        let reply = handler.handle(BridgeInteraction::default()).await;
        assert_eq!(reply.content.as_deref(), Some("exact"));

        let handler = router
            .resolve_component("betainvite:intent:community")
            .expect("prefix-handler");
        let reply = handler.handle(BridgeInteraction::default()).await;
        assert_eq!(reply.content.as_deref(), Some("prefix"));

        assert!(router.resolve_component("unbekannt").is_none());
    }

    #[test]
    fn command_definitionen_dedupen_nach_name() {
        let mut router = InteractionRouter::new();
        let spec = |name: &str| CommandSpec {
            definition: json!({ "name": name, "description": "x" }),
        };
        router.on_command("steam links", spec("steam"), Arc::new(Echo("a")));
        router.on_command("steam unlink", spec("steam"), Arc::new(Echo("b")));
        router.on_command("betainvite", spec("betainvite"), Arc::new(Echo("c")));
        assert_eq!(router.command_definitions().len(), 2);
        assert!(router.resolve_command("steam links").is_some());
        assert!(router.resolve_command("steam unlink").is_some());
        assert!(router.resolve_command("gibtsnicht").is_none());
    }
}
