use std::{collections::HashMap, path::PathBuf, sync::Arc};

use dl_central_db::kv;
use dl_discord::{
    BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter, ModalField, ModalSpec,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;

pub const LFG_GUILD_ID: u64 = crate::router::ROUTER_GUILD_ID;
pub const LFG_PANEL_KV_NS: &str = "lfg_panel";
pub const LFG_PANEL_MESSAGE_KEY: &str = "components_v2_message_id";
pub const LFG_PAYLOAD_FORMAT_KEY: &str = "payload_format";
pub const LFG_PAYLOAD_FORMAT: &str = "components_v2";
pub const LFG_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const LFG_ACCENT_GOLD: u64 = crate::router::ROUTER_ACCENT_GOLD;
pub const LFG_BANNER_DIR: &str = "assets/welcome-banners";
pub const LFG_PANEL_BANNER_FILENAME: &str = "router-hero.png";
pub const LFG_CREATE_START_CUSTOM_ID: &str = "lfg:create:start";
pub const LFG_PLACEHOLDER_TEXT: &str = "Platzhalter";
pub const LFG_CREATE_MODE_PREFIX: &str = "lfg:create:mode:";
pub const LFG_CREATE_MODAL_PREFIX: &str = "lfg:create:modal:";
pub const LFG_FIELD_RANK_RANGE: &str = "rank_range";
pub const LFG_FIELD_REQUESTED_SLOTS: &str = "requested_slots";
pub const LFG_EXPIRY_HOURS: i64 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfgMode {
    Casual,
    Ranked,
    StreetBrawl,
}

impl LfgMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Casual => "casual",
            Self::Ranked => "ranked",
            Self::StreetBrawl => "street_brawl",
        }
    }

    fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "casual" => Some(Self::Casual),
            "ranked" => Some(Self::Ranked),
            "street_brawl" => Some(Self::StreetBrawl),
            _ => None,
        }
    }

    fn modal_custom_id(self) -> String {
        format!("{LFG_CREATE_MODAL_PREFIX}{}", self.as_str())
    }

    fn mode_custom_id(self) -> String {
        format!("{LFG_CREATE_MODE_PREFIX}{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LfgRankRange {
    pub min: Option<i32>,
    pub max: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfgForumPostDraft {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LfgCreatedForumPost {
    pub thread_id: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfgPanelApplyOutput {
    pub guild_id: u64,
    pub channel_id: Option<u64>,
    pub dry_run: bool,
    pub payload_format: String,
    pub stored_payload_format: Option<String>,
    pub stored_message_id: Option<u64>,
    pub action: String,
    pub message_id: Option<u64>,
    pub blocked_reason: Option<String>,
    pub warnings: Vec<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfgPanelAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfgPanelMessage {
    pub message_id: u64,
    pub has_embeds: bool,
    pub has_components: bool,
    pub custom_ids: Vec<String>,
}

#[async_trait::async_trait]
pub trait LfgPanelPort: Send + Sync {
    async fn post_rich(
        &self,
        channel_id: u64,
        body: Map<String, Value>,
        attachments: &[LfgPanelAttachment],
    ) -> Result<u64, String>;

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
        attachments: &[LfgPanelAttachment],
    ) -> Result<(), String>;

    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<LfgPanelMessage>, String>;

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;

    async fn create_forum_post(
        &self,
        forum_channel_id: u64,
        draft: LfgForumPostDraft,
    ) -> Result<LfgCreatedForumPost, String>;

    async fn first_thread_message_id(&self, thread_id: u64) -> Result<Option<u64>, String>;

    async fn archive_and_lock_thread(&self, thread_id: u64) -> Result<(), String>;
}

pub struct LfgPanelInterface {
    pool: PgPool,
    port: Arc<dyn LfgPanelPort>,
    channel_id: Option<u64>,
    missing_channel_reason: Option<String>,
    cutover_active: bool,
}

impl LfgPanelInterface {
    pub fn new(pool: PgPool, port: Arc<dyn LfgPanelPort>, channel_id: Option<u64>) -> Arc<Self> {
        Self::new_with_channel_config(
            pool,
            port,
            channel_id,
            channel_id
                .is_none()
                .then(|| "DL_LFG_PANEL_CHANNEL_ID fehlt".to_string()),
            true,
        )
    }

    pub fn new_with_channel_config(
        pool: PgPool,
        port: Arc<dyn LfgPanelPort>,
        channel_id: Option<u64>,
        missing_channel_reason: Option<String>,
        cutover_active: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            channel_id,
            missing_channel_reason,
            cutover_active,
        })
    }

    pub async fn ensure_panel(&self) {
        if let Err(err) = self.apply_panel(true).await {
            tracing::warn!(%err, "LfgPanelInterface: Panel konnte nicht angewendet werden");
        }
    }

    pub fn target_channel_id(&self) -> Option<u64> {
        self.channel_id
    }

    pub fn cutover_active(&self) -> bool {
        self.cutover_active
    }

    pub async fn apply_panel(&self, confirm: bool) -> Result<LfgPanelApplyOutput, String> {
        let attachments = lfg_panel_attachments();
        let body = lfg_panel_body_for_attachments(&attachments);
        let stored_message_id = self.panel_message_id().await;
        let stored_payload_format = self.panel_payload_format().await;
        if !self.cutover_active {
            let mut output = LfgPanelApplyOutput {
                guild_id: LFG_GUILD_ID,
                channel_id: self.channel_id,
                dry_run: !confirm,
                payload_format: LFG_PAYLOAD_FORMAT.to_string(),
                stored_payload_format,
                stored_message_id,
                action: "blocked_cutover_disabled".to_string(),
                message_id: stored_message_id,
                blocked_reason: Some("cutover_disabled".to_string()),
                warnings: Vec::new(),
                payload: Value::Object(body),
            };
            output
                .warnings
                .push("LFG-Panel: Cutover ist nicht aktiv; confirm wird blockiert.".to_string());
            if confirm {
                return Err("LfgPanelInterface: cutover_disabled".to_string());
            }
            return Ok(output);
        }
        let recent = match self.channel_id {
            Some(channel_id) => match self.port.recent_bot_messages(channel_id, 15).await {
                Ok(messages) => Some(messages),
                Err(err) => {
                    tracing::warn!(%err, "LfgPanelInterface: History-Scan fehlgeschlagen");
                    None
                }
            },
            None => None,
        };
        let history_checked = self.channel_id.is_none() || recent.is_some();
        let history_message_id = recent.as_deref().and_then(find_existing_lfg_v2_panel);
        let planned_message_id = stored_message_id.or(history_message_id);
        let action = match (self.channel_id, planned_message_id) {
            (None, _) => "blocked_missing_channel",
            (Some(_), Some(_)) => "planned_edit",
            (Some(_), None) => "planned_post",
        };
        let mut output = LfgPanelApplyOutput {
            guild_id: LFG_GUILD_ID,
            channel_id: self.channel_id,
            dry_run: !confirm,
            payload_format: LFG_PAYLOAD_FORMAT.to_string(),
            stored_payload_format,
            stored_message_id,
            action: action.to_string(),
            message_id: planned_message_id,
            blocked_reason: self
                .channel_id
                .is_none()
                .then(|| self.missing_channel_reason.clone())
                .flatten(),
            warnings: Vec::new(),
            payload: Value::Object(body.clone()),
        };
        if self.channel_id.is_none() {
            let reason = self
                .missing_channel_reason
                .as_deref()
                .unwrap_or("DL_LFG_PANEL_CHANNEL_ID fehlt");
            output.warnings.push(format!(
                "LFG-Panel: Zielkanal fehlt ({reason}); DL_LFG_PANEL_CHANNEL_ID muss auf das Forum-/Panel-Ziel zeigen."
            ));
            if confirm {
                return Err(format!("LfgPanelInterface: {reason}"));
            }
            return Ok(output);
        }
        if !confirm {
            if self.channel_id.is_some() && !history_checked {
                output.warnings.push(
                    "LFG-Panel: History-Scan fehlgeschlagen; Dry-Run ohne Adoption.".to_string(),
                );
            }
            return Ok(output);
        }

        validate_lfg_panel_attachments(&attachments)?;
        let channel_id = self.channel_id.expect("checked channel_id");
        let mut ignored_history_message_id = None;
        if let Some(message_id) = stored_message_id {
            match self
                .port
                .edit_rich(channel_id, message_id, body.clone(), &attachments)
                .await
            {
                Ok(()) => {
                    self.store_panel_metadata(message_id).await;
                    output.dry_run = false;
                    output.action = "edited".to_string();
                    output.message_id = Some(message_id);
                    return Ok(output);
                }
                Err(err) if is_not_found_error(&err) => {
                    self.delete_panel_message_id().await;
                    ignored_history_message_id = Some(message_id);
                    output.warnings.push(format!(
                        "LFG-Panel: gespeicherte Message-ID {message_id} ist stale (404); KV wurde geloescht."
                    ));
                }
                Err(err) => return Err(err),
            }
        }

        if !history_checked {
            return Err(
                "LfgPanelInterface: ohne erfolgreichen History-Scan wird kein neues Panel gepostet"
                    .to_string(),
            );
        }

        if let Some(message_id) =
            history_message_id.filter(|message_id| Some(*message_id) != ignored_history_message_id)
        {
            self.port
                .edit_rich(channel_id, message_id, body.clone(), &attachments)
                .await
                .map_err(|err| {
                    format!(
                        "LfgPanelInterface: adoptiertes Panel {message_id} konnte nicht editiert werden: {err}"
                    )
                })?;
            self.store_panel_metadata(message_id).await;
            output.dry_run = false;
            output.action = "adopted_edit".to_string();
            output.message_id = Some(message_id);
            return Ok(output);
        }

        let message_id = self.port.post_rich(channel_id, body, &attachments).await?;
        self.store_panel_metadata(message_id).await;
        output.dry_run = false;
        output.action = "posted".to_string();
        output.message_id = Some(message_id);
        Ok(output)
    }

    async fn panel_message_id(&self) -> Option<u64> {
        kv::get(&self.pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
    }

    async fn panel_payload_format(&self) -> Option<String> {
        kv::get(&self.pool, LFG_PANEL_KV_NS, LFG_PAYLOAD_FORMAT_KEY)
            .await
            .ok()
            .flatten()
    }

    async fn store_panel_metadata(&self, message_id: u64) {
        if let Err(err) = kv::set(
            &self.pool,
            LFG_PANEL_KV_NS,
            LFG_PANEL_MESSAGE_KEY,
            &message_id.to_string(),
        )
        .await
        {
            tracing::warn!(%err, "LfgPanelInterface: Message-ID konnte nicht gespeichert werden");
        }
        if let Err(err) = kv::set(
            &self.pool,
            LFG_PANEL_KV_NS,
            LFG_PAYLOAD_FORMAT_KEY,
            LFG_PAYLOAD_FORMAT,
        )
        .await
        {
            tracing::warn!(%err, "LfgPanelInterface: Payload-Format konnte nicht gespeichert werden");
        }
    }

    async fn delete_panel_message_id(&self) {
        if let Err(err) = kv::delete(&self.pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY).await {
            tracing::warn!(%err, "LfgPanelInterface: stale Message-ID konnte nicht geloescht werden");
        }
    }
}

pub fn lfg_repo_root() -> PathBuf {
    crate::router::router_repo_root()
}

pub fn lfg_panel_attachments() -> Vec<LfgPanelAttachment> {
    vec![LfgPanelAttachment {
        id: 0,
        filename: LFG_PANEL_BANNER_FILENAME.to_string(),
        relative_path: format!("{LFG_BANNER_DIR}/{LFG_PANEL_BANNER_FILENAME}"),
    }]
}

pub fn lfg_panel_body() -> Map<String, Value> {
    let attachments = lfg_panel_attachments();
    lfg_panel_body_for_attachments(&attachments)
}

fn lfg_panel_body_for_attachments(attachments: &[LfgPanelAttachment]) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".to_string(), json!(LFG_COMPONENTS_V2_FLAG));
    body.insert(
        "allowed_mentions".to_string(),
        json!({ "parse": Vec::<String>::new() }),
    );
    let mut components = Vec::new();
    if !attachments.is_empty() {
        components.push(lfg_media_gallery(&attachments[0].filename));
    }
    components.push(lfg_text_display(LFG_PLACEHOLDER_TEXT.to_string()));
    components.push(lfg_action_row(vec![lfg_button(
        LFG_PLACEHOLDER_TEXT,
        1,
        LFG_CREATE_START_CUSTOM_ID,
    )]));
    body.insert(
        "components".to_string(),
        json!([{
            "type": 17,
            "accent_color": LFG_ACCENT_GOLD,
            "components": components,
        }]),
    );
    body.insert("attachments".to_string(), json!(attachments));
    body
}

fn lfg_media_gallery(filename: &str) -> Value {
    json!({
        "type": 12,
        "items": [{
            "media": { "url": format!("attachment://{filename}") },
        }],
    })
}

fn lfg_text_display(content: String) -> Value {
    json!({
        "type": 10,
        "content": content,
    })
}

fn lfg_action_row(components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "components": components,
    })
}

