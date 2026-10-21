use serde_json::{json, Map, Value};
use serenity::all::{ButtonStyle, ChannelId, CreateActionRow, CreateButton};
use sqlx::PgPool;

use crate::adapter::DiscordAdapter;
use crate::interactions::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};

pub const DM_OPTOUT_CUSTOM_ID: &str = "dm_optout:global";
pub const DM_OPTOUT_LABEL: &str = "Keine Bot-Nachrichten mehr";
pub const DM_OPTOUT_CONFIRMATION: &str = "Alles klar, der Bot schreibt dir ab jetzt keine Nachrichten mehr von sich aus. Antworten auf deine eigenen Befehle und Klicks bekommst du weiterhin.";
pub const DM_OPTOUT_FAILED: &str =
    "Das hat gerade nicht geklappt. Versuch es bitte gleich noch einmal.";

const MAX_CLASSIC_ACTION_ROWS: usize = 5;
const COMPONENTS_V2_FLAG: u64 = 1 << 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmIntent {
    Proactive(&'static str),
    Reply,
    Moderation,
    Security,
    Cleanup,
}

impl DmIntent {
    pub fn is_gated(self) -> bool {
        matches!(self, DmIntent::Proactive(_))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DmError {
    #[error("Nutzer hat Bot-DMs abbestellt")]
    OptedOut,
    #[error("DM-Opt-out-Pruefung nicht verfuegbar: {0}")]
    GateUnavailable(String),
    #[error(transparent)]
    Discord(Box<serenity::Error>),
}

impl From<serenity::Error> for DmError {
    fn from(err: serenity::Error) -> Self {
        DmError::Discord(Box::new(err))
    }
}

impl DmError {
    pub fn discord(&self) -> Option<&serenity::Error> {
        match self {
            DmError::Discord(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmDelivery {
    pub channel_id: u64,
    pub message_id: u64,
}

pub fn optout_button_row() -> Value {
    json!({
        "type": 1,
        "components": [{
            "type": 2,
            "style": 2,
            "label": DM_OPTOUT_LABEL,
            "custom_id": DM_OPTOUT_CUSTOM_ID,
        }]
    })
}

pub fn optout_action_row() -> CreateActionRow {
    CreateActionRow::Buttons(vec![CreateButton::new(DM_OPTOUT_CUSTOM_ID)
        .label(DM_OPTOUT_LABEL)
        .style(ButtonStyle::Secondary)])
}

fn carries_optout_button(body: &Map<String, Value>) -> bool {
    body.get("components")
        .map(|components| components.to_string().contains(DM_OPTOUT_CUSTOM_ID))
        .unwrap_or(false)
}

fn uses_components_v2(body: &Map<String, Value>) -> bool {
    body.get("flags")
        .and_then(Value::as_u64)
        .is_some_and(|flags| flags & COMPONENTS_V2_FLAG != 0)
}

pub fn attach_optout_button(body: &mut Map<String, Value>) {
    if carries_optout_button(body) {
        return;
    }
    let v2 = uses_components_v2(body);
    match body.get_mut("components") {
        Some(Value::Array(rows)) => {
            if v2 || rows.len() < MAX_CLASSIC_ACTION_ROWS {
                rows.push(optout_button_row());
            } else {
                tracing::warn!("Bot-DM hat keinen Platz mehr fuer den Opt-out-Knopf");
            }
        }
        _ => {
            body.insert("components".into(), json!([optout_button_row()]));
        }
    }
}

pub async fn check_dm_allowed(
    pool: Option<&PgPool>,
    user_id: u64,
    intent: DmIntent,
) -> Result<(), DmError> {
    if !intent.is_gated() {
        return Ok(());
    }
    let Some(pool) = pool else {
        return Err(DmError::GateUnavailable(
            "kein Datenbankzugang verknuepft".to_string(),
        ));
    };
    match dl_central_db::is_dm_opted_out(pool, user_id).await {
        Ok(false) => Ok(()),
        Ok(true) => {
            if let DmIntent::Proactive(feature) = intent {
                tracing::info!(user_id, feature, "Bot-DM unterdrueckt: globaler Opt-out");
            }
            Err(DmError::OptedOut)
        }
        Err(err) => Err(DmError::GateUnavailable(err.to_string())),
    }
}

impl DiscordAdapter {
    pub fn link_dm_gate(&self, pool: PgPool) {
        let _ = self.dm_gate.set(pool);
    }

    pub async fn open_dm_channel(
        &self,
        user_id: u64,
        intent: DmIntent,
    ) -> Result<ChannelId, DmError> {
        check_dm_allowed(self.dm_gate.get(), user_id, intent).await?;
        let channel = self
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await?;
        Ok(channel.id)
    }

    pub async fn deliver_dm(
        &self,
        user_id: u64,
        intent: DmIntent,
        body: &Map<String, Value>,
    ) -> Result<DmDelivery, DmError> {
        let channel_id = self.open_dm_channel(user_id, intent).await?;
        let mut body = body.clone();
        if intent.is_gated() {
            attach_optout_button(&mut body);
        }
        let message = self
            .http
            .send_message(channel_id, Vec::new(), &body)
            .await?;
        Ok(DmDelivery {
            channel_id: channel_id.get(),
            message_id: message.id.get(),
        })
    }
}

pub struct DmOptoutHandler {
    pool: PgPool,
}

impl DmOptoutHandler {
    pub fn new(pool: PgPool) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { pool })
    }
}

#[async_trait::async_trait]
impl InteractionHandler for DmOptoutHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        match dl_central_db::set_dm_opted_out(&self.pool, interaction.user_id, "dm_optout_button")
            .await
        {
            Ok(()) => {
                tracing::info!(
                    user_id = interaction.user_id,
                    "Globaler Bot-DM-Opt-out gesetzt"
                );
                BridgeReply::ephemeral_text(DM_OPTOUT_CONFIRMATION)
            }
            Err(err) => {
                tracing::warn!(%err, user_id = interaction.user_id, "Globaler Bot-DM-Opt-out fehlgeschlagen");
                BridgeReply::ephemeral_text(DM_OPTOUT_FAILED)
            }
        }
    }
}

pub fn register(router: &mut InteractionRouter, pool: PgPool) {
    router.on_custom_id(DM_OPTOUT_CUSTOM_ID, DmOptoutHandler::new(pool));
}