fn lfg_button(label: &str, style: u8, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
    })
}

fn validate_lfg_panel_attachments(attachments: &[LfgPanelAttachment]) -> Result<(), String> {
    let repo_root = lfg_repo_root();
    for attachment in attachments {
        let path = repo_root.join(&attachment.relative_path);
        if !path.is_file() {
            return Err(format!(
                "LFG-Banner `{}` fehlt; LFG-Panel wird nicht gepostet/editiert",
                path.display()
            ));
        }
    }
    Ok(())
}

fn find_existing_lfg_v2_panel(messages: &[LfgPanelMessage]) -> Option<u64> {
    messages
        .iter()
        .find(|message| {
            !message.has_embeds
                && message.has_components
                && message
                    .custom_ids
                    .iter()
                    .any(|custom_id| custom_id.starts_with("lfg:create:"))
        })
        .map(|message| message.message_id)
}

fn is_not_found_error(err: &str) -> bool {
    err.contains("HTTP 404") || err.contains("404")
}

fn lfg_mode_selection_components() -> Value {
    json!([{ "type": 1, "components": [
        lfg_button(LFG_PLACEHOLDER_TEXT, 2, &LfgMode::Casual.mode_custom_id()),
        lfg_button(LFG_PLACEHOLDER_TEXT, 1, &LfgMode::Ranked.mode_custom_id()),
        lfg_button(LFG_PLACEHOLDER_TEXT, 2, &LfgMode::StreetBrawl.mode_custom_id()),
    ]}])
}

fn lfg_create_modal(mode: LfgMode) -> ModalSpec {
    ModalSpec {
        custom_id: mode.modal_custom_id(),
        title: LFG_PLACEHOLDER_TEXT.to_string(),
        fields: vec![
            ModalField {
                custom_id: LFG_FIELD_RANK_RANGE.to_string(),
                label: LFG_PLACEHOLDER_TEXT.to_string(),
                placeholder: LFG_PLACEHOLDER_TEXT.to_string(),
                required: false,
                min_length: 0,
                max_length: 80,
                paragraph: false,
            },
            ModalField {
                custom_id: LFG_FIELD_REQUESTED_SLOTS.to_string(),
                label: LFG_PLACEHOLDER_TEXT.to_string(),
                placeholder: LFG_PLACEHOLDER_TEXT.to_string(),
                required: true,
                min_length: 1,
                max_length: 1,
                paragraph: false,
            },
        ],
    }
}

fn option_text(options: &HashMap<String, Value>, key: &str) -> String {
    options
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn parse_requested_slots(raw: &str) -> Option<i32> {
    raw.trim()
        .parse::<i32>()
        .ok()
        .filter(|slots| (1..=5).contains(slots))
}

fn parse_rank_range(raw: &str) -> Option<LfgRankRange> {
    let normalized = raw.trim().to_lowercase();
    if normalized.is_empty() {
        return Some(LfgRankRange {
            min: None,
            max: None,
        });
    }

    fn rank_token(token: &str) -> Option<i32> {
        let idx = crate::tempvoice::logic::rank_index(token.trim());
        (idx > 0).then(|| i32::try_from(idx).expect("rank index fits i32"))
    }

    fn range(min: i32, max: i32) -> Option<LfgRankRange> {
        (min <= max).then_some(LfgRankRange {
            min: Some(min),
            max: Some(max),
        })
    }

    if normalized.contains('-') {
        let parts = normalized.split('-').map(str::trim).collect::<Vec<_>>();
        if let [min, max] = parts.as_slice() {
            return range(rank_token(min)?, rank_token(max)?);
        }
        return None;
    }

    match normalized.split_whitespace().collect::<Vec<_>>().as_slice() {
        [rank] => {
            let rank = rank_token(rank)?;
            Some(LfgRankRange {
                min: Some(rank),
                max: Some(rank),
            })
        }
        [min, "bis", max] => range(rank_token(min)?, rank_token(max)?),
        _ => None,
    }
}

fn rank_name(index: i32) -> String {
    crate::tempvoice::logic::RANK_ORDER
        .get(index as usize)
        .map(|rank| crate::tempvoice::logic::capitalize(rank))
        .unwrap_or_else(|| LFG_PLACEHOLDER_TEXT.to_string())
}

fn rank_range_label(range: LfgRankRange) -> String {
    match (range.min, range.max) {
        (Some(min), Some(max)) if min == max => rank_name(min),
        (Some(min), Some(max)) => format!("{} bis {}", rank_name(min), rank_name(max)),
        _ => LFG_PLACEHOLDER_TEXT.to_string(),
    }
}

fn lfg_post_draft(
    owner_id: u64,
    mode: LfgMode,
    rank_range: LfgRankRange,
    requested_slots: i32,
) -> LfgForumPostDraft {
    let rank_label = rank_range_label(rank_range);
    let title = format!(
        "{LFG_PLACEHOLDER_TEXT} | {} | {rank_label} | {requested_slots}",
        mode.as_str()
    );
    let body = [
        LFG_PLACEHOLDER_TEXT.to_string(),
        format!("{LFG_PLACEHOLDER_TEXT}: <@{owner_id}>"),
        format!("{LFG_PLACEHOLDER_TEXT}: {}", mode.as_str()),
        format!("{LFG_PLACEHOLDER_TEXT}: {rank_label}"),
        format!("{LFG_PLACEHOLDER_TEXT}: {requested_slots}"),
        format!("{LFG_PLACEHOLDER_TEXT}: {LFG_EXPIRY_HOURS}h"),
    ]
    .join("\n");
    LfgForumPostDraft { title, body }
}

fn has_verified_rank_role(role_ids: &[u64]) -> bool {
    role_ids
        .iter()
        .any(|role_id| crate::router::VERIFIED_RANK_ROLE_IDS.contains(role_id))
}

fn discord_id_i64(id: u64, field: &str) -> Result<i64, String> {
    i64::try_from(id).map_err(|_| format!("{field} ist keine gueltige BIGINT-Discord-ID"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LfgReservationError {
    AlreadyOpen,
    Db(String),
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db_err| db_err.code())
        .is_some_and(|code| code == "23505")
}

impl LfgPanelInterface {
    async fn ranked_allowed(&self, guild_id: u64, user_id: u64, mode: LfgMode) -> bool {
        if mode != LfgMode::Ranked {
            return true;
        }
        has_verified_rank_role(&self.port.member_role_ids(guild_id, user_id).await)
    }

    async fn handle_start(&self) -> BridgeReply {
        BridgeReply {
            content: Some(LFG_PLACEHOLDER_TEXT.to_string()),
            components: Some(lfg_mode_selection_components()),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    async fn handle_mode(&self, interaction: BridgeInteraction, mode: LfgMode) -> BridgeReply {
        let guild_id = if interaction.guild_id == 0 {
            LFG_GUILD_ID
        } else {
            interaction.guild_id
        };
        if !self
            .ranked_allowed(guild_id, interaction.user_id, mode)
            .await
        {
            return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
        }
        BridgeReply {
            modal: Some(lfg_create_modal(mode)),
            ..BridgeReply::default()
        }
    }

    async fn handle_modal(&self, interaction: BridgeInteraction, mode: LfgMode) -> BridgeReply {
        let guild_id = if interaction.guild_id == 0 {
            LFG_GUILD_ID
        } else {
            interaction.guild_id
        };
        if !self
            .ranked_allowed(guild_id, interaction.user_id, mode)
            .await
        {
            return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
        }

        let Some(forum_channel_id) = self.channel_id else {
            return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
        };
        let Some(requested_slots) = parse_requested_slots(&option_text(
            &interaction.options,
            LFG_FIELD_REQUESTED_SLOTS,
        )) else {
            return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
        };
        let Some(rank_range) =
            parse_rank_range(&option_text(&interaction.options, LFG_FIELD_RANK_RANGE))
        else {
            return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
        };

        match self
            .reserve_lfg_post(
                guild_id,
                forum_channel_id,
                interaction.user_id,
                mode,
                rank_range,
                requested_slots,
            )
            .await
        {
            Ok(()) => {}
            Err(LfgReservationError::AlreadyOpen) => {
                return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
            }
            Err(LfgReservationError::Db(err)) => {
                tracing::error!(
                    %err,
                    owner_id = interaction.user_id,
                    "LFG-Reservation konnte nicht erstellt werden"
                );
                return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
            }
        }

        let draft = lfg_post_draft(interaction.user_id, mode, rank_range, requested_slots);
        let created = match self.port.create_forum_post(forum_channel_id, draft).await {
            Ok(created) => created,
            Err(err) => {
                tracing::error!(
                    %err,
                    owner_id = interaction.user_id,
                    thread_id = 0_u64,
                    "LFG-Forum-Post konnte nach DB-Reservation nicht erstellt werden"
                );
                self.delete_lfg_reservation(interaction.user_id).await;
                return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
            }
        };
        let starter_message_id = match self.port.first_thread_message_id(created.thread_id).await {
            Ok(message_id) => message_id,
            Err(err) => {
                tracing::warn!(
                    %err,
                    thread_id = created.thread_id,
                    "LFG-Starter-Message konnte nicht kontrolliert gefetcht werden"
                );
                None
            }
        };

        if let Err(err) = self
            .open_lfg_post_reservation(created.thread_id, starter_message_id, interaction.user_id)
            .await
        {
            tracing::error!(
                %err,
                owner_id = interaction.user_id,
                thread_id = created.thread_id,
                "LFG-Forum-Post konnte nach Discord-Erstellung nicht persistiert werden"
            );
            self.delete_lfg_reservation(interaction.user_id).await;
            if let Err(cleanup_err) = self.port.archive_and_lock_thread(created.thread_id).await {
                tracing::error!(
                    %cleanup_err,
                    owner_id = interaction.user_id,
                    thread_id = created.thread_id,
                    "LFG-Forum-Thread konnte nach Persistenzfehler nicht archiviert/gelockt werden"
                );
            }
            return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
        }

        BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT)
    }

    #[allow(clippy::too_many_arguments)]
    async fn reserve_lfg_post(
        &self,
        guild_id: u64,
        forum_channel_id: u64,
        owner_id: u64,
        mode: LfgMode,
        rank_range: LfgRankRange,
        requested_slots: i32,
    ) -> Result<(), LfgReservationError> {
        sqlx::query(
            "INSERT INTO voice.lfg_posts (
                 guild_id,
                 forum_channel_id,
                 thread_id,
                 starter_message_id,
                 lane_id,
                 owner_id,
                 mode,
                 rank_min,
                 rank_max,
                 requested_slots,
                 status,
                 created_at,
                 updated_at,
                 expires_at,
                 closed_at,
                 last_render_hash,
                 last_post_edit_at
             )
             VALUES (
                 $1, $2, NULL, NULL, NULL, $3, $4, $5, $6, $7, 'creating',
                 now(), now(), now() + INTERVAL '24 hours', NULL, NULL, NULL
             )",
        )
        .bind(discord_id_i64(guild_id, "guild_id").map_err(LfgReservationError::Db)?)
        .bind(
            discord_id_i64(forum_channel_id, "forum_channel_id")
                .map_err(LfgReservationError::Db)?,
        )
        .bind(discord_id_i64(owner_id, "owner_id").map_err(LfgReservationError::Db)?)
        .bind(mode.as_str())
        .bind(rank_range.min)
        .bind(rank_range.max)
        .bind(requested_slots)
        .execute(&self.pool)
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                LfgReservationError::AlreadyOpen
            } else {
                LfgReservationError::Db(err.to_string())
            }
        })?;
        Ok(())
    }

    async fn open_lfg_post_reservation(
        &self,
        thread_id: u64,
        starter_message_id: Option<u64>,
        owner_id: u64,
    ) -> Result<(), String> {
        let result = sqlx::query(
            "UPDATE voice.lfg_posts
                SET thread_id = $1,
                    starter_message_id = $2,
                    status = 'open',
                    updated_at = now()
              WHERE owner_id = $3
                AND status = 'creating'
                AND thread_id IS NULL",
        )
        .bind(discord_id_i64(thread_id, "thread_id")?)
        .bind(
            starter_message_id
                .map(|id| discord_id_i64(id, "starter_message_id"))
                .transpose()?,
        )
        .bind(discord_id_i64(owner_id, "owner_id")?)
        .execute(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        if result.rows_affected() != 1 {
            return Err(format!(
                "LFG-Reservation nicht eindeutig aktualisiert: rows_affected={}",
                result.rows_affected()
            ));
        }
        Ok(())
    }

    async fn delete_lfg_reservation(&self, owner_id: u64) {
        let owner_id_i64 = match discord_id_i64(owner_id, "owner_id") {
            Ok(owner_id) => owner_id,
            Err(err) => {
                tracing::error!(%err, owner_id, "LFG-Reservation-Cleanup konnte owner_id nicht binden");
                return;
            }
        };
        if let Err(err) = sqlx::query(
            "DELETE FROM voice.lfg_posts
              WHERE owner_id = $1
                AND status = 'creating'
                AND thread_id IS NULL",
        )
        .bind(owner_id_i64)
        .execute(&self.pool)
        .await
        {
            tracing::error!(
                %err,
                owner_id,
                "LFG-Reservation-Cleanup konnte Reservation-Row nicht loeschen"
            );
        }
    }
}

#[async_trait::async_trait]
impl InteractionHandler for LfgPanelInterface {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !self.cutover_active {
            return BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT);
        }
        if interaction.custom_id == LFG_CREATE_START_CUSTOM_ID {
            return self.handle_start().await;
        }
        if let Some(mode) = interaction
            .custom_id
            .strip_prefix(LFG_CREATE_MODE_PREFIX)
            .and_then(LfgMode::from_str)
        {
            return self.handle_mode(interaction, mode).await;
        }
        if let Some(mode) = interaction
            .custom_id
            .strip_prefix(LFG_CREATE_MODAL_PREFIX)
            .and_then(LfgMode::from_str)
        {
            return self.handle_modal(interaction, mode).await;
        }
        BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT)
    }
}

pub fn register(router: &mut InteractionRouter, interface: Arc<LfgPanelInterface>) {
    router.on_prefix("lfg:create:", interface);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::{BridgeInteraction, InteractionRouter};
    use serde_json::{json, Map, Value};
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn lfg_panel_body_ist_components_v2_mit_start_button_und_banner() {
        let attachments = lfg_panel_attachments();
        let body = lfg_panel_body_for_attachments(&attachments);

        assert_eq!(body.get("flags"), Some(&json!(LFG_COMPONENTS_V2_FLAG)));
        assert_eq!(body.get("allowed_mentions"), Some(&json!({"parse": []})));
        assert_eq!(
            body.get("attachments"),
            Some(&json!([{"id": 0, "filename": LFG_PANEL_BANNER_FILENAME}]))
        );
        let components = body["components"][0]["components"]
            .as_array()
            .expect("container components");
        assert!(components.iter().any(|component| {
            component["type"] == 12
                && component["items"][0]["media"]["url"]
                    == format!("attachment://{LFG_PANEL_BANNER_FILENAME}")
        }));
        let custom_ids: Vec<&str> = components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert_eq!(custom_ids, vec![LFG_CREATE_START_CUSTOM_ID]);
    }

    #[tokio::test]
    async fn lfg_panel_interface_postet_und_editiert_idempotent_ueber_kv() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let first = interface.apply_panel(true).await.expect("first apply");
        assert_eq!(first.action, "posted");
        assert_eq!(first.channel_id, Some(777));
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9101")
        );

        let second = interface.apply_panel(true).await.expect("second apply");
        assert_eq!(second.action, "edited");
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(port.edits.lock().expect("edits").len(), 1);
    }

    #[tokio::test]
    async fn lfg_panel_apply_postet_neu_wenn_kv_message_geloescht_ist() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        dl_central_db::kv::set(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY, "7777")
            .await
            .expect("kv set");
        let port = Arc::new(MockLfgPanelPort::default());
        port.not_found_edits.lock().expect("not found").push(7777);
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let output = interface.apply_panel(true).await.expect("apply");

        assert_eq!(output.action, "posted");
        assert_eq!(output.message_id, Some(9101));
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("stale (404)")));
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9101")
        );
    }

    #[tokio::test]
    async fn lfg_panel_apply_adoptiert_nur_lfg_create_v2_panel_aus_history() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.recent.lock().expect("recent") = vec![
            LfgPanelMessage {
                message_id: 7001,
                has_embeds: false,
                has_components: true,
                custom_ids: vec!["router_spawn_casual".to_string()],
            },
            LfgPanelMessage {
                message_id: 7002,
                has_embeds: true,
                has_components: true,
                custom_ids: vec![LFG_CREATE_START_CUSTOM_ID.to_string()],
            },
            LfgPanelMessage {
                message_id: 7003,
                has_embeds: false,
                has_components: true,
                custom_ids: vec![LFG_CREATE_START_CUSTOM_ID.to_string()],
            },
        ];
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let output = interface.apply_panel(true).await.expect("apply");

        assert_eq!(output.action, "adopted_edit");
        assert_eq!(output.message_id, Some(7003));
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        let edited_message_id = {
            let edits = port.edits.lock().expect("edits");
            assert_eq!(edits.len(), 1);
            edits[0].1
        };
        assert_eq!(edited_message_id, 7003);
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("7003")
        );
    }

    #[tokio::test]
    async fn lfg_panel_dry_run_braucht_keine_channel_config() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port.clone(), None);

        let output = interface.apply_panel(false).await.expect("dry run");

        assert!(output.dry_run);
        assert_eq!(output.action, "blocked_missing_channel");
        assert!(output.channel_id.is_none());
        assert_eq!(
            output.blocked_reason.as_deref(),
            Some("DL_LFG_PANEL_CHANNEL_ID fehlt")
        );
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
    }

    #[tokio::test]
    async fn lfg_panel_apply_blockt_confirm_bei_inaktivem_cutover() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new_with_channel_config(
            db.pool().clone(),
            port.clone(),
            Some(777),
            None,
            false,
        );

        let dry_run = interface.apply_panel(false).await.expect("dry run");
        assert!(dry_run.dry_run);
        assert_eq!(dry_run.action, "blocked_cutover_disabled");
        assert_eq!(dry_run.blocked_reason.as_deref(), Some("cutover_disabled"));
        assert!(interface.apply_panel(true).await.is_err());
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);

        let no_channel = LfgPanelInterface::new_with_channel_config(
            db.pool().clone(),
            port.clone(),
            None,
            Some("DL_LFG_PANEL_CHANNEL_ID fehlt".to_string()),
            false,
        );
        let no_channel_dry_run = no_channel.apply_panel(false).await.expect("dry run");
        assert_eq!(
            no_channel_dry_run.blocked_reason.as_deref(),
            Some("cutover_disabled")
        );
        assert!(no_channel.apply_panel(true).await.is_err());
    }

    #[tokio::test]
    async fn lfg_create_start_button_zeigt_ephemere_moduswahl() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port, Some(777));
        let mut router = InteractionRouter::new();
        register(&mut router, interface);

        let handler = router
            .resolve_component(LFG_CREATE_START_CUSTOM_ID)
            .expect("lfg handler");
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: LFG_CREATE_START_CUSTOM_ID.to_string(),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_PLACEHOLDER_TEXT));
        let custom_ids: Vec<String> = reply.components.as_ref().expect("components")[0]
            ["components"]
            .as_array()
            .expect("buttons")
            .iter()
            .map(|button| button["custom_id"].as_str().expect("custom_id").to_string())
            .collect();
        assert_eq!(
            custom_ids,
            vec![
                LfgMode::Casual.mode_custom_id(),
                LfgMode::Ranked.mode_custom_id(),
                LfgMode::StreetBrawl.mode_custom_id(),
            ]
        );
    }

    #[tokio::test]
    async fn lfg_create_interactions_blocken_bei_inaktivem_cutover() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new_with_channel_config(
            db.pool().clone(),
            port.clone(),
            Some(777),
            None,
            false,
        );

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LFG_CREATE_START_CUSTOM_ID.to_string(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_PLACEHOLDER_TEXT));
        assert!(reply.components.is_none());
        assert!(reply.modal.is_none());
        assert!(port.forum_posts.lock().expect("forum posts").is_empty());
    }

    #[test]
    fn lfg_rank_parser_akzeptiert_nur_bekannte_grammatik() {
        assert_eq!(
            parse_rank_range("phantom"),
            Some(LfgRankRange {
                min: Some(9),
                max: Some(9)
            })
        );
        assert_eq!(
            parse_rank_range("oracle  bis  phantom"),
            Some(LfgRankRange {
                min: Some(8),
                max: Some(9)
            })
        );
        assert_eq!(
            parse_rank_range("Ritualist bis Phantom"),
            Some(LfgRankRange {
                min: Some(5),
                max: Some(9)
            })
        );
        assert_eq!(
            parse_rank_range(""),
            Some(LfgRankRange {
                min: None,
                max: None
            })
        );

        for raw in [
            "Phantom bis Kartoffel",
            "Kartoffel bis Ritualist",
            "PHANTOM  bis  oracle",
            "Phantom-Oracle",
            "Müll",
            "Phantom Ritualist",
        ] {
            assert_eq!(parse_rank_range(raw), None, "raw={raw}");
        }
    }

    #[tokio::test]
    async fn lfg_ranked_mode_ohne_rankrolle_blockt_ephemeral() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port, Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Ranked.mode_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_PLACEHOLDER_TEXT));
        assert!(reply.modal.is_none());
    }

    #[tokio::test]
    async fn lfg_ranked_mode_mit_rankrolle_oeffnet_modal() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        *port.roles.lock().expect("roles") = vec![crate::router::VERIFIED_RANK_ROLE_IDS[0]];
        let interface = LfgPanelInterface::new(db.pool().clone(), port, Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Ranked.mode_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        let modal = reply.modal.expect("modal");
        assert_eq!(modal.custom_id, LfgMode::Ranked.modal_custom_id());
        assert_eq!(modal.title, LFG_PLACEHOLDER_TEXT);
        assert_eq!(modal.fields.len(), 2);
        assert_eq!(modal.fields[0].custom_id, LFG_FIELD_RANK_RANGE);
        assert_eq!(modal.fields[1].custom_id, LFG_FIELD_REQUESTED_SLOTS);
    }

    #[tokio::test]
    async fn lfg_modal_validiert_slots_und_postet_nicht_bei_muell() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port.clone(), Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Casual.modal_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                options: HashMap::from([
                    (
                        LFG_FIELD_RANK_RANGE.to_string(),
                        json!("Ritualist bis Phantom"),
                    ),
                    (LFG_FIELD_REQUESTED_SLOTS.to_string(), json!("x")),
                ]),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_PLACEHOLDER_TEXT));
        assert!(port.forum_posts.lock().expect("forum posts").is_empty());
    }

    #[tokio::test]
    async fn lfg_modal_erstellt_forum_post_und_persistiert_row() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9901;
        *port.first_message_id.lock().expect("first message") = Ok(Some(9902));
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Casual.modal_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 42,
                options: HashMap::from([
                    (
                        LFG_FIELD_RANK_RANGE.to_string(),
                        json!("Ritualist bis Phantom"),
                    ),
                    (LFG_FIELD_REQUESTED_SLOTS.to_string(), json!("3")),
                ]),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_PLACEHOLDER_TEXT));
        let (posted_channel_id, posted_title) = {
            let posts = port.forum_posts.lock().expect("forum posts");
            assert_eq!(posts.len(), 1);
            (posts[0].0, posts[0].1.title.clone())
        };
        assert_eq!(posted_channel_id, 777);
        assert!(posted_title.contains("casual"));
        assert!(posted_title.contains("Ritualist bis Phantom"));

        type LfgPostRow = (
            i64,
            i64,
            i64,
            Option<i64>,
            i64,
            String,
            Option<i32>,
            Option<i32>,
            i32,
            String,
        );
        let row: LfgPostRow = sqlx::query_as(
            "SELECT guild_id,
                        forum_channel_id,
                        thread_id,
                        starter_message_id,
                        owner_id,
                        mode,
                        rank_min,
                        rank_max,
                        requested_slots,
                        status
                   FROM voice.lfg_posts
                  WHERE thread_id = $1",
        )
        .bind(9901_i64)
        .fetch_one(&pool)
        .await
        .expect("lfg row");
        assert_eq!(row.0, i64::try_from(LFG_GUILD_ID).expect("guild id"));
        assert_eq!(row.1, 777);
        assert_eq!(row.2, 9901);
        assert_eq!(row.3, Some(9902));
        assert_eq!(row.4, 42);
        assert_eq!(row.5, "casual");
        assert_eq!(row.6, Some(5));
        assert_eq!(row.7, Some(9));
        assert_eq!(row.8, 3);
        assert_eq!(row.9, "open");

        let expires_in_hours: f64 = sqlx::query_scalar(
            "SELECT EXTRACT(EPOCH FROM (expires_at - created_at))::DOUBLE PRECISION / 3600.0
               FROM voice.lfg_posts
              WHERE thread_id = $1",
        )
        .bind(9901_i64)
        .fetch_one(&pool)
        .await
        .expect("expires diff");
        assert!((expires_in_hours - LFG_EXPIRY_HOURS as f64).abs() < 0.01);
    }

    #[tokio::test]
    async fn lfg_modal_blockt_zweiten_offenen_post_des_owners() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9911;
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));
        let interaction = || BridgeInteraction {
            custom_id: LfgMode::Casual.modal_custom_id(),
            guild_id: LFG_GUILD_ID,
            user_id: 44,
            options: HashMap::from([
                (
                    LFG_FIELD_RANK_RANGE.to_string(),
                    json!("Ritualist bis Phantom"),
                ),
                (LFG_FIELD_REQUESTED_SLOTS.to_string(), json!("2")),
            ]),
            ..BridgeInteraction::default()
        };

        let first = interface.handle(interaction()).await;
        assert!(first.ephemeral);
        *port.next_thread_id.lock().expect("next thread") = 9912;
        let second = interface.handle(interaction()).await;

        assert!(second.ephemeral);
        assert_eq!(second.content.as_deref(), Some(LFG_PLACEHOLDER_TEXT));
        assert_eq!(port.forum_posts.lock().expect("forum posts").len(), 1);
        let active_count: i64 = sqlx::query_scalar(
            "SELECT count(*)
               FROM voice.lfg_posts
              WHERE owner_id = $1
                AND status IN ('creating', 'open')",
        )
        .bind(44_i64)
        .fetch_one(&pool)
        .await
        .expect("active count");
        assert_eq!(active_count, 1);
    }

    #[tokio::test]
    async fn lfg_modal_loescht_reservation_wenn_discord_post_fehlschlaegt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.create_forum_post_error.lock().expect("create error") = Some("HTTP 500".to_string());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Casual.modal_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 45,
                options: HashMap::from([
                    (LFG_FIELD_RANK_RANGE.to_string(), json!("")),
                    (LFG_FIELD_REQUESTED_SLOTS.to_string(), json!("1")),
                ]),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert!(port.forum_posts.lock().expect("forum posts").is_empty());
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voice.lfg_posts WHERE owner_id = $1")
                .bind(45_i64)
                .fetch_one(&pool)
                .await
                .expect("row count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn lfg_modal_archiviert_thread_wenn_open_update_fehlschlaegt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        sqlx::query(
            "INSERT INTO voice.lfg_posts (
                 guild_id,
                 forum_channel_id,
                 thread_id,
                 starter_message_id,
                 lane_id,
                 owner_id,
                 mode,
                 rank_min,
                 rank_max,
                 requested_slots,
                 status,
                 created_at,
                 updated_at,
                 expires_at,
                 closed_at,
                 last_render_hash,
                 last_post_edit_at
             )
             VALUES (
                 $1, 777, 9913, NULL, NULL, 999, 'casual', NULL, NULL, 1, 'closed',
                 now(), now(), now() + INTERVAL '24 hours', now(), NULL, NULL
             )",
        )
        .bind(i64::try_from(LFG_GUILD_ID).expect("guild id"))
        .execute(&pool)
        .await
        .expect("existing thread row");
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9913;
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::Casual.modal_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 46,
                options: HashMap::from([
                    (LFG_FIELD_RANK_RANGE.to_string(), json!("")),
                    (LFG_FIELD_REQUESTED_SLOTS.to_string(), json!("1")),
                ]),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            port.archived_threads.lock().expect("archives").as_slice(),
            &[9913]
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voice.lfg_posts WHERE owner_id = $1")
                .bind(46_i64)
                .fetch_one(&pool)
                .await
                .expect("row count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn lfg_modal_persistiert_null_wenn_starter_fetch_fehlschlaegt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.next_thread_id.lock().expect("next thread") = 9903;
        *port.first_message_id.lock().expect("first message") = Err("HTTP 500".to_string());
        let interface = LfgPanelInterface::new(pool.clone(), port, Some(777));

        let reply = interface
            .handle(BridgeInteraction {
                custom_id: LfgMode::StreetBrawl.modal_custom_id(),
                guild_id: LFG_GUILD_ID,
                user_id: 43,
                options: HashMap::from([
                    (LFG_FIELD_RANK_RANGE.to_string(), json!("")),
                    (LFG_FIELD_REQUESTED_SLOTS.to_string(), json!("1")),
                ]),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        let starter_message_id: Option<i64> = sqlx::query_scalar(
            "SELECT starter_message_id FROM voice.lfg_posts WHERE thread_id = $1",
        )
        .bind(9903_i64)
        .fetch_one(&pool)
        .await
        .expect("starter_message_id");
        assert_eq!(starter_message_id, None);
    }

    type MockPost = (u64, Map<String, Value>, Vec<LfgPanelAttachment>);
    type MockEdit = (u64, u64, Map<String, Value>, Vec<LfgPanelAttachment>);

    struct MockLfgPanelPort {
        posts: StdMutex<Vec<MockPost>>,
        edits: StdMutex<Vec<MockEdit>>,
        not_found_edits: StdMutex<Vec<u64>>,
        recent: StdMutex<Vec<LfgPanelMessage>>,
        roles: StdMutex<Vec<u64>>,
        forum_posts: StdMutex<Vec<(u64, LfgForumPostDraft)>>,
        next_thread_id: StdMutex<u64>,
        first_message_id: StdMutex<Result<Option<u64>, String>>,
        create_forum_post_error: StdMutex<Option<String>>,
        archived_threads: StdMutex<Vec<u64>>,
        archive_thread_error: StdMutex<Option<String>>,
    }

    impl Default for MockLfgPanelPort {
        fn default() -> Self {
            Self {
                posts: StdMutex::default(),
                edits: StdMutex::default(),
                not_found_edits: StdMutex::default(),
                recent: StdMutex::default(),
                roles: StdMutex::default(),
                forum_posts: StdMutex::default(),
                next_thread_id: StdMutex::new(910_001),
                first_message_id: StdMutex::new(Ok(None)),
                create_forum_post_error: StdMutex::default(),
                archived_threads: StdMutex::default(),
                archive_thread_error: StdMutex::default(),
            }
        }
    }

    #[async_trait::async_trait]
    impl LfgPanelPort for MockLfgPanelPort {
        async fn post_rich(
            &self,
            channel_id: u64,
            body: Map<String, Value>,
            attachments: &[LfgPanelAttachment],
        ) -> Result<u64, String> {
            self.posts
                .lock()
                .expect("posts")
                .push((channel_id, body, attachments.to_vec()));
            Ok(9100 + self.posts.lock().expect("posts").len() as u64)
        }

        async fn edit_rich(
            &self,
            channel_id: u64,
            message_id: u64,
            body: Map<String, Value>,
            attachments: &[LfgPanelAttachment],
        ) -> Result<(), String> {
            if self
                .not_found_edits
                .lock()
                .expect("not found")
                .contains(&message_id)
            {
                return Err("Discord PATCH fehlgeschlagen: HTTP 404: Unknown Message".to_string());
            }
            self.edits.lock().expect("edits").push((
                channel_id,
                message_id,
                body,
                attachments.to_vec(),
            ));
            Ok(())
        }

        async fn recent_bot_messages(
            &self,
            _channel_id: u64,
            _limit: u8,
        ) -> Result<Vec<LfgPanelMessage>, String> {
            Ok(self.recent.lock().expect("recent").clone())
        }

        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            self.roles.lock().expect("roles").clone()
        }

        async fn create_forum_post(
            &self,
            forum_channel_id: u64,
            draft: LfgForumPostDraft,
        ) -> Result<LfgCreatedForumPost, String> {
            if let Some(err) = self
                .create_forum_post_error
                .lock()
                .expect("create error")
                .clone()
            {
                return Err(err);
            }
            self.forum_posts
                .lock()
                .expect("forum posts")
                .push((forum_channel_id, draft));
            Ok(LfgCreatedForumPost {
                thread_id: *self.next_thread_id.lock().expect("next thread"),
            })
        }

        async fn first_thread_message_id(&self, _thread_id: u64) -> Result<Option<u64>, String> {
            self.first_message_id.lock().expect("first message").clone()
        }

        async fn archive_and_lock_thread(&self, thread_id: u64) -> Result<(), String> {
            if let Some(err) = self
                .archive_thread_error
                .lock()
                .expect("archive error")
                .clone()
            {
                return Err(err);
            }
            self.archived_threads
                .lock()
                .expect("archives")
                .push(thread_id);
            Ok(())
        }
    }
}
